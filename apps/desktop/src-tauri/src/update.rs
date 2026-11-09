//! 检查更新（需求-0.1 第 2 节）。所有叮咪都从官网的 update.json 看有没有新版本，和用不用同步、用哪种同步无关。
//!
//! - 检查：启动 1 分钟后一次，之后每 6 小时（「设置 → 关于」里可以关掉自动检查，只在点按钮时查）。
//!   请求里只带版本号（User-Agent: Dingmi/x），不带别的。
//! - 有新版本只提示：主窗口顶上一条、托盘里「更新到 x」；点「更新」才下载，下载地址按顺序试。
//! - 下完用程序里的公钥（release-pubkey.txt）核对签名，对不上的删掉不装；点「重启更新」才静默安装。
//! - update.json 里的 min_version 比本机新时，顶上一直提示，同步要停下（见 too_old）。
//!   决定行为的几样（version、min_version、安装包的 sha256）发布时另外签了名（meta_signature，见 meta_text）：
//!   签名对不上、或者是没有这个签名的旧 update.json，就不认 min_version（更新照旧能用，安装包本身有签名）。
//!   免得更新通道被劫持时，有人写一个很大的 min_version 让所有叮咪停下同步。
//! - 检查时顺便取官网的 terms.json（协议的版本，见 terms.rs），同样的节奏、同样的 User-Agent。
//! - 发布新版本用 scripts/release.mjs（用法见它开头）。

use crate::{db, AppState};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// 官网上的版本信息。本地测试时可以用环境变量 DINGMI_UPDATE_URL 换成别的地址
const UPDATE_URL: &str = "https://dingmi.app/update.json";
/// 核对安装包签名的公钥（npm run release -- keygen 写进去的），私钥只在开发电脑上
const PUBKEY_FILE: &str = include_str!("../release-pubkey.txt");
const FIRST_CHECK: Duration = Duration::from_secs(60);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// 安装包最大多大，比这大的不下
const MAX_SIZE: u64 = 300 * 1024 * 1024;
/// 装好新版本后第一次打开时提示一句，记在 kv 里的是更新前的版本号
const UPDATED_FROM: &str = "updated_from";
/// 点了 ✕ 的那个版本，不再提示；出了更新的版本再提示
const DISMISSED: &str = "update_dismissed";
/// 上次检查到的 min_version：刚打开、还没检查时也知道自己是不是太旧了
const MIN_VERSION: &str = "update_min_version";
const BAD_FILE: &str = "下载的文件不对，没有安装";
const OFFLINE: &str = "连不上，过会儿再试";

/// 官网 update.json 的内容
#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct Info {
    pub version: String,
    #[serde(default)]
    pub date: String,
    /// 更新了什么，几行大白话
    #[serde(default)]
    pub notes: String,
    /// 安装包的下载地址，按顺序试
    pub urls: Vec<String>,
    /// 安装包多大（字节）
    pub size: u64,
    /// 安装包的签名（tauri signer sign 生成的 .sig 文件内容）
    pub signature: String,
    /// 比这个版本旧的叮咪要先更新才能同步，可以不写
    #[serde(default)]
    pub min_version: Option<String>,
    /// 安装包的 SHA-256（十六进制，官网下载页「核对安装包」显示的）；0.4.0 那时发的没有
    #[serde(default)]
    pub sha256: Option<String>,
    /// version、min_version、sha256 合在一起的签名（meta_text，和安装包同一把私钥、同样的格式）。没有的是旧的 update.json
    #[serde(default)]
    pub meta_signature: Option<String>,
}

/// 发布时签名的那段文字（scripts/release.mjs 的 metaText 一模一样）：一行一样，没有的写空
pub fn meta_text(version: &str, min_version: Option<&str>, sha256: Option<&str>) -> String {
    format!("dingmi-update-meta-v1\nversion:{version}\nmin_version:{}\nsha256:{}\n", min_version.unwrap_or(""), sha256.unwrap_or(""))
}

/// 核对 update.json 里决定行为的那几样的签名。对得上回 true
fn meta_ok(pubkey: &str, info: &Info) -> bool {
    let Some(sig) = info.meta_signature.as_deref().filter(|s| !s.trim().is_empty()) else { return false };
    let text = meta_text(&info.version, info.min_version.as_deref(), info.sha256.as_deref());
    verify_with(pubkey, text.as_bytes(), sig, &info.version).is_ok()
}

/// 签名对不上（或者没签）就不认 min_version、sha256。回 min_version 认不认
fn trust_meta(pubkey: &str, info: &mut Info) -> bool {
    if meta_ok(pubkey, info) {
        return true;
    }
    if info.min_version.is_some() {
        crate::diag::log("更新", "update.json 里的最低版本没有签名（或者签名对不上），不认它");
    }
    info.min_version = None;
    info.sha256 = None;
    false
}

#[derive(Serialize, Clone, Default)]
pub struct Status {
    /// 现在的版本
    pub current: String,
    /// idle 没有新版本 / available 有新版本 / downloading 正在下载 / ready 下载好了等着装
    pub state: String,
    /// 正在检查
    pub checking: bool,
    /// 新版本的版本号、说明、日期、大小
    pub version: Option<String>,
    pub notes: Option<String>,
    pub date: Option<String>,
    pub size: u64,
    /// 已经下载了多少字节
    pub got: u64,
    /// 这个新版本点过 ✕
    pub dismissed: bool,
    /// 本机比 min_version 旧
    pub too_old: bool,
    /// 下载、手动检查出的问题（自动检查连不上不算）
    pub error: Option<String>,
    pub checked_at: Option<String>,
}

static STATUS: LazyLock<Mutex<Status>> = LazyLock::new(|| Mutex::new(Status { state: "idle".into(), ..Default::default() }));
/// 最近一次检查到的新版本（下载地址和签名在这里）
static LATEST: Mutex<Option<Info>> = Mutex::new(None);
/// 下载好、核对过签名的安装包
static READY: Mutex<Option<PathBuf>> = Mutex::new(None);
static DISMISSED_V: Mutex<Option<String>> = Mutex::new(None);
static TOO_OLD: AtomicBool = AtomicBool::new(false);

/// 本机版本比官网写的 min_version 旧：同步要先停下，等用户更新。只读内存里的状态，随时可以调
pub fn too_old() -> bool {
    TOO_OLD.load(Ordering::Relaxed)
}

pub fn status(app: &AppHandle) -> Status {
    let mut s = STATUS.lock().unwrap().clone();
    s.current = current(app);
    s.too_old = too_old();
    s.dismissed = s.version.is_some() && *DISMISSED_V.lock().unwrap() == s.version;
    s
}

/// 现在是 idle / available / downloading / ready 哪一种
pub fn state() -> String {
    STATUS.lock().unwrap().state.clone()
}

fn current(app: &AppHandle) -> String {
    app.package_info().version.to_string()
}

/// 改状态，告诉主窗口，托盘那一项也跟着变
fn set(app: &AppHandle, f: impl FnOnce(&mut Status)) {
    f(&mut STATUS.lock().unwrap());
    let _ = app.emit("update-status", status(app));
    crate::update_tray(app);
}

/// 托盘菜单那一项
pub fn tray_text() -> String {
    let s = STATUS.lock().unwrap();
    match (&s.state[..], &s.version) {
        ("ready", Some(v)) => format!("重启更新到 {v}"),
        ("downloading", Some(v)) => format!("正在下载 {v}…"),
        ("available", Some(v)) => format!("更新到 {v}"),
        _ => "检查更新".into(),
    }
}

/// a 比 b 新吗（按 . 分开逐段比数字）
fn newer(a: &str, b: &str) -> bool {
    let parts = |s: &str| s.split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    parts(a) > parts(b)
}

/// 版本号只能是数字和点，要拿来拼文件名
fn valid_version(v: &str) -> bool {
    !v.is_empty() && v.len() <= 20 && v.chars().all(|c| c.is_ascii_digit() || c == '.') && !v.split('.').any(str::is_empty)
}

/// 读 update.json：缺了必须有的、版本号不对的都不认
fn parse_info(text: &str) -> Result<Info, String> {
    let mut info: Info = serde_json::from_str(text).map_err(|_| "官网上的版本信息看不懂，过会儿再试".to_string())?;
    info.min_version = info.min_version.filter(|v| !v.trim().is_empty());
    if !valid_version(&info.version) || info.min_version.as_deref().is_some_and(|v| !valid_version(v)) {
        return Err("官网上的版本号不对，过会儿再试".into());
    }
    info.urls.retain(|u| u.starts_with("https://") || u.starts_with("http://"));
    if info.urls.is_empty() || info.size == 0 || info.size > MAX_SIZE || info.signature.trim().is_empty() {
        return Err("官网上的版本信息不完整，过会儿再试".into());
    }
    Ok(info)
}

fn update_url() -> String {
    std::env::var("DINGMI_UPDATE_URL").ok().filter(|u| !u.trim().is_empty()).unwrap_or_else(|| UPDATE_URL.into())
}

/// 请求头里只写 Dingmi/版本号
fn client(version: &str) -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(format!("Dingmi/{version}"))
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default()
}

/// 取一次 update.json
async fn fetch_info(client: &reqwest::Client, url: &str) -> Result<Info, String> {
    let resp = client
        .get(url)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .ok()
        .filter(|r| r.status().is_success())
        .ok_or(OFFLINE)?;
    let text = resp.text().await.map_err(|_| OFFLINE)?;
    let mut info = parse_info(&text)?;
    trust_meta(pubkey_line(PUBKEY_FILE), &mut info);
    Ok(info)
}

fn unb64(b64: &str) -> Option<String> {
    STANDARD.decode(b64.trim()).ok().and_then(|b| String::from_utf8(b).ok())
}

/// 公钥文件里 # 开头的是说明，第一行别的就是公钥
fn pubkey_line(file: &str) -> &str {
    file.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#')).unwrap_or("")
}

/// 核对安装包的签名。签名是 base64 包着的 minisign 文本（tauri signer 的格式）；
/// 签名时写进去的版本号也要对得上，免得有人拿旧的安装包冒充新版本
fn verify_with(pubkey: &str, bytes: &[u8], sig: &str, version: &str) -> Result<(), String> {
    let pk = minisign_verify::PublicKey::from_base64(pubkey).map_err(|_| "程序里的公钥坏了")?;
    let sig = unb64(sig).and_then(|t| minisign_verify::Signature::decode(&t).ok()).ok_or(BAD_FILE)?;
    pk.verify(bytes, &sig, false).map_err(|_| BAD_FILE)?;
    let signed = sig.trusted_comment().split('\t').find_map(|f| f.strip_prefix("version:"));
    if signed != Some(version) {
        return Err(BAD_FILE.into());
    }
    Ok(())
}

fn verify(bytes: &[u8], info: &Info) -> Result<(), String> {
    verify_with(pubkey_line(PUBKEY_FILE), bytes, &info.signature, &info.version).and_then(|_| sha_ok(bytes, info))
}

/// 签过名的 update.json 里有 sha256 的话，安装包也要对得上（没签名的 sha256 在 trust_meta 里已经去掉了）
fn sha_ok(bytes: &[u8], info: &Info) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    match &info.sha256 {
        Some(want) => {
            let got: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
            if got.eq_ignore_ascii_case(want.trim()) { Ok(()) } else { Err(BAD_FILE.into()) }
        }
        None => Ok(()),
    }
}

/// 从一个地址下载，大小要和 update.json 里写的一样
async fn download_one(client: &reqwest::Client, url: &str, size: u64, progress: &mut impl FnMut(u64)) -> Result<Vec<u8>, String> {
    let mut resp = client.get(url).send().await.ok().filter(|r| r.status().is_success()).ok_or(OFFLINE)?;
    let mut buf = Vec::with_capacity(size as usize);
    while let Some(chunk) = resp.chunk().await.map_err(|_| OFFLINE)? {
        if (buf.len() + chunk.len()) as u64 > size {
            return Err(BAD_FILE.into());
        }
        buf.extend_from_slice(&chunk);
        progress(buf.len() as u64);
    }
    if buf.len() as u64 != size {
        return Err("没下载完整，过会儿再试".into());
    }
    Ok(buf)
}

/// 按顺序试几个下载地址，下完核对签名，回安装包的内容。一个地址不行就换下一个
async fn fetch_installer(client: &reqwest::Client, info: &Info, pubkey: &str, mut progress: impl FnMut(u64)) -> Result<Vec<u8>, String> {
    let mut err = OFFLINE.to_string();
    for url in &info.urls {
        progress(0);
        match download_one(client, url, info.size, &mut progress).await {
            Ok(buf) => match verify_with(pubkey, &buf, &info.signature, &info.version).and_then(|_| sha_ok(&buf, info)) {
                Ok(()) => return Ok(buf),
                Err(e) => err = e,
            },
            // 签名不对比连不上要紧，留着它
            Err(e) if err != BAD_FILE => err = e,
            Err(_) => {}
        }
    }
    Err(err)
}

fn update_dir(app: &AppHandle) -> PathBuf {
    app.state::<AppState>().data_dir.join("update")
}

fn installer_path(app: &AppHandle, version: &str) -> PathBuf {
    update_dir(app).join(format!("dingmi_{version}_x64-setup.exe"))
}

/// 记下 min_version，算出本机是不是太旧了
fn apply_min(app: &AppHandle, min: Option<&str>) {
    let old = min.is_some_and(|m| newer(m, &current(app)));
    // 变了就马上叫同步重来一遍：太旧了马上停，更新以后（或者官网降回来）马上接着同步
    if TOO_OLD.swap(old, Ordering::Relaxed) != old {
        crate::sync::restart(app);
    }
    let st = app.state::<AppState>();
    let c = st.db.lock().unwrap();
    let _ = match min {
        Some(m) => db::kv_set(&c, MIN_VERSION, m),
        None => db::kv_del(&c, MIN_VERSION),
    };
}

/// 检查一次。manual：点按钮检查的，连不上要告诉用户；自动检查连不上就算了
pub async fn check(app: &AppHandle, manual: bool) -> Result<(), String> {
    if cfg!(feature = "no-update") {
        return Err("测试版不检查更新".into());
    }
    {
        let mut s = STATUS.lock().unwrap();
        if s.checking {
            return Ok(());
        }
        s.checking = true;
    }
    set(app, |s| {
        if manual {
            s.error = None;
        }
    });
    let c = client(&current(app));
    // 顺便取一次协议的版本（terms.rs）：取不到、看不懂都不管，不影响检查更新
    {
        let (app, c) = (app.clone(), c.clone());
        tauri::async_runtime::spawn(async move { crate::terms::refresh(&app, &c).await });
    }
    let r = fetch_info(&c, &update_url()).await;
    if let Ok(info) = &r {
        apply_min(app, info.min_version.as_deref());
    }
    let cur = current(app);
    crate::diag::log(
        "更新",
        match &r {
            Ok(info) if newer(&info.version, &cur) => format!("检查更新：有新版本 {}", info.version),
            Ok(_) => "检查更新：已经是最新版".to_string(),
            Err(e) => format!("检查更新没成：{e}"),
        },
    );
    set(app, |s| {
        s.checking = false;
        match &r {
            Ok(info) => {
                s.checked_at = Some(db::now_sec());
                let busy = s.state == "downloading" || s.state == "ready";
                if busy {
                    // 正在下载、已经下载好的不打断；下载完再说
                    return;
                }
                s.error = None;
                if newer(&info.version, &cur) {
                    s.state = "available".into();
                    s.version = Some(info.version.clone());
                    s.notes = Some(info.notes.clone());
                    s.date = Some(info.date.clone());
                    s.size = info.size;
                    s.got = 0;
                    *LATEST.lock().unwrap() = Some(info.clone());
                } else {
                    s.state = "idle".into();
                    s.version = None;
                    s.notes = None;
                    s.date = None;
                    *LATEST.lock().unwrap() = None;
                }
            }
            Err(e) if manual => s.error = Some(e.clone()),
            Err(_) => {}
        }
    });
    r.map(|_| ())
}

/// 点「更新」：在后台下载、核对签名，好了变成 ready。没有新版本、已经在下载时什么都不做
pub fn download(app: &AppHandle) {
    let Some(info) = LATEST.lock().unwrap().clone() else { return };
    {
        let mut s = STATUS.lock().unwrap();
        if s.state != "available" {
            return;
        }
        s.state = "downloading".into();
        s.got = 0;
        s.error = None;
    }
    set(app, |_| {});
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let r = download_inner(&app, &info).await;
        crate::diag::log(
            "更新",
            match &r {
                Ok(_) => format!("新版本 {} 下载好了", info.version),
                Err(e) => format!("新版本 {} 没下载成：{e}", info.version),
            },
        );
        set(&app, |s| match r {
            Ok(path) => {
                *READY.lock().unwrap() = Some(path);
                s.state = "ready".into();
            }
            Err(e) => {
                s.state = "available".into();
                s.got = 0;
                s.error = Some(e);
            }
        });
    });
}

async fn download_inner(app: &AppHandle, info: &Info) -> Result<PathBuf, String> {
    let path = installer_path(app, &info.version);
    // 上次已经下载好了（比如下载完没来得及装就关了电脑）
    if std::fs::read(&path).is_ok_and(|b| verify(&b, info).is_ok()) {
        return Ok(path);
    }
    // 进度最多每 0.2 秒告诉主窗口一次
    let mut last = Instant::now();
    let progress = |got: u64| {
        if got == 0 || got == info.size || last.elapsed() >= Duration::from_millis(200) {
            last = Instant::now();
            STATUS.lock().unwrap().got = got;
            let _ = app.emit("update-status", status(app));
        }
    };
    let bytes = fetch_installer(&client(&current(app)), info, pubkey_line(PUBKEY_FILE), progress).await;
    let bytes = match bytes {
        Ok(b) => b,
        Err(e) => {
            let _ = std::fs::remove_file(&path);
            return Err(e);
        }
    };
    std::fs::create_dir_all(update_dir(app)).map_err(|e| format!("存不下安装包（{e}）"))?;
    std::fs::write(&path, &bytes).map_err(|e| format!("存不下安装包（{e}），看看磁盘是不是满了"))?;
    Ok(path)
}

/// 重启更新：再核对一遍签名，记下现在的版本，启动安装包后退出（静默安装，装完会重新打开叮咪）
pub fn install(app: &AppHandle) -> Result<(), String> {
    let path = READY.lock().unwrap().clone().ok_or("还没有下载好的新版本")?;
    let info = LATEST.lock().unwrap().clone().ok_or("还没有下载好的新版本")?;
    let bytes = std::fs::read(&path).map_err(|_| "下载好的安装包不见了，再点一次「更新」")?;
    if let Err(e) = verify(&bytes, &info) {
        crate::diag::log("更新", format!("安装包没通过核对：{e}"));
        let _ = std::fs::remove_file(&path);
        *READY.lock().unwrap() = None;
        set(app, |s| {
            s.state = "available".into();
            s.error = Some(e.clone());
        });
        return Err(e);
    }
    {
        let st = app.state::<AppState>();
        let c = st.db.lock().unwrap();
        db::kv_set(&c, UPDATED_FROM, &current(app)).map_err(|e| e.to_string())?;
    }
    // /P 只显示进度条、/UPDATE 覆盖安装、/R 装完重新打开
    std::process::Command::new(&path)
        .args(["/P", "/UPDATE", "/R"])
        .spawn()
        .map_err(|e| format!("打不开安装包（{e}）"))?;
    app.exit(0);
    Ok(())
}

/// 点 ✕：这个版本不再提示，出了更新的版本再提示
pub fn dismiss(app: &AppHandle) {
    let Some(v) = STATUS.lock().unwrap().version.clone() else { return };
    {
        let st = app.state::<AppState>();
        let _ = db::kv_set(&st.db.lock().unwrap(), DISMISSED, &v);
    }
    *DISMISSED_V.lock().unwrap() = Some(v);
    set(app, |_| {});
}

/// 更新后第一次打开：回一句"已经更新到 x"，顺便删掉下载的安装包
pub fn take_notice(app: &AppHandle) -> Option<String> {
    let st = app.state::<AppState>();
    let from = {
        let c = st.db.lock().unwrap();
        let from = db::kv_get(&c, UPDATED_FROM)?;
        let _ = db::kv_del(&c, UPDATED_FROM);
        from
    };
    let _ = std::fs::remove_dir_all(update_dir(app));
    let now = current(app);
    (from != now).then(|| format!("叮咪已经更新到 {now}"))
}

pub fn start(app: AppHandle) {
    crate::terms::load(&app);
    {
        let st = app.state::<AppState>();
        let c = st.db.lock().unwrap();
        *DISMISSED_V.lock().unwrap() = db::kv_get(&c, DISMISSED);
        let min = db::kv_get(&c, MIN_VERSION);
        TOO_OLD.store(min.is_some_and(|m| newer(&m, &current(&app))), Ordering::Relaxed);
    }
    // 同步比这里先启动，可能已经连上了：太旧的话叫它重来，马上停下
    if too_old() {
        crate::sync::restart(&app);
    }
    if cfg!(feature = "no-update") {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            let auto = app.state::<AppState>().settings.lock().unwrap().auto_update;
            if auto {
                let _ = check(&app, false).await;
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn versions() {
        assert!(newer("0.3.1", "0.3.0"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(newer("1.0", "0.99.99"));
        assert!(newer("1.0.1", "1.0"));
        assert!(!newer("0.3.0", "0.3.0"));
        assert!(!newer("0.2.9", "0.3.0"));
        assert!(valid_version("0.3.1") && valid_version("1"));
        assert!(!valid_version("../x") && !valid_version("") && !valid_version("1..2") && !valid_version("1.0."));
    }

    #[test]
    fn parse_update_json() {
        let full = r#"{"version":"0.2.0","date":"2026-11-05","notes":"修了点东西\n快了一点","urls":["https://dingmi.app/download/a.exe","http://mirror/a.exe"],"size":123,"signature":"c2ln","min_version":"0.1.5"}"#;
        let i = parse_info(full).unwrap();
        assert_eq!(i.version, "0.2.0");
        assert_eq!(i.urls.len(), 2);
        assert_eq!(i.min_version.as_deref(), Some("0.1.5"));
        assert_eq!(i.notes, "修了点东西\n快了一点");

        // date、notes、min_version 可以不写；min_version 写空的等于没写
        let i = parse_info(r#"{"version":"0.2.0","urls":["https://x/a.exe"],"size":1,"signature":"c2ln"}"#).unwrap();
        assert_eq!((i.date.as_str(), i.notes.as_str(), i.min_version), ("", "", None));
        let i = parse_info(r#"{"version":"0.2.0","urls":["https://x/a.exe"],"size":1,"signature":"c2ln","min_version":""}"#).unwrap();
        assert_eq!(i.min_version, None);
        // 多出来的字段不管
        assert!(parse_info(r#"{"version":"0.2.0","urls":["https://x/a.exe"],"size":1,"signature":"c2ln","new":1}"#).is_ok());

        // 缺了必须有的、版本号不对、没有能用的地址
        for bad in [
            r#"{"urls":["https://x/a.exe"],"size":1,"signature":"c2ln"}"#,
            r#"{"version":"0.2.0","size":1,"signature":"c2ln"}"#,
            r#"{"version":"0.2.0","urls":["https://x/a.exe"],"signature":"c2ln"}"#,
            r#"{"version":"0.2.0","urls":["https://x/a.exe"],"size":1}"#,
            r#"{"version":"0.2.0","urls":["https://x/a.exe"],"size":1,"signature":" "}"#,
            r#"{"version":"0.2.0","urls":[],"size":1,"signature":"c2ln"}"#,
            r#"{"version":"0.2.0","urls":["file:///c:/a.exe"],"size":1,"signature":"c2ln"}"#,
            r#"{"version":"../0.2","urls":["https://x/a.exe"],"size":1,"signature":"c2ln"}"#,
            r#"{"version":"0.2.0","urls":["https://x/a.exe"],"size":1,"signature":"c2ln","min_version":"abc"}"#,
            r#"{"version":"0.2.0","urls":["https://x/a.exe"],"size":0,"signature":"c2ln"}"#,
            "<html>404</html>",
        ] {
            assert!(parse_info(bad).is_err(), "{bad}");
        }
    }

    /// 测试用的一对密钥。回（公钥那一行，私钥）
    fn test_keys() -> (String, minisign::SecretKey) {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        (kp.pk.to_base64(), kp.sk)
    }

    /// 和 tauri signer sign --app-version 签出来的一样：base64 包着 .sig 文本，可信注释里有版本号
    fn sign(sk: &minisign::SecretKey, bytes: &[u8], version: &str) -> String {
        let trusted = format!("timestamp:1793880000\tfile:dingmi_{version}_x64-setup.exe\tversion:{version}");
        let sig = minisign::sign(None, sk, std::io::Cursor::new(bytes), Some(&trusted), Some("signature from tauri secret key")).unwrap();
        STANDARD.encode(sig.to_string())
    }

    #[test]
    fn signature() {
        let (pk, sk) = test_keys();
        let setup = b"pretend this is setup.exe".to_vec();
        let sig = sign(&sk, &setup, "0.2.0");
        assert!(verify_with(&pk, &setup, &sig, "0.2.0").is_ok());
        // 改一个字节就不认
        let mut bad = setup.clone();
        bad[3] ^= 1;
        assert_eq!(verify_with(&pk, &bad, &sig, "0.2.0").unwrap_err(), BAD_FILE);
        // 签的是 0.2.0，update.json 说是 0.3.0：拿旧安装包冒充新版本，不认
        assert!(verify_with(&pk, &setup, &sig, "0.3.0").is_err());
        // 别的私钥签的不认
        let (_, other) = test_keys();
        assert!(verify_with(&pk, &setup, &sign(&other, &setup, "0.2.0"), "0.2.0").is_err());
        // 签名不是 base64、不是 minisign
        assert!(verify_with(&pk, &setup, "not base64!", "0.2.0").is_err());
        assert!(verify_with(&pk, &setup, "bm90IGEgc2ln", "0.2.0").is_err());
        // 没写版本号的签名也不认
        let plain = minisign::sign(None, &sk, std::io::Cursor::new(&setup), Some("timestamp:1"), None).unwrap();
        assert!(verify_with(&pk, &setup, &STANDARD.encode(plain.to_string()), "0.2.0").is_err());
    }

    /// 发布脚本和这里签的是同一段文字（scripts/release.mjs 的 metaText，测试 apps/desktop/ui/lib/update-meta.node.test.ts 对的是同一个字符串）
    #[test]
    fn meta_text_matches_release_script() {
        assert_eq!(meta_text("0.5.0", Some("0.4.1"), Some("ab12")), "dingmi-update-meta-v1\nversion:0.5.0\nmin_version:0.4.1\nsha256:ab12\n");
        assert_eq!(meta_text("0.5.0", None, None), "dingmi-update-meta-v1\nversion:0.5.0\nmin_version:\nsha256:\n");
    }

    /// 攻击：更新通道被劫持，有人往 update.json 里写一个很大的 min_version（修之前 min_version 没签名，所有叮咪都会停下同步）。
    /// 现在 min_version 要和 version、sha256 一起签名，对不上、没签名都不认；安装包照旧能核对、能更新
    #[test]
    fn unsigned_min_version_is_ignored() {
        let (pk, sk) = test_keys();
        let setup = b"setup".to_vec();
        let sum: String = {
            use sha2::{Digest, Sha256};
            Sha256::digest(&setup).iter().map(|b| format!("{b:02x}")).collect()
        };
        let signed = |min: Option<&str>| sign(&sk, meta_text("0.5.0", min, Some(&sum)).as_bytes(), "0.5.0");
        let info = |min: Option<&str>, meta: Option<String>| Info {
            version: "0.5.0".into(),
            date: String::new(),
            notes: String::new(),
            urls: vec!["https://x/a.exe".into()],
            size: setup.len() as u64,
            signature: sign(&sk, &setup, "0.5.0"),
            min_version: min.map(str::to_string),
            sha256: Some(sum.clone()),
            meta_signature: meta,
        };
        // 正经发布的：签名对得上，认 min_version
        let mut ok = info(Some("0.4.1"), Some(signed(Some("0.4.1"))));
        assert!(trust_meta(&pk, &mut ok));
        assert_eq!(ok.min_version.as_deref(), Some("0.4.1"));
        // 劫持者把 min_version 改大（签名是原来那个）：不认
        let mut evil = info(Some("99.0"), Some(signed(Some("0.4.1"))));
        assert!(!trust_meta(&pk, &mut evil));
        assert_eq!(evil.min_version, None);
        // 劫持者自己加了 min_version，没有签名（旧格式的 update.json 也是这样）：不认
        let mut evil = info(Some("99.0"), None);
        assert!(!trust_meta(&pk, &mut evil));
        assert_eq!(evil.min_version, None);
        // 用别的私钥签的：不认
        let (_, other) = test_keys();
        let mut evil = info(Some("99.0"), Some(sign(&other, meta_text("0.5.0", Some("99.0"), Some(&sum)).as_bytes(), "0.5.0")));
        assert!(!trust_meta(&pk, &mut evil));
        // 拿旧版本的签名（签的是 0.4.0）冒充：不认
        let mut evil = info(Some("99.0"), Some(sign(&sk, meta_text("0.5.0", Some("99.0"), Some(&sum)).as_bytes(), "0.4.0")));
        assert!(!trust_meta(&pk, &mut evil));
        // 不认 min_version 也照样能核对安装包
        assert!(verify_with(&pk, &setup, &evil.signature, &evil.version).is_ok());
        // 签过名的 sha256 要和安装包对得上
        assert!(sha_ok(&setup, &ok).is_ok());
        assert_eq!(sha_ok(b"other", &ok).unwrap_err(), BAD_FILE);
    }

    /// 仓库里的公钥文件要能读出来
    #[test]
    fn pubkey_file() {
        assert!(minisign_verify::PublicKey::from_base64(pubkey_line(PUBKEY_FILE)).is_ok());
        assert_eq!(pubkey_line("# 说明\n\n  RWabc  \n# 后面的"), "RWabc");
        // 程序里的公钥不认测试私钥签的
        let (_, sk) = test_keys();
        let info = Info { version: "0.2.0".into(), date: String::new(), notes: String::new(), urls: vec![], size: 1, signature: sign(&sk, b"x", "0.2.0"), min_version: None, sha256: None, meta_signature: None };
        assert!(verify(b"x", &info).is_err());
    }

    /// 在 127.0.0.1 上起一个小 HTTP 服务：路径对应内容，别的回 404。回地址开头
    fn serve(files: Vec<(&'static str, Vec<u8>)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut req = Vec::new();
                let mut buf = [0u8; 1024];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    match s.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => req.extend_from_slice(&buf[..n]),
                    }
                }
                let line = String::from_utf8_lossy(&req).lines().next().unwrap_or("").to_string();
                let path = line.split(' ').nth(1).unwrap_or("");
                let ua = String::from_utf8_lossy(&req).lines().find_map(|l| l.to_ascii_lowercase().starts_with("user-agent:").then(|| l[11..].trim().to_string())).unwrap_or_default();
                let body = match files.iter().find(|(p, _)| *p == path) {
                    // /ua 回请求头里的 User-Agent，看看带了什么
                    _ if path == "/ua" => Some(ua.into_bytes()),
                    Some((_, b)) => Some(b.clone()),
                    None => None,
                };
                let head = match &body {
                    Some(b) => format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", b.len()),
                    None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
                };
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&body.unwrap_or_default());
            }
        });
        base
    }

    fn local_client() -> reqwest::Client {
        reqwest::Client::builder().user_agent("Dingmi/0.1.0").no_proxy().build().unwrap()
    }

    #[tokio::test]
    async fn download_falls_back_in_order() {
        let (pk, sk) = test_keys();
        let setup: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let mut tampered = setup.clone();
        tampered[100] ^= 1;
        let base = serve(vec![("/bad.exe", tampered), ("/good.exe", setup.clone()), ("/short.exe", setup[..1000].to_vec())]);
        // 关着的端口：连不上
        let closed = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}", l.local_addr().unwrap())
        };
        let info = |urls: Vec<String>| Info {
            version: "0.2.0".into(),
            date: String::new(),
            notes: String::new(),
            urls,
            size: setup.len() as u64,
            signature: sign(&sk, &setup, "0.2.0"),
            min_version: None,
            sha256: None,
            meta_signature: None,
        };
        let c = local_client();

        // 连不上 → 404 → 不完整 → 被改过 → 对的：最后拿到对的
        let urls = vec![format!("{closed}/a.exe"), format!("{base}/none.exe"), format!("{base}/short.exe"), format!("{base}/bad.exe"), format!("{base}/good.exe")];
        let mut last = 0;
        let got = fetch_installer(&c, &info(urls), &pk, |n| last = n).await.unwrap();
        assert_eq!(got, setup);
        assert_eq!(last, setup.len() as u64);

        // 只有被改过的：提示文件不对
        let err = fetch_installer(&c, &info(vec![format!("{base}/bad.exe"), format!("{closed}/a.exe")]), &pk, |_| {}).await.unwrap_err();
        assert_eq!(err, BAD_FILE);
        // 都连不上
        let err = fetch_installer(&c, &info(vec![format!("{closed}/a.exe"), format!("{base}/none.exe")]), &pk, |_| {}).await.unwrap_err();
        assert_eq!(err, OFFLINE);
    }

    #[tokio::test]
    async fn fetch_update_json() {
        let json = br#"{"version":"0.2.0","urls":["https://x/a.exe"],"size":1,"signature":"c2ln","min_version":"0.1.1"}"#.to_vec();
        let base = serve(vec![("/update.json", json), ("/bad.json", b"{}".to_vec())]);
        let c = local_client();
        let i = fetch_info(&c, &format!("{base}/update.json")).await.unwrap();
        // 这份 update.json 的 min_version 没签名：不认（见 unsigned_min_version_is_ignored）
        assert_eq!((i.version.as_str(), i.min_version.as_deref()), ("0.2.0", None));
        assert_eq!(fetch_info(&c, &format!("{base}/none.json")).await.unwrap_err(), OFFLINE);
        assert!(fetch_info(&c, &format!("{base}/bad.json")).await.is_err());
        // 请求头里只写版本号
        let ua = c.get(format!("{base}/ua")).send().await.unwrap().text().await.unwrap();
        assert_eq!(ua, "Dingmi/0.1.0");
        let ua = client("0.1.0").get(format!("{base}/ua")).send().await;
        // 装了代理的电脑上可能走代理连不到 127.0.0.1，连上了才比
        if let Ok(r) = ua {
            if r.status().is_success() {
                assert_eq!(r.text().await.unwrap(), "Dingmi/0.1.0");
            }
        }
    }

    /// 发布后手动核对：npm run release 生成的 update.json 和安装包，程序里的公钥要认。
    /// 用法：cargo test -p dingmi published_site -- --ignored（DINGMI_SITE_DIR 默认是仓库里的 release/site）
    #[test]
    #[ignore]
    fn published_site() {
        let dir = std::env::var("DINGMI_SITE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../release/site"));
        let info = parse_info(&std::fs::read_to_string(dir.join("update.json")).unwrap()).unwrap();
        let name = info.urls[0].rsplit('/').next().unwrap();
        let setup = std::fs::read(dir.join("download").join(name)).unwrap();
        assert_eq!(setup.len() as u64, info.size);
        verify(&setup, &info).unwrap();
    }
}
