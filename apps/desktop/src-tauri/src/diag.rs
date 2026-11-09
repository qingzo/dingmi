//! 诊断信息（需求-0.3 第 12 节）：平时记日志，出了问题用户自己导出一个 zip 发给作者。叮咪自己不往外发任何东西。
//!
//! - 日志在数据目录的 `logs/` 里，每天一个文件，留 7 天，单个文件太大就截掉前面的。
//! - 只记出错、启动退出、同步、检查更新、快捷键、窗口这类事情；**不记任何内容**：提醒和笔记的标题正文、
//!   附件的文件名、快捷框里打的字、AI 收发的内容、Key 和密码一律不进日志。写日志的地方自己注意，这里只管路径里的用户名。
//! - 意外退出（panic）记进日志，下次启动不提示。
//! - 导出：先在临时文件夹里把要放进去的几样写好，列给用户看（可以一个个打开看看），确认了再打成 zip 放到桌面。

use crate::{db, AppState};
use serde::Serialize;
use std::fmt::Display;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Manager};

static DIR: OnceLock<PathBuf> = OnceLock::new();
static WRITE: Mutex<()> = Mutex::new(());
/// 日志留几天
const KEEP_DAYS: i64 = 7;
/// 一天的日志超过这么大，截掉前面一半
const MAX_LOG: u64 = 1024 * 1024;

/// 启动时调一次：建好 logs/，删掉 7 天前的，接住意外退出
pub fn init(data_dir: &Path) {
    let dir = data_dir.join("logs");
    let _ = std::fs::create_dir_all(&dir);
    prune(&dir, chrono::Local::now().date_naive());
    let _ = DIR.set(dir);
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log("出错", format!("程序出错了：{info}"));
        prev(info);
    }));
}

/// 删掉 7 天前的日志（名字是 YYYY-MM-DD.log）
fn prune(dir: &Path, today: chrono::NaiveDate) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let day = name.strip_suffix(".log").and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
        if day.is_some_and(|d| (today - d).num_days() >= KEEP_DAYS) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// 记一行日志。kind 是哪方面（启动、同步、更新、快捷键、窗口、出错），msg 里不能有用户写的内容
pub fn log(kind: &str, msg: impl Display) {
    let Some(dir) = DIR.get() else { return };
    let now = chrono::Local::now();
    let line = format!("{} [{kind}] {}\n", now.format("%H:%M:%S"), hide_user(&msg.to_string()).replace('\n', " "));
    // 出错退出时可能正拿着锁：坏了的锁照样用
    let _g = WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let path = dir.join(format!("{}.log", now.format("%Y-%m-%d")));
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG) {
        trim(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
    }
}

/// 截掉前面一半，从一整行开始留
fn trim(path: &Path) {
    let Ok(text) = std::fs::read_to_string(path) else {
        let _ = std::fs::remove_file(path);
        return;
    };
    let mut half = text.len() / 2;
    while !text.is_char_boundary(half) {
        half += 1;
    }
    let start = text[half..].find('\n').map_or(text.len(), |i| half + i + 1);
    let _ = std::fs::write(path, format!("（前面的太长，截掉了）\n{}", &text[start..]));
}

/// 路径里的 Windows 用户名换成 <用户>：C:\Users\张三\AppData → C:\Users\<用户>\AppData
pub fn hide_user(s: &str) -> String {
    let Some(name) = std::env::var("USERNAME").ok().filter(|n| n.len() > 1) else {
        return s.to_string();
    };
    let mut out = s.to_string();
    for sep in ['\\', '/'] {
        let from = format!("{sep}Users{sep}{name}");
        let to = format!("{sep}Users{sep}<用户>");
        // 盘符后面的大小写不一定，按不分大小写找
        let lower = out.to_lowercase();
        let pat = from.to_lowercase();
        let mut res = String::new();
        let mut last = 0;
        for (i, _) in lower.match_indices(&pat) {
            res.push_str(&out[last..i]);
            res.push_str(&to);
            last = i + from.len();
        }
        res.push_str(&out[last..]);
        out = res;
    }
    out
}

// ---------- 导出 ----------

/// 诊断包里的一样东西，导出前列给用户看
#[derive(Serialize)]
pub struct Part {
    pub name: String,
    pub desc: String,
    /// 临时文件夹里的位置，「打开看看」用
    pub path: String,
}

pub fn temp_dir() -> PathBuf {
    std::env::temp_dir().join("dingmi-diag")
}

/// 写好要放进诊断包的几样，放在临时文件夹里
pub fn prepare(app: &AppHandle) -> Result<Vec<Part>, String> {
    let dir = temp_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("日志")).map_err(|e| format!("建不了临时文件夹：{e}"))?;
    let mut parts = Vec::new();
    let mut put = |name: &str, desc: &str, text: String| -> Result<(), String> {
        let p = dir.join(name);
        std::fs::write(&p, hide_user(&text)).map_err(|e| format!("写不了 {name}：{e}"))?;
        parts.push(Part { name: name.into(), desc: desc.into(), path: p.display().to_string() });
        Ok(())
    };
    put("电脑和叮咪.txt", "叮咪和 Windows 的版本、屏幕、WebView2、占用的内存", system_text(app))?;
    // 内容是 JSON，存成 .txt：很多电脑上 .json 没有默认打开方式，「打开看看」会没反应
    put("设置.txt", "各项设置的值；AI Key、同步密码、配对信息已经去掉", settings_text(app))?;
    put("同步.txt", "同步方式、最后同步的时间、几台电脑、最近的出错原因；不含服务器地址和文件夹位置", sync_text(app))?;
    put("数据条数.txt", "多少条提醒、笔记、附件，数据库多大；只有数字", counts_text(app))?;
    // 最近 7 天的日志
    let mut days = 0;
    if let Some(Ok(rd)) = DIR.get().map(std::fs::read_dir) {
        for e in rd.flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".log")) {
            if let Ok(text) = std::fs::read_to_string(e.path()) {
                let _ = std::fs::write(dir.join("日志").join(e.file_name()), hide_user(&text));
                days += 1;
            }
        }
    }
    parts.push(Part {
        name: "日志".into(),
        desc: format!("最近 {days} 天出过的错、同步和检查更新的记录；不记提醒和笔记的内容"),
        path: dir.join("日志").display().to_string(),
    });
    Ok(parts)
}

/// 打成 zip 放到桌面：叮咪诊断_2026-11-05_1430.zip。回 zip 的位置
pub fn export(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = temp_dir();
    if !dir.exists() {
        prepare(app)?;
    }
    let desk = app.path().desktop_dir().map_err(|e| format!("找不到桌面：{e}"))?;
    let stem = format!("叮咪诊断_{}", chrono::Local::now().format("%Y-%m-%d_%H%M"));
    let to = (1..100)
        .map(|n| desk.join(if n == 1 { format!("{stem}.zip") } else { format!("{stem} ({n}).zip") }))
        .find(|p| !p.exists())
        .unwrap_or_else(|| desk.join(format!("{stem}.zip")));
    let f = std::fs::File::create(&to).map_err(|e| format!("在桌面上存不下：{e}"))?;
    let mut z = zip::ZipWriter::new(f);
    let opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut add = |rel: String, path: &Path| -> Result<(), String> {
        let mut bytes = Vec::new();
        std::fs::File::open(path).and_then(|mut f| f.read_to_end(&mut bytes)).map_err(|e| e.to_string())?;
        z.start_file(rel, opt).map_err(|e| e.to_string())?;
        z.write_all(&bytes).map_err(|e| e.to_string())
    };
    for e in std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if e.path().is_dir() {
            for f in std::fs::read_dir(e.path()).map_err(|e| e.to_string())?.flatten() {
                add(format!("{name}/{}", f.file_name().to_string_lossy()), &f.path())?;
            }
        } else {
            add(name, &e.path())?;
        }
    }
    z.finish().map_err(|e| format!("没打包好：{e}"))?;
    let _ = std::fs::remove_dir_all(&dir);
    log("诊断", "导出了诊断信息");
    Ok(to)
}

fn system_text(app: &AppHandle) -> String {
    let monitors = app.available_monitors().unwrap_or_default();
    let screens: Vec<String> = monitors
        .iter()
        .map(|m| format!("{}×{} 缩放 {:.0}%", m.size().width, m.size().height, m.scale_factor() * 100.0))
        .collect();
    format!(
        "叮咪 {}\nWindows {}\n屏幕 {} 块：{}\nWebView2 {}\n叮咪占用的内存 {}\n导出时间 {}\n",
        app.package_info().version,
        windows_version(),
        monitors.len(),
        screens.join("；"),
        tauri::webview_version().unwrap_or_else(|_| crate::sys::webview2_version().trim().to_string()),
        memory().map_or("不知道".into(), |b| format!("{:.0} MB", b as f64 / 1024.0 / 1024.0)),
        chrono::Local::now().format("%Y-%m-%d %H:%M"),
    )
}

/// 设置：去掉 AI Key；快捷框自己加的、别名、不显示的里面是文件位置和名字，只留个数
fn settings_text(app: &AppHandle) -> String {
    let st = app.state::<AppState>();
    let s = st.settings.lock().unwrap().clone();
    let mut v = serde_json::to_value(&s).unwrap_or_default();
    if let Some(o) = v.as_object_mut() {
        o.insert("ai_key".into(), (if s.ai_key.trim().is_empty() { "（没填）" } else { "（填了，已去掉）" }).into());
        o.insert(
            "launcher".into(),
            serde_json::json!({
                "常用": s.launcher.pins.len(),
                "自己加的": s.launcher.custom.len(),
                "别名": s.launcher.aliases.len(),
                "不显示的": s.launcher.hidden.len(),
                "关掉的来源": s.launcher.off,
            }),
        );
    }
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// 同步：服务器地址、文件夹位置换掉，出错原因里也换掉
fn sync_text(app: &AppHandle) -> String {
    let s = crate::sync::sync_status(app.clone());
    let mut err = s.error.clone().unwrap_or_else(|| "没有".into());
    if let Some(x) = s.server.as_deref().filter(|x| !x.is_empty()) {
        err = err.replace(x, "<服务器>");
    }
    if let Some(x) = s.folder.as_deref().filter(|x| !x.is_empty()) {
        err = err.replace(x, "<同步文件夹>");
    }
    let way = match s.kind.as_str() {
        "server" => "自建服务器",
        "folder" => "网盘文件夹",
        _ => "没开",
    };
    let st = app.state::<AppState>();
    let pcs = db::list_devices(&st.db.lock().unwrap(), &st.device).map_or(0, |d| d.len());
    format!(
        "同步方式 {way}\n状态 {}\n最后连上 {}\n几台电脑 {pcs}\n最近的出错原因 {err}\n",
        s.state,
        s.last_online.as_deref().unwrap_or("没有"),
    )
}

fn counts_text(app: &AppHandle) -> String {
    let st = app.state::<AppState>();
    let c = st.db.lock().unwrap();
    let n = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(-1);
    let size = std::fs::metadata(st.data_dir.join("dingmi.db")).map_or(0, |m| m.len());
    format!(
        "提醒 {} 条（做完的 {}，回收站 {}）\n笔记 {} 条（回收站 {}）\n附件 {} 个，一共 {:.1} MB\n手机发来没处理的 {} 条\n数据库 {:.1} MB\n",
        n("SELECT COUNT(*) FROM reminders WHERE deleted_at IS NULL"),
        n("SELECT COUNT(*) FROM reminders WHERE deleted_at IS NULL AND done=1"),
        n("SELECT COUNT(*) FROM reminders WHERE deleted_at IS NOT NULL AND purged=0"),
        n("SELECT COUNT(*) FROM notes WHERE deleted_at IS NULL"),
        n("SELECT COUNT(*) FROM notes WHERE deleted_at IS NOT NULL AND purged=0"),
        n("SELECT COUNT(*) FROM attachments WHERE purged=0"),
        n("SELECT COALESCE(SUM(size),0) FROM attachments WHERE purged=0") as f64 / 1024.0 / 1024.0,
        n("SELECT COUNT(*) FROM inbox"),
        size as f64 / 1024.0 / 1024.0,
    )
}

#[cfg(windows)]
fn windows_version() -> String {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let key = wide(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
    let read = |name: &str| {
        let name = wide(name);
        let mut buf = [0u16; 128];
        let mut size = std::mem::size_of_val(&buf) as u32;
        // SAFETY: 输出缓冲区大小已经告诉了系统，放不下时返回错误、不会写出界
        let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size) };
        if r != 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)])
    };
    let build: u32 = read("CurrentBuild").parse().unwrap_or(0);
    // 注册表里的 ProductName 在 Windows 11 上也写着 Windows 10，按版本号分
    let name = if build >= 22000 { "11" } else { "10" };
    format!("{name} {}（版本号 {build}）", read("DisplayVersion"))
}

#[cfg(not(windows))]
fn windows_version() -> String {
    std::env::consts::OS.into()
}

/// 叮咪这个进程占了多少内存（工作集）
#[cfg(windows)]
fn memory() -> Option<u64> {
    use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: 结构体大小已经填好，GetCurrentProcess 是不用关的伪句柄
    unsafe {
        let mut m: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        m.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        (K32GetProcessMemoryInfo(GetCurrentProcess(), &mut m, m.cb) != 0).then_some(m.WorkingSetSize as u64)
    }
}

#[cfg(not(windows))]
fn memory() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_user_name() {
        std::env::set_var("USERNAME", "Zhang");
        assert_eq!(hide_user(r"C:\Users\Zhang\AppData\Roaming\app.dingmi"), r"C:\Users\<用户>\AppData\Roaming\app.dingmi");
        assert_eq!(hide_user(r"c:\users\zhang\Desktop 和 C:/Users/Zhang/x"), r"c:\Users\<用户>\Desktop 和 C:/Users/<用户>/x");
        assert_eq!(hide_user(r"D:\Zhang\notes"), r"D:\Zhang\notes");
    }

    #[test]
    fn keeps_seven_days_and_trims() {
        let dir = std::env::temp_dir().join(format!("dingmi-logs-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        for d in ["2026-10-29", "2026-10-30", "2026-11-05", "别的.txt"] {
            std::fs::write(dir.join(if d.ends_with(".txt") { d.to_string() } else { format!("{d}.log") }), "x").unwrap();
        }
        prune(&dir, chrono::NaiveDate::from_ymd_opt(2026, 11, 5).unwrap());
        let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        left.sort();
        // 7 天前（10-29）的删掉；不是日志的不动
        assert_eq!(left, ["2026-10-30.log", "2026-11-05.log", "别的.txt"]);

        let p = dir.join("big.log");
        std::fs::write(&p, (0..100).map(|i| format!("第 {i} 行\n")).collect::<String>()).unwrap();
        trim(&p);
        let t = std::fs::read_to_string(&p).unwrap();
        assert!(t.starts_with("（前面的太长，截掉了）\n第 ") && t.ends_with("第 99 行\n") && !t.contains("第 10 行"), "{t}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
