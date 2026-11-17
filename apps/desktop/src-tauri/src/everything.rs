//! 一键开启全盘搜索（需求-1.x 第 42.7 节）：没装 Everything 的人点一下，叮咪从 dingmi.app 下载 Everything 便携版、核对指纹，
//! 放到 %LOCALAPPDATA%\app.dingmi\everything（这台电脑自己的程序，不跟着数据搬家、不进备份），
//! 装 Everything 服务读整块硬盘（要管理员确认一次），用实例名 dingmi 在后台运行，不和用户自己装的 Everything 抢。
//! 默认不开。关掉、卸载叮咪时停掉它、删掉服务（服务是我们装的才删）、删掉文件。

use crate::filesearch::everything::INSTANCE;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// 固定用这一版（Everything 是 MIT 许可证，可以跟着发）；升级跟着叮咪发版，换地址和指纹
pub const VERSION: &str = "1.4.1.1032";
pub const URL: &str = "https://dingmi.app/download/Everything-1.4.1.1032.x64.zip";
/// 下载不成功时让用户自己下（同一个文件，指纹一样）：先给 dingmi.app 的，再给 voidtools 官网的
pub const MIRROR: &str = "https://www.voidtools.com/Everything-1.4.1.1032.x64.zip";
/// voidtools 官网公布的指纹（Everything-1.4.1.1032.sha256），对不上不用
const SHA256: &str = "698df475ec44e638f66f1b6a32d28fea613cec78d3b6310e6abe53431eeb940c";
const EXE: &str = "everything.exe";
/// 不开窗口地运行（CREATE_NO_WINDOW）
const NO_WINDOW: u32 = 0x0800_0000;

/// Everything 放在哪；调试版放在 DINGMI_DATA_DIR 下面，不碰装好的叮咪的
pub fn dir() -> PathBuf {
    crate::dev_data_dir()
        .map(|d| d.join("everything"))
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("app.dingmi").join("everything")))
        .unwrap_or_else(|| PathBuf::from("everything"))
}

fn exe() -> PathBuf {
    dir().join(EXE)
}

#[derive(Serialize)]
pub struct Status {
    /// 现在用哪种搜：everything 用户自己的 / mine 叮咪带的 / index Windows 索引 / 空的是都用不了
    pub engine: &'static str,
    /// 叮咪带的 Everything 下好了
    pub ours: bool,
    /// Everything 服务装着
    pub service: bool,
    /// 用户自己装了 Everything（开着或者没开）：不出一键开启
    pub theirs: bool,
    /// 叮咪带的是哪一版
    pub version: &'static str,
    /// 自己下载的地址（下载不成功时给用户）
    pub urls: [&'static str; 2],
}

pub fn status() -> Status {
    let engine = crate::filesearch::engine();
    let theirs = engine == "everything" || crate::filesearch::everything::installed().is_some();
    Status { engine, ours: exe().is_file(), service: service_path().is_some(), theirs, version: VERSION, urls: [URL, MIRROR] }
}

/// Everything 服务指向的程序（注册表里的 ImagePath）；没装是 None
fn service_path() -> Option<String> {
    let out = std::process::Command::new("reg")
        .args(["query", r"HKLM\SYSTEM\CurrentControlSet\Services\Everything", "/v", "ImagePath"])
        .creation_flags(NO_WINDOW)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let line = text.lines().find(|l| l.contains("ImagePath"))?;
    Some(line.split("REG_EXPAND_SZ").nth(1).or_else(|| line.split("REG_SZ").nth(1))?.trim().to_string())
}

/// 服务是我们装的（指向叮咪带的 everything.exe）
fn service_is_ours() -> bool {
    service_path().is_some_and(|p| crate::allow::norm(&p).contains(&crate::allow::norm(&exe().display().to_string())))
}

/// 一键开启：下载（from_file 时是用户自己下好、选的那个压缩包）、核对指纹、解开、装服务、在后台开起来。
/// 要管理员确认的那一步用户点了「否」就报错，已经解开的留着，下次不用再下。选文件时点了取消回空的错误
pub async fn enable(from_file: bool) -> Result<(), String> {
    if status().theirs {
        return Err("你已经装了 Everything，叮咪直接用它，不用再开".into());
    }
    if !exe().is_file() {
        let zip = if from_file { pick_zip().await? } else { download().await? };
        check(&zip)?;
        unpack(&zip)?;
    }
    write_ini()?;
    if !service_is_ours() && service_path().is_none() {
        tauri::async_runtime::spawn_blocking(|| elevated(&exe(), "-install-service")).await.map_err(|e| e.to_string())??;
        crate::diag::log("搜文件", "装好了 Everything 服务");
    }
    start();
    Ok(())
}

async fn download() -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(120)).build().map_err(|e| e.to_string())?;
    // 用户看到的说大白话，原因记进日志
    let r = client.get(URL).send().await.map_err(|e| {
        crate::diag::log("出错", format!("下载 Everything 没连上：{e}"));
        "下载 Everything 没连上：网络不通，或者被防火墙、代理拦了".to_string()
    })?;
    if !r.status().is_success() {
        return Err(format!("下载 Everything 没成功（服务器回了 {}）", r.status().as_u16()));
    }
    let b = r.bytes().await.map_err(|e| {
        crate::diag::log("出错", format!("下载 Everything 断了：{e}"));
        "下载 Everything 下到一半断了".to_string()
    })?;
    Ok(b.to_vec())
}

/// 用户自己下好的压缩包：选一下，读进来
async fn pick_zip() -> Result<Vec<u8>, String> {
    let Some(p) = tauri::async_runtime::spawn_blocking(|| crate::launcher::pick(false)).await.ok().flatten() else {
        return Err(String::new());
    };
    if std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0) > MAX_ZIP {
        return Err("选的不是 Everything 的压缩包（太大了）".into());
    }
    std::fs::read(&p).map_err(|e| format!("读不了这个文件：{e}"))
}

const MAX_ZIP: u64 = 20 * 1024 * 1024;

/// 核对是不是 voidtools 发的那一份：不管是下载的还是自己选的，指纹对不上都不用
fn check(zip: &[u8]) -> Result<(), String> {
    if zip.len() as u64 > MAX_ZIP {
        return Err("这不是 Everything 的压缩包（太大了）".into());
    }
    let got = Sha256::digest(zip).iter().map(|x| format!("{x:02x}")).collect::<String>();
    if got != SHA256 {
        crate::diag::log("出错", format!("Everything 的压缩包指纹对不上：{got}"));
        return Err(format!("这个文件和官方的 Everything {VERSION}（64 位便携版）对不上，没有用它"));
    }
    Ok(())
}

/// 从 zip 里只拿 everything.exe（语言文件不要，界面不出来）
fn unpack(zip: &[u8]) -> Result<(), String> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|e| format!("Everything 的压缩包坏了：{e}"))?;
    let mut f = z.by_name(EXE).map_err(|_| "Everything 的压缩包里没有 everything.exe".to_string())?;
    let mut buf = Vec::new();
    std::io::Read::read_to_end(&mut f, &mut buf).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    let tmp = dir().join("everything.exe.part");
    std::fs::write(&tmp, &buf).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, exe()).map_err(|e| e.to_string())
}

/// 设置：不出托盘图标、不检查更新、不自己要管理员（读硬盘靠服务）、所有固定的盘都建索引
fn write_ini() -> Result<(), String> {
    let ini = "[Everything]\r\napp_data=0\r\nrun_as_admin=0\r\nshow_tray_icon=0\r\nrun_in_background=1\r\ncheck_for_updates_on_startup=0\r\n\
               auto_include_fixed_volumes=1\r\nauto_include_removable_volumes=0\r\nlanguage=2052\r\n";
    std::fs::write(dir().join("Everything.ini"), ini).map_err(|e| e.to_string())
}

/// 在后台开起来（已经开着就不管）
pub fn start() {
    if !exe().is_file() || crate::filesearch::everything::window().is_some_and(|(who, _)| who == "mine") {
        return;
    }
    let d = dir();
    let r = std::process::Command::new(exe())
        .args(["-startup", "-instance", INSTANCE, "-config"])
        .arg(d.join("Everything.ini"))
        .arg("-db")
        .arg(d.join("Everything.db"))
        .creation_flags(NO_WINDOW)
        .spawn();
    if let Err(e) = r {
        crate::diag::log("出错", format!("Everything 开不起来：{e}"));
    }
}

/// 叮咪退出时：叮咪带的 Everything 也关掉
pub fn stop() {
    if exe().is_file() {
        let _ = std::process::Command::new(exe()).args(["-instance", INSTANCE, "-exit"]).creation_flags(NO_WINDOW).status();
    }
}

/// 关掉全盘搜索：停掉、删服务（我们装的才删，要管理员确认）、删文件
pub fn disable() -> Result<(), String> {
    stop();
    if service_is_ours() {
        elevated(&exe(), "-uninstall-service")?;
        crate::diag::log("搜文件", "删掉了 Everything 服务");
    }
    // 刚退出的 Everything 可能还拿着文件，等一下再删
    for _ in 0..10 {
        if std::fs::remove_dir_all(dir()).is_ok() || !dir().exists() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    Err(format!("Everything 的文件没删干净，可以自己删掉 {}", dir().display()))
}

/// 卸载叮咪时（安装包的卸载步骤带着 --remove-everything 调过来）：开过一键全盘搜索的，照「关掉」做一遍
pub fn remove_for_uninstall() {
    if exe().is_file() || service_is_ours() {
        let _ = disable();
    }
}

/// 以管理员身份运行一下、等它做完；用户在确认框里点了「否」也报错
#[cfg(windows)]
fn elevated(exe: &std::path::Path, args: &str) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (verb, file, params) = (wide("runas"), wide(&exe.display().to_string()), wide(args));
    // SAFETY: 字符串都以 0 结尾、在调用期间有效；拿到的进程句柄用完关掉
    unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = params.as_ptr();
        info.nShow = 0;
        if ShellExecuteExW(&mut info) == 0 {
            // 1223：用户点了「否」
            return Err(if GetLastError() == 1223 { "没有允许（管理员确认点了「否」），没开成".into() } else { "要管理员确认的那一步没成功".into() });
        }
        if info.hProcess.is_null() {
            return Ok(());
        }
        WaitForSingleObject(info.hProcess, 60_000);
        let mut code = 0u32;
        GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
        if code != 0 {
            crate::diag::log("出错", format!("Everything {args} 返回 {code}"));
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn elevated(_: &std::path::Path, _: &str) -> Result<(), String> {
    Err("只支持 Windows".into())
}

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(not(windows))]
trait NoWindow {
    fn creation_flags(&mut self, _: u32) -> &mut Self;
}
#[cfg(not(windows))]
impl NoWindow for std::process::Command {
    fn creation_flags(&mut self, _: u32) -> &mut Self {
        self
    }
}
