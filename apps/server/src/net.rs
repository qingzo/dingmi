//! 一个端口，三种连接：看连接开头的几个字节决定交给谁。
//! - `DNGM`：叮咪电脑端，走 Noise 加密握手，只认登记过的设备
//! - `0x16`：浏览器的 HTTPS（TLS 握手）：手机网页（收件箱和手机版）
//! - `GET ` / `HEAD`：普通 HTTP，只用来下载根证书
//!
//! 其他一律断开。同一个 IP 失败 5 次（握手失败、陌生设备、手机令牌不对），屏蔽 10 分钟。
//!
//! 上面是开源版（直连）的样子。官方版在宝塔后面（[`Server::load_proxied`]）：不要 IP、不要自签证书，
//! 不用这里的 [`Server::serve`]，自己在本机端口上收普通 HTTP、按域名分好以后把同步的请求交给 `http::respond`
//! （电脑走 `wss://域名/sync`，里面跑的和直连一样，见 [`Server::device`]）。

use crate::admin;
use crate::ext::{Ext, FileLimits, Opening, SelfHosted, Who};
use crate::http;
use crate::store::{now, Store, DEVICE_INVITE_MINUTES, MAX_PENDING_INVITES};
use crate::tls::{self, Tls};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD as B64};
use base64::Engine;
use dingmi_sync::{refuse, Account, Ack, BetaInfo, Hello, Msg, Pc, PhoneAsk, PhoneJoin, Record, Refusal, Welcome, INBOX_CHUNK, MAGIC, VERSION};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, OwnedSemaphorePermit, Semaphore};
use tokio::time::timeout;
use tokio_rustls::TlsAcceptor;

/// 电脑端至少每 30 秒发一条消息，超过这么久没动静就断开
const IDLE_TIMEOUT: Duration = Duration::from_secs(90);
/// 电脑端发 Wait 后，最多等多久没有新东西就回 Pong
const WAIT: Duration = Duration::from_secs(25);
const MAX_FAILS: u32 = 5;
const BAN: Duration = Duration::from_secs(600);
/// 手机一次最多发多大（加密后）：手机网页限制一次 50 MB，加上说明和加密多出来的一点
pub const MAX_INBOX_ITEM: usize = 52 * 1024 * 1024;
/// 还没被电脑取走的最多存多少：防止手机令牌泄露后被人塞满服务器
pub const MAX_INBOX_COUNT: u64 = 1000;
pub const MAX_INBOX_TOTAL: u64 = 1024 * 1024 * 1024;
/// 一次 Pull 最多回多少（数据密文的大小 / 条数）
const PULL_BYTES: usize = 2 * 1024 * 1024;
const PULL_COUNT: usize = 500;
/// 一个附件（图片或文件，加密后）最大多大。附件本身限 50 MB，见需求文档 6.12
pub const MAX_FILE: u64 = 60 * 1024 * 1024;
/// 手机发来的东西多少天没被电脑取走就删掉（每天收尾时）：电脑一直没开，也不让它一直占着服务器
pub const INBOX_DAYS: i64 = 30;
/// 几个防滥用的上限（Server::load_with 用默认的，测试里调小）
#[derive(Clone, Debug)]
pub struct Caps {
    /// 握手最多等多久：TLS 握手、电脑的 `DNGM` + Noise 握手（直连和 WebSocket 升级来的都算）
    pub handshake: Duration,
    /// 最多同时开多少个连接（电脑、手机、反向代理转过来的都算），再多的直接关掉
    pub conns: usize,
    /// 手机网页同时最多几个上传（收件箱、附件）：上传的内容先整个收进内存，人多了会撑爆内存
    pub uploads: usize,
    /// 上传名额满了最多等多久，等不到回 503
    pub upload_wait: Duration,
    /// 手机令牌不对时，把请求体读掉（不留）最多读多久再回 401
    pub drain: Duration,
}

impl Default for Caps {
    fn default() -> Self {
        Caps {
            handshake: Duration::from_secs(10),
            conns: 4096,
            uploads: 8,
            upload_wait: Duration::from_secs(30),
            drain: Duration::from_secs(30),
        }
    }
}
/// 违规记录最多记多少个 IP：满了先清掉最早记下的（IPv6 一个人就有一大片地址，不设上限会被撑大）
const MAX_STRIKES: usize = 10_000;

pub fn log(msg: impl std::fmt::Display) {
    println!("[{}] {msg}", now());
}

pub fn hash_token(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn random_id() -> String {
    rand::random::<[u8; 16]>().iter().map(|b| format!("{b:02x}")).collect()
}

fn clean_name(name: &str) -> String {
    let n: String = name.trim().chars().filter(|c| !c.is_control()).take(40).collect();
    if n.is_empty() {
        "未命名设备".into()
    } else {
        n
    }
}

/// 附件放在 files/<空间>/<附件编号>
pub fn space_files(dir: &Path, space: &str) -> PathBuf {
    dir.join("files").join(space)
}

/// 手机发来、还没被电脑取走的放在 inbox/<编号>
pub fn inbox_path(dir: &Path, id: &str) -> PathBuf {
    dir.join("inbox").join(id)
}

/// 说不行：协议 5 起的电脑回 Refused（带 code，电脑按它区分），旧电脑回它认识的 Error。
/// 这是按规矩拒绝（停用、满了、到期……），不是服务器出错（那个是 Server::err，会告诉扩展点）
fn refused(v: u32, code: &str, msg: &str) -> Msg {
    if v >= REFUSED_SINCE {
        Msg::Refused { code: code.into(), msg: msg.into() }
    } else {
        Msg::Error { msg: msg.into() }
    }
}

#[derive(Default)]
struct Strike {
    fails: u32,
    until: Option<Instant>,
}

/// 违规记录：IP → 失败几次、屏蔽到什么时候；`order` 是记下的先后，满了（MAX_STRIKES）从最早的清，不用在锁里全表扫
#[derive(Default)]
struct Strikes {
    map: HashMap<IpAddr, Strike>,
    order: VecDeque<IpAddr>,
}

/// 按什么记违规：IPv4 就是它自己；IPv6 按 /64 记（一家人、一台服务器通常就有一整个 /64，换着地址试也算同一个）
fn strike_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let s = v6.segments();
                IpAddr::V6(std::net::Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
            }
        },
        v4 => v4,
    }
}

/// 叫醒正在 Wait 的电脑的原因
#[derive(Clone)]
enum News {
    /// 手机发来了东西
    Inbox,
    /// 某台设备上传了记录：电脑是设备公钥，手机是 "phone:<手机 id>"
    Records(String),
}

/// 记录编号：字母、数字、- 和 _
fn valid_key(uid: &str) -> bool {
    !uid.is_empty() && uid.len() <= 64 && uid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// 图片编号会拼进文件路径，只认 UUID 那样的十六进制和 -
fn valid_file(uid: &str) -> bool {
    !uid.is_empty() && uid.len() <= 64 && uid.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// 手机发来的东西没收下的原因
pub enum Reject {
    /// 令牌不对或者手机已作废
    Token,
    /// 收件箱满了
    Full,
    /// 内容不像加密过的数据
    Bad,
    Server,
    /// 空间满了（管理员设了容量上限）
    Quota,
    /// 单个附件超过了上限（扩展点的 FileLimits，带着给人看的话）
    TooBig(String),
    /// 空间停用了
    Off,
    /// 扩展点说这个空间停了（叮咪同步到期，带着给人看的话）
    Stopped(String),
}

/// 附件接着传对不上（见 Server::file_put）时回的话：电脑这一轮同步出错，下一轮从头传这个附件
pub const FILE_RESTART: &str = "附件要从头传";

/// 协议 5 起才有的：说得清的拒绝原因（Msg::Refused、Welcome.refused）
const REFUSED_SINCE: u32 = 5;

/// 扩展点说空间停了（Ext::stopped）时还能做的：问状态，和交给扩展点自己处理的那几条（续费、客服核对码、邮箱）
fn ok_when_stopped(msg: &Msg) -> bool {
    matches!(msg, Msg::Ping | Msg::AccountGet) || ext_msg(msg)
}

/// 交给扩展点处理的消息（Ext::handle）：叮咪同步才有，开源版回「这台服务器不支持」
fn ext_msg(msg: &Msg) -> bool {
    matches!(msg, Msg::Renew { .. } | Msg::SupportNew | Msg::EmailSet { .. } | Msg::EmailDel | Msg::SpaceDelete { .. })
}

/// 没登记过的设备没登记成的原因（Server::register）
enum Fail {
    /// 兑换码加入的第一步：回 salt
    JoinSalt(String),
    /// 和以前一样：什么也不回，直接断开，算一次违规
    Unknown,
    /// 说得清的原因：协议 5 起的电脑能收到，旧电脑照旧直接断开
    Refused(Refusal),
    /// 只是问问公测（扩展点回的），回完断开
    Beta(BetaInfo),
}

/// 见 Server.log_hook：(类型, 做了什么, 说明)
pub type LogHook = Box<dyn Fn(&str, &str, &str) + Send + Sync>;

/// 直连要的（开源版）：服务器的 IP、自签证书。官方版在宝塔后面（Server::load_proxied），没有这些：证书全在宝塔那边
struct Direct {
    ip: IpAddr,
    tls: Tls,
    mobileconfig: Arc<String>,
    ca_der: Vec<u8>,
    ca_fingerprint: String,
}

pub struct Server {
    dir: PathBuf,
    pub store: Store,
    noise_private: Vec<u8>,
    /// 开源版（直连）才有；官方版在宝塔后面是 None
    direct: Option<Direct>,
    strikes: Mutex<Strikes>,
    pub(crate) caps: Caps,
    /// 同时开着的连接（Caps.conns 个名额）：每个连接拿一个，断开时还回去
    conns: Arc<Semaphore>,
    /// 手机网页同时在收的上传（Caps.uploads 个名额）
    pub(crate) uploads: Arc<Semaphore>,
    /// 登记新设备、添加手机时：「还能不能再加（Ext::room）」和真正加进去之间不让别的登记插进来，免得一起登记的超过上限
    admit: Mutex<()>,
    /// 每个空间一个通知频道：手机发来新东西、别的电脑上传了记录时，叫醒正在 Wait 的电脑
    news: Mutex<HashMap<String, broadcast::Sender<News>>>,
    /// 现在连着的电脑：公钥 → 连接数（一台电脑同时会开好几个连接：等消息的、上传的、下载附件的）
    online: Mutex<HashMap<String, usize>>,
    /// 什么时候启动的（Unix 秒），后台概况用
    pub started_at: i64,
    /// 官方版的后台设了它：自动做的事（停用满期删掉空间）也记进后台的操作日志。参数：类型、做了什么、说明
    pub log_hook: std::sync::OnceLock<LogHook>,
    /// 扩展点（ext.rs）：开源版是 SelfHosted，官方版编译时换成自己的
    ext: Arc<dyn Ext>,
    /// 用手机加入（需求-1.x 第 34.3 节）：8 位数字 → 等手机允许的那台电脑。只在内存里，10 分钟过期
    phone_reqs: Mutex<HashMap<String, PhoneReq>>,
    /// 手机输错数字的次数：手机编号 → (次数, 从什么时候数起)
    phone_misses: Mutex<HashMap<String, (u32, Instant)>>,
    /// 手机允许时生成的邀请码（哈希 → 什么时候过期）：凭它登记的连接日志记「用手机加入」，不记「配对」。过期的下次加的时候清掉
    phone_invites: Mutex<HashMap<String, Instant>>,
}

/// 等手机允许的一台电脑（Hello.phone_ask 建、手机网页允许或拒绝、Hello.phone_poll 取走）
struct PhoneReq {
    key: String,
    name: String,
    ip: IpAddr,
    until: Instant,
    expires_at: String,
    /// 手机允许了：(邀请码, 加密好的空间密钥)
    ok: Option<(String, String)>,
    declined: bool,
    /// 哪部手机看过这个数字（GET）：只有它能允许、拒绝
    seen_by: Option<String>,
}

/// 用手机加入的数字多久有效
const PHONE_JOIN_TTL: Duration = Duration::from_secs(600);
/// 电脑等手机允许，一次最多等多久（再久就回「还在等」，电脑再问一次）
const PHONE_POLL_WAIT: Duration = Duration::from_secs(20);
/// 同一个 IP 同时最多有几个在等的
const PHONE_REQS_PER_IP: usize = 5;
const MAX_PHONE_REQS: usize = 10_000;
/// 一部手机 10 分钟内最多输错几次数字
const PHONE_MAX_MISSES: u32 = 10;

/// 连着的时候记在 Server.online 里，断开时（不管怎么断的）减掉
struct Online<'a> {
    server: &'a Server,
    key: String,
}

impl Drop for Online<'_> {
    fn drop(&mut self) {
        let mut m = self.server.online.lock().unwrap();
        if let Some(n) = m.get_mut(&self.key) {
            *n -= 1;
            if *n == 0 {
                m.remove(&self.key);
            }
        }
    }
}

impl Server {
    /// 开源版：扩展点用 SelfHosted（什么都不做）
    pub fn load(dir: &Path) -> Result<Arc<Server>, String> {
        Self::load_with(dir, Arc::new(SelfHosted))
    }

    /// 带着扩展点起来（官方版用）。数据库打开以后先让扩展点建它的表（Ext::open），出错就不启动
    pub fn load_with(dir: &Path, ext: Arc<dyn Ext>) -> Result<Arc<Server>, String> {
        Self::load_caps(dir, ext, Caps::default())
    }

    /// 同上，自己定几个上限（测试用）
    pub fn load_caps(dir: &Path, ext: Arc<dyn Ext>, caps: Caps) -> Result<Arc<Server>, String> {
        Self::build(dir, ext, caps, true)
    }

    /// 官方版：在宝塔后面，只收本机转过来的普通 HTTP。不要 IP、自签证书、根证书（init 也就不用 --ip），
    /// 别的（数据库、服务器密钥、扩展点、防滥用的上限）和直连的一样。连接自己收（不用 [`Server::serve`]），
    /// 每个连接先拿一个名额（[`Server::conn_slot`]），同步的请求交给 `http::respond`
    pub fn load_proxied(dir: &Path, ext: Arc<dyn Ext>) -> Result<Arc<Server>, String> {
        Self::build(dir, ext, Caps::default(), false)
    }

    /// `direct`：要不要直连的那一套（IP、自签证书）
    fn build(dir: &Path, ext: Arc<dyn Ext>, caps: Caps, direct: bool) -> Result<Arc<Server>, String> {
        let store = Store::open(dir)?;
        ext.open(&store)?;
        if !ext.joins() {
            let n = store.clear_joins();
            if n > 0 {
                log(format!("清掉了 {n} 个空间的盒子 3（自建服务器用不了兑换码加入，不存它）"));
            }
        }
        let ip: Option<IpAddr> = match direct {
            true => Some(
                store
                    .kv_get("ip")
                    .and_then(|s| s.parse().ok())
                    .ok_or("还没有初始化，先执行 dingmi-server init --ip <公网IP>")?,
            ),
            false => None,
        };
        let noise_private = store
            .kv_get("noise_private")
            .and_then(|s| B64.decode(s).ok())
            .ok_or("找不到服务器密钥，先执行 dingmi-server init")?;
        let direct = match ip {
            Some(ip) => {
                let tls = Tls::load(dir, ip, &store)?;
                let ca = tls::ca_der(dir)?;
                Some(Direct {
                    ip,
                    tls,
                    mobileconfig: Arc::new(tls::mobileconfig(&ca)),
                    ca_fingerprint: tls::fingerprint(&ca),
                    ca_der: ca,
                })
            }
            None => None,
        };
        // 这份数据的「代」，见 Welcome.epoch；0.4 之前装的服务器没有，第一次启动时补上
        if store.kv_get("epoch").is_none_or(|e| e.is_empty()) {
            store.kv_set("epoch", &random_id()).map_err(|e| e.to_string())?;
        }
        Ok(Arc::new(Server {
            dir: dir.into(),
            store,
            noise_private,
            direct,
            strikes: Mutex::new(Strikes::default()),
            conns: Arc::new(Semaphore::new(caps.conns)),
            uploads: Arc::new(Semaphore::new(caps.uploads)),
            caps,
            admit: Mutex::new(()),
            news: Mutex::new(HashMap::new()),
            online: Mutex::new(HashMap::new()),
            phone_reqs: Mutex::new(HashMap::new()),
            phone_misses: Mutex::new(HashMap::new()),
            phone_invites: Mutex::new(Default::default()),
            started_at: chrono::Local::now().timestamp(),
            log_hook: std::sync::OnceLock::new(),
            ext,
        }))
    }

    /// 数据目录
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 现在连着的电脑（公钥）
    pub fn online(&self) -> Vec<String> {
        self.online.lock().unwrap().keys().cloned().collect()
    }

    /// iPhone 装根证书用的描述文件；官方版在宝塔后面（没有自签证书）是 None
    pub fn mobileconfig(&self) -> Option<&str> {
        self.direct.as_ref().map(|d| d.mobileconfig.as_str())
    }

    /// 根证书本身（DER），给安卓手机安装用；官方版是 None
    pub fn ca_der(&self) -> Option<&[u8]> {
        self.direct.as_ref().map(|d| d.ca_der.as_slice())
    }

    /// 在宝塔后面（官方版，Server::load_proxied）
    pub fn proxied(&self) -> bool {
        self.direct.is_none()
    }

    /// 一个连接的名额（Caps.conns）：官方版自己收连接时，每个连接先拿一个，满了是 None（直接关掉那个连接）。
    /// 名额跟着连接走，断开时还回去；升级成 WebSocket 的交给那条连接带走（见 `http::respond`）
    pub fn conn_slot(&self) -> Option<OwnedSemaphorePermit> {
        self.conns.clone().try_acquire_owned().ok()
    }

    pub async fn serve(self: Arc<Self>, listener: TcpListener) {
        loop {
            match listener.accept().await {
                Ok((sock, addr)) => {
                    // 名额满了直接关掉（sock 在这里丢掉）；名额跟着连接走，断开时还回去
                    let Ok(permit) = self.conns.clone().try_acquire_owned() else {
                        continue;
                    };
                    let s = self.clone();
                    tokio::spawn(async move { s.handle(sock, addr.ip(), permit).await });
                }
                Err(e) => {
                    log(format!("接受连接失败：{e}"));
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
    }

    /// 每小时看一次：每天备份数据库（留几份见 admin::BACKUP_DAYS）、清掉过期的邀请码、删掉停用满了天数的空间、证书快到期就换新
    pub async fn daily(self: Arc<Self>) {
        loop {
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            if self.store.kv_get("last_daily").as_deref() != Some(today.as_str()) {
                let _ = self.store.kv_set("last_daily", &today);
                let keep = admin::days(&self.store, &admin::BACKUP_DAYS) as usize;
                let backup = self.store.backup(&self.dir, &today, keep);
                if let Err(e) = &backup {
                    log(format!("备份失败：{e}"));
                }
                self.store.purge_invites();
                // 7 天前开始传、一直没传完的附件
                let before = (chrono::Local::now() - chrono::TimeDelta::days(7)).format("%Y-%m-%d %H:%M:%S").to_string();
                for (space, uid) in self.store.stale_uploads(&before) {
                    if valid_file(&uid) {
                        let _ = std::fs::remove_file(self.file_path(&space, &uid));
                    }
                }
                // 手机发来、INBOX_DAYS 天都没被电脑取走的
                let n = self.purge_inbox(&(chrono::Local::now() - chrono::TimeDelta::days(INBOX_DAYS)).format("%Y-%m-%d %H:%M:%S").to_string());
                if n > 0 {
                    log(format!("手机发来的 {n} 条超过 {INBOX_DAYS} 天没被电脑取走，删掉了"));
                }
                let days = admin::days(&self.store, &admin::DELETE_DAYS);
                for (space, label) in admin::purge_due_with(&self.store, &self.dir, |sp| self.ext.may_purge(&self.store, sp)) {
                    log(format!("停用满 {days} 天，删掉了空间 {space}"));
                    if let Some(f) = self.log_hook.get() {
                        f("space", "删除空间", &format!("「{label}」停用满 {days} 天，连数据一起删掉了"));
                    }
                }
                // 官方版在宝塔后面，没有自签证书，不用换
                if let Some(d) = &self.direct {
                    match d.tls.renew(&self.dir, d.ip, &self.store) {
                        Ok(true) => log("HTTPS 证书已换新"),
                        Ok(false) => {}
                        Err(e) => log(format!("换 HTTPS 证书失败：{e}")),
                    }
                }
                // 叮咪同步（官方版）：到期删除、外部备份……开源版什么都不做
                self.ext.daily(&self, backup);
            }
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    }

    /// `permit` 是这个连接占的名额（Caps.conns）：这里返回时还回去；升级成 WebSocket 的交给那条连接带走。
    /// 只给直连（开源版）用：官方版在宝塔后面不走这里，自己收连接（见文件开头）
    async fn handle(self: Arc<Self>, sock: TcpStream, ip: IpAddr, permit: OwnedSemaphorePermit) {
        let Some(direct) = &self.direct else {
            return;
        };
        if self.banned(ip) {
            return;
        }
        let Ok(Ok(head)) = timeout(self.caps.handshake, peek_head(&sock)).await else {
            return;
        };
        if &head == MAGIC {
            self.device(sock, ip).await;
        } else if head[0] == 0x16 {
            // 没装证书的手机会在这一步失败，这是正常的，不算违规
            let acceptor = TlsAcceptor::from(direct.tls.config());
            if let Ok(Ok(tls)) = timeout(self.caps.handshake, acceptor.accept(sock)).await {
                http::serve(tls, true, ip, self.clone(), permit).await;
            }
        } else if head.starts_with(b"GET ") || head.starts_with(b"HEAD") {
            http::serve(sock, false, ip, self.clone(), permit).await;
        } else {
            self.strike(ip);
        }
    }

    /// 叮咪电脑端：握手、核对设备，然后一条一条回它发来的消息
    /// `sock` 是直连的 TCP，或者 `wss://域名/sync` 升级来的 WebSocket（http.rs），里面跑的东西一样
    /// 读开头的 `DNGM` 和握手一起限时（Caps.handshake）：WebSocket 升级来的连接，以前连上以后什么都不发就一直占着
    pub(crate) async fn device<S: AsyncRead + AsyncWrite + Unpin + Send>(&self, mut sock: S, ip: IpAddr) {
        let handshake = async {
            let mut magic = [0u8; 4];
            sock.read_exact(&mut magic).await.ok().filter(|_| &magic == MAGIC)?;
            dingmi_sync::accept(sock, &self.noise_private).await.ok()
        };
        let Ok(Some(pending)) = timeout(self.caps.handshake, handshake).await else {
            self.strike(ip);
            return;
        };
        let pubkey = B64.encode(&pending.remote);
        let v = pending.hello.version;
        // 用手机加入（协议 7）：要数字、等手机允许，回完就断开；电脑这时还没登记，用的是临时的设备钥匙
        if let Some(ask) = pending.hello.phone_ask.clone() {
            let w = match self.phone_ask(ask, ip) {
                Ok(phone) => Welcome { version: VERSION, phone: Some(phone), ..Default::default() },
                Err(why) => Welcome { version: VERSION, refused: Some(why), ..Default::default() },
            };
            let _ = pending.welcome(&w).await;
            return;
        }
        if let Some(ask) = pending.hello.phone_poll.clone() {
            let w = match self.phone_poll(&ask).await {
                Ok(phone) => Welcome { version: VERSION, phone: Some(phone), ..Default::default() },
                Err(why) => Welcome { version: VERSION, refused: Some(why), ..Default::default() },
            };
            let _ = pending.welcome(&w).await;
            return;
        }
        // joined：这次连接是凭兑换码加入登记进来的（扩展点核对过加入核对值）。只有这一次 Welcome 里带盒子 3；
        // 已经登记过的设备 Hello 里带了 join 也不给（不然拿一个不带空间密钥的配对码登记的设备也能要到盒子 3，离线猜同步密码）
        let (space_id, joined) = match self.store.device(&pubkey) {
            Some(d) if !d.revoked => (d.space_id, false),
            _ => match self.register(&pending.hello, &pubkey, ip) {
                Ok((space, by_ext)) => (space, by_ext && pending.hello.join.is_some()),
                // 开通前问问公测开没开（协议 6）：回完就断开，不登记、不算违规
                Err(Fail::Beta(beta)) => {
                    let _ = pending.welcome(&Welcome { version: VERSION, beta: Some(beta), ..Default::default() }).await;
                    return;
                }
                // 兑换码加入的第一步（协议 7）：回 salt 就断开，不登记、不算违规
                Err(Fail::JoinSalt(salt)) => {
                    let _ = pending.welcome(&Welcome { version: VERSION, join_salt: Some(salt), ..Default::default() }).await;
                    return;
                }
                // 和以前一样：什么也不回，直接断开，算一次违规
                Err(Fail::Unknown) => {
                    log(format!("拒绝了一个没有登记的设备（{ip}）"));
                    self.ext.event(self, ip, None, Some(&pubkey), "被拒：没有登记的设备");
                    self.strike(ip);
                    return;
                }
                // 说得清的原因：协议 5 起的电脑能收到，旧电脑照旧直接断开
                Err(Fail::Refused(why)) => {
                    log(format!("没有登记新设备（{ip}）：{}", why.msg));
                    self.ext.event(self, ip, None, Some(&pubkey), &format!("被拒：{}", why.msg));
                    if v >= REFUSED_SINCE {
                        let _ = pending.welcome(&Welcome { version: VERSION, refused: Some(why), ..Default::default() }).await;
                    }
                    return;
                }
            },
        };
        self.store.touch_device(&pubkey);
        self.ext.event(self, ip, Some(&space_id), Some(&pubkey), "连上");
        let v2 = v >= 2;
        // 兑换码加入登记的这一次多带盒子 3（新电脑用同步密码打开它拿到空间密钥）；平时不带
        let welcome = Welcome {
            version: VERSION,
            space: if joined { self.store.space_with_box3(&space_id) } else { self.store.space(&space_id) },
            // 官方版在宝塔后面没有自签根证书：空的（电脑看 web_url，手机网页走域名，不用装证书）
            ca_fingerprint: self.direct.as_ref().map(|d| d.ca_fingerprint.clone()).unwrap_or_default(),
            web_url: self.store.kv_get("web_url").unwrap_or_default(),
            epoch: self.store.kv_get("epoch").unwrap_or_default(),
            account: Some(self.account(&space_id)),
            refused: None,
            beta: None,
            join_salt: None,
            phone: None,
        };
        let Ok(mut conn) = pending.welcome(&welcome).await else {
            return;
        };
        // 连上就订阅：两次 Wait 之间到的通知会先攒在这里，不会漏
        let mut news = self.subscribe(&space_id);
        *self.online.lock().unwrap().entry(pubkey.clone()).or_default() += 1;
        let _online = Online { server: self, key: pubkey.clone() };
        loop {
            let Ok(Ok(msg)) = timeout(IDLE_TIMEOUT, conn.recv::<Msg>()).await else {
                break;
            };
            // 连着的时候被作废、空间被停用（命令行或后台），下一条消息就断开
            if let Err(why) = self.still_ok(&pubkey, &space_id) {
                let off = why == admin::SPACE_OFF;
                let _ = conn.send(&if off { refused(v, refuse::SPACE_OFF, why) } else { self.err(why) }).await;
                break;
            }
            // 扩展点说空间停了（叮咪同步到期）：停止同步，只能问状态和交给扩展点的那几条（续费、客服核对码、邮箱）
            let stop = if ok_when_stopped(&msg) { None } else { self.ext.stopped(self, &space_id) };
            let reply = match (stop, msg) {
                (Some(r), _) => refused(v, &r.code, &r.msg),
                (None, Msg::Wait) => self.wait(&mut news, &pubkey, v2).await,
                (None, other) => self.reply(&space_id, &pubkey, ip, v, other),
            };
            if conn.send(&reply).await.is_err() {
                break;
            }
        }
    }

    /// 没登记过的设备要登记：凭配对码里的邀请码、恢复凭证（没有电脑能生成配对码时）；
    /// 都没带（或者只问公测）就问扩展点（Ext::open_space，叮咪同步用兑换码、公测开通新空间、兑换码加入）。
    /// 回 (空间, 是不是扩展点放进来的)
    fn register(&self, hello: &Hello, pubkey: &str, ip: IpAddr) -> Result<(String, bool), Fail> {
        let say = |code: &str, msg: &str| Fail::Refused(Refusal { code: code.into(), msg: msg.into() });
        let (space, how) = match (&hello.invite, &hello.recovery) {
            (Some(t), _) if !hello.ask_beta => (self.store.invite_space(&hash_token(t)).ok_or(Fail::Unknown)?, Some("配对")),
            (None, Some(r)) if !hello.ask_beta => {
                (self.store.space_by_recovery(&hash_token(r)).ok_or(Fail::Unknown)?, Some("用恢复码登记"))
            }
            // 开通的那一条连接日志扩展点自己记（带着套餐和天数）
            _ => match self.ext.open_space(self, hello, pubkey, ip) {
                Opening::Space(space) => (space, None),
                Opening::Beta(beta) => return Err(Fail::Beta(beta)),
                Opening::JoinSalt(salt) => return Err(Fail::JoinSalt(salt)),
                Opening::Refused(why) => return Err(Fail::Refused(why)),
                Opening::Unknown => return Err(Fail::Unknown),
            },
        };
        if self.store.disabled_at(&space).is_some() {
            return Err(say(refuse::SPACE_OFF, admin::SPACE_OFF));
        }
        if let Some(why) = self.ext.stopped(self, &space) {
            return Err(Fail::Refused(why));
        }
        // 从问「还能不能加」到真正加进去，别的登记、添加手机等着（不然同时登记的几台都看到还有空位）
        let _admit = self.admit.lock().unwrap();
        self.ext.room(self, &space).map_err(Fail::Refused)?;
        // 邀请码到这里才用掉：设备满了被拒的，作废一台后还能接着用这个配对码
        if hello.invite.is_some() && self.store.take_invite(&hash_token(hello.invite.as_deref().unwrap_or_default())).is_none() {
            return Err(Fail::Unknown);
        }
        let name = clean_name(&hello.name);
        self.store.add_device(pubkey, &space, &name).map_err(|_| Fail::Unknown)?;
        let by_phone = hello.invite.as_deref().is_some_and(|t| self.phone_invites.lock().unwrap().remove(&hash_token(t)).is_some());
        let how = if by_phone { Some("用手机加入") } else { how };
        log(format!("新设备登记好了：{name}（{ip}）"));
        let by_ext = how.is_none();
        if let Some(how) = how {
            self.ext.event(self, ip, Some(&space), Some(pubkey), how);
        }
        Ok((space, by_ext))
    }

    /// 回给电脑一条 Error：服务器这边出错了（不是按规矩拒绝）。告诉扩展点（官方版数「出错突然变多」）
    fn err(&self, msg: &str) -> Msg {
        self.ext.error();
        Msg::Error { msg: msg.into() }
    }

    /// 这个空间的账（Welcome.account、AccountGet）：服务器自己算用了多少、容量上限、满没满、几台设备，再交给扩展点
    pub fn account(&self, space: &str) -> Account {
        let quota = self.store.quota(space);
        let (r, f) = self.store.usage(space);
        let used = r + f;
        let base = Account {
            used,
            quota,
            storage: admin::storage(used, quota).into(),
            devices: self.store.device_count(space),
            ..Default::default()
        };
        let mut a = self.ext.account(self, space, base);
        a.join = self.store.join_lock(space).is_some();
        a.join_off = self.store.join_turned_off(space);
        a
    }

    /// 用手机加入的第一步（Hello.phone_ask）：记下这台电脑的临时公钥，给一个 8 位数字。
    /// 同一个 IP（IPv6 按 /64，和违规记录一样）同时最多 PHONE_REQS_PER_IP 个：不然换着地址就能把名额占满
    fn phone_ask(&self, ask: PhoneAsk, ip: IpAddr) -> Result<PhoneJoin, Refusal> {
        let gone = |msg: &str| Refusal { code: refuse::PHONE_GONE.into(), msg: msg.into() };
        if STANDARD.decode(&ask.key).map(|k| k.len()).ok() != Some(32) {
            return Err(gone("这台电脑的临时钥匙不对"));
        }
        let now = Instant::now();
        let mut m = self.phone_reqs.lock().unwrap();
        m.retain(|_, r| r.until > now);
        let net = strike_key(ip);
        if m.len() >= MAX_PHONE_REQS || m.values().filter(|r| strike_key(r.ip) == net).count() >= PHONE_REQS_PER_IP {
            return Err(gone("要的数字太多了，过几分钟再试"));
        }
        let code = loop {
            let c = format!("{:08}", rand::random::<u32>() % 100_000_000);
            if !m.contains_key(&c) {
                break c;
            }
        };
        let expires_at = (chrono::Local::now() + chrono::TimeDelta::seconds(PHONE_JOIN_TTL.as_secs() as i64))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let req = PhoneReq {
            key: ask.key,
            name: clean_name(&ask.name),
            ip,
            until: now + PHONE_JOIN_TTL,
            expires_at: expires_at.clone(),
            ok: None,
            declined: false,
            seen_by: None,
        };
        m.insert(code.clone(), req);
        Ok(PhoneJoin { code, expires_at, invite: None, sealed: None })
    }

    /// 电脑等手机允许（Hello.phone_poll）：最多等 PHONE_POLL_WAIT；允许了带上邀请码和加密好的空间密钥。
    /// 取走了也先留着（到过期为止）：交给电脑的那一下断了，电脑再问一次还能拿到，不用重新要数字
    async fn phone_poll(&self, ask: &PhoneAsk) -> Result<PhoneJoin, Refusal> {
        let gone = |msg: &str| Refusal { code: refuse::PHONE_GONE.into(), msg: msg.into() };
        let deadline = Instant::now() + PHONE_POLL_WAIT;
        loop {
            {
                let m = self.phone_reqs.lock().unwrap();
                let Some(r) = m.get(&ask.code).filter(|r| r.key == ask.key && r.until > Instant::now()) else {
                    return Err(gone("这个数字过期了，再要一个"));
                };
                if r.declined {
                    return Err(gone("手机上点了「不是我」，这次没加进去"));
                }
                if let Some((invite, sealed)) = r.ok.clone() {
                    return Ok(PhoneJoin { code: ask.code.clone(), expires_at: r.expires_at.clone(), invite: Some(invite), sealed: Some(sealed) });
                }
                if Instant::now() >= deadline {
                    return Ok(PhoneJoin { code: ask.code.clone(), expires_at: r.expires_at.clone(), invite: None, sealed: None });
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// 这部手机 10 分钟内数字输错太多次了（看数字、允许、拒绝一起算）
    fn phone_locked(misses: &HashMap<String, (u32, Instant)>, phone: &str, now: Instant) -> bool {
        misses.get(phone).is_some_and(|(n, since)| *n >= PHONE_MAX_MISSES && now - *since < PHONE_JOIN_TTL)
    }

    /// 这部手机数字输错了一次
    fn phone_missed(misses: &mut HashMap<String, (u32, Instant)>, phone: &str, now: Instant) {
        if misses.len() > 10_000 {
            misses.retain(|_, (_, since)| now - *since < PHONE_JOIN_TTL);
        }
        let e = misses.entry(phone.to_string()).or_insert((0, now));
        if now - e.1 >= PHONE_JOIN_TTL {
            *e = (0, now);
        }
        e.0 += 1;
    }

    /// 手机网页输了数字（`GET /m/api/join/<数字>`）：回这台电脑的 (名字, 临时公钥)，记下是这部手机看的（只有它能允许、拒绝）。
    /// 数字不对记一次，一部手机 10 分钟内错太多回 Err(true)，平常的不对是 Err(false)
    pub fn phone_req(&self, phone: &str, code: &str) -> Result<(String, String), bool> {
        let now = Instant::now();
        let mut misses = self.phone_misses.lock().unwrap();
        if Self::phone_locked(&misses, phone, now) {
            return Err(true);
        }
        if let Some(r) = self.phone_reqs.lock().unwrap().get_mut(code).filter(|r| r.until > now && r.ok.is_none() && !r.declined) {
            r.seen_by = Some(phone.to_string());
            return Ok((r.name.clone(), r.key.clone()));
        }
        Self::phone_missed(&mut misses, phone, now);
        Err(false)
    }

    /// 手机上点了「一样，允许」（带着加密好的空间密钥）或「不是我」（sealed 是 None）。允许时生成邀请码，
    /// 电脑凭它登记（照样要过停用、到期、设备上限）。
    /// 只认刚用 GET 看过这个数字的那部手机：不然拿着随便哪个手机令牌就能一直 POST 扫数字，搅黄别人的加入；
    /// 对不上和看数字时一样记一次输错。Err(true) 是输错太多次了
    pub fn phone_answer(&self, ip: IpAddr, space: &str, phone: &str, code: &str, sealed: Option<String>) -> Result<(), (bool, String)> {
        let now = Instant::now();
        let mut misses = self.phone_misses.lock().unwrap();
        if Self::phone_locked(&misses, phone, now) {
            return Err((true, "数字输错太多次了，过 10 分钟再试".into()));
        }
        let mut m = self.phone_reqs.lock().unwrap();
        let Some(r) = m.get_mut(code).filter(|r| r.until > now && r.ok.is_none() && !r.declined && r.seen_by.as_deref() == Some(phone)) else {
            Self::phone_missed(&mut misses, phone, now);
            return Err((false, "这个数字过期了，让电脑上再要一个".into()));
        };
        drop(misses);
        let Some(sealed) = sealed else {
            r.declined = true;
            self.ext.event(self, ip, Some(space), Some(phone), &format!("用手机加入：拦下了「{}」", r.name));
            return Ok(());
        };
        if let Err(why) = self.ext.room(self, space) {
            return Err((false, why.msg));
        }
        let (invite, _) = self.store.new_invite(space, DEVICE_INVITE_MINUTES).map_err(|_| (false, "服务器出了点问题，再试一次".to_string()))?;
        self.store.trim_invites(space, MAX_PENDING_INVITES);
        self.ext.event(self, ip, Some(space), Some(phone), &format!("用手机加入：允许了「{}」", r.name));
        let mut invites = self.phone_invites.lock().unwrap();
        invites.retain(|_, until| *until > now);
        invites.insert(hash_token(&invite), now + Duration::from_secs(DEVICE_INVITE_MINUTES as u64 * 60));
        r.ok = Some((invite, sealed));
        Ok(())
    }
    /// 还能不能再加一台设备（扩展点定，开源版不限）：手机网页生成配对码前问
    pub fn room(&self, space: &str) -> Result<(), String> {
        self.ext.room(self, space).map_err(|r| r.msg)
    }

    /// 记一条连接日志（扩展点，开源版不记）
    pub fn event(&self, ip: IpAddr, space: &str, device: &str, what: &str) {
        self.ext.event(self, ip, Some(space), Some(device), what);
    }

    /// 单个附件的限额（扩展点定，开源版不限）
    pub fn file_limits(&self, space: &str) -> FileLimits {
        self.ext.file_limits(self, space)
    }

    /// 空间满了时给人看的话
    pub fn full_msg(&self, space: &str) -> String {
        admin::full_msg(&self.store, space, self.ext.full_hint(self))
    }

    /// 空间满了、手机发来的收不下时给手机看的话
    pub fn inbox_full_msg(&self, space: &str) -> String {
        let q = self.store.quota(space).map(admin::fmt_size).unwrap_or_default();
        format!("空间满了（上限 {q}），这一条收不下。打开电脑上的叮咪取走手机发来的、删掉一些附件，{}", self.ext.full_hint(self))
    }

    fn still_ok(&self, pubkey: &str, space: &str) -> Result<(), &'static str> {
        if !self.store.device(pubkey).is_some_and(|d| !d.revoked && d.space_id == space) {
            return Err("这台电脑已经作废了，要用的话重新配对");
        }
        if self.store.disabled_at(space).is_some() {
            return Err(admin::SPACE_OFF);
        }
        Ok(())
    }

    /// 等到有这台电脑该知道的新东西，或者 25 秒到了回 Pong。
    /// 自己上传的记录不叫醒自己；版本 1 的电脑端不同步记录，只为手机发来的东西叫醒它
    async fn wait(&self, news: &mut broadcast::Receiver<News>, me: &str, v2: bool) -> Msg {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let wake = match tokio::time::timeout_at(deadline, news.recv()).await {
                Err(_) | Ok(Err(broadcast::error::RecvError::Closed)) => return Msg::Pong,
                // 通知太多漏掉了一些，保险起见当成有新东西
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => true,
                Ok(Ok(News::Inbox)) => true,
                Ok(Ok(News::Records(from))) => v2 && from != me,
            };
            if wake {
                while news.try_recv().is_ok() {}
                return if v2 { Msg::Changed } else { Msg::InboxNotify };
            }
        }
    }

    /// `v` 是这台电脑的协议版本：5 起说不行时回 Refused（带 code），旧的回 Error
    fn reply(&self, space: &str, me: &str, ip: IpAddr, v: u32, msg: Msg) -> Msg {
        let full = || refused(v, refuse::FULL, &self.full_msg(space));
        match msg {
            Msg::Ping => Msg::Pong,
            // 叮咪同步才有的（续费、客服核对码、邮箱）：交给扩展点，开源版回「这台服务器不支持」
            m if ext_msg(&m) => match self.ext.handle(self, &Who { space, device: me, ip }, m) {
                Ok(reply) => reply,
                Err(r) => refused(v, &r.code, &r.msg),
            },
            Msg::AccountGet => Msg::AccountInfo { account: self.account(space) },
            Msg::SetupSpace { salt, box1, box2, recovery, join } => {
                // 自建服务器（Ext::joins 是 false）不存盒子 3
                let join = join.filter(|_| self.ext.joins()).map(|j| (j.box3, hash_token(&j.proof)));
                if self.store.setup_space(space, &salt, &box1, &box2, &hash_token(&recovery), join.as_ref().map(|(b, h)| (b.as_str(), h.as_str()))) {
                    Msg::Ok
                } else {
                    self.err("这个空间已经设过同步密码了")
                }
            }
            Msg::SetLock { salt, box1, join } => {
                let join = join.filter(|_| self.ext.joins()).map(|j| (j.box3, hash_token(&j.proof)));
                if self.store.set_lock(space, &salt, &box1, join.as_ref().map(|(b, h)| (b.as_str(), h.as_str()))) {
                    log("换了同步密码");
                    Msg::Ok
                } else {
                    self.err("这个空间还没设同步密码")
                }
            }
            Msg::JoinSet { join, salt } => {
                if !self.ext.joins() {
                    return refused(v, refuse::REDEEM_OFF, crate::ext::JOIN_OFF_MSG);
                }
                if self.store.set_join(space, Some((&join.box3, &hash_token(&join.proof))), salt.as_deref()) {
                    log("打开了兑换码加入");
                    self.ext.event(self, ip, Some(space), Some(me), "打开兑换码加入");
                    Msg::Ok
                } else if salt.is_some() && self.store.space(space).salt.is_some() {
                    // 不是服务器出错：别的电脑刚换了同步密码，用新的再开一次
                    refused(v, refuse::JOIN_BAD, "同步密码刚在别的电脑上换过了，用新的同步密码再打开一次")
                } else {
                    self.err("这个空间还没设同步密码")
                }
            }
            Msg::JoinOff => {
                self.store.set_join(space, None, None);
                log("关掉了兑换码加入");
                self.ext.event(self, ip, Some(space), Some(me), "关掉兑换码加入");
                Msg::Ok
            }
            Msg::SetRecovery { box2, recovery } => {
                if self.store.set_recovery(space, &box2, &hash_token(&recovery)) {
                    log("换了恢复码");
                    Msg::Ok
                } else {
                    self.err("这个空间还没设同步密码")
                }
            }
            Msg::PhoneAdd { name } => {
                // 电脑加手机合起来可能有上限（扩展点定，叮咪同步按套餐）；问和加之间别的登记等着（见 Server.admit）
                let _admit = self.admit.lock().unwrap();
                if let Err(r) = self.ext.room(self, space) {
                    self.ext.event(self, ip, Some(space), Some(me), &format!("添加手机被拒：{}", r.msg));
                    return refused(v, &r.code, &r.msg);
                }
                let (id, token) = (random_id(), B64.encode(rand::random::<[u8; 16]>()));
                let name = clean_name(&name);
                match self.store.add_phone(&id, space, &name, &hash_token(&token)) {
                    Ok(()) => {
                        log(format!("添加了手机：{name}"));
                        self.ext.event(self, ip, Some(space), Some(&id), "添加手机");
                        Msg::PhoneAdded { id, token }
                    }
                    Err(_) => self.err("添加手机失败"),
                }
            }
            Msg::InviteNew => match self.store.new_invite(space, DEVICE_INVITE_MINUTES) {
                Ok((token, expires_at)) => {
                    self.store.trim_invites(space, MAX_PENDING_INVITES);
                    log("电脑端生成了一个配对码");
                    Msg::Invited { token, expires_at }
                }
                Err(_) => self.err("生成配对码失败"),
            },
            Msg::PhoneList => Msg::Phones { phones: self.store.phones(space) },
            Msg::PcList => Msg::Pcs {
                pcs: self
                    .store
                    .devices()
                    .into_iter()
                    .filter(|d| d.space_id == space && !d.revoked)
                    .map(|d| Pc { me: d.pubkey == me, id: d.pubkey, name: d.name, created_at: d.created_at, last_seen: d.last_seen })
                    .collect(),
            },
            Msg::PcRevoke { id } => {
                if id == me {
                    self.err("不能作废这台电脑自己：不想同步了用「停止同步」")
                } else if self.store.device(&id).is_some_and(|d| d.space_id == space && !d.revoked) && self.store.revoke_device(&id) {
                    self.ext.event(self, ip, Some(space), Some(me), "作废了另一台电脑");
                    Msg::Ok
                } else {
                    self.err("没有这台电脑")
                }
            }
            Msg::PhoneRevoke { id } => {
                if self.store.revoke_phone(space, &id) {
                    Msg::Ok
                } else {
                    self.err("没有这部手机")
                }
            }
            Msg::InboxList => Msg::InboxItems { items: self.store.inbox(space) },
            Msg::InboxGet { id, offset } => self.inbox_chunk(space, id, offset),
            Msg::InboxDel { id } => {
                if self.store.del_inbox(space, &id) {
                    let _ = std::fs::remove_file(self.inbox_dir().join(&id));
                }
                Msg::Ok
            }
            Msg::Push { items } => match self.push_records(space, me, items) {
                Ok(acks) => Msg::Acks { acks },
                Err(_) => full(),
            },
            Msg::Pull { since } => {
                let (items, more) = self.pull_records(space, since);
                Msg::Records { items, more }
            }
            Msg::FilePut { uid, offset, data, done } => self.file_put(space, &uid, offset, &data, done, v, full),
            Msg::FileGet { uid, offset } => self.file_get(space, uid, offset),
            Msg::FileDel { uid } => {
                if valid_file(&uid) && self.store.file_del(space, &uid) {
                    let _ = std::fs::remove_file(self.file_path(space, &uid));
                }
                Msg::Ok
            }
            _ => self.err("服务器不认识这条消息，可能需要更新服务器"),
        }
    }

    /// 存一批记录（电脑的 Push 和手机版的 /m/api/push 共用）；存下了就叫醒同一空间里别的设备。
    /// 容量满了照收（只拦附件）；超出上限很多（见 admin::fits_records）才整批不收（Reject::Quota）；变小的（比如删东西）总能推
    pub fn push_records(&self, space: &str, me: &str, items: Vec<Record>) -> Result<Vec<Ack>, Reject> {
        let blobs: Vec<Option<Vec<u8>>> = items
            .iter()
            .map(|it| {
                let ok = valid_key(&it.uid) && !it.ver.is_empty() && it.ver.len() <= 64;
                STANDARD.decode(&it.blob).ok().filter(|_| ok)
            })
            .collect();
        let new: u64 = blobs.iter().flatten().map(|b| b.len() as u64).sum();
        let old = self.store.items_size(space, &items.iter().map(|it| it.uid.as_str()).collect::<Vec<_>>());
        // 记录（文字）对容量放宽，满了也照收，只有超出很多才拦（需求-1.0 第 17.3 节：满了只拦附件）
        if new > old && !admin::fits_records(&self.store, space, new - old) {
            return Err(Reject::Quota);
        }
        let rows: Vec<(&str, &str, Option<&[u8]>)> =
            items.iter().zip(&blobs).map(|(it, b)| (it.uid.as_str(), it.ver.as_str(), b.as_deref())).collect();
        // 一整批一个事务；存的时候出错就整批都算没存上，电脑下次再传
        let oks = self.store.push_items(space, &rows).unwrap_or_else(|_| vec![false; items.len()]);
        let stored = oks.iter().any(|ok| *ok);
        let acks: Vec<Ack> = items.into_iter().zip(oks).map(|(it, ok)| Ack { uid: it.uid, ok }).collect();
        if stored {
            self.notify(space, News::Records(me.into()));
        }
        Ok(acks)
    }

    /// 序号大于 since 的记录。返回 (记录, 后面还有没有)
    pub fn pull_records(&self, space: &str, since: u64) -> (Vec<Record>, bool) {
        let (rows, more) = self.store.pull_items(space, since, PULL_BYTES, PULL_COUNT);
        let items = rows
            .into_iter()
            .map(|(uid, ver, seq, blob)| Record { uid, ver, seq, blob: STANDARD.encode(blob) })
            .collect();
        (items, more)
    }

    /// 手机版一次传上来的整个附件（已经加密过），存好并标成传完
    pub fn file_store(&self, space: &str, uid: &str, bytes: &[u8]) -> Result<(), Reject> {
        // nonce 24 + 认证码 16
        if !valid_file(uid) || bytes.len() < 40 || bytes.len() as u64 > MAX_FILE {
            return Err(Reject::Bad);
        }
        // 单个附件的上限（扩展点定：叮咪同步按套餐，32.1 第 5 条；开源版不限）
        let lim = self.file_limits(space);
        if lim.cap.is_some_and(|cap| bytes.len() as u64 > cap) {
            return Err(Reject::TooBig(lim.too_big));
        }
        let old = self.store.file_size(space, uid);
        if bytes.len() as u64 > old && !admin::fits(&self.store, space, bytes.len() as u64 - old) {
            return Err(Reject::Quota);
        }
        let path = self.file_path(space, uid);
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(path.parent().expect("附件路径总有上一级"))?;
            std::fs::write(&path, bytes)
        };
        write().map_err(|_| Reject::Server)?;
        self.store.file_put(space, uid, bytes.len() as u64, true).map_err(|_| Reject::Server)
    }

    /// 一个传完了的附件（加密后的）整个读出来，给手机版用；没有或还没传完返回 None
    pub fn file_bytes(&self, space: &str, uid: &str) -> Option<Vec<u8>> {
        if !valid_file(uid) || !self.store.file_complete(space, uid) {
            return None;
        }
        std::fs::read(self.file_path(space, uid)).ok()
    }

    fn inbox_dir(&self) -> PathBuf {
        self.dir.join("inbox")
    }

    /// 图片放在 files/<空间>/<图片编号>，编号先用 valid_file 核对过，拼不出别的路径
    fn file_path(&self, space: &str, uid: &str) -> PathBuf {
        space_files(&self.dir, space).join(uid)
    }

    /// 附件按块上传。放不下时回 `full()`；传到一半放不下的，把已经传上来的那一半删掉（不然它一直占着地方）
    #[allow(clippy::too_many_arguments)]
    fn file_put(&self, space: &str, uid: &str, offset: u64, data: &str, done: bool, v: u32, full: impl FnOnce() -> Msg) -> Msg {
        if !valid_file(uid) {
            return self.err("图片编号不对");
        }
        let Ok(bytes) = STANDARD.decode(data) else {
            return self.err("内容不对");
        };
        // 接着传（offset > 0）：服务器上要有这个附件、还没传完、已经有的正好是 offset 那么多，
        // 不然中间会空出一段或者盖掉一段。回错误，电脑这一轮同步出错，下一轮从头传
        if offset > 0 && (self.store.file_complete(space, uid) || self.store.file_size(space, uid) != offset) {
            return self.err(FILE_RESTART);
        }
        if offset + bytes.len() as u64 > MAX_FILE {
            return self.err("图片太大了");
        }
        // 单个附件的上限（扩展点定：叮咪同步按套餐，32.1 第 5 条）：电脑本来就不传超过的，这里兜底；传到一半的删掉
        let lim = self.file_limits(space);
        if lim.cap.is_some_and(|cap| offset + bytes.len() as u64 > cap) {
            if !self.store.file_complete(space, uid) && self.store.file_del(space, uid) {
                let _ = std::fs::remove_file(self.file_path(space, uid));
            }
            return refused(v, refuse::TOO_BIG, &lim.too_big);
        }
        // 从头传时旧的会被清掉；只算比服务器上已有的多出来的那部分
        let (size, old) = (offset + bytes.len() as u64, self.store.file_size(space, uid));
        if size > old && !admin::fits(&self.store, space, size - old) {
            if offset > 0 && !self.store.file_complete(space, uid) && self.store.file_del(space, uid) {
                let _ = std::fs::remove_file(self.file_path(space, uid));
            }
            return full();
        }
        let path = self.file_path(space, uid);
        let write = || -> std::io::Result<u64> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // 从头开始传时清掉以前没传完的
            let mut f = OpenOptions::new().create(true).write(true).truncate(offset == 0).open(&path)?;
            f.seek(SeekFrom::Start(offset))?;
            f.write_all(&bytes)?;
            Ok(f.metadata()?.len())
        };
        match write().map(|size| self.store.file_put(space, uid, size, done)) {
            Ok(Ok(())) => Msg::Ok,
            _ => self.err("服务器存不下这张图片"),
        }
    }

    fn file_get(&self, space: &str, uid: String, offset: u64) -> Msg {
        if !valid_file(&uid) || !self.store.file_complete(space, &uid) {
            return self.err("这张图片还没传完");
        }
        let read = || -> std::io::Result<(Vec<u8>, bool)> {
            let mut f = std::fs::File::open(self.file_path(space, &uid))?;
            let len = f.metadata()?.len();
            f.seek(SeekFrom::Start(offset))?;
            let mut buf = Vec::with_capacity(INBOX_CHUNK);
            f.take(INBOX_CHUNK as u64).read_to_end(&mut buf)?;
            let done = offset + buf.len() as u64 >= len;
            Ok((buf, done))
        };
        match read() {
            Ok((buf, done)) => Msg::FileChunk { uid, offset, data: STANDARD.encode(buf), done },
            Err(_) => self.err("读不了这张图片"),
        }
    }

    fn notify(&self, space: &str, news: News) {
        if let Some(tx) = self.news.lock().unwrap().get(space) {
            let _ = tx.send(news);
        }
    }

    fn inbox_chunk(&self, space: &str, id: String, offset: u64) -> Msg {
        // 先在数据库里核对，id 不会被拿来拼出别的路径
        if !self.store.has_inbox(space, &id) {
            return self.err("没有这一条");
        }
        let read = || -> std::io::Result<(Vec<u8>, bool)> {
            let mut f = std::fs::File::open(self.inbox_dir().join(&id))?;
            let len = f.metadata()?.len();
            f.seek(SeekFrom::Start(offset))?;
            let mut buf = Vec::with_capacity(INBOX_CHUNK);
            f.take(INBOX_CHUNK as u64).read_to_end(&mut buf)?;
            let done = offset + buf.len() as u64 >= len;
            Ok((buf, done))
        };
        match read() {
            Ok((buf, done)) => Msg::InboxChunk { id, offset, data: STANDARD.encode(buf), done },
            // 数据库里有、文件没了（比如拿旧备份恢复过，那一条早被别的电脑取走了）：把这一条也删掉，
            // 不然电脑每次都取、每次都出错、断开重连，一直转圈
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.store.del_inbox(space, &id);
                self.err("这一条在服务器上已经没有了")
            }
            Err(_) => self.err("读不了这一条"),
        }
    }

    /// 核对手机令牌，返回 (手机 id, 空间 id)。在读请求体之前调用，令牌不对算一次违规。
    /// 空间停用了回 Reject::Off（不算违规）
    pub fn phone(&self, ip: IpAddr, token: Option<&str>) -> Result<(String, String), Reject> {
        let (phone, space) = token.and_then(|t| self.store.phone_by_token(&hash_token(t))).ok_or_else(|| {
            log(format!("拒绝了一个手机令牌不对的请求（{ip}）"));
            self.strike(ip);
            Reject::Token
        })?;
        if self.store.disabled_at(&space).is_some() {
            return Err(Reject::Off);
        }
        // 扩展点说空间停了（叮咪同步到期）：手机网页也显示（403 和那句话）
        if let Some(why) = self.ext.stopped(self, &space) {
            return Err(Reject::Stopped(why.msg));
        }
        self.ext.event(self, ip, Some(&space), Some(&phone), "手机连上");
        Ok((phone, space))
    }

    /// 手机网页要发一条 `size` 字节的（已经加密过）：读内容之前先看收不收得下（读完以后 receive 再看一次）。
    /// 单个附件的上限（扩展点定，叮咪同步按套餐）一条按一个附件算；收件箱里还没取走的也算进空间的容量
    /// （满了和附件一样不收：电脑取走、删掉以后就腾出来了）；另外每个空间最多 MAX_INBOX_COUNT 条、MAX_INBOX_TOTAL
    pub fn inbox_room(&self, space: &str, size: u64) -> Result<(), Reject> {
        let lim = self.file_limits(space);
        if lim.cap.is_some_and(|cap| size > cap) {
            return Err(Reject::TooBig(lim.too_big));
        }
        let (count, total) = self.store.inbox_usage(space);
        if count >= MAX_INBOX_COUNT || total + size > MAX_INBOX_TOTAL {
            return Err(Reject::Full);
        }
        if !admin::fits(&self.store, space, total + size) {
            return Err(Reject::Quota);
        }
        Ok(())
    }

    /// 收件箱里 `before` 之前发来、一直没被电脑取走的：删掉记录和文件，返回删了几条
    pub fn purge_inbox(&self, before: &str) -> usize {
        let ids = self.store.stale_inbox(before);
        for id in &ids {
            if valid_key(id) {
                let _ = std::fs::remove_file(self.inbox_dir().join(id));
            }
        }
        ids.len()
    }

    /// 手机网页发来一条（已经加密过）
    pub fn receive(&self, phone: &str, space: &str, body: &[u8]) -> Result<(), Reject> {
        let (phone, space) = (phone.to_string(), space.to_string());
        // 临时公钥 32 + nonce 24 + 认证码 16
        if body.len() < 72 {
            return Err(Reject::Bad);
        }
        self.inbox_room(&space, body.len() as u64)?;
        let id = random_id();
        std::fs::create_dir_all(self.inbox_dir()).map_err(|_| Reject::Server)?;
        std::fs::write(self.inbox_dir().join(&id), body).map_err(|_| Reject::Server)?;
        self.store.add_inbox(&id, &space, &phone, body.len() as u64).map_err(|_| Reject::Server)?;
        self.store.touch_phone(&phone);
        self.notify(&space, News::Inbox);
        Ok(())
    }

    fn subscribe(&self, space: &str) -> broadcast::Receiver<News> {
        self.news
            .lock()
            .unwrap()
            .entry(space.into())
            .or_insert_with(|| broadcast::channel(16).0)
            .subscribe()
    }

    pub fn banned(&self, ip: IpAddr) -> bool {
        let m = self.strikes.lock().unwrap();
        m.map.get(&strike_key(ip)).and_then(|s| s.until).is_some_and(|u| u > Instant::now())
    }

    fn strike(&self, ip: IpAddr) {
        let now = Instant::now();
        let key = strike_key(ip);
        let mut m = self.strikes.lock().unwrap();
        if !m.map.contains_key(&key) {
            // 满了：清掉最早记下的（一次一个，不全表扫）
            while m.map.len() >= MAX_STRIKES {
                let Some(old) = m.order.pop_front() else { break };
                m.map.remove(&old);
            }
            m.order.push_back(key);
        }
        let s = m.map.entry(key).or_default();
        if s.until.is_some_and(|u| u <= now) {
            *s = Strike::default();
        }
        s.fails += 1;
        if s.fails >= MAX_FAILS {
            s.fails = 0;
            s.until = Some(now + BAN);
            log(format!("{ip} 失败次数太多，屏蔽 10 分钟"));
        }
    }
}

/// 不取走数据，先看看连接开头的 4 个字节
async fn peek_head(sock: &TcpStream) -> std::io::Result<[u8; 4]> {
    let mut buf = [0u8; 4];
    loop {
        let n = sock.peek(&mut buf).await?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        if n >= 4 {
            return Ok(buf);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[cfg(test)]
#[path = "net_abuse_tests.rs"]
mod abuse_tests;
#[cfg(test)]
#[path = "net_join_tests.rs"]
mod join_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin;
    use crate::testkit::*;
    use dingmi_sync::{connect, generate_keypair, Space};

    #[tokio::test]
    async fn pairing_flow() {
        let (server, addr, _) = start("pair").await;
        let spub = server_public(&server);
        let (dev, _) = generate_keypair().unwrap();

        // 没有邀请码的陌生设备连不上
        assert!(dial(addr, &dev, &spub, None).await.is_err());

        let tok = invite(&server, "2999-01-01 00:00:00");
        let (mut conn, w) = dial(addr, &dev, &spub, Some(tok.clone())).await.unwrap();
        assert_eq!(w.space, Space::default());
        assert_eq!(w.ca_fingerprint.len(), 32 * 3 - 1);
        // 这份数据的「代」：电脑记着，服务器拿旧备份恢复后会变（见 admin::restore_backup）
        assert_eq!(w.epoch, server.store.kv_get("epoch").unwrap());
        assert_eq!(w.epoch.len(), 32);
        assert_eq!(ask(&mut conn, setup("salt")).await, Msg::Ok);
        assert!(matches!(ask(&mut conn, setup("x")).await, Msg::Error { .. }));
        assert_eq!(ask(&mut conn, Msg::Ping).await, Msg::Pong);
        drop(conn);

        // 登记过的设备不用再带邀请码，能拿到空间信息
        let (_, w) = dial(addr, &dev, &spub, None).await.unwrap();
        assert_eq!(w.space, Space { salt: Some("salt".into()), box1: Some("b1".into()), box2: Some("b2".into()), box3: None });
        assert_eq!(server.store.devices()[0].name, "测试电脑");

        // 邀请码只能用一次
        let (dev2, _) = generate_keypair().unwrap();
        assert!(dial(addr, &dev2, &spub, Some(tok)).await.is_err());
        // 过期的邀请码不能用
        let old = invite(&server, "2000-01-01 00:00:00");
        assert!(dial(addr, &dev2, &spub, Some(old)).await.is_err());

        // 登记过的电脑能给另一台电脑生成配对码，那台加入的是同一个空间
        let (mut conn, _) = dial(addr, &dev, &spub, None).await.unwrap();
        let Msg::Invited { token: first, .. } = ask(&mut conn, Msg::InviteNew).await else { panic!("没拿到配对码") };
        let Msg::Invited { token, expires_at } = ask(&mut conn, Msg::InviteNew).await else { panic!("没拿到配对码") };
        // 电脑上生成的配对码带空间密钥，24 小时内有效（在家生成、第二天到公司也能用）
        let at = |m| (chrono::Local::now() + chrono::TimeDelta::minutes(m)).format("%Y-%m-%d %H:%M:%S").to_string();
        assert!(expires_at > at(23 * 60) && expires_at < at(24 * 60 + 1));
        // 一个空间最多留 10 个没用的：再要 9 个，最早那个就作废了，刚才那个还在
        for _ in 0..9 {
            assert!(matches!(ask(&mut conn, Msg::InviteNew).await, Msg::Invited { .. }));
        }
        drop(conn);
        assert!(!server.store.invite_exists(&hash_token(&first)));
        assert!(server.store.invite_exists(&hash_token(&token)));
        let (_, w) = dial(addr, &dev2, &spub, Some(token)).await.unwrap();
        assert_eq!(w.space.salt.as_deref(), Some("salt"));

        // 没有配对码：凭恢复凭证登记新电脑；凭证不对的连不上
        let (dev3, _) = generate_keypair().unwrap();
        assert!(dial_recovery(addr, &dev3, &spub, "wrong").await.is_err());
        let (mut conn, w) = dial_recovery(addr, &dev3, &spub, "rec").await.unwrap();
        assert_eq!(w.space.box2.as_deref(), Some("b2"));
        // 换恢复码：旧的凭证作废；换同步密码：新的 salt 和盒子 1
        assert_eq!(ask(&mut conn, Msg::SetRecovery { box2: "b2new".into(), recovery: "rec2".into() }).await, Msg::Ok);
        assert_eq!(ask(&mut conn, Msg::SetLock { salt: "salt2".into(), box1: "b1new".into(), join: None }).await, Msg::Ok);
        drop(conn);
        // 前面已经失败了 4 次，再错一次这个 IP 就会被屏蔽，先清掉
        *server.strikes.lock().unwrap() = Strikes::default();
        let (dev4, _) = generate_keypair().unwrap();
        assert!(dial_recovery(addr, &dev4, &spub, "rec").await.is_err());
        let (_, w) = dial_recovery(addr, &dev4, &spub, "rec2").await.unwrap();
        assert_eq!(w.space, Space { salt: Some("salt2".into()), box1: Some("b1new".into()), box2: Some("b2new".into()), box3: None });

        // 作废之后连不上（两台都叫"测试电脑"，一起作废）
        assert_eq!(server.store.revoke("测试电脑").len(), 2);
        assert_eq!(server.store.revoke("找回的电脑").len(), 2);
        assert!(dial(addr, &dev, &spub, None).await.is_err());
    }

    #[tokio::test]
    async fn phone_inbox_flow() {
        let (server, addr, dir) = start("inbox").await;
        let spub = server_public(&server);
        let (dev, _) = generate_keypair().unwrap();
        let (mut conn, _) = dial(addr, &dev, &spub, Some(invite(&server, "2999-01-01 00:00:00"))).await.unwrap();

        let Msg::PhoneAdded { id: phone_id, token } = ask(&mut conn, Msg::PhoneAdd { name: "iPhone".into() }).await else {
            panic!("没拿到手机令牌");
        };
        let Msg::Phones { phones } = ask(&mut conn, Msg::PhoneList).await else { panic!() };
        assert_eq!(phones.len(), 1);
        assert_eq!(phones[0].name, "iPhone");

        // 电脑在等，手机发来一条，电脑马上被叫醒
        conn.send(&Msg::Wait).await.unwrap();
        let body = vec![9u8; 300 * 1024];
        let post = |tok: &str, len: usize| {
            format!(
                "POST /m/api/inbox HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {tok}\r\nContent-Length: {}\r\n\r\n",
                len
            )
        };
        let out = https(addr, &dir, &post(&token, body.len()), &body).await;
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
        assert_eq!(woke(&mut conn).await, Msg::Changed);

        // 旧版（协议 1）的电脑端照样被叫醒，回的是它认识的 InboxNotify
        let (mut old, _) = dial_v(addr, &dev, &spub, None, 1).await.unwrap();
        old.send(&Msg::Wait).await.unwrap();
        assert!(https(addr, &dir, &post(&token, 100), &[7u8; 100]).await.starts_with("HTTP/1.1 200"));
        assert_eq!(woke(&mut old).await, Msg::InboxNotify);

        let Msg::InboxItems { items } = ask(&mut conn, Msg::InboxList).await else { panic!() };
        assert_eq!(items.len(), 2);
        assert_eq!(ask(&mut conn, Msg::InboxDel { id: items[1].id.clone() }).await, Msg::Ok);
        assert_eq!((items[0].phone.as_str(), items[0].size), ("iPhone", body.len() as u64));

        // 分块下载，拼起来和发的一样
        let mut got = Vec::new();
        loop {
            let get = Msg::InboxGet { id: items[0].id.clone(), offset: got.len() as u64 };
            let Msg::InboxChunk { data, done, .. } = ask(&mut conn, get).await else { panic!() };
            got.extend(STANDARD.decode(data).unwrap());
            if done {
                break;
            }
        }
        assert_eq!(got, body);
        assert_eq!(ask(&mut conn, Msg::InboxDel { id: items[0].id.clone() }).await, Msg::Ok);
        let Msg::InboxItems { items } = ask(&mut conn, Msg::InboxList).await else { panic!() };
        assert!(items.is_empty());

        // 没东西时等 25 秒回 Pong 太慢，这里只确认别的空间的 id 读不到
        assert!(matches!(ask(&mut conn, Msg::InboxGet { id: "../dingmi-server.db".into(), offset: 0 }).await, Msg::Error { .. }));

        // 令牌不对、作废之后都发不进来
        assert!(https(addr, &dir, &post("wrong", body.len()), &body).await.starts_with("HTTP/1.1 401"));
        assert_eq!(ask(&mut conn, Msg::PhoneRevoke { id: phone_id }).await, Msg::Ok);
        assert!(https(addr, &dir, &post(&token, body.len()), &body).await.starts_with("HTTP/1.1 401"));
    }

    #[tokio::test]
    async fn phone_sync_api() {
        let (server, addr, dir) = start("phone-sync").await;
        let spub = server_public(&server);
        let (dev, _) = generate_keypair().unwrap();
        let (mut conn, _) = dial(addr, &dev, &spub, Some(invite(&server, "2999-01-01 00:00:00"))).await.unwrap();
        assert_eq!(ask(&mut conn, setup("salt")).await, Msg::Ok);
        let Msg::PhoneAdded { id: phone_id, token } = ask(&mut conn, Msg::PhoneAdd { name: "iPhone".into() }).await else {
            panic!("没加上手机")
        };
        let push = Msg::Push { items: vec![record("r1", "0001-a", b"one")] };
        assert!(matches!(ask(&mut conn, push).await, Msg::Acks { .. }));
        let req = |method: &str, path: &str, tok: &str, len: usize| {
            format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {tok}\r\nContent-Length: {len}\r\n\r\n")
        };
        let body = |out: &str| -> serde_json::Value {
            assert!(out.starts_with("HTTP/1.1 200"), "{out}");
            serde_json::from_str(out.split("\r\n\r\n").nth(1).unwrap()).unwrap()
        };

        // 手机拿到 salt 和盒子 1（盒子 2 用不着，不给），拉到电脑传的记录
        let v = body(&https(addr, &dir, &req("GET", "/m/api/space", &token, 0), b"").await);
        assert_eq!((v["salt"].as_str(), v["box1"].as_str(), v.get("box2")), (Some("salt"), Some("b1"), None));
        let v = body(&https(addr, &dir, &req("GET", "/m/api/pull?since=0", &token, 0), b"").await);
        assert_eq!(v["items"][0]["uid"], "r1");
        assert_eq!(v["items"][0]["blob"], STANDARD.encode(b"one"));
        let seq = v["items"][0]["seq"].as_u64().unwrap();
        let v = body(&https(addr, &dir, &req("GET", &format!("/m/api/pull?since={seq}"), &token, 0), b"").await);
        assert!(v["items"].as_array().unwrap().is_empty());

        // 电脑在等，手机推一条新的、一条旧版本的：新的存下并叫醒电脑，旧的不收
        conn.send(&Msg::Wait).await.unwrap();
        let items = serde_json::json!({ "items": [
            { "uid": "n1", "ver": "0002-p", "blob": STANDARD.encode(b"note") },
            { "uid": "r1", "ver": "0000-p", "blob": STANDARD.encode(b"old") },
        ]})
        .to_string();
        let v = body(&https(addr, &dir, &req("POST", "/m/api/push", &token, items.len()), items.as_bytes()).await);
        assert_eq!((v["acks"][0]["ok"].as_bool(), v["acks"][1]["ok"].as_bool()), (Some(true), Some(false)));
        assert_eq!(woke(&mut conn).await, Msg::Changed);
        let Msg::Records { items, .. } = ask(&mut conn, Msg::Pull { since: seq }).await else { panic!() };
        assert_eq!(items.iter().map(|r| r.uid.as_str()).collect::<Vec<_>>(), ["n1"]);

        // 图片：传完的能下，没有的 404，编号拼不出别的路径
        let put = Msg::FilePut { uid: "abc1".into(), offset: 0, data: STANDARD.encode(b"cipher"), done: true };
        assert_eq!(ask(&mut conn, put).await, Msg::Ok);
        let out = https(addr, &dir, &req("GET", "/m/api/file/abc1", &token, 0), b"").await;
        assert!(out.starts_with("HTTP/1.1 200") && out.ends_with("cipher"), "{out}");
        assert!(https(addr, &dir, &req("GET", "/m/api/file/nope", &token, 0), b"").await.starts_with("HTTP/1.1 404"));
        let out = https(addr, &dir, &req("GET", "/m/api/file/..%2Fdingmi-server.db", &token, 0), b"").await;
        assert!(out.starts_with("HTTP/1.1 404"), "{out}");

        // 手机传上来的附件（已经加密好的整个文件）：手机和电脑都能下载；太短的、编号不对的不收
        let blob = vec![7u8; 300 * 1024];
        let out = https(addr, &dir, &req("POST", "/m/api/file/abc2", &token, blob.len()), &blob).await;
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
        let Msg::FileChunk { data, done, .. } = ask(&mut conn, Msg::FileGet { uid: "abc2".into(), offset: 0 }).await else {
            panic!("电脑下载不了")
        };
        assert_eq!((STANDARD.decode(data).unwrap().len(), done), (INBOX_CHUNK, false));
        let out = https(addr, &dir, &req("GET", "/m/api/file/abc2", &token, 0), b"").await;
        assert!(out.starts_with("HTTP/1.1 200") && out.len() > blob.len(), "{}", &out[..80]);
        assert!(https(addr, &dir, &req("POST", "/m/api/file/abc3", &token, 5), b"short").await.starts_with("HTTP/1.1 400"));
        assert!(https(addr, &dir, &req("POST", "/m/api/file/not-hex!", &token, 64), &[1u8; 64]).await.starts_with("HTTP/1.1 400"));

        // 令牌不对、作废之后什么都拿不到
        assert!(https(addr, &dir, &req("GET", "/m/api/pull", "wrong", 0), b"").await.starts_with("HTTP/1.1 401"));
        assert_eq!(ask(&mut conn, Msg::PhoneRevoke { id: phone_id }).await, Msg::Ok);
        assert!(https(addr, &dir, &req("GET", "/m/api/space", &token, 0), b"").await.starts_with("HTTP/1.1 401"));
    }

    #[tokio::test]
    async fn records_sync_between_devices() {
        let (server, addr, _) = start("records").await;
        let spub = server_public(&server);
        let (a, _) = generate_keypair().unwrap();
        let (b, _) = generate_keypair().unwrap();
        let (mut ca, _) = dial(addr, &a, &spub, Some(invite(&server, "2999-01-01 00:00:00"))).await.unwrap();
        let (mut cb, _) = dial(addr, &b, &spub, Some(invite(&server, "2999-01-01 00:00:00"))).await.unwrap();

        // B 在等，A 上传两条，B 被叫醒，拉到这两条
        cb.send(&Msg::Wait).await.unwrap();
        let push = Msg::Push { items: vec![record("r1", "0001", b"one"), record("n1", "0001", b"two")] };
        let Msg::Acks { acks } = ask(&mut ca, push).await else { panic!() };
        assert!(acks.iter().all(|a| a.ok));
        assert_eq!(woke(&mut cb).await, Msg::Changed);
        let Msg::Records { items, more } = ask(&mut cb, Msg::Pull { since: 0 }).await else { panic!() };
        assert!(!more);
        assert_eq!(items.iter().map(|i| i.uid.as_str()).collect::<Vec<_>>(), ["r1", "n1"]);
        let last = items[1].seq;

        // 旧版本不覆盖新版本；新版本覆盖后拿到新序号，只拉得到变了的那条
        let push = Msg::Push { items: vec![record("r1", "0000", b"old"), record("r1", "0002", b"new")] };
        let Msg::Acks { acks } = ask(&mut cb, push).await else { panic!() };
        assert_eq!(acks.iter().map(|a| a.ok).collect::<Vec<_>>(), [false, true]);
        let Msg::Records { items, .. } = ask(&mut ca, Msg::Pull { since: last }).await else { panic!() };
        assert_eq!(items.len(), 1);
        assert_eq!((items[0].ver.as_str(), STANDARD.decode(&items[0].blob).unwrap()), ("0002", b"new".to_vec()));

        // 一模一样的版本再传一次算成功（重新配对后会把本机的都再传一遍）
        let Msg::Acks { acks } = ask(&mut ca, Msg::Push { items: vec![record("r1", "0002", b"new")] }).await else { panic!() };
        assert!(acks[0].ok);

        // 编号不对的不收
        let Msg::Acks { acks } = ask(&mut ca, Msg::Push { items: vec![record("../x", "1", b"x")] }).await else { panic!() };
        assert!(!acks[0].ok);

        // 图片：分两块传，传完才能下载；下载拼起来一样；删掉后读不到
        let uid = "0190aaaa-bbbb-7ccc-8ddd-eeeeffff0000";
        let put = |offset: u64, data: &[u8], done: bool| Msg::FilePut { uid: uid.into(), offset, data: STANDARD.encode(data), done };
        assert_eq!(ask(&mut ca, put(0, b"hello ", false)).await, Msg::Ok);
        assert!(matches!(ask(&mut cb, Msg::FileGet { uid: uid.into(), offset: 0 }).await, Msg::Error { .. }));
        assert_eq!(ask(&mut ca, put(6, b"image", true)).await, Msg::Ok);
        let Msg::FileChunk { data, done, .. } = ask(&mut cb, Msg::FileGet { uid: uid.into(), offset: 0 }).await else { panic!() };
        assert!(done);
        assert_eq!(STANDARD.decode(data).unwrap(), b"hello image");
        assert_eq!(ask(&mut cb, Msg::FileDel { uid: uid.into() }).await, Msg::Ok);
        assert!(matches!(ask(&mut ca, Msg::FileGet { uid: uid.into(), offset: 0 }).await, Msg::Error { .. }));
        assert!(matches!(ask(&mut ca, put(0, b"x", true)).await, Msg::Ok));
        let bad = Msg::FilePut { uid: "../../etc".into(), offset: 0, data: STANDARD.encode(b"x"), done: true };
        assert!(matches!(ask(&mut ca, bad).await, Msg::Error { .. }));
    }

    #[tokio::test]
    async fn quota_and_disable_while_connected() {
        let (server, addr, dir) = start("admin").await;
        let spub = server_public(&server);
        let (dev, _) = generate_keypair().unwrap();
        let (mut conn, _) = dial(addr, &dev, &spub, Some(invite(&server, "2999-01-01 00:00:00"))).await.unwrap();
        let space = server.store.latest_space().unwrap();
        let Msg::PhoneAdded { token, .. } = ask(&mut conn, Msg::PhoneAdd { name: "iPhone".into() }).await else {
            panic!()
        };
        let Msg::Acks { acks } = ask(&mut conn, Msg::Push { items: vec![record("r1", "0001", &[1u8; 100])] }).await
        else {
            panic!()
        };
        assert!(acks[0].ok);

        // 容量上限 195（需求-1.0 第 17.3 节：满了只拦附件）：文字照收，超了 5% 以内也收；附件传不上去，回 Refused full
        admin::set_quota(&server.store, &space, Some(195)).unwrap();
        let Msg::Acks { acks } = ask(&mut conn, Msg::Push { items: vec![record("r2", "0001", &[2u8; 100])] }).await
        else {
            panic!("满了文字也该收")
        };
        assert!(acks[0].ok);
        assert!(!admin::fits(&server.store, &space, 1), "已经超了");
        assert!(admin::fits_records(&server.store, &space, 4), "文字有 5% 的余量");
        assert!(!admin::fits_records(&server.store, &space, 1000), "超出 5% 就不收了（作者定：满了要升级或者删内容）");
        let put = Msg::FilePut { uid: "abc1".into(), offset: 0, data: STANDARD.encode([0u8; 200]), done: true };
        assert!(matches!(ask(&mut conn, put.clone()).await,
            Msg::Refused { code, msg } if code == refuse::FULL && msg.contains("空间满了") && msg.contains("暂时只在这台电脑上")));
        // 旧电脑（协议 4）照旧收到 Error
        let (mut old, _) = dial_v(addr, &dev, &spub, None, 4).await.unwrap();
        assert!(matches!(ask(&mut old, put).await, Msg::Error { msg } if msg.contains("空间满了")));
        drop(old);
        // 电脑上看得到满了
        let Msg::AccountInfo { account } = ask(&mut conn, Msg::AccountGet).await else { panic!() };
        assert_eq!((account.storage.as_str(), account.quota, account.used), ("full", Some(195), 200));
        // 传到一半放不下的：已经传上来的那一半删掉，不一直占着地方
        admin::set_quota(&server.store, &space, Some(500)).unwrap();
        let half = |offset: u64, done: bool| Msg::FilePut { uid: "abc2".into(), offset, data: STANDARD.encode([0u8; 200]), done };
        assert_eq!(ask(&mut conn, half(0, false)).await, Msg::Ok);
        assert!(matches!(ask(&mut conn, half(200, true)).await, Msg::Refused { .. }));
        assert_eq!(server.store.usage(&space), (200, 0));
        // 快满了（90%）
        admin::set_quota(&server.store, &space, Some(220)).unwrap();
        let Msg::AccountInfo { account } = ask(&mut conn, Msg::AccountGet).await else { panic!() };
        assert_eq!(account.storage, "near_full");
        // 手机这边：文字一共 400，上限 390（超出 5% 以内照收），附件放不下
        admin::set_quota(&server.store, &space, Some(390)).unwrap();
        let req = |method: &str, path: &str, len: usize| {
            format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Length: {len}\r\n\r\n")
        };
        // 手机也一样：文字照收，附件 507
        let items = serde_json::json!({ "items": [{ "uid": "n1", "ver": "1", "blob": STANDARD.encode([3u8; 200]) }] })
            .to_string();
        let out = https(addr, &dir, &req("POST", "/m/api/push", items.len()), items.as_bytes()).await;
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
        let out = https(addr, &dir, &req("POST", "/m/api/file/abc9", 300), &[5u8; 300]).await;
        assert!(out.starts_with("HTTP/1.1 507") && out.contains("空间满了"), "{out}");
        admin::set_quota(&server.store, &space, None).unwrap();

        // 连着的时候空间被停用：下一条消息回「停用了」并断开；重连也一样；手机回 403（不是作废的 401）
        admin::disable(&server.store, &space).unwrap();
        assert!(matches!(ask(&mut conn, Msg::Ping).await, Msg::Refused { code, msg } if code == refuse::SPACE_OFF && msg == admin::SPACE_OFF));
        assert!(conn.recv::<Msg>().await.is_err(), "该断开了");
        let (mut conn, _) = dial(addr, &dev, &spub, None).await.unwrap();
        assert!(matches!(ask(&mut conn, Msg::Ping).await, Msg::Refused { .. }));
        let out = https(addr, &dir, &req("GET", "/m/api/space", 0), b"").await;
        assert!(out.starts_with("HTTP/1.1 403"), "{out}");
        // 停用的空间不能用配对码加新电脑
        let (dev2, _) = generate_keypair().unwrap();
        let tok = invite(&server, "2999-01-01 00:00:00");
        let (_, w) = dial(addr, &dev2, &spub, Some(tok.clone())).await.unwrap();
        assert_eq!(refused_code(&w), refuse::SPACE_OFF);
        assert!(dial_v(addr, &dev2, &spub, Some(tok), 4).await.is_err(), "旧电脑照旧直接断开");

        // 恢复后照常；连着的时候被作废，下一条消息就断开
        admin::restore(&server.store, &space).unwrap();
        let (mut conn, _) = dial(addr, &dev, &spub, None).await.unwrap();
        assert_eq!(ask(&mut conn, Msg::Ping).await, Msg::Pong);
        let pubkey = server.store.devices()[0].pubkey.clone();
        admin::revoke_computer(&server.store, &pubkey).unwrap();
        assert!(matches!(ask(&mut conn, Msg::Ping).await, Msg::Error { msg } if msg.contains("作废")));
    }

    #[tokio::test]
    async fn app_browsers_and_robots() {
        let (_, addr, _) = start("inapp").await;
        // 微信里扫码打开证书：先给「请在浏览器里打开」
        let out =
            raw(addr, b"GET /ca HTTP/1.1\r\nHost: x\r\nUser-Agent: Mozilla/5.0 (iPhone) MicroMessenger/8.0.47\r\n\r\n")
                .await;
        assert!(
            out.starts_with("HTTP/1.1 200") && out.contains("text/html") && out.contains("请在浏览器里打开"),
            "{out}"
        );
        // 不让搜索引擎收录
        let out = raw(addr, b"GET /robots.txt HTTP/1.1\r\nHost: x\r\n\r\n").await;
        assert!(out.contains("Disallow: /") && out.to_ascii_lowercase().contains("x-robots-tag: noindex"), "{out}");
    }

    /// 拿旧备份恢复的整个过程（作者不手动测这个，靠这条）：备份之后又传了东西 → 停服务、restore-backup → 重新起来。
    /// 电脑端看到代变了会从头拉、把本机的全部重传（见电脑端 sync::server_restored），这里照着做一遍，看服务器这边对不对得上
    #[tokio::test]
    async fn restore_backup_then_resync() {
        let dir = std::env::temp_dir().join(format!("dingmi-server-test-restore-{}", rand::random::<u64>()));
        crate::init(&dir, "127.0.0.1".parse().unwrap(), 7788).unwrap();
        let (dev, _) = generate_keypair().unwrap();
        let uid = "0190aaaa-bbbb-7ccc-8ddd-eeeeffff0001";
        let put = |data: &[u8]| Msg::FilePut { uid: uid.into(), offset: 0, data: STANDARD.encode(data), done: true };

        // 第一次起：配对，传 r1 和一个附件，备份；之后又传了 r2
        let epoch1;
        {
            let server = Server::load(&dir).unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let task = tokio::spawn(server.clone().serve(listener));
            let spub = server_public(&server);
            let (mut c, w) = dial(addr, &dev, &spub, Some(invite(&server, "2999-01-01 00:00:00"))).await.unwrap();
            epoch1 = w.epoch.clone();
            assert!(matches!(ask(&mut c, Msg::Push { items: vec![record("r1", "0001", b"one")] }).await, Msg::Acks { .. }));
            assert_eq!(ask(&mut c, put(b"picture")).await, Msg::Ok);
            server.store.backup(&dir, "2000-01-01", 14).unwrap();
            assert!(matches!(ask(&mut c, Msg::Push { items: vec![record("r2", "0001", b"two")] }).await, Msg::Acks { .. }));
            drop(c);
            task.abort();
            let _ = task.await;
            // 等连接都断干净，数据库文件才能改名
            for _ in 0..50 {
                if Arc::strong_count(&server) == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        // 附件目录也没了（比如服务器整个丢了，只剩下载下来的备份）
        std::fs::remove_dir_all(dir.join("files")).unwrap();
        admin::restore_backup(&dir, "2000-01-01").unwrap();

        // 恢复后起来：代变了；服务器上只有 r1、附件文件没了
        let server = Server::load(&dir).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(server.clone().serve(listener));
        let spub = server_public(&server);
        let (mut c, w) = dial(addr, &dev, &spub, None).await.expect("备份之前配对的电脑照样连得上");
        assert_ne!(w.epoch, epoch1);
        let Msg::Records { items, .. } = ask(&mut c, Msg::Pull { since: 0 }).await else { panic!() };
        assert_eq!(items.iter().map(|r| r.uid.as_str()).collect::<Vec<_>>(), ["r1"]);
        assert!(matches!(ask(&mut c, Msg::FileGet { uid: uid.into(), offset: 0 }).await, Msg::Error { .. }));

        // 电脑把本机的全部重传：r1 一样的版本算成功，r2 补上，附件补上
        let all = Msg::Push { items: vec![record("r1", "0001", b"one"), record("r2", "0001", b"two")] };
        let Msg::Acks { acks } = ask(&mut c, all).await else { panic!() };
        assert!(acks.iter().all(|a| a.ok));
        assert_eq!(ask(&mut c, put(b"picture")).await, Msg::Ok);
        let Msg::Records { items, .. } = ask(&mut c, Msg::Pull { since: 0 }).await else { panic!() };
        assert_eq!(items.iter().map(|r| r.uid.as_str()).collect::<Vec<_>>(), ["r1", "r2"]);
        let Msg::FileChunk { data, done, .. } = ask(&mut c, Msg::FileGet { uid: uid.into(), offset: 0 }).await else { panic!() };
        assert!(done);
        assert_eq!(STANDARD.decode(data).unwrap(), b"picture");
    }

    #[tokio::test]
    async fn ca_over_plain_http() {
        let (_, addr, _) = start("http").await;
        let out = raw(addr, b"GET /ca HTTP/1.1\r\nHost: x\r\n\r\n").await;
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
        assert!(out.contains("application/x-apple-aspen-config"));
        assert!(out.contains("com.apple.security.root"));
        let out = raw(addr, b"GET /ca.crt HTTP/1.1\r\nHost: x\r\n\r\n").await;
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
        assert!(out.contains("application/x-x509-ca-cert"));
        let out = raw(addr, b"GET /m/ HTTP/1.1\r\nHost: x\r\n\r\n").await;
        assert!(out.starts_with("HTTP/1.1 400"), "{out}");
    }

    #[tokio::test]
    async fn https_serves_phone_page() {
        let (_, addr, dir) = start("https").await;
        // 清单只给安卓的 Chrome（分享菜单要），别的浏览器加到桌面时存当前网址，见 http::wants_manifest
        let chrome = "User-Agent: Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.6723.58 Mobile Safari/537.36\r\n";
        let out = https(addr, &dir, &format!("GET /m/abc.def HTTP/1.1\r\nHost: 127.0.0.1\r\n{chrome}\r\n"), b"").await;
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
        assert!(out.contains("发到叮咪"));
        // tweetnacl 已经内嵌进页面
        assert!(out.contains("nacl"));
        assert!(!out.contains("/*NACL*/"));
        // 带着 CSP（只认网页里那两段脚本）；以前的网址用按令牌给的清单
        assert!(out.to_ascii_lowercase().contains("content-security-policy: default-src 'none'"), "{out}");
        assert!(out.contains(r#"href="/m/abc.def/app.webmanifest""#));
        // 新的网址令牌在 # 后面，服务器只看到 /m/：照样给网页，用同一份清单
        let out = https(addr, &dir, &format!("GET /m/ HTTP/1.1\r\nHost: 127.0.0.1\r\n{chrome}\r\n"), b"").await;
        assert!(out.starts_with("HTTP/1.1 200") && out.contains("发到叮咪"), "{out}");
        assert!(out.contains(r#"href="/m/app.webmanifest""#));
        // iPhone 上不给清单（主屏幕存当前网址，# 后面的配对信息才不丢）
        let iphone = "User-Agent: Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1\r\n";
        let out = https(addr, &dir, &format!("GET /m/ HTTP/1.1\r\nHost: 127.0.0.1\r\n{iphone}\r\n"), b"").await;
        assert!(out.starts_with("HTTP/1.1 200") && out.contains("发到叮咪") && !out.contains(r#"<link rel="manifest""#), "{out}");
        let out = https(addr, &dir, "GET /m/app.webmanifest HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n", b"").await;
        assert!(out.starts_with("HTTP/1.1 200") && out.contains(r#""start_url":"/m/""#), "{out}");
        let out = https(addr, &dir, "GET /m-icon.png HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n", b"").await;
        assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    }

    #[tokio::test]
    async fn bans_after_repeated_garbage() {
        let (_, addr, _) = start("ban").await;
        for _ in 0..MAX_FAILS {
            raw(addr, b"XXXXXXXX").await;
        }
        assert_eq!(raw(addr, b"GET /ca HTTP/1.1\r\nHost: x\r\n\r\n").await, "");
    }

    /// 电脑走 `GET /sync` 升级成的 WebSocket（宝塔 / Nginx 转过来的样子），和直连一样能配对、收发；
    /// 不是 WebSocket 握手的 /sync 请求不当成电脑
    #[tokio::test]
    async fn device_over_websocket() {
        let (server, addr, _) = start("ws").await;
        let spub = server_public(&server);
        let (dev, _) = generate_keypair().unwrap();
        let over_ws = |invite: Option<String>| {
            let spub = spub.clone();
            let dev = dev.clone();
            async move {
                let mut s = TcpStream::connect(addr).await?;
                dingmi_sync::ws::client_handshake(&mut s, "sync.example.com").await?;
                let hello = Hello { version: VERSION, name: "走 443 的电脑".into(), invite, ..Default::default() };
                connect(dingmi_sync::ws::WsStream::client(s), &dev, &spub, &hello).await
            }
        };
        assert!(over_ws(None).await.is_err(), "没登记、没邀请码的设备连不上");
        let tok = invite(&server, "2999-01-01 00:00:00");
        let (mut conn, w) = over_ws(Some(tok)).await.unwrap();
        assert_eq!(w.epoch, server.store.kv_get("epoch").unwrap());
        assert_eq!(ask(&mut conn, setup("salt")).await, Msg::Ok);
        assert_eq!(ask(&mut conn, Msg::Ping).await, Msg::Pong);
        drop(conn);
        // 登记过以后，WebSocket 和直连都认它
        let (mut conn, _) = over_ws(None).await.unwrap();
        assert_eq!(ask(&mut conn, Msg::Ping).await, Msg::Pong);
        let (mut conn, _) = dial(addr, &dev, &spub, None).await.unwrap();
        assert_eq!(ask(&mut conn, Msg::Ping).await, Msg::Pong);
        assert_eq!(server.store.devices()[0].name, "走 443 的电脑");
        // 普通的 GET /sync 不是 WebSocket，当成网页请求（不会升级）
        assert!(!raw(addr, b"GET /sync HTTP/1.1
Host: x

").await.contains("101"));
    }

    /// 官方版那样在宝塔后面（Server::load_proxied）：不要 IP、证书；只说普通 HTTP，当成已经是 HTTPS；
    /// 电脑只走 /sync 的 WebSocket，配对码给同步域名
    #[tokio::test]
    async fn behind_proxy() {
        let (server, addr, dir) = start_proxied("proxied", Arc::new(crate::ext::SelfHosted)).await;
        assert!(server.proxied() && server.ca_der().is_none() && server.mobileconfig().is_none());
        assert!(!dir.join("ca.crt.pem").exists() && !dir.join("server.crt.pem").exists(), "不生成自签证书");
        // 开源版的样子（直连）照旧要 init --ip
        assert!(Server::load(&dir).err().unwrap().contains("init --ip"));
        // 没有根证书：/ca 是 404；手机网页直接给（宝塔那边已经是 HTTPS）
        assert!(raw(addr, b"GET /ca HTTP/1.1\r\nHost: sync.example.com\r\n\r\n").await.starts_with("HTTP/1.1 404"));
        assert!(raw(addr, b"GET /ca.crt HTTP/1.1\r\nHost: sync.example.com\r\n\r\n").await.starts_with("HTTP/1.1 404"));
        let out = raw(addr, b"GET /m/ HTTP/1.1\r\nHost: sync.example.com\r\n\r\n").await;
        assert!(out.starts_with("HTTP/1.1 200") && out.contains("text/html"), "{out}");
        // 配对码：没设同步域名时生成不了（没有 IP）；设了是 域名:443
        assert!(admin::invite(&server.store, None).is_err());
        server.store.kv_set(admin::HOST_SYNC_KEY, "sync.example.com").unwrap();
        let inv = admin::invite(&server.store, None).unwrap();
        assert!(inv.code.starts_with("dingmi://sync.example.com:443/"), "{}", inv.code);
        let code: dingmi_sync::PairCode = inv.code.parse().unwrap();
        // 电脑：直连的 DNGM 不认；走 WebSocket 照常登记、同步，Welcome 里没有根证书指纹
        let spub = server_public(&server);
        let (dev, _) = generate_keypair().unwrap();
        assert!(dial(addr, &dev, &spub, Some(code.invite.clone())).await.is_err());
        let mut s = TcpStream::connect(addr).await.unwrap();
        dingmi_sync::ws::client_handshake(&mut s, "sync.example.com").await.unwrap();
        let hello = Hello { version: VERSION, name: "宝塔后面".into(), invite: Some(code.invite), ..Default::default() };
        let (mut conn, w) = connect(dingmi_sync::ws::WsStream::client(s), &dev, &spub, &hello).await.unwrap();
        assert_eq!((w.ca_fingerprint.as_str(), w.epoch.is_empty()), ("", false));
        assert_eq!(ask(&mut conn, setup("salt")).await, Msg::Ok);
        assert_eq!(ask(&mut conn, Msg::Ping).await, Msg::Pong);
        assert_eq!(server.store.devices()[0].name, "宝塔后面");
    }

    /// 电脑在「设置 → 同步」里列出、作废空间里的另一台电脑（设备满了、电脑丢了）；不能作废自己、别的空间的
    #[tokio::test]
    async fn list_and_revoke_pcs() {
        let (server, addr, _) = start("pcs").await;
        let spub = server_public(&server);
        let (a, _) = generate_keypair().unwrap();
        let (b, _) = generate_keypair().unwrap();
        let (mut ca, _) = dial(addr, &a, &spub, Some(invite(&server, "2999-01-01 00:00:00"))).await.unwrap();
        assert_eq!(ask(&mut ca, setup("salt")).await, Msg::Ok);
        let Msg::Invited { token, .. } = ask(&mut ca, Msg::InviteNew).await else { panic!() };
        let (mut cb, _) = dial(addr, &b, &spub, Some(token)).await.unwrap();
        let Msg::Pcs { pcs } = ask(&mut ca, Msg::PcList).await else { panic!() };
        assert_eq!(pcs.len(), 2);
        let mine = pcs.iter().find(|p| p.me).unwrap().id.clone();
        let other = pcs.iter().find(|p| !p.me).unwrap().id.clone();
        assert!(matches!(ask(&mut ca, Msg::PcRevoke { id: mine }).await, Msg::Error { msg } if msg.contains("停止同步")));
        // 别的空间的电脑作废不了
        server.store.create_space("s2").unwrap();
        let (c, c_pub) = generate_keypair().unwrap();
        let tok2 = format!("tok{}", rand::random::<u64>());
        server.store.add_invite(&hash_token(&tok2), "s2", "2999-01-01 00:00:00").unwrap();
        let (_cc, _) = dial(addr, &c, &spub, Some(tok2)).await.unwrap();
        let c_id = B64.encode(&c_pub);
        assert!(matches!(ask(&mut ca, Msg::PcRevoke { id: c_id }).await, Msg::Error { .. }));
        // 作废另一台：它下一条消息就被断开，也连不上了
        assert_eq!(ask(&mut ca, Msg::PcRevoke { id: other.clone() }).await, Msg::Ok);
        assert!(!matches!(ask(&mut cb, Msg::Ping).await, Msg::Pong));
        assert!(dial(addr, &b, &spub, None).await.is_err());
        let Msg::Pcs { pcs } = ask(&mut ca, Msg::PcList).await else { panic!() };
        assert_eq!(pcs.len(), 1);
    }

    // ---------- 叮咪同步才有的消息（需求-1.0 第 32.1 节第 12 条：开源版不带这些） ----------

    /// 开源版收到兑换、公测、续费、客服核对码、邮箱：都回「这台服务器不支持」（redeem_off / beta_off），
    /// 不建空间、不登记、不算违规；账里 official 是 false，手机网页的限额是不限
    #[tokio::test]
    async fn self_hosted_refuses_official_messages() {
        let (server, addr, dir) = start("selfhost").await;
        let (s, spub) = (&server.store, server_public(&server));
        let local: IpAddr = "127.0.0.1".parse().unwrap();
        let (dev, _) = generate_keypair().unwrap();
        let terms = Some("2026-11-07".to_string());

        // 兑换码开通：说清楚这台服务器没开，不建空间、不登记这台电脑
        let hello = Hello { version: VERSION, name: "开通的电脑".into(), redeem: Some("DM-2222-3333-4444-5555".into()), terms: terms.clone(), ..Default::default() };
        let (_, w) = dial_hello(addr, &dev, &spub, hello).await.unwrap();
        assert_eq!(refused_code(&w), refuse::REDEEM_OFF);
        assert!(w.account.is_none());
        // 就算 kv 里写了 redeem=on（以前的开关），开源版也不认：配置改不了扩展点
        s.kv_set("redeem", "on").unwrap();
        let hello = Hello { version: VERSION, name: "开通的电脑".into(), redeem: Some("DM-2222-3333-4444-5555".into()), terms: terms.clone(), ..Default::default() };
        let (_, w) = dial_hello(addr, &dev, &spub, hello).await.unwrap();
        assert_eq!(refused_code(&w), refuse::REDEEM_OFF);
        // 公测开通、只问公测
        let hello = Hello { version: VERSION, name: "公测的电脑".into(), beta: true, terms: terms.clone(), ..Default::default() };
        let (_, w) = dial_hello(addr, &dev, &spub, hello).await.unwrap();
        assert_eq!(refused_code(&w), refuse::BETA_OFF);
        let (_, w) = dial_hello(addr, &dev, &spub, Hello { version: VERSION, ask_beta: true, ..Default::default() }).await.unwrap();
        assert_eq!(w.beta, Some(BetaInfo::default()), "公测没开");
        assert!(s.latest_space().is_none(), "一个空间都没建");
        assert!(s.devices().is_empty(), "也没登记");
        assert!(!server.banned(local), "说得清的拒绝不算违规");
        // 旧电脑（协议 4）带着兑换码：照旧直接断开
        let old = Hello { version: 4, name: "旧电脑".into(), redeem: Some("DM-2222-3333-4444-5555".into()), ..Default::default() };
        assert!(dial_hello(addr, &dev, &spub, old).await.is_err());

        // 配对以后：账里只有容量和台数，不是叮咪同步
        let (mut conn, w) = dial(addr, &dev, &spub, Some(invite(&server, "2999-01-01 00:00:00"))).await.unwrap();
        let a = w.account.unwrap();
        assert!(!a.official && a.plan.is_empty() && a.space.is_empty() && a.expires_at.is_none());
        assert_eq!((a.devices, a.max_devices, a.max_image, a.max_file, a.storage.as_str()), (1, None, None, None, "ok"));
        let Msg::AccountInfo { account } = ask(&mut conn, Msg::AccountGet).await else { panic!() };
        assert_eq!(account, a);

        // 续费、客服核对码、邮箱：Refused redeem_off，说清楚；旧电脑（协议 4）收到 Error
        for m in [
            Msg::Renew { code: "DM-2222-3333-4444-5555".into(), terms: terms.clone() },
            Msg::SupportNew,
            Msg::EmailSet { email: "me@example.com".into() },
            Msg::EmailDel,
        ] {
            let r = ask(&mut conn, m.clone()).await;
            assert!(matches!(&r, Msg::Refused { code, msg } if code == refuse::REDEEM_OFF && !msg.is_empty()), "{m:?} → {r:?}");
        }
        let (mut old, _) = dial_v(addr, &dev, &spub, None, 4).await.unwrap();
        assert!(matches!(ask(&mut old, Msg::SupportNew).await, Msg::Error { .. }));
        // 连接照常能用
        assert_eq!(ask(&mut conn, Msg::Ping).await, Msg::Pong);
        assert!(matches!(ask(&mut conn, Msg::Pull { since: 0 }).await, Msg::Records { .. }));
        assert_eq!(admin::spaces(s).len(), 1);

        // 附件没有单个的限额（只有 60 MB 的总上限）：11 MB 的分块传上去照收；手机网页问限额是不限
        for i in 0..11u64 {
            let put = Msg::FilePut { uid: "aa01".into(), offset: i << 20, data: STANDARD.encode(vec![1u8; 1 << 20]), done: i == 10 };
            assert_eq!(ask(&mut conn, put).await, Msg::Ok);
        }
        assert_eq!(s.usage(&s.latest_space().unwrap()).1, 11 << 20);
        let Msg::PhoneAdded { token, .. } = ask(&mut conn, Msg::PhoneAdd { name: "iPhone".into() }).await else { panic!() };
        let req = format!("GET /m/api/limits HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\n\r\n");
        let out = https(addr, &dir, &req, b"").await;
        let v: serde_json::Value = serde_json::from_str(out.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!((v["max_image"].clone(), v["max_file"].clone()), (serde_json::Value::Null, serde_json::Value::Null));
        // 很多台设备也不拦（开源版不限台数）
        for i in 0..12 {
            assert!(matches!(ask(&mut conn, Msg::PhoneAdd { name: format!("手机{i}") }).await, Msg::PhoneAdded { .. }));
        }
    }

    /// 开源版的数据库不建叮咪同步的表和列；官方服务器用过的库（有那些表和列）开源版照样能打开、照常同步
    #[tokio::test]
    async fn official_tables_are_ignored() {
        let (server, addr, dir) = start("tables").await;
        let s = &server.store;
        let tables: Vec<String> = s
            .c()
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for t in ["plans", "codes", "code_batches", "conn_log", "support_codes", "beta_claims", "alert_state", "alert_log", "offsite_gone"] {
            assert!(!tables.contains(&t.to_string()), "开源版不该建 {t}");
        }
        // 像官方服务器用过的库：多出那些列和表、空间带着套餐和到期
        s.c()
            .execute_batch(
                "ALTER TABLE spaces ADD COLUMN plan TEXT; ALTER TABLE spaces ADD COLUMN expires_at TEXT;
                 ALTER TABLE spaces ADD COLUMN short_id TEXT; ALTER TABLE spaces ADD COLUMN email TEXT;
                 CREATE UNIQUE INDEX spaces_short ON spaces(short_id);
                 CREATE TABLE support_codes(code TEXT PRIMARY KEY, space_id TEXT NOT NULL, device TEXT NOT NULL, expires_at TEXT NOT NULL);
                 CREATE TABLE conn_log(id INTEGER PRIMARY KEY AUTOINCREMENT, at TEXT NOT NULL, ip TEXT NOT NULL, space_id TEXT, device TEXT, action TEXT NOT NULL);",
            )
            .unwrap();
        let a = admin::invite(s, None).unwrap().space;
        s.c().execute("UPDATE spaces SET plan='standard', expires_at='2000-01-01', short_id='S-AAAAA' WHERE id=?1", [&a]).unwrap();
        let spub = server_public(&server);
        let (dev, _) = generate_keypair().unwrap();
        let (mut conn, w) = dial(addr, &dev, &spub, Some(invite(&server, "2999-01-01 00:00:00"))).await.unwrap();
        assert!(w.refused.is_none() && !w.account.unwrap().official, "到期日是官方版的事，开源版不管");
        assert!(matches!(ask(&mut conn, Msg::Push { items: vec![record("r1", "1", b"x")] }).await, Msg::Acks { .. }));
        // 新建的空间不填那些列也不出错（短编号是 NULL，唯一索引允许好几个 NULL）
        admin::invite(s, None).unwrap();
        assert_eq!(admin::spaces(s).len(), 2);
        // 停用满期删掉：别的表里的东西开源版不碰
        s.set_disabled(&a, Some("2000-01-01 00:00:00"));
        assert_eq!(admin::purge_due(s, &dir).len(), 1);
        drop(conn);
        let s2 = Store::open(&dir).unwrap();
        assert_eq!(admin::spaces(&s2).len(), 1);
    }
}
