//! 数据放在哪（需求-1.x 第 42.8 节）。默认在 %APPDATA%\app.dingmi；换了位置的话，原来那里只留一个 location.json 记着搬到哪了。
//! 正在用的数据库不能边用边搬：设置里选好以后只记下「要搬到哪」，重新打开叮咪，在打开数据库之前搬好。
//! 同步的钥匙在 Windows 凭据管理器里，和放在哪无关，同一台电脑换位置不影响同步。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const FILE: &str = "location.json";
/// 搬的时候不动的：位置记录本身、调试版的 WebView2 目录
const SKIP: [&str; 3] = [FILE, "location.json.tmp", "EBWebView"];
/// 判断文件夹是不是空的时候不算的（资源管理器自己放的）
const JUNK: [&str; 2] = ["desktop.ini", "thumbs.db"];
/// 选的文件夹里已经有别的东西：在里面新建这个文件夹放
pub const SUB: &str = "叮咪数据";

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
struct Loc {
    /// 数据现在在哪；没有是默认位置
    #[serde(default)]
    dir: Option<String>,
    /// 下次打开时要搬到这里
    #[serde(default)]
    moving: Option<String>,
    /// 要去的地方本来就是叮咪的数据（重装系统后指回去）：直接换过去用，不搬
    #[serde(default)]
    existing: bool,
}

fn read(home: &Path) -> Loc {
    std::fs::read_to_string(home.join(FILE)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn write(home: &Path, l: &Loc) -> std::io::Result<()> {
    std::fs::create_dir_all(home)?;
    let tmp = home.join("location.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(l)?)?;
    std::fs::rename(tmp, home.join(FILE))
}

/// 数据现在在哪
pub fn current(home: &Path) -> PathBuf {
    read(home).dir.map(PathBuf::from).unwrap_or_else(|| home.to_path_buf())
}

/// 同一个文件夹（不分大小写，斜杠方向、结尾的斜杠不算）
pub fn same(a: &Path, b: &Path) -> bool {
    key(a) == key(b)
}

fn key(p: &Path) -> String {
    let p = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    p.to_string_lossy().trim_start_matches(r"\\?\").replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// a 在 b 里面（或者就是 b）
fn under(a: &Path, b: &Path) -> bool {
    let (a, b) = (key(a), key(b));
    a == b || a.starts_with(&format!("{b}\\"))
}

/// 启动时、打开数据库之前做的事
pub struct Opened {
    pub dir: PathBuf,
    /// 这次刚从哪里搬过来（设置里存着的绝对路径要跟着改）
    pub moved_from: Option<PathBuf>,
    /// 要告诉用户的：搬好了 / 没搬成
    pub notice: Option<String>,
    /// 这时日志还没开始记（日志放在数据文件夹里），先攒着，定下文件夹以后再记
    pub logs: Vec<(&'static str, String)>,
}

/// 要搬的先搬好，再定下这次用哪个文件夹。找不到那个文件夹（U 盘拔了、盘符变了）时问用户，不悄悄新建一份空的
pub fn open(home: &Path) -> Opened {
    let mut l = read(home);
    let from = l.dir.as_deref().map(PathBuf::from).unwrap_or_else(|| home.to_path_buf());
    let mut out = Opened { dir: from.clone(), moved_from: None, notice: None, logs: Vec::new() };
    if let Some(to) = l.moving.take().map(PathBuf::from) {
        let r = if l.existing { Ok(None) } else { move_all(&from, &to) };
        match r {
            Ok(aside) => {
                out.logs.push(("数据", format!("数据{}：{} → {}", if l.existing { "换到已有的" } else { "搬好了" }, from.display(), to.display())));
                let aside = aside.map(|a| format!("。那里原来的一份旧数据没有盖掉，挪到了 {}，用不着的话可以删掉", a.display())).unwrap_or_default();
                l.dir = (!same(&to, home)).then(|| to.display().to_string());
                out.moved_from = (!l.existing).then(|| from.clone());
                out.notice = Some(format!("数据现在放在 {}{aside}", to.display()));
            }
            Err(e) => {
                out.logs.push(("出错", format!("数据没搬成：{e}")));
                out.notice = Some(format!("数据没搬成，还在原来的地方：{e}"));
            }
        }
        l.existing = false;
        if let Err(e) = write(home, &l) {
            out.logs.push(("出错", format!("记不下数据放在哪：{e}")));
        }
    }
    out.dir = l.dir.as_deref().map(PathBuf::from).unwrap_or_else(|| home.to_path_buf());
    while !same(&out.dir, home) && !out.dir.join("dingmi.db").is_file() {
        match ask_missing(&out.dir) {
            Missing::Retry => continue,
            Missing::Default => {
                l.dir = None;
                let _ = write(home, &l);
                out.logs.push(("数据", format!("找不到 {}，这次用默认位置", out.dir.display())));
                out.dir = home.to_path_buf();
            }
            Missing::Quit => std::process::exit(0),
        }
    }
    out
}

enum Missing {
    Retry,
    Default,
    Quit,
}

#[cfg(windows)]
fn ask_missing(dir: &Path) -> Missing {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDCONTINUE, IDTRYAGAIN, MB_CANCELTRYCONTINUE, MB_ICONWARNING};
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let text = wide(&format!(
        "叮咪的数据放在这个文件夹，现在找不到了：\n{}\n\n\
         U 盘、移动硬盘拔了，或者盘符变了的话，插回去再点「重试」。\n\n\
         点「继续」这次先用默认位置（C 盘）打开，那里是空的；找到原来的文件夹后，\
         在「设置 → 数据 → 数据放在哪」里选回它就行。\n\n点「取消」不打开叮咪。",
        dir.display()
    ));
    let title = wide("叮咪：找不到数据文件夹");
    // SAFETY: 两个字符串都以 0 结尾，在调用期间一直有效
    let r = unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_CANCELTRYCONTINUE | MB_ICONWARNING) };
    match r {
        IDTRYAGAIN => Missing::Retry,
        IDCONTINUE => Missing::Default,
        _ => Missing::Quit,
    }
}

#[cfg(not(windows))]
fn ask_missing(_: &Path) -> Missing {
    Missing::Default
}

/// 把 from 里的东西（除了 SKIP）都搬到 to：先全部复制、核对大小，都对了才删原来的；中途出错，复制过去的删掉，原来的不动。
/// to 里已经有同名的（搬回默认位置时，那里可能有找不到数据文件夹那次新建的一份）：不盖掉，先挪进 to 里的「旧数据-时间」，回挪到了哪
fn move_all(from: &Path, to: &Path) -> Result<Option<PathBuf>, String> {
    std::fs::create_dir_all(to).map_err(|e| format!("建不了 {}：{e}", to.display()))?;
    let names: Vec<_> = entries(from).map_err(|e| format!("读不了 {}：{e}", from.display()))?;
    let clash: Vec<_> = names.iter().filter(|n| to.join(n).exists()).collect();
    let aside = (!clash.is_empty()).then(|| to.join(format!("旧数据-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"))));
    let put_back = |moved: &[&std::ffi::OsString]| {
        if let Some(a) = &aside {
            for n in moved {
                let _ = std::fs::rename(a.join(n), to.join(n));
            }
            let _ = std::fs::remove_dir(a);
        }
    };
    if let Some(a) = &aside {
        std::fs::create_dir_all(a).map_err(|e| format!("建不了 {}：{e}", a.display()))?;
        for (i, n) in clash.iter().enumerate() {
            if let Err(e) = std::fs::rename(to.join(n), a.join(n)) {
                put_back(&clash[..i]);
                return Err(format!("{} 里原来的 {} 挪不开：{e}", to.display(), n.to_string_lossy()));
            }
        }
    }
    let mut done = Vec::new();
    for name in &names {
        let dst = to.join(name);
        if let Err(e) = copy_tree(&from.join(name), &dst) {
            for d in done.iter().chain([&dst]) {
                remove(d);
            }
            put_back(&clash);
            return Err(e);
        }
        done.push(dst);
    }
    for name in &names {
        remove(&from.join(name));
    }
    Ok(aside)
}

/// 文件夹里除了 SKIP 的东西
fn entries(dir: &Path) -> std::io::Result<Vec<std::ffi::OsString>> {
    Ok(std::fs::read_dir(dir)?.flatten().map(|e| e.file_name()).filter(|n| !SKIP.iter().any(|s| n.to_string_lossy().eq_ignore_ascii_case(s))).collect())
}

/// 复制一个文件或文件夹，每个文件复制完核对大小
fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    let meta = std::fs::metadata(src).map_err(|e| format!("读不了 {}：{e}", src.display()))?;
    if meta.is_dir() {
        std::fs::create_dir_all(dst).map_err(|e| format!("建不了 {}：{e}", dst.display()))?;
        for e in std::fs::read_dir(src).map_err(|e| format!("读不了 {}：{e}", src.display()))?.flatten() {
            copy_tree(&e.path(), &dst.join(e.file_name()))?;
        }
        return Ok(());
    }
    let n = std::fs::copy(src, dst).map_err(|e| format!("复制 {} 出错：{e}", src.display()))?;
    let got = std::fs::metadata(dst).map(|m| m.len()).unwrap_or(0);
    if n != meta.len() || got != meta.len() {
        return Err(format!("{} 复制过去大小不对", src.display()));
    }
    Ok(())
}

fn remove(p: &Path) {
    let _ = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
}

/// 文件夹一共多大（字节）
fn size_of(p: &Path) -> u64 {
    match std::fs::metadata(p) {
        Ok(m) if m.is_dir() => std::fs::read_dir(p).map(|r| r.flatten().map(|e| size_of(&e.path())).sum()).unwrap_or(0),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

/// 网盘客户端同步的文件夹：数据库放在里面会被网盘边改边传，容易坏
fn cloud(p: &Path) -> bool {
    const NAMES: [&str; 18] = [
        "onedrive", "dropbox", "google drive", "googledrive", "icloud", "icloud drive", "iclouddrive", "baidunetdisk", "baidusyncdisk", "百度网盘",
        "百度云同步盘", "nutstore", "坚果云", "我的坚果云", "aliyundrive", "阿里云盘", "synologydrive", "seafile",
    ];
    let k = key(p);
    let by_name = k.split('\\').any(|part| NAMES.contains(&part) || part.starts_with("onedrive -") || part.starts_with("onedrive-"));
    let by_env = ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"].iter().filter_map(std::env::var_os).any(|d| under(p, Path::new(&d)));
    by_name || by_env
}

#[cfg(windows)]
fn free_space(p: &Path) -> Option<u64> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let w: Vec<u16> = p.as_os_str().to_string_lossy().encode_utf16().chain([0]).collect();
    let mut free = 0u64;
    // SAFETY: 路径以 0 结尾；只要可用空间，另外两个输出传空指针
    let ok = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
    (ok != 0).then_some(free)
}

#[cfg(not(windows))]
fn free_space(_: &Path) -> Option<u64> {
    None
}

/// 选了一个文件夹，能不能放、怎么放（设置里换位置之前先看一眼，真搬之前再看一次）
#[derive(Serialize, Debug, PartialEq)]
pub struct Check {
    /// 选的是哪个文件夹（真搬之前拿它再看一次）
    pub pick: String,
    /// 真正放进去的文件夹（选的文件夹里已经有别的东西，就是里面的「叮咪数据」）
    pub dir: String,
    /// empty 搬过去 / dingmi 那里本来就是叮咪的数据，换过去用 / 空着是不行，见 msg
    pub kind: String,
    pub msg: String,
    /// 现在的数据一共多大
    pub size: u64,
}

pub fn check(home: &Path, pick: &Path) -> Check {
    let cur = current(home);
    let size = entries(&cur).map(|ns| ns.iter().map(|n| size_of(&cur.join(n))).sum()).unwrap_or(0);
    let picked = pick.display().to_string();
    let bad = |msg: &str| Check { pick: picked.clone(), dir: picked.clone(), kind: String::new(), msg: msg.into(), size };
    if !pick.is_dir() {
        return bad("这个文件夹不在了，重新选一个");
    }
    if same(pick, &cur) {
        return bad("数据现在就放在这里");
    }
    // 搬回默认位置：那里除了位置记录应该是空的
    let back_home = same(pick, home);
    if !back_home && (under(pick, &cur) || under(&cur, pick)) {
        return bad("不能选现在放数据的文件夹里面的文件夹，也不能选把它包在里面的文件夹");
    }
    if cloud(pick) {
        return bad("这是网盘同步的文件夹，数据库放在里面会被网盘边改边传，容易坏。换一个本地的文件夹；想多一份保险，用「设置 → 数据」里的备份");
    }
    let has_db = |d: &Path| d.join("dingmi.db").is_file();
    let empty = |d: &Path| entries(d).map(|ns| ns.iter().all(|n| JUNK.contains(&n.to_string_lossy().to_lowercase().as_str()))).unwrap_or(false);
    let dir = if has_db(pick) || empty(pick) || back_home {
        pick.to_path_buf()
    } else {
        let sub = pick.join(SUB);
        if sub.exists() && !has_db(&sub) && !empty(&sub) {
            return bad("这个文件夹里已经有别的东西，里面的「叮咪数据」也不是空的。新建一个空文件夹再选");
        }
        sub
    };
    let shown = dir.display().to_string();
    if has_db(&dir) && !back_home {
        return Check { pick: picked, dir: shown, kind: "dingmi".into(), msg: String::new(), size };
    }
    // 能不能写：在要放的地方（还没建的话在上一层）试着建一个文件
    let probe_in = if dir.exists() { dir.clone() } else { pick.to_path_buf() };
    let probe = probe_in.join(format!(".dingmi-probe-{}", std::process::id()));
    if std::fs::write(&probe, b"x").is_err() {
        return bad("这个文件夹写不进去（可能是只读的、或者要管理员权限），换一个");
    }
    let _ = std::fs::remove_file(&probe);
    if let Some(free) = free_space(&probe_in) {
        // 留 50 MB 余量
        if free < size + 50 * 1024 * 1024 {
            return Check { pick: picked, dir: shown, kind: String::new(), msg: format!("那块盘的空间不够：要 {} MB，只剩 {} MB", size / 1048576 + 50, free / 1048576), size };
        }
    }
    Check { pick: picked, dir: shown, kind: "empty".into(), msg: String::new(), size }
}

/// 记下要搬到哪；下次打开叮咪时搬（见 open）
pub fn plan(home: &Path, c: &Check) -> std::io::Result<()> {
    let mut l = read(home);
    l.moving = Some(c.dir.clone());
    l.existing = c.kind == "dingmi";
    write(home, &l)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("dingmi-loc-{name}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn move_and_back() {
        let home = tmp("home");
        std::fs::write(home.join("dingmi.db"), b"db").unwrap();
        std::fs::create_dir_all(home.join("attachments")).unwrap();
        std::fs::write(home.join("attachments").join("a.png"), b"png").unwrap();
        std::fs::create_dir_all(home.join("EBWebView")).unwrap();
        let far = tmp("far");
        // 空文件夹：搬过去
        let c = check(&home, &far);
        assert_eq!((c.kind.as_str(), c.msg.as_str()), ("empty", ""));
        assert_eq!(c.size, 5);
        plan(&home, &c).unwrap();
        let o = open(&home);
        assert!(same(&o.dir, &far) && o.moved_from.as_deref().is_some_and(|f| same(f, &home)));
        assert_eq!(std::fs::read(far.join("attachments").join("a.png")).unwrap(), b"png");
        // 原来的地方只留位置记录和调试版的 WebView2 目录
        assert!(!home.join("dingmi.db").exists() && home.join(FILE).exists() && home.join("EBWebView").exists());
        // 再打开还是那里
        assert!(same(&open(&home).dir, &far));
        // 搬回默认位置；那里有一份找不到文件夹时新建的：不盖掉，挪进「旧数据-时间」
        std::fs::write(home.join("dingmi.db"), b"tmp").unwrap();
        let c = check(&home, &home);
        assert_eq!(c.kind, "empty");
        plan(&home, &c).unwrap();
        let o = open(&home);
        assert!(same(&o.dir, &home) && !far.join("dingmi.db").exists());
        assert_eq!(std::fs::read(home.join("dingmi.db")).unwrap(), b"db");
        let old = entries(&home).unwrap().into_iter().find(|n| n.to_string_lossy().starts_with("旧数据-")).expect("旧数据");
        assert_eq!(std::fs::read(home.join(&old).join("dingmi.db")).unwrap(), b"tmp");
        assert!(o.notice.unwrap().contains("旧数据"));
        assert_eq!(read(&home), Loc::default());
    }

    #[test]
    fn picks_that_do_not_work() {
        let home = tmp("home2");
        std::fs::write(home.join("dingmi.db"), b"db").unwrap();
        assert_eq!(check(&home, &home).msg, "数据现在就放在这里");
        let inside = home.join("sub");
        std::fs::create_dir_all(&inside).unwrap();
        assert!(check(&home, &inside).kind.is_empty());
        assert!(check(&home, &home.join("没有这个")).kind.is_empty());
        let od = tmp("od").join("OneDrive");
        std::fs::create_dir_all(&od).unwrap();
        assert!(check(&home, &od).msg.contains("网盘"));
        // 已经有别的东西：放进里面的「叮咪数据」
        let busy = tmp("busy");
        std::fs::write(busy.join("别的.txt"), b"x").unwrap();
        let c = check(&home, &busy);
        assert_eq!((c.kind.as_str(), c.dir), ("empty", busy.join(SUB).display().to_string()));
        // 本来就是叮咪的数据：换过去用
        let old = tmp("old");
        std::fs::write(old.join("dingmi.db"), b"old").unwrap();
        let c = check(&home, &old);
        assert_eq!(c.kind, "dingmi");
        plan(&home, &c).unwrap();
        let o = open(&home);
        assert!(same(&o.dir, &old) && o.moved_from.is_none());
        assert_eq!(std::fs::read(home.join("dingmi.db")).unwrap(), b"db");
    }

    #[test]
    fn failed_copy_keeps_the_original() {
        let home = tmp("home3");
        std::fs::write(home.join("dingmi.db"), b"db").unwrap();
        // 要去的地方是个文件：建不了文件夹，搬不成
        let blocker = tmp("blk").join("file");
        std::fs::write(&blocker, b"x").unwrap();
        let mut l = read(&home);
        l.moving = Some(blocker.display().to_string());
        write(&home, &l).unwrap();
        let o = open(&home);
        assert!(same(&o.dir, &home) && o.notice.unwrap().contains("没搬成"));
        assert!(home.join("dingmi.db").exists());
        assert_eq!(read(&home).moving, None);
    }
}
