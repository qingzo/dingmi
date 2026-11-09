//! 多设备同步：配对、同步密码、电脑之间同步记录、取走手机发来的东西。设计见 docs/方案/同步方案.md。
//!
//! 同步方式一台电脑只有一种：自建服务器（kv 的 "sync"）或者网盘文件夹（kv 的 "sync_folder"），开一种时会先收掉另一种。
//!
//! 两个后台任务：
//! - 接收：服务器方式是一条长连接。连上后先拉一遍别的电脑改过的、取走收件箱，然后一直发 Wait 等着，
//!   有新东西服务器马上回 Changed，25 秒没有就回 Pong（兼做心跳）。断了按 1s、2s、4s… 最多 60s 重连。
//!   网盘文件夹方式每 10 秒看一遍文件夹（见 folder.rs）。
//! - 上传：本机数据一变（`poke`），等 2 秒没有新的改动，再把改过的传上去（另开一个短连接，或者写进文件夹）。
//!
//! 另外可以开一份「备份」（backup.rs），只往另一个文件夹里写，和同步方式互不影响。

pub mod backup;
mod crypto;
mod engine;
mod folder;
mod inbox;
mod official;
mod shrink;
pub(crate) mod records;
mod secret;
pub mod transport;

use crate::{db, notify_data, notify_queue, AppState};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use dingmi_sync::{refuse, Account, BetaInfo, Hello, Join, JoinLock, Msg, PairCode, Pc, Phone, PhoneAsk, PhoneJoin, Space, Welcome, VERSION};
use secret::{DATA_KEY, DEVICE_KEY, SPACE_KEY};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{watch, Notify};
use tokio::time::timeout;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 先试域名时等多久：连不上就赶紧改直连，别让人干等
const WSS_TIMEOUT: Duration = Duration::from_secs(6);
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);
/// 服务器最多等 25 秒回 Wait，这里多留一些
const WAIT_TIMEOUT: Duration = Duration::from_secs(40);
const MAX_BACKOFF: u64 = 60;
/// 本机改了数据后，等这么久没有新的改动再上传（笔记自动保存时一直在改）
const PUSH_DEBOUNCE: Duration = Duration::from_secs(2);
/// 就算没人叫，也隔这么久看一眼有没有没传上去的
const PUSH_EVERY: Duration = Duration::from_secs(60);
/// 网盘文件夹方式：隔这么久看一遍文件夹里有没有别的电脑新写的
const FOLDER_EVERY: Duration = Duration::from_secs(10);
/// 版本太旧、同步停着时，隔这么久看一眼更新了没有（检查更新会改 too_old）
const OUTDATED_EVERY: Duration = Duration::from_secs(60);
/// 网盘文件夹方式：隔这么久更新一下「这台电脑最后出现的时间」
const HEARTBEAT_EVERY: Duration = Duration::from_secs(5 * 60);
/// 停止或者换同步方式前，最后拉一遍的时间上限
const LAST_PULL: Duration = Duration::from_secs(15);

const PAIR_REJECTED: &str = "服务器没有接受这台电脑：配对码可能已经用过或者过期了。在已经配对的电脑上「设置 → 同步」里重新生成一个，或者在服务器上执行 dingmi-server invite";
const NO_REPLY: &str = "服务器没有回应";
/// 这台叮咪要的服务器最低协议版本（需求-0.4 第 15.5 节）。电脑改了要服务器也跟着改的消息时把它调高，
/// 自建服务器没跟着升级的用户就会看到「服务器版本太旧」，而不是莫名其妙的出错
const MIN_SERVER: u32 = 4;
const SESSION_REJECTED: &str = "服务器不再接受这台电脑，可能已经被作废了，需要重新配对";
const NEED_RECOVERY: &str = "这个配对码是在服务器上生成的，不带空间密钥：请输入恢复码。手边有已经同步的电脑的话，也可以在那台电脑上生成配对码";

/// 网盘文件夹同步的设置，存在 kv 表的 "sync_folder"；主密钥在 Windows 凭据管理器里
#[derive(Serialize, Deserialize, Clone)]
struct FolderCfg {
    /// 「叮咪同步」文件夹的完整路径
    dir: String,
    /// 本机的设备名
    name: String,
}

/// 服务器配对信息里不需要保密的部分，存在 kv 表的 "sync"；密钥在 Windows 凭据管理器里
#[derive(Serialize, Deserialize, Clone)]
struct Config {
    host: String,
    port: u16,
    /// 服务器的 Noise 公钥（base64url）
    server_public: String,
    /// 本机的设备名
    name: String,
    /// 服务器绑的域名网址（Welcome.web_url，比如 `https://sync.example.com`）；有的话先走 `wss://域名/sync`。
    /// 每次连上都按服务器说的更新，没绑是空的
    #[serde(default)]
    web: String,
}

#[derive(Serialize, Clone, Default)]
pub struct Status {
    /// off 没开启 / locked 配对过但还差同步密码 / connecting 正在连 / online 已连接 / offline 连不上 /
    /// outdated 这个版本太旧（官网的 min_version），同步先停下，等更新 /
    /// expired 叮咪同步已到期（停止同步，续费后接着同步；详情在 account）
    pub state: String,
    /// 连的是叮咪同步（sync.dingmi.app）
    pub official: bool,
    /// 上次连上时服务器说的这个空间的账：套餐、到期、容量、设备数（协议 5 起；旧服务器、没连上过是 None）
    pub account: Option<Account>,
    /// 空间满了没传上去、先只在这台电脑上的图片和文件有几个（需求-1.0 第 17.3 节）；0 是都传上去了
    pub held_files: u32,
    /// 超过叮咪同步套餐单个文件的大小（图片压不下来也算）、不传、只在这台电脑上的有几个（32.1 第 5 条）
    pub only_here_files: u32,
    /// locked 时：空间还没设同步密码（刚用兑换码开通），要设一个（输两次），syncUnlock 会回新的恢复码
    pub need_setup: bool,
    /// 服务器不收这台电脑的原因（dingmi_sync::refuse 里的 code），比如 deleted：叮咪同步到期太久、数据删了，要重新开通
    pub refused: Option<String>,
    /// locked 时：这台电脑没有空间密钥，要输恢复码（输同步密码没用）
    pub need_recovery: bool,
    /// server 自建服务器 / folder 网盘文件夹；没开启时是空的
    pub kind: String,
    pub server: Option<String>,
    /// 网盘文件夹方式：「叮咪同步」文件夹的位置
    pub folder: Option<String>,
    pub device: Option<String>,
    pub error: Option<String>,
    pub last_online: Option<String>,
    /// 设备名输入框的默认值
    pub default_name: String,
}

pub struct SyncState {
    status: Mutex<Status>,
    /// 配对、解锁、停止之后加一，让后台连接按新的配置重来
    restart: watch::Sender<u64>,
    /// 本机数据变了，叫上传任务起来
    poke: Notify,
    /// 用手机加入：这次的临时钥匙、数字，手机允许后拿到的配对信息（只在内存里）
    phone: Mutex<Option<PhoneState>>,
}

struct PhoneState {
    secret: [u8; 32],
    public: [u8; 32],
    code: String,
    pair: Option<PairCode>,
    /// 已经凭手机给的邀请码登记好了（邀请码用掉了），只是同步密码输错了：再输时只解锁，不再配对
    registered: bool,
}

impl SyncState {
    pub fn new() -> Self {
        SyncState {
            status: Mutex::new(Status { state: "off".into(), default_name: default_name(), ..Default::default() }),
            restart: watch::channel(0).0,
            poke: Notify::new(),
            phone: Mutex::new(None),
        }
    }

    /// 本机数据变了（有没有要上传的由上传任务自己看）
    pub fn poke(&self) {
        self.poke.notify_one();
    }
}

fn default_name() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "我的电脑".into())
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn load_config(app: &AppHandle) -> Option<Config> {
    let st = app.state::<AppState>();
    let text = db::kv_get(&st.db.lock().unwrap(), "sync")?;
    serde_json::from_str(&text).ok()
}

fn save_config(app: &AppHandle, cfg: &Config) -> Result<(), String> {
    let st = app.state::<AppState>();
    let text = serde_json::to_string(cfg).map_err(err)?;
    let c = st.db.lock().unwrap();
    db::kv_set(&c, "sync", &text).map_err(err)
}

fn load_folder(app: &AppHandle) -> Option<FolderCfg> {
    let st = app.state::<AppState>();
    let text = db::kv_get(&st.db.lock().unwrap(), "sync_folder")?;
    serde_json::from_str(&text).ok()
}

fn publish(app: &AppHandle, state: &str, cfg: Option<&Config>, error: Option<String>) {
    let shown = cfg.map(|c| ("server", format!("{}:{}", c.host, c.port), c.name.clone()));
    publish_as(app, state, shown, error);
}

fn publish_folder(app: &AppHandle, state: &str, cfg: &FolderCfg, error: Option<String>) {
    publish_as(app, state, Some(("folder", cfg.dir.clone(), cfg.name.clone())), error);
}

/// shown：(方式, 服务器地址或文件夹位置, 设备名)；没开启同步时是 None
fn publish_as(app: &AppHandle, state: &str, shown: Option<(&str, String, String)>, error: Option<String>) {
    let st = app.state::<AppState>();
    let fresh = db::kv_get(&st.db.lock().unwrap(), FRESH_KEY).is_some();
    let snapshot = {
        let mut s = st.sync.status.lock().unwrap();
        // 状态或出错原因变了才记日志：网盘文件夹每 10 秒看一遍、连不上服务器时每分钟重连一次，不能每次都记；
        // 「正在连」不记，连上了、连不上了才记。服务器地址、文件夹位置不记
        static LOGGED: Mutex<Option<(String, Option<String>)>> = Mutex::new(None);
        let now = (state.to_string(), error.clone());
        let mut logged = LOGGED.lock().unwrap();
        if state != "connecting" && logged.as_ref() != Some(&now) {
            let mut why = error.clone().unwrap_or_default();
            if let Some((_, place, _)) = shown.as_ref().filter(|x| !x.1.is_empty()) {
                why = why.replace(place.as_str(), "<同步的地方>");
            }
            crate::diag::log("同步", format!("{state}{}", if why.is_empty() { String::new() } else { format!("：{why}") }));
            *logged = Some(now);
        }
        drop(logged);
        s.state = state.into();
        let kind = shown.as_ref().map(|x| x.0).unwrap_or_default();
        s.kind = kind.into();
        s.server = shown.as_ref().filter(|x| x.0 == "server").map(|x| x.1.clone());
        s.folder = shown.as_ref().filter(|x| x.0 == "folder").map(|x| x.1.clone());
        s.device = shown.as_ref().map(|x| x.2.clone());
        s.error = error;
        s.need_setup = state == "locked" && kind == "server" && fresh;
        s.need_recovery = state == "locked" && kind == "server" && !fresh && secret::get(SPACE_KEY).is_none();
        s.official = shown
            .as_ref()
            .is_some_and(|x| x.0 == "server" && official::is_official(x.1.rsplit_once(':').map_or(x.1.as_str(), |h| h.0)));
        if state == "online" {
            s.last_online = Some(db::now_sec());
            s.refused = None;
        }
        if shown.is_none() || kind != "server" {
            s.account = None;
            s.held_files = 0;
            s.only_here_files = 0;
            s.refused = None;
        }
        if shown.is_none() {
            s.last_online = None;
        }
        s.clone()
    };
    // 上手清单：开了同步
    if snapshot.state != "off" {
        crate::onboard::sync_on(app);
    }
    let _ = app.emit("sync-status", snapshot);
    crate::update_tray(app);
}

/// 服务器说了点关于这个空间的新东西（Welcome、AccountInfo、上传完）：记下来，变了就告诉界面。
/// `account`：None 是不改；`held`：(满了先留下的, 太大不传的)，None 是不改
fn note_account(app: &AppHandle, account: Option<Option<Account>>, held: Option<(u32, u32)>, refused: Option<Option<String>>) {
    let st = app.state::<AppState>();
    let snapshot = {
        let mut s = st.sync.status.lock().unwrap();
        let before = (s.account.clone(), s.held_files, s.only_here_files, s.refused.clone());
        if let Some(a) = account {
            s.account = a;
        }
        if let Some((h, o)) = held {
            s.held_files = h;
            s.only_here_files = o;
        }
        if let Some(r) = refused {
            s.refused = r;
        }
        if before == (s.account.clone(), s.held_files, s.only_here_files, s.refused.clone()) {
            return;
        }
        s.clone()
    };
    let _ = app.emit("sync-status", snapshot);
}

/// 刚用兑换码开通、空间还没设同步密码（kv 里记一笔，设好了删掉）
const FRESH_KEY: &str = "sync_fresh";

/// 服务器不收（协议 5 起 Welcome.refused）：变成 Err，原话给人看
fn check_refused(welcome: &Welcome) -> Result<(), String> {
    match &welcome.refused {
        Some(r) => Err(r.msg.clone()),
        None => Ok(()),
    }
}

/// 叮咪同步到期了（上次连上时服务器说的）
fn expired_now(app: &AppHandle) -> bool {
    app.state::<AppState>().sync.status.lock().unwrap().account.as_ref().is_some_and(|a| a.expired)
}

/// 托盘上显示的一行同步状态；没开启时是 None
pub fn tray_line(app: &AppHandle) -> Option<String> {
    let s = app.state::<AppState>().sync.status.lock().unwrap().clone();
    let text = match s.state.as_str() {
        "off" => return None,
        "locked" => "同步：还差同步密码".to_string(),
        "outdated" => "同步：请先更新叮咪".to_string(),
        "expired" => "同步：叮咪同步已到期".to_string(),
        "keys_lost" => "同步：要重新加入".to_string(),
        "keys_retry" => "同步：正在重试读钥匙".to_string(),
        "connecting" => "同步：正在连接…".to_string(),
        // 叮咪同步快到期（7 天内、明天）：托盘上也提一句（需求-1.0 第 18.4 节）
        "online" => match s.account.as_ref().filter(|a| a.remind == "expire_7d" || a.remind == "expire_1d").and_then(|a| a.days_left) {
            Some(d) if d <= 1 => "同步：叮咪同步明天到期".to_string(),
            Some(d) => format!("同步：叮咪同步还有 {d} 天到期"),
            None => "同步：已连接".to_string(),
        },
        _ if s.kind == "folder" => "同步：找不到同步文件夹".to_string(),
        _ => match s.last_online {
            Some(t) => format!("同步：连不上服务器（上次 {}）", &t[5..16]),
            None => "同步：连不上服务器".to_string(),
        },
    };
    Some(text)
}

/// 同步出问题了、要用户处理（托盘图标上的小红点）：还差同步密码（比如别的电脑换了密码）、版本太旧、找不到同步文件夹。
/// 连不上服务器多半是一时没网，不算
pub fn has_problem(app: &AppHandle) -> bool {
    let s = app.state::<AppState>().sync.status.lock().unwrap().clone();
    matches!(s.state.as_str(), "locked" | "outdated" | "expired" | "keys_lost") || (s.state == "offline" && s.kind == "folder")
}

/// 让后台同步按现在的设置重来一遍（配对、解锁、停止之后，或者版本是不是太旧变了）
pub fn restart(app: &AppHandle) {
    app.state::<AppState>().sync.restart.send_modify(|n| *n += 1);
}

#[allow(clippy::too_many_arguments)]
async fn dial(
    host: &str,
    port: u16,
    web: &str,
    device_private: &[u8],
    server_public: &[u8],
    name: &str,
    invite: Option<String>,
    recovery: Option<String>,
    rejected: &str,
) -> Result<(transport::Conn, Welcome), String> {
    let hello = Hello { version: VERSION, name: name.into(), invite, recovery, ..Default::default() };
    dial_hello(host, port, web, device_private, server_public, &hello, rejected).await
}

/// dial 的本体，Hello 由调的地方给（用兑换码开通时带 redeem）
async fn dial_hello(
    host: &str,
    port: u16,
    web: &str,
    device_private: &[u8],
    server_public: &[u8],
    hello: &Hello,
    rejected: &str,
) -> Result<(transport::Conn, Welcome), String> {
    // 地址本身是域名（叮咪同步）：只走 wss://域名/sync，不退回直连（官方服务器不对外开端口）
    let tcp = if let Some(only) = domain_web(host, port) {
        transport::wss(&only, CONNECT_TIMEOUT).await?
    } else {
        // 自建服务器（地址是 IP）：绑了域名先走 wss://域名/sync；通道建不起来（域名、证书、反向代理没转 WebSocket）再直连 IP:端口。
        // 通道通了、服务器不认这台电脑的，照常报错，不再试直连
        let mut via = None;
        if transport::web_host(web).is_some() {
            match transport::wss(web, WSS_TIMEOUT).await {
                Ok(s) => via = Some(s),
                Err(e) => crate::diag::log("同步", format!("走域名没连上，改直连 {host}:{port}：{e}")),
            }
        }
        match via {
            Some(s) => s,
            None => transport::direct(host, port, CONNECT_TIMEOUT).await?,
        }
    };
    let (conn, welcome) = timeout(CONNECT_TIMEOUT, dingmi_sync::connect(tcp, device_private, server_public, hello))
        .await
        .map_err(|_| "服务器没有回应".to_string())?
        .map_err(|_| rejected.to_string())?;
    server_new_enough(welcome.version)?;
    Ok((conn, welcome))
}

/// 服务器地址是域名时（叮咪同步：配对码、配置里是 `sync.dingmi.app:443`）要走的 `https://域名`，只走 wss、不直连；
/// 地址是 IP（自建服务器，init 时写的 IP）是 None，照旧直连、绑了域名先试 wss。
/// 以前存的叮咪同步是 `sync.dingmi.app:7788`（那时直连 7788），也当 443
fn domain_web(host: &str, port: u16) -> Option<String> {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']');
    if h.is_empty() || h.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    if port == 443 || official::is_official(h) {
        Some(format!("https://{}", h.to_ascii_lowercase()))
    } else {
        Some(format!("https://{}:{port}", h.to_ascii_lowercase()))
    }
}

/// 服务器说的域名网址（没绑或者不是 https 的当没有）
fn web_of(welcome: &Welcome) -> String {
    let web = welcome.web_url.trim().trim_end_matches('/');
    if transport::web_host(web).is_some() { web.to_string() } else { String::new() }
}

/// 服务器的域名变了（刚绑上、换了、去掉了）就记下来，下次按新的连
fn remember_web(app: &AppHandle, cfg: &Config, welcome: &Welcome) {
    let web = web_of(welcome);
    if web != cfg.web {
        let mut next = cfg.clone();
        next.web = web;
        let _ = save_config(app, &next);
    }
}

/// 服务器太旧：先别同步，说清楚要升级服务器（升级顺序是先升服务器、再升电脑）
fn server_new_enough(version: u32) -> Result<(), String> {
    if version >= MIN_SERVER {
        return Ok(());
    }
    crate::diag::log("同步", format!("服务器版本太旧：协议第 {version} 版，这台叮咪要第 {MIN_SERVER} 版以上"));
    Err("服务器版本太旧，请先升级服务器：换成新的 dingmi-server 再重启（自建服务器的升级方法见使用文档《服务器部署》）".into())
}

/// 拿到的钥匙：主密钥、空间密钥，第一台电脑建空间时还有新的恢复码（给用户看的写法）
struct Unlocked {
    master: crypto::DataKey,
    space_key: Option<crypto::SpaceKey>,
    new_recovery: Option<String>,
}

/// 拿到主密钥：
/// - 空间还没设过（第一台电脑）：生成主密钥、空间密钥和恢复码，建好空间；
/// - 输了恢复码：打开盒子 2，主密钥和空间密钥都在里面，不用同步密码；
/// - 否则：用同步密码和空间密钥（来自配对码）打开盒子 1
///
/// `joins`：服务器是叮咪同步（见 [`joins_on`]）。只有叮咪同步才传盒子 3：自建服务器用不了兑换码加入，传上去只会让
/// 偷到服务器数据库的人能离线猜同步密码
async fn unlock(
    conn: &mut transport::Conn,
    space: &Space,
    password: String,
    space_key: Option<crypto::SpaceKey>,
    recovery: Option<crypto::RecoveryCode>,
    joins: bool,
) -> Result<Unlocked, String> {
    let Some(salt) = space.salt.clone() else {
        check_password(&password)?;
        // 同步密码够长、不太常见：一起传盒子 3，开着兑换码加入（需求-1.x 第 34.2 节）
        let (ns, join) = tokio::task::spawn_blocking(move || -> Result<_, String> {
            let ns = crypto::new_space(&password, true)?;
            let join = match &ns.space_key {
                Some(k) if joins && crypto::join_allowed(&password) => Some(join_lock(&password, &ns.salt, k)?),
                _ => None,
            };
            Ok((ns, join))
        })
        .await
        .map_err(err)??;
        let setup = Msg::SetupSpace {
            salt: ns.salt,
            box1: ns.box1,
            box2: ns.box2,
            recovery: crypto::recovery_token(&ns.recovery),
            join,
        };
        call(conn, &setup).await?;
        return Ok(Unlocked {
            master: ns.master,
            space_key: ns.space_key,
            new_recovery: Some(crypto::recovery_text(&ns.recovery)),
        });
    };
    if let Some(code) = recovery {
        let (master, space_key) = crypto::open_box2(&code, space.box2.as_deref().unwrap_or_default())?;
        return Ok(Unlocked { master, space_key, new_recovery: None });
    }
    let space_key = space_key.ok_or(NEED_RECOVERY)?;
    let box1 = space.box1.clone().unwrap_or_default();
    let master = tokio::task::spawn_blocking(move || crypto::unlock(&password, &salt, &box1, Some(&space_key)))
        .await
        .map_err(err)??;
    Ok(Unlocked { master, space_key: Some(space_key), new_recovery: None })
}

/// 盒子 3 和加入核对值（SetupSpace、SetLock、JoinSet 里带）
fn join_lock(password: &str, salt: &str, space_key: &crypto::SpaceKey) -> Result<JoinLock, String> {
    let (box3, proof) = crypto::join_lock(password, salt, space_key)?;
    Ok(JoinLock { box3, proof })
}

/// 服务器是叮咪同步（开了兑换码）：才有兑换码加入，才传盒子 3
fn joins_on(welcome: &Welcome) -> bool {
    welcome.account.as_ref().is_some_and(|a| a.official)
}

/// 换同步密码时：新的 salt、盒子 1，叮咪同步、密码够长时再加盒子 3（服务器上用户关掉了的不收，见 Store::set_lock）
fn relock(password: &str, master: &crypto::DataKey, space_key: Option<&crypto::SpaceKey>, joins: bool) -> Result<Msg, String> {
    let (salt, box1) = crypto::lock(password, master, space_key)?;
    let join = match space_key {
        Some(k) if joins && crypto::join_allowed(password) => Some(join_lock(password, &salt, k)?),
        _ => None,
    };
    Ok(Msg::SetLock { salt, box1, join })
}

/// 把拿到的钥匙存进凭据管理器
fn keep(keys: &Unlocked) -> Result<(), String> {
    secret::set(DATA_KEY, &keys.master)?;
    if let Some(k) = &keys.space_key {
        secret::set(SPACE_KEY, k)?;
    }
    Ok(())
}

fn space_key() -> Option<crypto::SpaceKey> {
    secret::get(SPACE_KEY).and_then(|k| k.try_into().ok())
}

/// 用户输的恢复码：空的当成没输
fn recovery_input(text: Option<String>) -> Result<Option<crypto::RecoveryCode>, String> {
    match text.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(t) => crypto::parse_recovery(t).map(Some),
    }
}

fn check_password(password: &str) -> Result<(), String> {
    if password.chars().count() < 8 {
        return Err("同步密码至少 8 位".into());
    }
    Ok(())
}

// ---------- 命令 ----------

#[tauri::command]
pub fn sync_status(app: AppHandle) -> Status {
    app.state::<AppState>().sync.status.lock().unwrap().clone()
}

/// 粘贴配对码、设同步密码。第一台电脑会建好空间，回新的恢复码（要请用户存好）；之后的电脑用同一个同步密码。
/// 配对码是服务器上生成的（不带空间密钥）时，要输恢复码，这时不用同步密码。
/// replace：本机已有数据时，先备份、清空本机，再用服务器上的（只在加入已有数据的空间、密码也对时生效）；否则两边合在一起
#[tauri::command]
pub async fn sync_pair(
    app: AppHandle,
    code: String,
    password: String,
    name: String,
    replace: bool,
    recovery: Option<String>,
) -> Result<Option<String>, String> {
    let code: PairCode = code.parse().map_err(|e: &str| e.to_string())?;
    let recovery = recovery_input(recovery)?;
    if recovery.is_none() {
        check_password(&password)?;
    }
    let name = clean_name(&name);
    let (device_private, _) = dingmi_sync::generate_keypair().map_err(err)?;
    let (mut conn, welcome) = dial(
        &code.host,
        code.port,
        "",
        &device_private,
        &code.server_public,
        &name,
        Some(code.invite.clone()),
        None,
        PAIR_REJECTED,
    )
    .await?;
    // 服务器说清楚了不收（空间设备满了、到期了、停用了）
    check_refused(&welcome)?;
    // 新服务器认了这台电脑，才收掉原来的同步方式
    retire(&app).await;
    // 到这里服务器已经登记了这台电脑、邀请码也用掉了：先把配对信息和空间密钥存下来，
    // 这样同步密码输错了可以直接重试，不用再要一个配对码
    secret::set(DEVICE_KEY, &device_private)?;
    secret::delete(DATA_KEY);
    secret::delete(SPACE_KEY);
    let space_key: Option<crypto::SpaceKey> = code.space_key.and_then(|k| k.try_into().ok());
    if let Some(k) = &space_key {
        secret::set(SPACE_KEY, k)?;
    }
    let cfg = Config { host: code.host, port: code.port, server_public: B64.encode(&code.server_public), name, web: web_of(&welcome) };
    save_config(&app, &cfg)?;
    let existing = welcome.space.salt.is_some();
    let key = unlock(&mut conn, &welcome.space, password, space_key, recovery, joins_on(&welcome)).await;
    if let Ok(k) = &key {
        keep(k)?;
    }
    {
        // 新配对：从头拉取。合并时本机已有的数据都传上去，和空间里已有的合在一起
        let st = app.state::<AppState>();
        let c = st.db.lock().unwrap();
        db::kv_del(&c, "sync_seq").map_err(err)?;
        db::kv_del(&c, "sync_epoch").map_err(err)?;
        if replace && existing && key.is_ok() {
            let name = format!("替换前-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"));
            db::snapshot(&c, &st.data_dir, &name)?;
            db::wipe_synced(&c).map_err(err)?;
        } else {
            db::mark_all_dirty(&c).map_err(err)?;
        }
    }
    notify_data(&app);
    notify_queue(&app);
    restart(&app);
    key.map(|k| k.new_recovery)
}

/// 立即同步：叫上传任务马上传，接收的连接重连一次（重连后会先拉一遍）
#[tauri::command]
pub fn sync_now(app: AppHandle) {
    app.state::<AppState>().sync.poke();
    restart(&app);
}

/// 配对过、但还没有（或者丢了）主密钥时，重新输入同步密码；没有空间密钥的电脑输恢复码。
/// 空间还没设同步密码时（刚用兑换码开通，Status.need_setup）就是设同步密码：建好空间，回新的恢复码（要请用户存好）；
/// 别的时候回 None
#[tauri::command]
pub async fn sync_unlock(app: AppHandle, password: String, recovery: Option<String>) -> Result<Option<String>, String> {
    if let Some(fc) = load_folder(&app) {
        let space = folder::read_space(Path::new(&fc.dir))?;
        let key = tokio::task::spawn_blocking(move || crypto::unlock(&password, &space.salt, &space.box1, None))
            .await
            .map_err(err)??;
        secret::set(DATA_KEY, &key)?;
        restart(&app);
        return Ok(None);
    }
    unlock_paired(&app, password, recovery_input(recovery)?, false).await
}

/// 已经配对、登记好的电脑输同步密码（或恢复码）解锁。`replace` 见 sync_pair：
/// 用手机加入时同步密码第一次输错了（邀请码已经用掉），再输对时照样按用户选的替换本机的数据
async fn unlock_paired(app: &AppHandle, password: String, recovery: Option<crypto::RecoveryCode>, replace: bool) -> Result<Option<String>, String> {
    let cfg = load_config(app).ok_or("这台电脑还没有配对")?;
    let device = secret::get(DEVICE_KEY).ok_or("这台电脑的设备密钥不见了，需要停止同步后重新配对")?;
    let server_public = B64.decode(&cfg.server_public).map_err(err)?;
    let (mut conn, welcome) =
        dial(&cfg.host, cfg.port, &cfg.web, &device, &server_public, &cfg.name, None, None, SESSION_REJECTED).await?;
    check_refused(&welcome)?;
    let keys = unlock(&mut conn, &welcome.space, password, space_key(), recovery, joins_on(&welcome)).await?;
    keep(&keys)?;
    {
        let st = app.state::<AppState>();
        let c = st.db.lock().unwrap();
        let _ = db::kv_del(&c, FRESH_KEY);
        // 没有主密钥时同步一直在等，本机的还没传上去：这时清掉不会丢服务器上的
        if replace && keys.new_recovery.is_none() {
            let name = format!("替换前-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"));
            db::snapshot(&c, &st.data_dir, &name)?;
            db::wipe_synced(&c).map_err(err)?;
            db::kv_del(&c, "sync_seq").map_err(err)?;
            db::kv_del(&c, "sync_epoch").map_err(err)?;
        }
    }
    if replace {
        notify_data(app);
        notify_queue(app);
    }
    restart(app);
    Ok(keys.new_recovery)
}

// ---------- 叮咪同步（需求-1.0 第 18、30、31 节） ----------

/// 用兑换码连上叮咪同步：服务器核对兑换码、新建空间、登记这台电脑。返回 (设备私钥, 连接, Welcome)。
/// 地址和公钥由调的地方给（平时是 official.rs 里写死的，测试里是本机的服务器）
async fn redeem_dial(
    host: &str,
    port: u16,
    web: &str,
    server_public: &[u8],
    code: &str,
    name: &str,
    email: Option<String>,
) -> Result<(Vec<u8>, transport::Conn, Welcome), String> {
    let code = code.trim();
    if code.is_empty() {
        return Err("先填兑换码".into());
    }
    let hello = Hello { name: name.into(), redeem: Some(code.into()), ..open_hello(email)? };
    open_dial(host, port, web, server_public, &hello).await
}

/// 领公测免费体验（32.1 第 4 条）：服务器开着公测、有名额才成，和兑换码开通一样新建空间、登记这台电脑
async fn beta_dial(
    host: &str,
    port: u16,
    web: &str,
    server_public: &[u8],
    name: &str,
    email: Option<String>,
) -> Result<(Vec<u8>, transport::Conn, Welcome), String> {
    let hello = Hello { name: name.into(), beta: true, ..open_hello(email)? };
    open_dial(host, port, web, server_public, &hello).await
}

/// 开通叮咪同步（兑换码、公测）时的 Hello：带同意的协议版本，邮箱选填（先在这里核对格式）
fn open_hello(email: Option<String>) -> Result<Hello, String> {
    let email = email.map(|e| e.trim().to_string()).filter(|e| !e.is_empty());
    if let Some(e) = &email {
        check_email(e)?;
    }
    Ok(Hello { version: VERSION, terms: Some(crate::terms::sync_version()), email, ..Default::default() })
}

/// 开通用的连接：新生成一把设备密钥，服务器不收的话把原话报出来
async fn open_dial(host: &str, port: u16, web: &str, server_public: &[u8], hello: &Hello) -> Result<(Vec<u8>, transport::Conn, Welcome), String> {
    let (device_private, _) = dingmi_sync::generate_keypair().map_err(err)?;
    let (conn, welcome) =
        dial_hello(host, port, web, &device_private, server_public, hello, "叮咪同步的服务器没有回应，稍后再试").await?;
    if let Some(r) = welcome.refused.as_ref().filter(|r| r.code == refuse::REDEEM_JOINABLE) {
        return Err(format!("{JOINABLE_MARK}{}", r.msg));
    }
    check_refused(&welcome)?;
    Ok((device_private, conn, welcome))
}

// ---------- 新电脑方便加入（需求-1.x 第 34 节） ----------

/// 兑换码 + 同步密码加入叮咪同步：先拿兑换码换 salt，用同步密码算出核对值，服务器核对上了登记这台电脑、给盒子 3；
/// 盒子 3 打开是空间密钥，再和平时一样打开盒子 1。replace 和配对时一样（本机有数据时用服务器上的替换这台电脑）
async fn join_dial(
    host: &str,
    port: u16,
    web: &str,
    server_public: &[u8],
    code: &str,
    password: String,
    name: &str,
) -> Result<(Vec<u8>, Welcome, Unlocked), String> {
    let code = code.trim().to_string();
    if code.is_empty() {
        return Err("先填兑换码".into());
    }
    check_password(&password)?;
    let no_reply = "叮咪同步的服务器没有回应，稍后再试";
    let (tmp, _) = dingmi_sync::generate_keypair().map_err(err)?;
    let ask = Hello { version: VERSION, join_ask: Some(code.clone()), ..Default::default() };
    let (_, w) = dial_hello(host, port, web, &tmp, server_public, &ask, no_reply).await?;
    check_refused(&w)?;
    let salt = w.join_salt.ok_or("服务器版本太旧，用不了兑换码加入")?;
    let pw = password.clone();
    let (proof, pkey) = tokio::task::spawn_blocking(move || crypto::join_proof(&pw, &salt)).await.map_err(err)??;
    let (device_private, _) = dingmi_sync::generate_keypair().map_err(err)?;
    let hello = Hello { version: VERSION, name: name.into(), join: Some(Join { code, proof }), ..Default::default() };
    let (_, welcome) = dial_hello(host, port, web, &device_private, server_public, &hello, no_reply).await?;
    check_refused(&welcome)?;
    let space = welcome.space.clone();
    let k = crypto::open_box3(&pkey, space.box3.as_deref().unwrap_or_default())?;
    let (salt, box1) = (space.salt.unwrap_or_default(), space.box1.unwrap_or_default());
    let master = tokio::task::spawn_blocking(move || crypto::unlock(&password, &salt, &box1, Some(&k))).await.map_err(err)??;
    Ok((device_private, welcome, Unlocked { master, space_key: Some(k), new_recovery: None }))
}

/// 加进了一个已有的空间（兑换码加入、恢复码加入）：收掉原来的同步方式，存钥匙和配置，合并或替换本机的数据
async fn joined(app: &AppHandle, device_private: Vec<u8>, welcome: Welcome, keys: Unlocked, server_public: &[u8], name: String, replace: bool) -> Result<Account, String> {
    retire(app).await;
    secret::set(DEVICE_KEY, &device_private)?;
    keep(&keys)?;
    let web = match web_of(&welcome) {
        w if w.is_empty() => official::WEB.to_string(),
        w => w,
    };
    save_config(app, &Config { host: official::HOST.into(), port: official::PORT, server_public: B64.encode(server_public), name, web })?;
    {
        let st = app.state::<AppState>();
        let c = st.db.lock().unwrap();
        db::kv_del(&c, "sync_seq").map_err(err)?;
        db::kv_del(&c, "sync_epoch").map_err(err)?;
        if replace {
            let name = format!("替换前-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"));
            db::snapshot(&c, &st.data_dir, &name)?;
            db::wipe_synced(&c).map_err(err)?;
        } else {
            db::mark_all_dirty(&c).map_err(err)?;
        }
    }
    notify_data(app);
    notify_queue(app);
    let account = welcome.account.clone().unwrap_or_default();
    note_account(app, Some(welcome.account), Some((0, 0)), Some(None));
    restart(app);
    Ok(account)
}

/// 兑换码 + 同步密码加入叮咪同步（「叮咪同步 → 已经在别的电脑上开通过 → 兑换码」）
#[tauri::command]
pub async fn official_join(app: AppHandle, code: String, password: String, name: String, replace: bool) -> Result<Account, String> {
    let server_public = official::server_public()?;
    let name = clean_name(&name);
    let (device_private, welcome, keys) =
        join_dial(official::HOST, official::PORT, official::WEB, &server_public, &code, password, &name).await?;
    joined(&app, device_private, welcome, keys, &server_public, name, replace).await
}

#[derive(Serialize)]
pub struct PhoneWait {
    /// 8 位数字，手机上输它
    pub code: String,
    /// 什么时候失效
    pub expires_at: String,
    /// 两边核对的 4 个字
    pub words: String,
}

/// 用手机加入（需求-1.x 第 34.3 节）的第一步：生成这次的临时钥匙，向服务器要 8 位数字
#[tauri::command]
pub async fn official_phone_start(app: AppHandle, name: String) -> Result<PhoneWait, String> {
    let server_public = official::server_public()?;
    let (secret, public) = crypto::phone_keypair();
    let name = clean_name(&name);
    let (tmp, _) = dingmi_sync::generate_keypair().map_err(err)?;
    let ask = PhoneAsk { key: STANDARD.encode(public), name, code: String::new() };
    let hello = Hello { version: VERSION, phone_ask: Some(ask), ..Default::default() };
    let (_, w) = dial_hello(official::HOST, official::PORT, official::WEB, &tmp, &server_public, &hello, "叮咪同步的服务器没有回应，稍后再试").await?;
    check_refused(&w)?;
    let pj = w.phone.ok_or("服务器版本太旧，用不了手机加入")?;
    let words = crypto::phone_words(&public);
    *app.state::<AppState>().sync.phone.lock().unwrap() = Some(PhoneState { secret, public, code: pj.code.clone(), pair: None, registered: false });
    Ok(PhoneWait { code: pj.code, expires_at: pj.expires_at, words })
}

/// 等手机允许：最多等 20 秒，回 true 是手机允许了（接着 official_phone_join 输同步密码）；false 是还在等；
/// 手机点了「不是我」、数字过期了回 Err
#[tauri::command]
pub async fn official_phone_poll(app: AppHandle) -> Result<bool, String> {
    let server_public = official::server_public()?;
    let st = app.state::<AppState>();
    let (code, public, secret) = {
        let g = st.sync.phone.lock().unwrap();
        let p = g.as_ref().ok_or("先要一个数字")?;
        if p.pair.is_some() {
            return Ok(true);
        }
        (p.code.clone(), p.public, p.secret)
    };
    let (tmp, _) = dingmi_sync::generate_keypair().map_err(err)?;
    let ask = PhoneAsk { key: STANDARD.encode(public), name: String::new(), code };
    let hello = Hello { version: VERSION, phone_poll: Some(ask), ..Default::default() };
    let (_, w) = dial_hello(official::HOST, official::PORT, official::WEB, &tmp, &server_public, &hello, "叮咪同步的服务器没有回应，稍后再试").await?;
    check_refused(&w)?;
    let Some(PhoneJoin { invite: Some(invite), sealed: Some(sealed), .. }) = w.phone else {
        return Ok(false);
    };
    let k = crypto::open_phone(&secret, &STANDARD.decode(sealed).map_err(|_| "手机发来的钥匙格式不对")?)?;
    let pair = PairCode { host: official::HOST.into(), port: official::PORT, server_public, invite, space_key: Some(k.to_vec()) };
    if let Some(p) = app.state::<AppState>().sync.phone.lock().unwrap().as_mut() {
        p.pair = Some(pair);
    }
    Ok(true)
}

/// 手机允许了：输同步密码加入（和粘贴配对码一样）
#[tauri::command]
pub async fn official_phone_join(app: AppHandle, password: String, name: String, replace: bool) -> Result<(), String> {
    let (pair, registered) = {
        let st = app.state::<AppState>();
        let g = st.sync.phone.lock().unwrap();
        let p = g.as_ref().ok_or("手机还没允许")?;
        (p.pair.clone().ok_or("手机还没允许")?, p.registered)
    };
    let done = if registered {
        // 上次已经登记好了、只是同步密码不对：邀请码用掉了，这次只解锁
        unlock_paired(&app, password, None, replace).await.map(|_| ())
    } else {
        check_password(&password)?;
        let r = sync_pair(app.clone(), pair.to_string(), password, name, replace, None).await.map(|_| ());
        // 服务器已经登记了这台电脑（配置、手机给的空间密钥都存好了）才失败的，多半是同步密码不对：记下来，下次只解锁
        if r.is_err() && load_config(&app).is_some_and(|c| c.host == pair.host) && space_key().map(|k| k.to_vec()) == pair.space_key {
            if let Some(p) = app.state::<AppState>().sync.phone.lock().unwrap().as_mut() {
                p.registered = true;
            }
        }
        r
    };
    done?;
    *app.state::<AppState>().sync.phone.lock().unwrap() = None;
    Ok(())
}

/// 核对同步密码（这台电脑上有主密钥）：服务器给的 salt 和盒子 1 用它打得开、打出来是同一把主密钥
async fn check_sync_password(welcome: &Welcome, password: &str) -> Result<(crypto::DataKey, crypto::SpaceKey, String), String> {
    let master: crypto::DataKey = secret::get(DATA_KEY).and_then(|k| k.try_into().ok()).ok_or("还差同步密码，先在上面解锁")?;
    let k = space_key().ok_or("这台电脑上没有空间密钥，先用恢复码解锁")?;
    let (salt, box1, pw) = (welcome.space.salt.clone().unwrap_or_default(), welcome.space.box1.clone().unwrap_or_default(), password.to_string());
    let s2 = salt.clone();
    let got = tokio::task::spawn_blocking(move || crypto::unlock(&pw, &s2, &box1, Some(&k))).await.map_err(err)??;
    if got != master {
        return Err("同步密码不对".into());
    }
    Ok((master, k, salt))
}

/// 打开兑换码加入（这项上线前开通的空间补上、关掉以后再打开）：输一次同步密码，要 10 位以上、不太常见
#[tauri::command]
pub async fn official_join_on(app: AppHandle, password: String) -> Result<Account, String> {
    if !crypto::join_allowed(&password) {
        return Err(format!("同步密码要 {} 位以上、不太常见才能开兑换码加入：先在下面换一个长一点的同步密码", crypto::JOIN_MIN_CHARS));
    }
    let (_, _, welcome) = ask(&app, Msg::AccountGet).await?;
    let (_, k, salt) = check_sync_password(&welcome, &password).await?;
    let s2 = salt.clone();
    let join = tokio::task::spawn_blocking(move || join_lock(&password, &s2, &k)).await.map_err(err)??;
    ask(&app, Msg::JoinSet { join, salt: Some(salt) }).await?;
    official_account(app).await
}

/// 关掉兑换码加入：服务器删掉盒子 3 和核对值
#[tauri::command]
pub async fn official_join_off(app: AppHandle) -> Result<Account, String> {
    ask(&app, Msg::JoinOff).await?;
    official_account(app).await
}

/// 删掉服务器上的数据（需求-1.x 第 35.3 节）：输同步密码确认；先把还没拉到的拉下来，删完这台电脑回到「不同步」，
/// 本机数据不动
#[tauri::command]
pub async fn official_delete(app: AppHandle, password: String) -> Result<(), String> {
    let (_, _, welcome) = ask(&app, Msg::AccountGet).await?;
    let (_, _, salt) = check_sync_password(&welcome, &password).await?;
    // 服务器开着兑换码加入时拿加入核对值再核对一次（只偷到设备钥匙的人删不了）；没开的服务器不看它
    let (proof, _) = tokio::task::spawn_blocking(move || crypto::join_proof(&password, &salt)).await.map_err(err)??;
    let _ = timeout(LAST_PULL, last_pull(&app)).await;
    ask(&app, Msg::SpaceDelete { proof: Some(proof) }).await?;
    forget(&app);
    restart(&app);
    Ok(())
}

/// 问问公测开没开（开通前，32.1 第 4 条）：用一把临时的设备密钥连上去，服务器回完就断开，不登记
async fn beta_ask(host: &str, port: u16, web: &str, server_public: &[u8]) -> Result<BetaInfo, String> {
    let (device_private, _) = dingmi_sync::generate_keypair().map_err(err)?;
    let hello = Hello { version: VERSION, ask_beta: true, ..Default::default() };
    let (_, welcome) = dial_hello(host, port, web, &device_private, server_public, &hello, "叮咪同步的服务器没有回应").await?;
    Ok(welcome.beta.unwrap_or_default())
}

/// 邮箱的简单核对（和服务器一样：一个 @，后面是带点的域名，没有空格）
fn check_email(e: &str) -> Result<(), String> {
    let ok = e.len() <= 100
        && e.split_once('@').is_some_and(|(u, h)| {
            !u.is_empty() && h.contains('.') && !h.starts_with('.') && !h.ends_with('.') && !h.contains("..") && !h.contains('@')
        })
        && !e.chars().any(|c| c.is_whitespace() || c.is_control() || "<>()[],;:\"\\".contains(c));
    if ok { Ok(()) } else { Err("邮箱的格式不对，写成 name@example.com 这样".into()) }
}

/// 开通（兑换码、公测）都要先勾「同意协议」：界面上没勾点不了，这里再拦一道
fn check_agree(agree: bool) -> Result<(), String> {
    if agree { Ok(()) } else { Err("先勾选「我已阅读并同意」".into()) }
}

/// 开通、续费时带的协议版本，没取到过官网的 terms.json 时才用这个（平时用 terms.json 里的 sync_terms.version，
/// 见 terms.rs sync_version；需求-1.x 第 36.6 节）。协议在后台改、发布，不用再改这里；
/// 服务器记的是它那边现在生效的版本，这个只是「同意了」的凭据，像日期就行（SITE.md 4.5）
pub const TERMS_FALLBACK: &str = "2026-11-08";

/// 公测免费体验现在怎样（「设置 → 同步 → 叮咪同步」开通那一步用）：能不能领、还剩几个名额、到哪天、多大。
/// 连不上、旧服务器都当没开
#[tauri::command]
pub async fn official_beta_info() -> Result<BetaInfo, String> {
    let server_public = official::server_public()?;
    beta_ask(official::HOST, official::PORT, official::WEB, &server_public).await
}

/// 领公测免费体验开通叮咪同步：之后和兑换码开通一样设同步密码、存恢复码
#[tauri::command]
pub async fn official_beta(app: AppHandle, name: String, email: Option<String>, agree: bool) -> Result<Account, String> {
    check_agree(agree)?;
    let server_public = official::server_public()?;
    let name = clean_name(&name);
    let (device_private, _conn, welcome) =
        beta_dial(official::HOST, official::PORT, official::WEB, &server_public, &name, email).await?;
    opened(&app, device_private, welcome, &server_public, name).await
}

/// 用兑换码开通叮咪同步（「设置 → 同步 → 叮咪同步」）：核对兑换码（有效 / 已经用过 / 不存在会直接报出来）→
/// 服务器新建一个空间、登记这台电脑 → 收掉原来的同步方式。回这个空间的账（套餐、到哪天）。
/// 下一步和第一台电脑一样设同步密码：调 sync_unlock（输两次的那个密码），它回恢复码，要请用户存好。
/// 设备名在这里就要给（登记时用）；邮箱选填；agree 是勾了「同意协议」（没勾不开通）
#[tauri::command]
pub async fn official_redeem(app: AppHandle, code: String, name: String, email: Option<String>, agree: bool) -> Result<Account, String> {
    check_agree(agree)?;
    let server_public = official::server_public()?;
    let name = clean_name(&name);
    let (device_private, _conn, welcome) =
        redeem_dial(official::HOST, official::PORT, official::WEB, &server_public, &code, &name, email).await?;
    opened(&app, device_private, welcome, &server_public, name).await
}

/// 开通时填的码已经开过空间、那个空间能用兑换码加入（REDEEM_JOINABLE）：报错前面加上这个，界面接到「输同步密码加入」
pub const JOINABLE_MARK: &str = "joinable:";

/// 开通成了（兑换码、公测）：收掉原来的同步方式，记下配置，本机的数据全部要传上去，下一步设同步密码
async fn opened(app: &AppHandle, device_private: Vec<u8>, welcome: Welcome, server_public: &[u8], name: String) -> Result<Account, String> {
    let app = app.clone();
    retire(&app).await;
    secret::set(DEVICE_KEY, &device_private)?;
    let web = match web_of(&welcome) {
        w if w.is_empty() => official::WEB.to_string(),
        w => w,
    };
    let cfg = Config { host: official::HOST.into(), port: official::PORT, server_public: B64.encode(server_public), name, web };
    save_config(&app, &cfg)?;
    {
        // 新空间：本机已有的数据全部传上去
        let st = app.state::<AppState>();
        let c = st.db.lock().unwrap();
        db::kv_del(&c, "sync_seq").map_err(err)?;
        db::kv_set(&c, "sync_epoch", &welcome.epoch).map_err(err)?;
        db::kv_set(&c, FRESH_KEY, "1").map_err(err)?;
        db::mark_all_dirty(&c).map_err(err)?;
    }
    let account = welcome.account.clone().unwrap_or_default();
    note_account(&app, Some(welcome.account), Some((0, 0)), Some(None));
    restart(&app);
    Ok(account)
}

/// 唯一的电脑没了（30.4）：新电脑用恢复码登记进叮咪同步的空间，打开盒子 2 拿到主密钥和空间密钥。
/// password 写了就顺便换成这个同步密码（忘了原来的；别的设备下次提示输新密码），空的就不换。
/// 之后要请用户「换一个恢复码」（sync_new_recovery）
#[tauri::command]
pub async fn official_recover(app: AppHandle, recovery: String, name: String, password: String) -> Result<Account, String> {
    let code = crypto::parse_recovery(recovery.trim())?;
    let new_password = (!password.is_empty()).then_some(password);
    if let Some(p) = &new_password {
        check_password(p)?;
    }
    let server_public = official::server_public()?;
    let name = clean_name(&name);
    let (device_private, _) = dingmi_sync::generate_keypair().map_err(err)?;
    let (mut conn, welcome) = dial(
        official::HOST,
        official::PORT,
        official::WEB,
        &device_private,
        &server_public,
        &name,
        None,
        Some(crypto::recovery_token(&code)),
        "叮咪同步不认这个恢复码：看看是不是输错了，或者已经换过新的恢复码",
    )
    .await?;
    check_refused(&welcome)?;
    let joins = joins_on(&welcome);
    let keys = unlock(&mut conn, &welcome.space, String::new(), None, Some(code), joins).await?;
    if let Some(p) = new_password {
        let (m, k) = (keys.master, keys.space_key);
        let msg = tokio::task::spawn_blocking(move || relock(&p, &m, k.as_ref(), joins)).await.map_err(err)??;
        call(&mut conn, &msg).await?;
    }
    retire(&app).await;
    secret::set(DEVICE_KEY, &device_private)?;
    keep(&keys)?;
    let web = match web_of(&welcome) {
        w if w.is_empty() => official::WEB.to_string(),
        w => w,
    };
    save_config(&app, &Config { host: official::HOST.into(), port: official::PORT, server_public: B64.encode(&server_public), name, web })?;
    {
        let st = app.state::<AppState>();
        let c = st.db.lock().unwrap();
        db::kv_del(&c, "sync_seq").map_err(err)?;
        db::kv_del(&c, "sync_epoch").map_err(err)?;
        db::mark_all_dirty(&c).map_err(err)?;
    }
    notify_data(&app);
    notify_queue(&app);
    let account = welcome.account.clone().unwrap_or_default();
    note_account(&app, Some(welcome.account), Some((0, 0)), Some(None));
    restart(&app);
    Ok(account)
}

/// 续费（「设置 → 同步 → 续费」填兑换码）：同一档接在到期日后面，换大容量马上生效、剩下的接在后面。
/// 到期了（30 天内）也能续，续上就接着同步。回续好以后的账
#[tauri::command]
pub async fn official_renew(app: AppHandle, code: String) -> Result<Account, String> {
    let code = code.trim().to_string();
    if code.is_empty() {
        return Err("先填兑换码".into());
    }
    // 带上现在的协议版本，服务器更新同意的记录（32.1 第 7 条）
    let (Msg::AccountInfo { account }, ..) = ask(&app, Msg::Renew { code, terms: Some(crate::terms::sync_version()) }).await? else {
        return Err("服务器回的内容不对".into());
    };
    let was_expired = expired_now(&app);
    note_account(&app, Some(Some(account.clone())), None, None);
    if was_expired {
        restart(&app);
    }
    Ok(account)
}

#[derive(Serialize)]
pub struct SupportCode {
    /// 6 位数，给客服看
    pub code: String,
    /// 到什么时候（10 分钟）
    pub expires_at: String,
}

/// 联系客服（30.6）：向服务器要一个 6 位数的客服核对码，10 分钟有效；再要一个，旧的作废
#[tauri::command]
pub async fn official_support_code(app: AppHandle) -> Result<SupportCode, String> {
    match ask(&app, Msg::SupportNew).await? {
        (Msg::Support { code, expires_at }, ..) => Ok(SupportCode { code, expires_at }),
        _ => Err("服务器回的内容不对，可能要先升级服务器".into()),
    }
}

/// 叮咪同步留的邮箱（32.1 第 6 条）：写了是设置 / 改，空的（None 或空字符串）是删掉。回改好以后的账
#[tauri::command]
pub async fn official_email(app: AppHandle, email: Option<String>) -> Result<Account, String> {
    let email = email.map(|e| e.trim().to_string()).filter(|e| !e.is_empty());
    let msg = match email {
        Some(e) => {
            check_email(&e)?;
            Msg::EmailSet { email: e }
        }
        None => Msg::EmailDel,
    };
    let (Msg::AccountInfo { account }, ..) = ask(&app, msg).await? else {
        return Err("服务器版本太旧，留不了邮箱".into());
    };
    note_account(&app, Some(Some(account.clone())), None, None);
    Ok(account)
}

/// 现在这个空间的账（套餐、到期、容量、设备数），连上去问一次；顺便更新同步状态里的 account
#[tauri::command]
pub async fn official_account(app: AppHandle) -> Result<Account, String> {
    let (reply, _, welcome) = ask(&app, Msg::AccountGet).await?;
    let account = match reply {
        Msg::AccountInfo { account } => account,
        // 旧服务器没有这条消息，有 Welcome 里的就用它
        _ => welcome.account.ok_or("服务器版本太旧，看不到套餐和到期")?,
    };
    note_account(&app, Some(Some(account.clone())), None, None);
    Ok(account)
}

/// 换一个恢复码：旧的作废，回新的（给用户看的写法），要请用户存好
#[tauri::command]
pub async fn sync_new_recovery(app: AppHandle) -> Result<String, String> {
    let master: crypto::DataKey =
        secret::get(DATA_KEY).and_then(|k| k.try_into().ok()).ok_or("还差同步密码，先在上面解锁")?;
    let (code, box2) = crypto::new_recovery(&master, space_key().as_ref());
    ask(&app, Msg::SetRecovery { box2, recovery: crypto::recovery_token(&code) }).await?;
    Ok(crypto::recovery_text(&code))
}

/// 换同步密码：不用旧密码（这台电脑上有主密钥），只给主密钥换一把锁，数据不用重新上传；
/// 别的电脑和手机已经有主密钥，照常同步，以后新加入的电脑用新密码。开着备份的话，备份也换成新密码
#[tauri::command]
pub async fn sync_change_password(app: AppHandle, password: String) -> Result<(), String> {
    check_password(&password)?;
    let master: crypto::DataKey =
        secret::get(DATA_KEY).and_then(|k| k.try_into().ok()).ok_or("还差同步密码，先在上面解锁")?;
    let new_password = password.clone();
    if let Some(fc) = load_folder(&app) {
        let (salt, box1) = tokio::task::spawn_blocking(move || crypto::lock(&password, &master, None)).await.map_err(err)??;
        folder::write_space(Path::new(&fc.dir), &folder::SpaceFile { v: 1, salt, box1 })?;
    } else {
        let k = space_key();
        let (_, _, welcome) = ask(&app, Msg::Ping).await?;
        let joins = joins_on(&welcome);
        let msg = tokio::task::spawn_blocking(move || relock(&password, &master, k.as_ref(), joins)).await.map_err(err)??;
        ask(&app, msg).await?;
        // 账上的「兑换码加入」跟着变（新密码不够长就关了），设置页马上看得到
        let _ = official_account(app.clone()).await;
    }
    backup::relock(&app, new_password).await;
    Ok(())
}

/// 停止同步：先尽量把还没拉到的拉下来，再删掉本机的同步设置和密钥，本机数据不动。
/// 服务器上的登记要在服务器上用 revoke 作废；网盘文件夹里的东西不动
#[tauri::command]
pub async fn sync_stop(app: AppHandle) -> Result<(), String> {
    retire(&app).await;
    restart(&app);
    Ok(())
}

/// 收掉现在的同步方式（停止、换方式、开另一种之前）：最后拉一遍，再删掉设置和密钥
async fn retire(app: &AppHandle) {
    let _ = timeout(LAST_PULL, last_pull(app)).await;
    forget(app);
}

/// 删掉这台电脑上的同步设置和密钥（本机数据不动）
fn forget(app: &AppHandle) {
    secret::delete(DEVICE_KEY);
    secret::delete(DATA_KEY);
    secret::delete(SPACE_KEY);
    let st = app.state::<AppState>();
    let c = st.db.lock().unwrap();
    for k in ["sync", "sync_seq", "sync_epoch", "sync_folder", "folder_cursor", FRESH_KEY] {
        let _ = db::kv_del(&c, k);
    }
}

/// 换方式前把旧地方还没拉到的拉下来（别的电脑刚改的、手机发来的），连不上就算了
async fn last_pull(app: &AppHandle) -> Result<(), String> {
    let key: crypto::DataKey = secret::get(DATA_KEY).and_then(|k| k.try_into().ok()).ok_or("还差同步密码")?;
    let st = app.state::<AppState>();
    if let Some(fc) = load_folder(app) {
        let app2 = app.clone();
        tokio::task::spawn_blocking(move || {
            let st = app2.state::<AppState>();
            let dir = PathBuf::from(&fc.dir);
            folder::pull(&st.db, &st.data_dir, &folder::Folder { dir: &dir, me: &st.device, key: &key })
        })
        .await
        .map_err(err)??;
    } else if load_config(app).is_some() {
        let mut conn = open(app).await?;
        engine::pull(&st.db, &st.data_dir, &mut conn, &key).await?;
        inbox::fetch(app, &mut conn, &key, &mut HashSet::new()).await?;
    }
    notify_data(app);
    notify_queue(app);
    Ok(())
}

fn clean_name(name: &str) -> String {
    let name: String = name.trim().chars().take(40).collect();
    if name.is_empty() { default_name() } else { name }
}

// ---------- 网盘文件夹 ----------

/// 这台电脑上找到的网盘同步文件夹
#[derive(Serialize)]
pub struct Candidate {
    /// 网盘的名字，比如「坚果云」
    pub label: String,
    pub path: String,
    pub probe: FolderProbe,
}

/// 选中的文件夹里有没有叮咪的数据
#[derive(Serialize)]
pub struct FolderProbe {
    /// 「叮咪同步」文件夹会放在哪
    pub dir: String,
    /// empty 没有叮咪数据（第一台电脑）/ space 已经在用（加入）/ bad 用不了，原因在 error
    pub state: String,
    pub devices: usize,
    pub error: Option<String>,
}

/// 用户选的文件夹 → 「叮咪数据」文件夹：选的就是它本身时直接用。
/// 以前叫「叮咪同步」：选的是旧名字的、或者里面已经有旧名字的（别的电脑、以前建的）就接着用它，不另建新的
fn sync_dir(path: &str) -> PathBuf {
    let p = PathBuf::from(path.trim());
    if p.file_name().is_some_and(|n| n == folder::DIR_NAME || n == folder::OLD_DIR_NAME) {
        return p;
    }
    let old = p.join(folder::OLD_DIR_NAME);
    if old.is_dir() && !p.join(folder::DIR_NAME).exists() {
        return old;
    }
    p.join(folder::DIR_NAME)
}

fn probe_dir(path: &str) -> FolderProbe {
    let dir = sync_dir(path);
    let shown = dir.display().to_string();
    let parent_ok = dir.parent().is_some_and(|p| p.is_dir());
    if path.trim().is_empty() || !parent_ok {
        return FolderProbe { dir: shown, state: "bad".into(), devices: 0, error: Some("找不到这个文件夹".into()) };
    }
    match folder::probe(&dir) {
        Ok(folder::Probe::Empty) => FolderProbe { dir: shown, state: "empty".into(), devices: 0, error: None },
        Ok(folder::Probe::Space { devices }) => FolderProbe { dir: shown, state: "space".into(), devices, error: None },
        Err(e) => FolderProbe { dir: shown, state: "bad".into(), devices: 0, error: Some(e) },
    }
}

/// 列出这台电脑上找到的网盘同步文件夹（找到哪个列哪个）
#[tauri::command]
pub fn sync_folder_candidates() -> Vec<Candidate> {
    let home = std::env::var("USERPROFILE").map(PathBuf::from).unwrap_or_default();
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for (var, label) in [("OneDrive", "OneDrive"), ("OneDriveConsumer", "OneDrive"), ("OneDriveCommercial", "OneDrive（工作或学校）")] {
        if let Ok(p) = std::env::var(var) {
            found.push((label.into(), PathBuf::from(p)));
        }
    }
    // 坚果云的同步文件夹在 Nutstore\<数字>\我的坚果云，或者 Nutstore\我的坚果云
    let nut = home.join("Nutstore");
    found.push(("坚果云".into(), nut.join("我的坚果云")));
    if let Ok(rd) = std::fs::read_dir(&nut) {
        for e in rd.flatten() {
            found.push(("坚果云".into(), e.path().join("我的坚果云")));
        }
    }
    found.push(("百度网盘同步空间".into(), home.join("BaiduSyncdisk")));
    found.push(("Dropbox".into(), home.join("Dropbox")));
    found.push(("iCloud 云盘".into(), home.join("iCloudDrive")));
    let mut seen = HashSet::new();
    found
        .into_iter()
        .filter(|(_, p)| p.is_dir() && seen.insert(p.to_string_lossy().to_lowercase()))
        .map(|(label, p)| {
            let path = p.display().to_string();
            Candidate { label, probe: probe_dir(&path), path }
        })
        .collect()
}

/// 看看选的文件夹里有没有叮咪的数据，决定下一步是「第一台电脑」还是「加入」
#[tauri::command]
pub fn sync_folder_probe(path: String) -> FolderProbe {
    probe_dir(&path)
}

/// 开启网盘文件夹同步：文件夹里还没有叮咪的数据就建一个（设同步密码），有就加入（核对同步密码）。
/// replace：加入时用文件夹里的替换本机（先备份）；否则两边合在一起
#[tauri::command]
pub async fn sync_folder_start(app: AppHandle, path: String, password: String, name: String, replace: bool) -> Result<(), String> {
    let dir = sync_dir(&path);
    let name = clean_name(&name);
    let probe = folder::probe(&dir)?;
    let key = match probe {
        folder::Probe::Empty => {
            check_password(&password)?;
            let ns = tokio::task::spawn_blocking(move || crypto::new_space(&password, false)).await.map_err(err)??;
            folder::create(&dir, &folder::SpaceFile { v: 1, salt: ns.salt, box1: ns.box1 })?;
            ns.master
        }
        folder::Probe::Space { .. } => {
            let space = folder::read_space(&dir)?;
            tokio::task::spawn_blocking(move || crypto::unlock(&password, &space.salt, &space.box1, None))
                .await
                .map_err(err)??
        }
    };
    // 密码核对得上才收掉原来的同步方式
    retire(&app).await;
    secret::set(DATA_KEY, &key)?;
    let cfg = FolderCfg { dir: dir.display().to_string(), name };
    {
        let st = app.state::<AppState>();
        let c = st.db.lock().unwrap();
        db::kv_set(&c, "sync_folder", &serde_json::to_string(&cfg).map_err(err)?).map_err(err)?;
        if replace && matches!(probe, folder::Probe::Space { .. }) {
            let backup = format!("替换前-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"));
            db::snapshot(&c, &st.data_dir, &backup)?;
            db::wipe_synced(&c).map_err(err)?;
        } else {
            db::mark_all_dirty(&c).map_err(err)?;
        }
    }
    notify_data(&app);
    notify_queue(&app);
    restart(&app);
    app.state::<AppState>().sync.poke();
    Ok(())
}

/// 网盘文件夹里的几台电脑和各自最后出现的时间
#[tauri::command]
pub fn sync_folder_devices(app: AppHandle) -> Result<Vec<folder::Seen>, String> {
    let fc = load_folder(&app).ok_or("这台电脑没有用网盘文件夹同步")?;
    let key: crypto::DataKey = secret::get(DATA_KEY).and_then(|k| k.try_into().ok()).ok_or("还差同步密码")?;
    let st = app.state::<AppState>();
    let dir = PathBuf::from(&fc.dir);
    Ok(folder::devices(&folder::Folder { dir: &dir, me: &st.device, key: &key }))
}

/// 看一遍文件夹：拉别的电脑新写的；到时候了就更新一下自己最后出现的时间。返回本机有没有变化
async fn folder_round(app: &AppHandle, cfg: &FolderCfg, key: crypto::DataKey, beat: bool) -> Result<bool, String> {
    let (app2, cfg) = (app.clone(), cfg.clone());
    tokio::task::spawn_blocking(move || {
        let st = app2.state::<AppState>();
        let dir = PathBuf::from(&cfg.dir);
        let f = folder::Folder { dir: &dir, me: &st.device, key: &key };
        let changed = folder::pull(&st.db, &st.data_dir, &f)?;
        if beat {
            folder::heartbeat(&f, &cfg.name)?;
        }
        Ok(changed)
    })
    .await
    .map_err(err)?
}

/// 网盘文件夹方式的接收任务：每 10 秒看一遍，一直到被叫停（换设置、立即同步）
async fn folder_session(app: &AppHandle, cfg: &FolderCfg, key: crypto::DataKey) {
    {
        let st = app.state::<AppState>();
        let _ = db::set_own_device(&st.db.lock().unwrap(), &st.device, &cfg.name);
        st.sync.poke();
    }
    let mut beat_at: Option<Instant> = None;
    loop {
        let beat = beat_at.is_none_or(|t| t.elapsed() >= HEARTBEAT_EVERY);
        match folder_round(app, cfg, key, beat).await {
            Ok(changed) => {
                if beat {
                    beat_at = Some(Instant::now());
                }
                publish_folder(app, "online", cfg, None);
                if changed {
                    crate::reload_settings(app);
                    notify_data(app);
                    notify_queue(app);
                }
            }
            Err(e) => publish_folder(app, "offline", cfg, Some(e)),
        }
        tokio::time::sleep(FOLDER_EVERY).await;
    }
}

/// 把本机改过的写进文件夹
async fn push_folder(app: &AppHandle, cfg: &FolderCfg) -> Result<usize, String> {
    let key: crypto::DataKey = secret::get(DATA_KEY).and_then(|k| k.try_into().ok()).ok_or("还差同步密码")?;
    let (app2, cfg) = (app.clone(), cfg.clone());
    tokio::task::spawn_blocking(move || {
        let st = app2.state::<AppState>();
        let dir = PathBuf::from(&cfg.dir);
        // 以前在叮咪同步里因为太大没传的：网盘文件夹不限大小，清掉记号、照常写进去
        let none = records::Limits { me: st.device.clone(), ..Default::default() };
        records::unmark_fitting(&st.db.lock().unwrap(), &none).map_err(err)?;
        folder::push(&st.db, &st.data_dir, &folder::Folder { dir: &dir, me: &st.device, key: &key })
    })
    .await
    .map_err(err)?
}

// ---------- 给另一台电脑配对 ----------

#[derive(Serialize)]
pub struct Invite {
    /// 完整的配对码，粘贴到另一台电脑上
    pub code: String,
    pub expires_at: String,
}

/// 让服务器发一个新的配对码，再带上这台电脑的空间密钥：另一台电脑只要输同步密码
#[tauri::command]
pub async fn sync_invite(app: AppHandle) -> Result<Invite, String> {
    let space_key = space_key().ok_or("这台电脑还没有空间密钥，先在上面解锁")?;
    let (reply, cfg, _) = ask(&app, Msg::InviteNew).await.map_err(|e| {
        if e == NO_REPLY {
            "服务器没有回应。服务器上的 dingmi-server 可能还是旧版本，更新后再试".to_string()
        } else {
            e
        }
    })?;
    let Msg::Invited { token, expires_at } = reply else {
        return Err("服务器回的内容不对".into());
    };
    let server_public = B64.decode(&cfg.server_public).map_err(err)?;
    // 叮咪同步以前存的端口是 7788：配对码里一律写 443（新电脑认得出是域名，只走 wss）
    let port = if official::is_official(&cfg.host) { official::PORT } else { cfg.port };
    let code = PairCode { host: cfg.host, port, server_public, invite: token, space_key: Some(space_key.to_vec()) };
    Ok(Invite { code: code.to_string(), expires_at })
}

// ---------- 手机 ----------

/// 用这台电脑的身份另开一个连接（换同步方式前最后拉一遍时用）
async fn open(app: &AppHandle) -> Result<transport::Conn, String> {
    let cfg = load_config(app).ok_or("这台电脑还没有开启同步，连不上服务器")?;
    let device = secret::get(DEVICE_KEY).ok_or("这台电脑的设备密钥不见了，需要重新配对")?;
    let server_public = B64.decode(&cfg.server_public).map_err(err)?;
    let (conn, _) = dial(&cfg.host, cfg.port, &cfg.web, &device, &server_public, &cfg.name, None, None, SESSION_REJECTED).await?;
    Ok(conn)
}

/// 在连接上问一句，服务器回 Error、Refused 时变成 Err
pub async fn call(conn: &mut transport::Conn, msg: &Msg) -> Result<Msg, String> {
    conn.send(msg).await.map_err(err)?;
    match timeout(REPLY_TIMEOUT, conn.recv::<Msg>()).await {
        Ok(Ok(Msg::Error { msg } | Msg::Refused { msg, .. })) => Err(msg),
        Ok(Ok(reply)) => Ok(reply),
        _ => Err(NO_REPLY.into()),
    }
}

/// 用这台电脑的身份临时连一次服务器，问一句、拿回答案就断开（不影响后台连接）
async fn ask(app: &AppHandle, msg: Msg) -> Result<(Msg, Config, Welcome), String> {
    if load_folder(app).is_some() {
        return Err("网盘文件夹方式用不了这个：手机和配对码要连服务器（自建服务器或官方同步）".into());
    }
    let cfg = load_config(app).ok_or("这台电脑还没有开启同步")?;
    let device = secret::get(DEVICE_KEY).ok_or("这台电脑的设备密钥不见了，需要重新配对")?;
    let server_public = B64.decode(&cfg.server_public).map_err(err)?;
    let (mut conn, welcome) = dial(&cfg.host, cfg.port, &cfg.web, &device, &server_public, &cfg.name, None, None, SESSION_REJECTED).await?;
    check_refused(&welcome)?;
    if welcome.account.is_some() {
        note_account(app, Some(welcome.account.clone()), None, None);
    }
    conn.send(&msg).await.map_err(err)?;
    match timeout(REPLY_TIMEOUT, conn.recv::<Msg>()).await {
        Ok(Ok(Msg::Error { msg } | Msg::Refused { msg, .. })) => Err(msg),
        Ok(Ok(reply)) => Ok((reply, cfg, welcome)),
        _ => Err(NO_REPLY.into()),
    }
}

#[tauri::command]
pub async fn phone_list(app: AppHandle) -> Result<Vec<Phone>, String> {
    match ask(&app, Msg::PhoneList).await? {
        (Msg::Phones { phones }, ..) => Ok(phones),
        _ => Err("服务器回的内容不对".into()),
    }
}

/// 添加手机后给手机用的两个网址：先装根证书，再打开收件箱网页。
/// 服务器给手机网页绑了域名（正规证书）时不用装证书，ca_url 是空的
#[derive(Serialize)]
pub struct PhoneLink {
    pub id: String,
    pub url: String,
    pub ca_url: String,
    pub fingerprint: String,
}

/// 手机网页的网址开头：服务器绑了域名用域名（只认 https://，和电脑走 wss 时的规矩一样，见 web_of）；
/// 服务器地址本身是域名（叮咪同步）用它；都没有的用 https://IP:端口
fn phone_base(welcome: &Welcome, cfg: &Config) -> (String, bool) {
    match (web_of(welcome).as_str(), domain_web(&cfg.host, cfg.port)) {
        ("", Some(d)) => (d, true),
        ("", None) => (format!("https://{}:{}", cfg.host, cfg.port), false),
        (u, _) => (u.to_string(), true),
    }
}

/// 手机网页的网址：令牌、收件箱公钥、空间密钥都放在 # 后面（浏览器不会发给服务器，也就不会进 Nginx 的访问日志）。
/// 手机网页从 # 里读；以前的 /m/<令牌>.<公钥> 网址照样能用（见 apps/mobile/app.ts）
fn phone_url(base: &str, token: &str, public: &str, space_key: &str) -> String {
    format!("{base}/m/#t={token}.{public}&k={space_key}")
}

#[tauri::command]
pub async fn phone_add(app: AppHandle, name: String) -> Result<PhoneLink, String> {
    let key: crypto::DataKey = secret::get(DATA_KEY)
        .and_then(|k| k.try_into().ok())
        .ok_or("还差同步密码，先在上面解锁")?;
    let space_key = space_key().ok_or("这台电脑还没有空间密钥，先在「设置 → 同步」里解锁")?;
    let name = if name.trim().is_empty() { "iPhone".to_string() } else { name };
    let (Msg::PhoneAdded { id, token }, cfg, welcome) = ask(&app, Msg::PhoneAdd { name }).await? else {
        return Err("服务器回的内容不对".into());
    };
    // 令牌是服务器给的，要拼进网址：只认 base64url
    if token.is_empty() || !token.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err("服务器回的内容不对".into());
    }
    let public = B64.encode(crypto::inbox_public(&key));
    let (base, trusted) = phone_base(&welcome, &cfg);
    Ok(PhoneLink {
        id,
        url: phone_url(&base, &token, &public, &B64.encode(space_key)),
        ca_url: if trusted { String::new() } else { format!("http://{}:{}/ca", cfg.host, cfg.port) },
        fingerprint: welcome.ca_fingerprint,
    })
}

#[tauri::command]
pub async fn phone_revoke(app: AppHandle, id: String) -> Result<(), String> {
    ask(&app, Msg::PhoneRevoke { id }).await.map(|_| ())
}

/// 空间里的电脑（设备满了、电脑丢了时作废不用的，需求-1.0 第 30.4 节）
#[tauri::command]
pub async fn pc_list(app: AppHandle) -> Result<Vec<Pc>, String> {
    match ask(&app, Msg::PcList).await? {
        (Msg::Pcs { pcs }, ..) => Ok(pcs),
        _ => Err("服务器回的内容不对".into()),
    }
}

/// 作废空间里的另一台电脑：它下一条消息就会被断开、以后也连不上；不能作废这台自己
#[tauri::command]
pub async fn pc_revoke(app: AppHandle, id: String) -> Result<(), String> {
    ask(&app, Msg::PcRevoke { id }).await.map(|_| ())
}

// ---------- 后台连接 ----------

/// 连上之后先拉一遍、取走收件箱，再一直等新的；返回时说明连接断了：(是否连上过, 原因)
async fn session(app: &AppHandle, cfg: &Config, device: &[u8], key: &crypto::DataKey) -> (bool, String) {
    let Ok(server_public) = B64.decode(&cfg.server_public) else {
        return (false, "配对信息坏了，需要重新配对".into());
    };
    let (mut conn, welcome) = match dial(&cfg.host, cfg.port, &cfg.web, device, &server_public, &cfg.name, None, None, SESSION_REJECTED).await {
        Ok(c) => c,
        Err(e) => return (false, e),
    };
    // 服务器说清楚了不收（比如叮咪同步到期太久、数据删了）：照常按连不上处理，原因记在 refused 里
    if let Some(r) = &welcome.refused {
        note_account(app, Some(None), None, Some(Some(r.code.clone())));
        return (false, r.msg.clone());
    }
    remember_web(app, cfg, &welcome);
    if welcome.account.is_some() {
        note_account(app, Some(welcome.account.clone()), None, None);
    }
    // 叮咪同步到期了：先不同步（服务器也不收），状态是 expired；一分钟左右再连上来看看续上没有
    if let Some(a) = welcome.account.as_ref().filter(|a| a.expired) {
        return (false, expired_text(a));
    }
    let st = app.state::<AppState>();
    if server_restored(&st.db.lock().unwrap(), &welcome.epoch) {
        crate::diag::log("同步", "服务器拿旧备份恢复过：从头拉，本机的全部重新传一遍");
    }
    publish(app, "online", Some(cfg), None);
    // 把这台电脑的名字告诉别的电脑（指定设备提醒时用），顺便叫上传任务把没传的传上去
    let _ = db::set_own_device(&st.db.lock().unwrap(), &st.device, &cfg.name);
    st.sync.poke();
    let mut bad = HashSet::new();
    let mut woke = true;
    loop {
        if woke {
            match engine::pull(&st.db, &st.data_dir, &mut conn, key).await {
                Ok(true) => {
                    crate::reload_settings(app);
                    notify_data(app);
                    notify_queue(app);
                }
                Ok(false) => {}
                Err(e) => return (true, format!("同步时出错：{e}")),
            }
            if let Err(e) = inbox::fetch(app, &mut conn, key, &mut bad).await {
                return (true, format!("取手机发来的东西时出错：{e}"));
            }
        }
        if conn.send(&Msg::Wait).await.is_err() {
            return (true, "和服务器的连接断了".into());
        }
        woke = match timeout(WAIT_TIMEOUT, conn.recv::<Msg>()).await {
            Ok(Ok(Msg::Changed | Msg::InboxNotify)) => true,
            Ok(Ok(Msg::Pong)) => false,
            _ => return (true, "和服务器的连接断了".into()),
        };
    }
}

/// 服务器这份数据的「代」（Welcome.epoch）和上次见到的不一样：服务器拿旧备份恢复过，
/// 备份之后的记录、附件（备份里没有附件）服务器上都没了，而这台电脑以为早传过了、拉取的位置也比服务器上的还靠后。
/// 从头拉，把本机的全部标成要上传（附件也会重新传）。服务器按版本比较，重复传的不会出问题。
/// 第一次见到（刚配对、旧版电脑升级上来）只记下，不重传；旧服务器没有「代」，不管。返回是不是恢复过
fn server_restored(c: &rusqlite::Connection, epoch: &str) -> bool {
    if epoch.is_empty() {
        return false;
    }
    let seen = db::kv_get(c, "sync_epoch");
    if seen.as_deref() == Some(epoch) {
        return false;
    }
    let _ = db::kv_set(c, "sync_epoch", epoch);
    if seen.is_none_or(|s| s.is_empty()) {
        return false;
    }
    let _ = db::kv_del(c, "sync_seq");
    let _ = db::mark_all_dirty(c);
    true
}

/// 叮咪同步已到期时给人看的一句（和服务器说的一样）
fn expired_text(a: &Account) -> String {
    match &a.delete_on {
        Some(d) => format!("叮咪同步已到期，续费后接着同步。电脑上的数据都在，服务器上的留到 {d}"),
        None => "叮咪同步已到期，续费后接着同步。电脑上的数据都在".into(),
    }
}

/// 另开一个连接，把本机改过的传上去。空间满了（叮咪同步）：附件先留在这台电脑上，文字照常传，
/// 状态里的 held_files 是留下了几个
async fn push_now(app: &AppHandle, cfg: &Config) -> Result<usize, String> {
    let device = secret::get(DEVICE_KEY).ok_or("设备密钥不见了")?;
    let key = secret::get(DATA_KEY)
        .and_then(|k| crypto::DataKey::try_from(k).ok())
        .ok_or("还差同步密码")?;
    let server_public = B64.decode(&cfg.server_public).map_err(err)?;
    let (mut conn, welcome) = dial(&cfg.host, cfg.port, &cfg.web, &device, &server_public, &cfg.name, None, None, SESSION_REJECTED).await?;
    check_refused(&welcome)?;
    // 已经知道满了就不去试附件了（试一次要传一块 256 KB 才知道）
    let files = welcome.account.as_ref().is_none_or(|a| a.storage != "full");
    let st = app.state::<AppState>();
    // 叮咪同步的套餐限单个图片 / 附件多大（协议 6；旧服务器、自建的没有，不限）
    let limits = records::Limits {
        max_image: welcome.account.as_ref().and_then(|a| a.max_image),
        max_file: welcome.account.as_ref().and_then(|a| a.max_file),
        me: st.device.clone(),
    };
    let pushed = engine::push_with(&st.db, &st.data_dir, &mut conn, &key, files, &limits).await?;
    note_account(app, welcome.account.map(Some), Some((pushed.held, pushed.only_here)), None);
    Ok(pushed.count)
}

/// 点开一个还没下载的文件附件时：另开一个连接把它下载下来（网盘文件夹方式从文件夹里取）
pub async fn download_attachment(app: &AppHandle, uid: &str, file: &str) -> Result<(), String> {
    if let Some(fc) = load_folder(app) {
        let key: crypto::DataKey =
            secret::get(DATA_KEY).and_then(|k| k.try_into().ok()).ok_or("还差同步密码，先在「设置 → 同步」里解锁")?;
        let (app2, uid, file) = (app.clone(), uid.to_string(), file.to_string());
        return tokio::task::spawn_blocking(move || {
            let st = app2.state::<AppState>();
            let dir = PathBuf::from(&fc.dir);
            folder::fetch_file(&st.db, &st.data_dir, &folder::Folder { dir: &dir, me: &st.device, key: &key }, &uid, &file)
        })
        .await
        .map_err(err)?;
    }
    // 太大、叮咪同步不传的：原文件只在那台设备上，服务器上没有，说清楚
    let only_on = {
        let st = app.state::<AppState>();
        let c = st.db.lock().unwrap();
        db::get_attachment(&c, file).ok().flatten().and_then(|a| a.only_on).map(|d| (db::device_name(&c, &d), d == st.device))
    };
    if let Some((name, mine)) = only_on {
        return Err(if mine {
            "这个文件超过了叮咪同步单个文件的大小，没有传上去，原文件只在这台电脑上".to_string()
        } else {
            format!("这个文件超过了叮咪同步单个文件的大小，没有传上来，原文件只在「{}」上", name.unwrap_or_else(|| "另一台电脑".into()))
        });
    }
    let cfg = load_config(app).ok_or("这个文件在别的设备上，这台电脑还没开启同步，下载不了")?;
    let device = secret::get(DEVICE_KEY).ok_or("设备密钥不见了")?;
    let key = secret::get(DATA_KEY)
        .and_then(|k| crypto::DataKey::try_from(k).ok())
        .ok_or("还差同步密码，先在「设置 → 同步」里解锁")?;
    let server_public = B64.decode(&cfg.server_public).map_err(err)?;
    let (mut conn, _) = dial(&cfg.host, cfg.port, &cfg.web, &device, &server_public, &cfg.name, None, None, SESSION_REJECTED).await?;
    let st = app.state::<AppState>();
    engine::fetch_file(&st.db, &st.data_dir, &mut conn, &key, uid, file).await
}

/// 上传任务：本机数据变了就等它停下来 2 秒，有没传的就传；连不上的话 10 秒后再看
pub fn start_pusher(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let st = app.state::<AppState>();
        loop {
            let _ = timeout(PUSH_EVERY, st.sync.poke.notified()).await;
            while timeout(PUSH_DEBOUNCE, st.sync.poke.notified()).await.is_ok() {}
            let (folder_cfg, server_cfg) = (load_folder(&app), load_config(&app));
            if folder_cfg.is_none() && server_cfg.is_none() {
                continue;
            }
            // 叮咪同步到期了：服务器不收，不去白连（续费后接收那边会重连、叫醒这里）
            if crate::update::too_old() || expired_now(&app) || !records::has_dirty(&st.db.lock().unwrap()) {
                continue;
            }
            let pushed = match (&folder_cfg, &server_cfg) {
                (Some(fc), _) => push_folder(&app, fc).await,
                (None, Some(c)) => {
                    let r = push_now(&app, c).await;
                    push_error(&app, c, r.as_ref().err().cloned());
                    r
                }
                _ => continue,
            };
            if pushed.is_err() {
                tokio::time::sleep(Duration::from_secs(10)).await;
                st.sync.poke();
            }
        }
    });
}

/// 连着服务器、但上传被拒了（比如管理员设的容量上限满了，需求-0.4 第 15.4 节）：状态还是「连上了」，
/// 把原因挂在上面，标题栏和设置里能看到；传上去了就清掉
fn push_error(app: &AppHandle, cfg: &Config, error: Option<String>) {
    let changed = {
        let st = app.state::<AppState>();
        let s = st.sync.status.lock().unwrap();
        s.state == "online" && s.error != error
    };
    if changed {
        publish(app, "online", Some(cfg), error);
    }
}

/// 同步进行中检查更新发现版本太旧：等到这时候就停下，回到上面的循环开头
async fn wait_outdated() {
    while !crate::update::too_old() {
        tokio::time::sleep(OUTDATED_EVERY).await;
    }
}

/// 凭据管理器这次没读出来：30 秒后再试，设置里改了什么也马上再试
const KEYS_RETRY: Duration = Duration::from_secs(30);

async fn keys_retry_wait(rx: &mut watch::Receiver<u64>) {
    tokio::select! {
        _ = tokio::time::sleep(KEYS_RETRY) => {}
        _ = rx.changed() => {}
    }
}

enum Woke {
    Restart,
    Done((bool, String)),
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut rx = app.state::<AppState>().sync.restart.subscribe();
        let mut backoff = 1;
        loop {
            let _ = rx.borrow_and_update();
            // 官网说这个版本太旧了：旧版本可能把数据弄乱，同步先停下，其他功能照常用
            if crate::update::too_old() {
                match (load_folder(&app), load_config(&app)) {
                    (Some(fc), _) => publish_folder(&app, "outdated", &fc, None),
                    (None, Some(c)) => publish(&app, "outdated", Some(&c), None),
                    _ => publish(&app, "off", None, None),
                }
                tokio::select! {
                    _ = tokio::time::sleep(OUTDATED_EVERY) => {}
                    _ = rx.changed() => {}
                }
                continue;
            }
            if let Some(fc) = load_folder(&app) {
                let key = match secret::read(DATA_KEY) {
                    Ok(k) => k.and_then(|k| crypto::DataKey::try_from(k).ok()),
                    Err(e) => {
                        publish_folder(&app, "keys_retry", &fc, Some(e));
                        keys_retry_wait(&mut rx).await;
                        continue;
                    }
                };
                match key {
                    Some(key) => {
                        publish_folder(&app, "connecting", &fc, None);
                        tokio::select! {
                            _ = folder_session(&app, &fc, key) => {}
                            _ = rx.changed() => {}
                            _ = wait_outdated() => {}
                        }
                    }
                    None => {
                        publish_folder(&app, "locked", &fc, None);
                        let _ = rx.changed().await;
                    }
                }
                continue;
            }
            let Some(cfg) = load_config(&app) else {
                publish(&app, "off", None, None);
                let _ = rx.changed().await;
                continue;
            };
            // 钥匙在凭据管理器里：真的没有（重装过系统、数据从别的电脑拷来）要重新加入；读出错了过一会儿再试，
            // 不能当成没开同步（以前那样会显示成从来没开过，用户再加入一次白占一个设备名额）
            let device = match secret::read(DEVICE_KEY) {
                Ok(Some(d)) => d,
                Ok(None) => {
                    publish(&app, "keys_lost", Some(&cfg), None);
                    let _ = rx.changed().await;
                    continue;
                }
                Err(e) => {
                    publish(&app, "keys_retry", Some(&cfg), Some(e));
                    keys_retry_wait(&mut rx).await;
                    continue;
                }
            };
            let key = match secret::read(DATA_KEY) {
                Ok(k) => k.and_then(|k| crypto::DataKey::try_from(k).ok()),
                Err(e) => {
                    publish(&app, "keys_retry", Some(&cfg), Some(e));
                    keys_retry_wait(&mut rx).await;
                    continue;
                }
            };
            let Some(key) = key else {
                publish(&app, "locked", Some(&cfg), None);
                let _ = rx.changed().await;
                continue;
            };
            publish(&app, "connecting", Some(&cfg), None);
            let woke = tokio::select! {
                r = session(&app, &cfg, &device, &key) => Woke::Done(r),
                _ = rx.changed() => Woke::Restart,
                _ = wait_outdated() => Woke::Restart,
            };
            let Woke::Done((was_online, why)) = woke else {
                backoff = 1;
                continue;
            };
            if was_online {
                backoff = 1;
            }
            let state = if expired_now(&app) { "expired" } else { "offline" };
            publish(&app, state, Some(&cfg), Some(why));
            let woke = tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(backoff)) => Woke::Done((false, String::new())),
                _ = rx.changed() => Woke::Restart,
            };
            backoff = match woke {
                Woke::Restart => 1,
                Woke::Done(_) => (backoff * 2).min(MAX_BACKOFF),
            };
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 地址是域名（叮咪同步）只走 wss://域名/sync；IP（自建服务器）照旧直连
    #[test]
    fn domain_address_only_wss() {
        assert_eq!(domain_web("sync.dingmi.app", 443).as_deref(), Some("https://sync.dingmi.app"));
        // 以前存的 7788 也当 443
        assert_eq!(domain_web("Sync.Dingmi.App", 7788).as_deref(), Some("https://sync.dingmi.app"));
        assert_eq!(domain_web("sync-test.example.com", 443).as_deref(), Some("https://sync-test.example.com"));
        assert_eq!(domain_web("sync.example.com", 8443).as_deref(), Some("https://sync.example.com:8443"));
        for ip in ["203.0.113.7", "127.0.0.1", "::1", "[::1]", ""] {
            assert_eq!(domain_web(ip, 7788), None, "{ip}");
        }
        // 叮咪同步的配对码（官方服务器生成的、电脑生成的）认得出来
        let code: PairCode = format!("dingmi://sync.dingmi.app:443/{}/inv", B64.encode([7u8; 32])).parse().unwrap();
        assert_eq!(domain_web(&code.host, code.port).as_deref(), Some(official::WEB));
        assert_eq!(official::PORT, 443);
    }

    /// 网盘里的文件夹改名「叮咪数据」以后：新建用新名字，选的或者里面已有旧名字「叮咪同步」的接着用旧的
    #[test]
    fn sync_dir_old_and_new_name() {
        let base = std::env::temp_dir().join(format!("dingmi-syncdir-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        let b = base.to_string_lossy().to_string();
        assert_eq!(sync_dir(&b), base.join("叮咪数据"));
        assert_eq!(sync_dir(&base.join("叮咪数据").to_string_lossy()), base.join("叮咪数据"));
        assert_eq!(sync_dir(&base.join("叮咪同步").to_string_lossy()), base.join("叮咪同步"));
        std::fs::create_dir_all(base.join("叮咪同步")).unwrap();
        assert_eq!(sync_dir(&b), base.join("叮咪同步"), "别的电脑用旧版建的，接着用");
        std::fs::create_dir_all(base.join("叮咪数据")).unwrap();
        assert_eq!(sync_dir(&b), base.join("叮咪数据"), "两个都有时用新的");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn restored_server_resyncs() {
        let dir = std::env::temp_dir().join(format!("dingmi-epoch-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let c = db::open(&dir).unwrap();
        db::kv_set(&c, "sync_seq", "42").unwrap();
        // 旧服务器、第一次见到：不动
        assert!(!server_restored(&c, ""));
        assert!(!server_restored(&c, "a"));
        assert!(!server_restored(&c, "a"));
        assert_eq!(db::kv_get(&c, "sync_seq").as_deref(), Some("42"));
        // 代变了：从头拉、全部重传
        c.execute("INSERT INTO devices(uid,name,ver,dirty) VALUES('d1','x','1',0)", []).unwrap();
        assert!(server_restored(&c, "b"));
        assert_eq!(db::kv_get(&c, "sync_seq"), None);
        let dirty: i64 = c.query_row("SELECT dirty FROM devices WHERE uid='d1'", [], |r| r.get(0)).unwrap();
        assert_eq!(dirty, 1);
        assert!(!server_restored(&c, "b"));
    }

    #[test]
    fn old_server_is_refused() {
        assert!(server_new_enough(MIN_SERVER).is_ok());
        assert!(server_new_enough(VERSION).is_ok());
        assert!(server_new_enough(MIN_SERVER - 1).unwrap_err().contains("服务器版本太旧"));
        // 自己要的不能比自己的协议还新，不然连自己这一版的服务器都连不上
        const { assert!(MIN_SERVER <= VERSION) };
    }

    #[test]
    fn phone_link_uses_web_domain() {
        let cfg = Config { host: "1.2.3.4".into(), port: 7788, server_public: String::new(), name: String::new(), web: String::new() };
        let mut w = Welcome::default();
        assert_eq!(phone_base(&w, &cfg), ("https://1.2.3.4:7788".to_string(), false));
        // 服务器给手机网页绑了域名：用域名，不用装证书
        w.web_url = "https://sync.example.com/".into();
        assert_eq!(phone_base(&w, &cfg), ("https://sync.example.com".to_string(), true));
        // 攻击：服务器给的「域名」不是 https（http://、javascript:、乱写的）：不用，退回 https://IP:端口
        for bad in ["http://sync.example.com", "javascript:alert(1)//", "sync.example.com", "https://", "file:///C:/x"] {
            w.web_url = bad.into();
            assert_eq!(phone_base(&w, &cfg), ("https://1.2.3.4:7788".to_string(), false), "{bad}");
        }
        // 令牌、公钥、空间密钥都在 # 后面，路径里只有 /m/
        let url = phone_url("https://sync.example.com", "tok_-1", "PUB", "KEY");
        assert_eq!(url, "https://sync.example.com/m/#t=tok_-1.PUB&k=KEY");
        assert_eq!(url.split('#').next(), Some("https://sync.example.com/m/"));
    }

    /// 只有叮咪同步（账上 official）才传盒子 3；自建服务器、旧服务器（没有账）不传
    #[test]
    fn box3_only_for_official() {
        assert!(!joins_on(&Welcome::default()));
        let mut w = Welcome { account: Some(Account::default()), ..Default::default() };
        assert!(!joins_on(&w));
        w.account.as_mut().unwrap().official = true;
        assert!(joins_on(&w));
        let k = crypto::SpaceKey::default();
        let Msg::SetLock { join, .. } = relock("一个够长的同步密码啊", &[1u8; 32], Some(&k), false).unwrap() else { panic!() };
        assert!(join.is_none(), "自建服务器换密码不带盒子 3");
        let Msg::SetLock { join, .. } = relock("一个够长的同步密码啊", &[1u8; 32], Some(&k), true).unwrap() else { panic!() };
        assert!(join.is_some());
    }
    /// 用兑换码开通叮咪同步（本机起的服务器代替 sync.dingmi.app）：开通 → 和第一台电脑一样设同步密码、拿恢复码 →
    /// 续费、要客服核对码；码不对、用过了说得清
    #[tokio::test]
    async fn redeem_with_local_server() {
        // 开源的 dingmi-server 不带叮咪同步：用测试里像叮咪同步那样回话的扩展点（engine::tests::FakeOfficial）
        let fake = std::sync::Arc::new(engine::tests::FakeOfficial::default());
        let codes = ["DM-AAAA-BBBB-CCCC-DDDD".to_string(), "DM-EEEE-FFFF-GGGG-HHHH".to_string()];
        fake.codes.lock().unwrap().extend(codes.iter().map(|c| (c.clone(), false)));
        let (_server, port, spub, _) = engine::tests::local_server_with("redeem", fake.clone()).await;

        let (dev, mut conn, w) = redeem_dial("127.0.0.1", port, "", &spub, &codes[0], "新电脑", None).await.unwrap();
        let a = w.account.unwrap();
        assert_eq!((a.plan.as_str(), a.official, a.expired), ("标准", true, false));
        assert!(a.space.starts_with("S-"));
        // 空间还是空的：像第一台电脑一样设同步密码，生成主密钥、空间密钥和恢复码
        assert!(w.space.salt.is_none());
        let keys = unlock(&mut conn, &w.space, "password1".into(), None, None, true).await.unwrap();
        let recovery = crypto::parse_recovery(&keys.new_recovery.unwrap()).unwrap();
        drop(conn);

        // 再连上（不带兑换码）：密码 + 空间密钥打开盒子 1
        let (mut conn, w) = dial("127.0.0.1", port, "", &dev, &spub, "新电脑", None, None, SESSION_REJECTED).await.unwrap();
        assert_eq!(unlock(&mut conn, &w.space, "password1".into(), keys.space_key, None, true).await.unwrap().master, keys.master);
        // 续费：同一档接在后面
        let before = w.account.unwrap().expires_at.unwrap();
        let Msg::AccountInfo { account } = call(&mut conn, &Msg::Renew { code: codes[1].clone(), terms: None }).await.unwrap() else { panic!() };
        assert!(account.expires_at.unwrap() > before);
        // 用过的码：call 把 Refused 变成 Err，原话给人看
        let e = call(&mut conn, &Msg::Renew { code: codes[1].clone(), terms: None }).await.unwrap_err();
        assert!(e.contains("用过"), "{e}");
        let Msg::Support { code, .. } = call(&mut conn, &Msg::SupportNew).await.unwrap() else { panic!() };
        assert_eq!(fake.support.lock().unwrap().get(&code).map(String::as_str), Some("新电脑"));

        // 唯一的电脑没了：凭恢复码登记一台新的、打开盒子 2
        let (dev3, _) = dingmi_sync::generate_keypair().unwrap();
        let token = Some(crypto::recovery_token(&recovery));
        let (mut c3, w3) = dial("127.0.0.1", port, "", &dev3, &spub, "找回的电脑", None, token, "x").await.unwrap();
        assert!(check_refused(&w3).is_ok());
        assert_eq!(unlock(&mut c3, &w3.space, String::new(), None, Some(recovery), true).await.unwrap().master, keys.master);

        // 码不对、用过了：开通前就报出来
        let e = redeem_dial("127.0.0.1", port, "", &spub, "DM-2222-3333-4444-5555", "x", None).await.err().unwrap();
        assert!(e.contains("不存在"), "{e}");
        let e = redeem_dial("127.0.0.1", port, "", &spub, &codes[0], "x", None).await.err().unwrap();
        assert!(e.contains("用过"), "{e}");
        assert_eq!(redeem_dial("127.0.0.1", port, "", &spub, " ", "x", None).await.err().unwrap(), "先填兑换码");
    }

    /// 兑换码 + 同步密码加入（需求-1.x 第 34.2 节）：第一台用够长的同步密码开通（自动带盒子 3）→ 第二台填同一个码和
    /// 同步密码，拿到同一把主密钥和空间密钥；密码不对、短密码的空间说得清；开通时又填了这个码接到「输同步密码加入」
    #[tokio::test]
    async fn join_with_code_local_server() {
        let fake = std::sync::Arc::new(engine::tests::FakeOfficial::default());
        let codes = ["DM-AAAA-BBBB-CCCC-DDDD".to_string(), "DM-EEEE-FFFF-GGGG-HHHH".to_string()];
        fake.codes.lock().unwrap().extend(codes.iter().map(|c| (c.clone(), false)));
        let (_server, port, spub, _) = engine::tests::local_server_with("join-code", fake.clone()).await;
        let pw = "家里那台的同步密码很长".to_string();

        let (_, mut conn, w) = redeem_dial("127.0.0.1", port, "", &spub, &codes[0], "家里的电脑", None).await.unwrap();
        let keys = unlock(&mut conn, &w.space, pw.clone(), None, None, true).await.unwrap();
        let Msg::AccountInfo { account } = call(&mut conn, &Msg::AccountGet).await.unwrap() else { panic!() };
        assert!(account.join, "密码够长，开通时就开着兑换码加入");

        // 第二台：同一个码 + 同步密码
        let (_, w2, k2) = join_dial("127.0.0.1", port, "", &spub, &codes[0], pw.clone(), "工作室电脑").await.unwrap();
        assert!(w2.refused.is_none());
        assert_eq!((k2.master, k2.space_key), (keys.master, keys.space_key));
        // 密码不对：服务器说「还能再试几次」
        let e = join_dial("127.0.0.1", port, "", &spub, &codes[0], "不对的同步密码啊啊".into(), "x").await.err().unwrap();
        assert!(e.contains("同步密码不对"), "{e}");
        // 开通时又填了这个码：接到「输同步密码加入」
        let e = redeem_dial("127.0.0.1", port, "", &spub, &codes[0], "x", None).await.err().unwrap();
        assert!(e.starts_with(JOINABLE_MARK), "{e}");

        // 短密码开通的空间不开兑换码加入；换成长密码（SetLock 带盒子 3）就开了
        let (dev, mut conn, w) = redeem_dial("127.0.0.1", port, "", &spub, &codes[1], "笔记本", None).await.unwrap();
        let keys = unlock(&mut conn, &w.space, "short-pw".into(), None, None, true).await.unwrap();
        let e = join_dial("127.0.0.1", port, "", &spub, &codes[1], "short-pw".into(), "x").await.err().unwrap();
        assert!(e.contains("没开"), "{e}");
        let msg = relock("换成一个长一点的密码", &keys.master, keys.space_key.as_ref(), true).unwrap();
        assert_eq!(call(&mut conn, &msg).await.unwrap(), Msg::Ok);
        drop((dev, conn));
        let (_, _, k) = join_dial("127.0.0.1", port, "", &spub, &codes[1], "换成一个长一点的密码".into(), "y").await.unwrap();
        assert_eq!(k.master, keys.master);
    }

    /// 要连一个真的 dingmi-server，平时跳过：
    /// DINGMI_TEST_PAIR=<invite --new-space 的配对码> cargo test --lib -- --ignored pair_with_real_server
    #[tokio::test]
    #[ignore]
    async fn pair_with_real_server() {
        let code: PairCode = std::env::var("DINGMI_TEST_PAIR").expect("要设 DINGMI_TEST_PAIR").parse().unwrap();
        let (h, port, spub) = (code.host.as_str(), code.port, code.server_public.as_slice());
        let (dev, _) = dingmi_sync::generate_keypair().unwrap();
        let invite = Some(code.invite.clone());
        let (mut conn, w) = dial(h, port, "", &dev, spub, "测试电脑", invite, None, PAIR_REJECTED).await.unwrap();
        assert!(w.space.salt.is_none(), "要用一个新空间的配对码（invite --new-space）");
        // 第一台电脑建空间：生成主密钥、空间密钥和恢复码
        let first = unlock(&mut conn, &w.space, "password1".into(), None, None, false).await.unwrap();
        let (master, k) = (first.master, first.space_key.unwrap());
        let recovery = crypto::parse_recovery(&first.new_recovery.unwrap()).unwrap();

        // 已经登记的电脑不用邀请码；同步密码加空间密钥打开盒子 1，密码错了、没有空间密钥都打不开
        let (mut conn, w) = dial(h, port, "", &dev, spub, "测试电脑", None, None, SESSION_REJECTED).await.unwrap();
        assert_eq!(unlock(&mut conn, &w.space, "password1".into(), Some(k), None, false).await.unwrap().master, master);
        let wrong = unlock(&mut conn, &w.space, "password2".into(), Some(k), None, false).await;
        assert_eq!(wrong.err().as_deref(), Some("同步密码不对"));
        let no_key = unlock(&mut conn, &w.space, "password1".into(), None, None, false).await;
        assert_eq!(no_key.err().as_deref(), Some(NEED_RECOVERY));

        // 另一台电脑用这台生成的配对码加入
        let Msg::Invited { token, .. } = call(&mut conn, &Msg::InviteNew).await.unwrap() else { panic!("没拿到配对码") };
        let (dev2, _) = dingmi_sync::generate_keypair().unwrap();
        let (mut c2, w2) = dial(h, port, "", &dev2, spub, "第二台", Some(token), None, PAIR_REJECTED).await.unwrap();
        assert_eq!(unlock(&mut c2, &w2.space, "password1".into(), Some(k), None, false).await.unwrap().master, master);

        // 没有配对码：凭恢复码登记新电脑，不用同步密码就拿到主密钥和空间密钥
        let (dev3, _) = dingmi_sync::generate_keypair().unwrap();
        let token = Some(crypto::recovery_token(&recovery));
        let (mut c3, w3) = dial(h, port, "", &dev3, spub, "找回的电脑", None, token, "服务器不认这个恢复码").await.unwrap();
        let got = unlock(&mut c3, &w3.space, String::new(), None, Some(recovery), false).await.unwrap();
        assert_eq!((got.master, got.space_key), (master, Some(k)));

        // 用过的配对码，别的电脑不能再用
        let (dev4, _) = dingmi_sync::generate_keypair().unwrap();
        let again = dial(h, port, "", &dev4, spub, "别的电脑", Some(code.invite), None, PAIR_REJECTED);
        assert_eq!(again.await.err().as_deref(), Some(PAIR_REJECTED));
    }
}
