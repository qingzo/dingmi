//! 启动器：找这台电脑上能打开的东西（开始菜单里的程序、微软商店应用、桌面上的快捷方式，加上自己加的文件夹、文件、网址），
//! 取图标、算拼音，打开。
//!
//! 列表在后台线程里找（要用 COM），找好了放在内存里，变了就通知各个窗口（launcher-changed）。
//! 图标缓存在数据文件夹的 icons 里，第二次找就很快。别名、放在桌面组件上的、自己加的、不显示的都在设置里
//! （Settings.launcher，只属于这台电脑：每台电脑装的程序不一样）；用过几次记在 kv "launch_stats"。

use crate::{db, AppState};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Item {
    /// app:<开始菜单里的解析名> / desk:<桌面上快捷方式的路径> / path:<自己加的文件或文件夹> / url:<自己加的网址>
    pub id: String,
    pub name: String,
    /// app 开始菜单里的程序 / store 微软商店应用 / desk 桌面上的快捷方式 / folder / file / link
    pub kind: &'static str,
    /// 真正在哪：程序的 exe、快捷方式指向的地方、文件夹、网址；不知道时为空
    pub path: String,
    /// 名字拆成一段一段：一个汉字是一段（列出所有读音，多音字有几个），连着的字母数字是一段（小写）。网页按拼音、首字母匹配
    pub pys: Vec<Vec<String>>,
    /// data:image/png;base64,…；拿不到时为空
    pub icon: String,
    /// 卸载、帮助、说明文档这类，默认不出现
    pub junk: bool,
    /// 自己加的文件、文件夹已经不在了
    pub missing: bool,
    /// Windows 记的打开过几次（开始菜单「最常用」用的那份）：刚开始用启动器、还没记下用过几次时拿来排顺序
    pub runs: u32,
}

/// 设置里启动器的那部分
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Config {
    /// 放在桌面组件「常用」里的，按顺序
    pub pins: Vec<String>,
    pub aliases: Vec<Alias>,
    pub custom: Vec<Custom>,
    /// 不想在启动器里看到的
    pub hidden: Vec<String>,
    /// 关掉的来源：start 开始菜单 / store 商店应用 / desktop 桌面
    pub off: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Alias {
    pub alias: String,
    pub id: String,
}

/// 自己加的：target 是文件、文件夹的路径或者网址；args 是打开时带的参数（拖进来的快捷方式里带的，比如浏览器的某个用户）
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Custom {
    pub name: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub args: String,
}

/// 用过几次、最后一次什么时候、打什么字时打开的是哪个（越用越准）
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Stats {
    pub n: HashMap<String, u32>,
    pub last: HashMap<String, i64>,
    /// 打的字 → [打开的 id, 时间]
    pub q: HashMap<String, (String, i64)>,
}

const STATS_KV: &str = "launch_stats";
/// 记住多少个"打什么字开哪个"，多了把最早的扔掉
const MAX_LEARNED: usize = 400;
/// 打开启动器时，上次找过超过这么久就在后台再找一遍（装了新程序能自己出现）
const STALE: Duration = Duration::from_secs(5 * 60);
const ICON_PX: i32 = 64;

static ITEMS: Mutex<Vec<Item>> = Mutex::new(Vec::new());
static LAST: Mutex<Option<Instant>> = Mutex::new(None);
static SCANNING: AtomicBool = AtomicBool::new(false);
static AGAIN: AtomicBool = AtomicBool::new(false);

pub fn items() -> Vec<Item> {
    ITEMS.lock().unwrap().clone()
}

/// 启动时找一遍，之后每半小时再找一遍
pub fn start(app: AppHandle) {
    std::thread::spawn(move || loop {
        scan_now(&app);
        std::thread::sleep(Duration::from_secs(30 * 60));
    });
}

pub fn refresh_if_old(app: &AppHandle) {
    let old = LAST.lock().unwrap().is_none_or(|t| t.elapsed() > STALE);
    if old {
        scan(app);
    }
}

/// 在后台重新找一遍。正在找的话，找完再来一遍（自己加的东西变了时不会漏掉）
pub fn scan(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || scan_now(&app));
}

fn scan_now(app: &AppHandle) {
    if SCANNING.swap(true, Ordering::SeqCst) {
        AGAIN.store(true, Ordering::SeqCst);
        return;
    }
    loop {
        let st = app.state::<AppState>();
        let custom = st.settings.lock().unwrap().launcher.custom.clone();
        let list = collect(&custom, &st.data_dir.join("icons"));
        let changed = {
            let mut cur = ITEMS.lock().unwrap();
            let changed = *cur != list;
            *cur = list;
            changed
        };
        *LAST.lock().unwrap() = Some(Instant::now());
        if changed {
            let _ = app.emit("launcher-changed", ());
        }
        if !AGAIN.swap(false, Ordering::SeqCst) {
            break;
        }
    }
    SCANNING.store(false, Ordering::SeqCst);
}

/// 「重新找一遍」：图标也重新取
pub fn rescan(app: &AppHandle) {
    let dir = app.state::<AppState>().data_dir.join("icons");
    let _ = std::fs::remove_dir_all(dir);
    scan(app);
}

fn collect(custom: &[Custom], icons: &Path) -> Vec<Item> {
    let _ = std::fs::create_dir_all(icons);
    let mut list = shell::scan(icons);
    for c in custom {
        list.push(shell::custom(c, icons));
    }
    list
}

// ---------- 名字 ----------

/// 名字拆成一段一段（见 Item.pys）。「QQ音乐」→ [qq] [yin] [yue, le]；「Visual Studio Code」→ [visual] [studio] [code]；
/// 小写字母后面跟大写字母也分开，「ToDesk」→ [to] [desk]，打 td 能找到（「VMware」「XMind」不分）
pub fn pinyin(name: &str) -> Vec<Vec<String>> {
    use pinyin::ToPinyinMulti;
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut word = String::new();
    let mut prev = ' ';
    for c in name.chars() {
        let camel = prev.is_ascii_lowercase() && c.is_ascii_uppercase();
        prev = c;
        if c.is_ascii_alphanumeric() {
            if camel {
                out.push(vec![std::mem::take(&mut word)]);
            }
            word.push(c.to_ascii_lowercase());
            continue;
        }
        if !word.is_empty() {
            out.push(vec![std::mem::take(&mut word)]);
        }
        if let Some(m) = c.to_pinyin_multi() {
            let mut r: Vec<String> = Vec::new();
            for p in m {
                let p = p.plain().to_string();
                if !r.contains(&p) {
                    r.push(p);
                }
            }
            out.push(r);
        }
    }
    if !word.is_empty() {
        out.push(vec![word]);
    }
    out
}

/// 卸载程序、帮助、说明文档、官网链接这类，默认不出现
pub fn is_junk(name: &str, path: &str) -> bool {
    const WORDS: [&str; 16] = [
        "uninstall", "卸载", "readme", "read me", "说明书", "帮助", "help", "manual", "documentation", "website", "官网",
        "官方网站", "license", "许可协议", "release notes", "更新日志",
    ];
    const EXTS: [&str; 9] = [".chm", ".pdf", ".txt", ".htm", ".html", ".url", ".rtf", ".md", ".hlp"];
    let n = name.to_lowercase();
    let p = path.to_lowercase();
    WORDS.iter().any(|w| n.contains(w)) || EXTS.iter().any(|e| p.ends_with(e)) || p.starts_with("http")
}

// ---------- 用过几次 ----------

pub fn stats(app: &AppHandle) -> Stats {
    let c = app.state::<AppState>();
    let c = c.db.lock().unwrap();
    db::kv_get(&c, STATS_KV).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn record(app: &AppHandle, id: &str, query: Option<&str>) {
    let mut s = stats(app);
    let now = chrono::Utc::now().timestamp_millis();
    *s.n.entry(id.to_string()).or_default() += 1;
    s.last.insert(id.to_string(), now);
    if let Some(q) = query.map(|q| q.trim().to_lowercase()).filter(|q| !q.is_empty() && q.chars().count() <= 20) {
        s.q.insert(q, (id.to_string(), now));
        if s.q.len() > MAX_LEARNED {
            let mut by_time: Vec<_> = s.q.iter().map(|(k, v)| (v.1, k.clone())).collect();
            by_time.sort();
            for (_, k) in by_time.into_iter().take(s.q.len() - MAX_LEARNED) {
                s.q.remove(&k);
            }
        }
    }
    let st = app.state::<AppState>();
    let c = st.db.lock().unwrap();
    let _ = db::kv_set(&c, STATS_KV, &serde_json::to_string(&s).unwrap_or_default());
}

// ---------- 打开 ----------

/// how：open 打开 / admin 用管理员身份打开 / dir 打开所在位置。query 是打开时输入框里的字（记下来，下次排前面）
pub fn open(app: &AppHandle, id: &str, how: &str, query: Option<&str>) -> Result<(), String> {
    let it = items().into_iter().find(|i| i.id == id).ok_or("找不到了，可能已经卸载")?;
    if it.missing {
        return Err(format!("「{}」已经不在了：{}", it.name, it.path));
    }
    let own = id.split_once(':').map_or("", |(_, rest)| rest);
    match how {
        "dir" => {
            let at = if it.path.is_empty() || it.kind == "link" { own } else { it.path.as_str() };
            if it.kind == "link" || it.kind == "store" || !Path::new(at).exists() {
                return Err("不知道它在哪个文件夹".into());
            }
            shell::select(at)?;
        }
        "admin" => {
            let file = match it.kind {
                "store" | "link" | "folder" => return Err("这个不能用管理员身份打开".into()),
                "desk" => own,
                _ if it.path.is_empty() => return Err("这个程序不能用管理员身份打开".into()),
                _ => it.path.as_str(),
            };
            shell::run(file, Some("runas"))?;
        }
        _ => match it.kind {
            "app" | "store" => shell::run(&format!("shell:AppsFolder\\{own}"), None)?,
            "desk" => shell::run(own, None)?,
            _ if it.id.starts_with("desk:") => shell::run(own, None)?,
            _ => {
                // 自己加的：带上记下的参数
                let args = app.state::<crate::AppState>().settings.lock().unwrap().launcher.custom.iter().find(|c| c.target == it.path).map(|c| c.args.clone()).unwrap_or_default();
                shell::run_args(&it.path, None, &args)?
            }
        },
    }
    record(app, id, query);
    Ok(())
}

pub use shell::{pick, run};

/// 新加进「自己加的」能不能收：网址（不带参数），或者这次用户在对话框里选过、拖进来的路径
/// （参数只能是拖进来的快捷方式里本来就有的），见 allow.rs
pub fn custom_ok(c: &Custom) -> bool {
    let t = c.target.trim();
    let lower = t.to_ascii_lowercase();
    let url = lower.starts_with("https://") || lower.starts_with("http://");
    (url && c.args.trim().is_empty()) || (crate::allow::granted(t) && crate::allow::granted_args(t, &c.args))
}

/// 拖进「自己加的」的文件：快捷方式（.lnk）记下名字和它指向的真实程序、参数，删了快捷方式也打得开；
/// 网页快捷方式（.url）记下网址；别的（程序、文件、文件夹）记路径，名字用文件名（不带扩展名）
pub fn resolve_drops(paths: &[String]) -> Vec<Custom> {
    paths.iter().filter_map(|p| shell::resolve_drop(p)).collect()
}

// ---------- 看看文件夹里有什么（桌面组件上点文件夹的小箭头） ----------

#[derive(Serialize)]
pub struct Peek {
    pub total: usize,
    pub files: Vec<PeekFile>,
}

#[derive(Serialize)]
pub struct PeekFile {
    pub name: String,
    pub path: String,
    pub dir: bool,
    /// 修改时间，毫秒
    pub modified: i64,
}

/// 最新改过的几个（不算隐藏的、系统的，比如 desktop.ini）
pub fn peek(dir: &str, limit: usize) -> Result<Peek, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("打不开这个文件夹：{e}"))?;
    let mut files: Vec<PeekFile> = rd
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            if hidden(&m) {
                return None;
            }
            let modified = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64);
            Some(PeekFile { name: e.file_name().to_string_lossy().into_owned(), path: e.path().to_string_lossy().into_owned(), dir: m.is_dir(), modified })
        })
        .collect();
    let total = files.len();
    files.sort_by_key(|f| std::cmp::Reverse(f.modified));
    files.truncate(limit);
    Ok(Peek { total, files })
}

#[cfg(windows)]
fn hidden(m: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    // FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM
    m.file_attributes() & 0x6 != 0
}

#[cfg(not(windows))]
fn hidden(_: &std::fs::Metadata) -> bool {
    false
}

#[cfg(windows)]
mod shell {
    use super::{is_junk, pinyin, Custom, Item, ICON_PX};
    use base64::Engine;
    use sha2::{Digest, Sha256};
    use std::collections::{HashMap, HashSet};
    use std::path::Path;
    use windows::core::{Interface, GUID, HRESULT, HSTRING, PCWSTR, PWSTR};
    use windows::Win32::Foundation::{ERROR_CANCELLED, SIZE};
    use windows::Win32::Graphics::Gdi::{
        DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
    };
    use windows::Win32::Storage::EnhancedStorage::{PKEY_Link_Arguments, PKEY_Link_TargetParsingPath};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IBindCtx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IEnumShellItems, IFileOpenDialog, IShellItem, IShellItem2, IShellItemImageFactory, SHCreateItemFromParsingName,
        SHGetKnownFolderItem, SHGetKnownFolderPath, ShellExecuteExW, BHID_EnumItems, FOLDERID_AppsFolder, FOLDERID_Desktop,
        FOLDERID_PublicDesktop, FOS_PICKFOLDERS, KF_FLAG_DEFAULT, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW,
        SIGDN_FILESYSPATH, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING, SIIGBF_ICONONLY,
    };
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    /// 这个线程用 COM 期间初始化着，用完还原
    struct Com(bool);
    impl Com {
        fn init() -> Com {
            // SAFETY: 只在本线程初始化，Drop 时配对还原；已经用别的方式初始化过时不还原
            Com(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok())
        }
    }
    impl Drop for Com {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: 和 init 里成功的那次配对
                unsafe { CoUninitialize() }
            }
        }
    }

    /// 系统分配的字符串：读出来后释放
    fn take(p: PWSTR) -> String {
        // SAFETY: p 是系统用 CoTaskMemAlloc 分配的、以 0 结尾的字符串，读完由我们释放
        unsafe {
            let s = p.to_string().unwrap_or_default();
            CoTaskMemFree(Some(p.0 as _));
            s
        }
    }

    fn item_at(path: &str) -> Option<IShellItem> {
        // SAFETY: 传入的是有效的字符串
        unsafe { SHCreateItemFromParsingName(&HSTRING::from(path), None::<&IBindCtx>) }.ok()
    }

    /// 快捷方式（开始菜单里的程序也是）指向哪里
    fn target(item: &IShellItem) -> String {
        item.cast::<IShellItem2>()
            .ok()
            // SAFETY: 属性键是系统定义的常量
            .and_then(|i| unsafe { i.GetString(&PKEY_Link_TargetParsingPath) }.ok())
            .map(take)
            .unwrap_or_default()
    }

    fn known_folder(id: &GUID) -> Option<String> {
        // SAFETY: id 是有效的 GUID
        unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }.ok().map(take)
    }

    /// 开始菜单里程序的解析名有的是 {已知文件夹 GUID}\相对路径，有的直接是路径
    fn resolve(parsing: &str) -> String {
        if let Some((g, rest)) = parsing.strip_prefix('{').and_then(|s| s.split_once("}\\")) {
            if let Some(base) = GUID::try_from(g).ok().and_then(|g| known_folder(&g)) {
                return format!("{base}\\{rest}");
            }
        }
        if parsing.len() > 2 && parsing.as_bytes()[1] == b':' {
            return parsing.to_string();
        }
        String::new()
    }

    pub fn scan(icons: &Path) -> Vec<Item> {
        let _com = Com::init();
        let used = usage();
        let mut list = apps(icons, &used);
        let names: HashSet<String> = list.iter().map(|i| i.name.to_lowercase()).collect();
        let targets: HashSet<String> = list.iter().filter(|i| !i.path.is_empty()).map(|i| i.path.to_lowercase()).collect();
        for d in desktop(icons, &used) {
            // 桌面上的快捷方式开始菜单里也有的，不重复
            if !names.contains(&d.name.to_lowercase()) && (d.path.is_empty() || !targets.contains(&d.path.to_lowercase())) {
                list.push(d);
            }
        }
        list
    }

    /// 开始菜单（所有用户的和自己的）、微软商店应用：都在"应用"这个虚拟文件夹里
    fn apps(icons: &Path, used: &HashMap<String, u32>) -> Vec<Item> {
        let mut out = Vec::new();
        let runs = |keys: &[&str]| keys.iter().filter(|k| !k.is_empty()).map(|k| used.get(&k.to_lowercase()).copied().unwrap_or(0)).max().unwrap_or(0);
        // SAFETY: COM 已初始化；枚举到的对象用完自动释放
        unsafe {
            let Ok(folder) = SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None) else { return out };
            let Ok(en) = folder.BindToHandler::<_, IEnumShellItems>(None::<&IBindCtx>, &BHID_EnumItems) else { return out };
            let mut seen = HashSet::new();
            loop {
                let mut one = [None];
                let mut got = 0u32;
                if en.Next(&mut one, Some(&mut got)).is_err() || got == 0 {
                    break;
                }
                let Some(it) = one[0].take() else { break };
                let Ok(name) = it.GetDisplayName(SIGDN_NORMALDISPLAY).map(take) else { continue };
                let Ok(parsing) = it.GetDisplayName(SIGDN_PARENTRELATIVEPARSING).map(take) else { continue };
                if name.is_empty() || !seen.insert(parsing.clone()) {
                    continue;
                }
                // 商店应用的解析名是 包名!应用名
                let store = parsing.contains('!') && !parsing.contains('\\');
                let mut path = target(&it);
                if path.is_empty() && !store {
                    path = resolve(&parsing);
                }
                let id = format!("app:{parsing}");
                let pys = pinyin(&name);
                out.push(Item {
                    runs: runs(&[&parsing, &path]),
                    icon: icon(&it, icons, &id),
                    junk: is_junk(&name, if path.is_empty() { &parsing } else { &path }),
                    kind: if store { "store" } else { "app" },
                    id,
                    name,
                    path,
                    pys,
                    missing: false,
                });
            }
        }
        out
    }

    /// 桌面上（自己的和公共的）的快捷方式、程序
    fn desktop(icons: &Path, used: &HashMap<String, u32>) -> Vec<Item> {
        let mut out = Vec::new();
        for fid in [FOLDERID_Desktop, FOLDERID_PublicDesktop] {
            let Some(dir) = known_folder(&fid) else { continue };
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
                if !matches!(ext.as_str(), "lnk" | "url" | "exe" | "appref-ms") {
                    continue;
                }
                let full = p.to_string_lossy().into_owned();
                let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let it = item_at(&full);
                let path = match (&it, ext.as_str()) {
                    (_, "exe") => full.clone(),
                    (Some(it), _) => target(it),
                    _ => String::new(),
                };
                let id = format!("desk:{full}");
                let pys = pinyin(&name);
                out.push(Item {
                    runs: used.get(&path.to_lowercase()).copied().unwrap_or(0),
                    icon: it.as_ref().map(|it| icon(it, icons, &id)).unwrap_or_default(),
                    junk: is_junk(&name, if ext == "url" { "x.url" } else { &path }),
                    // 指向文件夹的快捷方式当文件夹用（桌面组件上能看里面的文件）
                    kind: if !path.is_empty() && Path::new(&path).is_dir() { "folder" } else { "desk" },
                    id,
                    name,
                    path,
                    pys,
                    missing: false,
                });
            }
        }
        out
    }

    pub fn custom(c: &Custom, icons: &Path) -> Item {
        let link = c.target.starts_with("http://") || c.target.starts_with("https://");
        let p = Path::new(&c.target);
        let name = if c.name.trim().is_empty() {
            p.file_name().map_or(c.target.clone(), |n| n.to_string_lossy().into_owned())
        } else {
            c.name.trim().to_string()
        };
        let pys = pinyin(&name);
        let exists = !link && p.exists();
        let id = format!("{}:{}", if link { "url" } else { "path" }, c.target);
        let _com = Com::init();
        Item {
            icon: if exists { item_at(&c.target).map(|it| icon(&it, icons, &id)).unwrap_or_default() } else { String::new() },
            kind: if link {
                "link"
            } else if p.is_dir() {
                "folder"
            } else {
                "file"
            },
            missing: !link && !exists,
            junk: false,
            runs: 0,
            id,
            name,
            path: c.target.clone(),
            pys,
        }
    }

    /// Windows 记的每个程序打开过几次（注册表 UserAssist，名字是 ROT13 编码的）。
    /// 名字是开始菜单里的解析名、程序路径，或者 {已知文件夹}\相对路径；键都转成小写，{已知文件夹} 开头的另外按完整路径、
    /// 快捷方式再按它指向的程序各记一份。要在初始化了 COM 的线程里调
    fn usage() -> HashMap<String, u32> {
        use windows::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
        use windows::Win32::System::Registry::{RegCloseKey, RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ};
        const ROOT: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\UserAssist";
        let rot13 = |s: &str| -> String {
            s.chars()
                .map(|c| match c {
                    'a'..='z' => ((c as u8 - b'a' + 13) % 26 + b'a') as char,
                    'A'..='Z' => ((c as u8 - b'A' + 13) % 26 + b'A') as char,
                    _ => c,
                })
                .collect()
        };
        let mut out = HashMap::new();
        // SAFETY: 打开的键用完都关掉；缓冲区长度和传入的一致
        unsafe {
            let mut root = HKEY::default();
            if RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(ROOT), None, KEY_READ, &mut root) != ERROR_SUCCESS {
                return out;
            }
            for i in 0.. {
                let mut name = [0u16; 128];
                let mut len = name.len() as u32;
                if RegEnumKeyExW(root, i, Some(PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None) != ERROR_SUCCESS {
                    break;
                }
                let sub = format!(r"{}\Count", String::from_utf16_lossy(&name[..len as usize]));
                let mut key = HKEY::default();
                if RegOpenKeyExW(root, &HSTRING::from(sub), None, KEY_READ, &mut key) != ERROR_SUCCESS {
                    continue;
                }
                for j in 0.. {
                    let mut vname = [0u16; 1024];
                    let mut vlen = vname.len() as u32;
                    let mut data = [0u8; 128];
                    let mut dlen = data.len() as u32;
                    let r = RegEnumValueW(key, j, Some(PWSTR(vname.as_mut_ptr())), &mut vlen, None, None, Some(data.as_mut_ptr()), Some(&mut dlen));
                    // 数据比缓冲区大的跳过，没有更多了才停
                    if r == ERROR_MORE_DATA {
                        continue;
                    }
                    if r != ERROR_SUCCESS {
                        break;
                    }
                    // Windows 7 起每项 72 字节，第 4~8 字节是打开次数
                    if dlen < 8 {
                        continue;
                    }
                    let n = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
                    if n == 0 {
                        continue;
                    }
                    let name = rot13(&String::from_utf16_lossy(&vname[..vlen as usize]));
                    let full = resolve(&name);
                    // 从任务栏、开始菜单的快捷方式打开的，记在快捷方式名下：算到它指向的程序上
                    let via = if full.to_lowercase().ends_with(".lnk") { item_at(&full).map(|it| target(&it)).unwrap_or_default() } else { String::new() };
                    for k in [name.to_lowercase(), full.to_lowercase(), via.to_lowercase()] {
                        if !k.is_empty() {
                            *out.entry(k).or_insert(0) += n;
                        }
                    }
                }
                let _ = RegCloseKey(key);
            }
            let _ = RegCloseKey(root);
        }
        out
    }

    // ---------- 图标 ----------

    /// 先看缓存，没有再向系统要，存成 png
    fn icon(item: &IShellItem, dir: &Path, id: &str) -> String {
        let hash = Sha256::digest(id.as_bytes());
        let file = dir.join(format!("{}.png", hash[..8].iter().map(|b| format!("{b:02x}")).collect::<String>()));
        let png = std::fs::read(&file).ok().or_else(|| {
            let png = render(item)?;
            let _ = std::fs::write(&file, &png);
            Some(png)
        });
        png.map(|p| format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(p))).unwrap_or_default()
    }

    fn render(item: &IShellItem) -> Option<Vec<u8>> {
        let f: IShellItemImageFactory = item.cast().ok()?;
        // SAFETY: 系统给的位图用完删掉
        unsafe {
            let bmp = f.GetImage(SIZE { cx: ICON_PX, cy: ICON_PX }, SIIGBF_ICONONLY).ok()?;
            let rgba = pixels(bmp);
            let _ = DeleteObject(HGDIOBJ(bmp.0));
            let (w, h, data) = rgba?;
            let mut out = Vec::new();
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header().ok()?.write_image_data(&data).ok()?;
            Some(out)
        }
    }

    /// 32 位位图读成 RGBA。系统给的是预乘过透明度的 BGRA；整张都没有透明度的当成不透明
    unsafe fn pixels(bmp: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
        let mut bm = BITMAP::default();
        // SAFETY: bm 的大小和传入的一致
        if unsafe { GetObjectW(HGDIOBJ(bmp.0), size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as _)) } == 0 {
            return None;
        }
        let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
        if w <= 0 || h <= 0 || w > 1024 || h > 1024 {
            return None;
        }
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                // 负数：从上往下
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        // SAFETY: 缓冲区正好放得下 w×h 个 32 位像素
        let lines = unsafe {
            let dc = GetDC(None);
            let n = GetDIBits(dc, bmp, 0, h as u32, Some(buf.as_mut_ptr() as _), &mut info, DIB_RGB_COLORS);
            ReleaseDC(None, dc);
            n
        };
        if lines == 0 {
            return None;
        }
        let alpha = buf.chunks(4).any(|p| p[3] != 0);
        for p in buf.chunks_mut(4) {
            p.swap(0, 2);
            if !alpha {
                p[3] = 255;
            } else if p[3] > 0 && p[3] < 255 {
                let a = p[3] as u32;
                for c in &mut p[..3] {
                    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        Some((w as u32, h as u32, buf))
    }

    // ---------- 打开 ----------

    /// 用系统的方式打开（相当于双击）；verb 是 runas 时用管理员身份。弹出管理员确认后点了"否"不算出错
    pub fn run(file: &str, verb: Option<&str>) -> Result<(), String> {
        run_args(file, verb, "")
    }

    /// 带参数打开（自己加的快捷方式里带的参数）
    pub fn run_args(file: &str, verb: Option<&str>, args: &str) -> Result<(), String> {
        let _com = Com::init();
        let file_h = HSTRING::from(file);
        let verb_h = verb.map(HSTRING::from);
        let args_h = (!args.trim().is_empty()).then(|| HSTRING::from(args));
        // 程序在它自己的文件夹里启动（有的程序要读旁边的文件）
        let dir = Path::new(file).parent().filter(|d| d.is_dir()).map(|d| HSTRING::from(d.as_os_str()));
        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
            lpVerb: verb_h.as_ref().map_or(PCWSTR::null(), |v| PCWSTR(v.as_ptr())),
            lpFile: PCWSTR(file_h.as_ptr()),
            lpParameters: args_h.as_ref().map_or(PCWSTR::null(), |a| PCWSTR(a.as_ptr())),
            lpDirectory: dir.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        // SAFETY: info 里的字符串在调用期间都有效
        match unsafe { ShellExecuteExW(&mut info) } {
            Ok(()) => Ok(()),
            Err(e) if e.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => Ok(()),
            Err(e) => Err(format!("打不开：{}", e.message())),
        }
    }

    /// 拖进来的一个文件变成「自己加的」一项（见 launcher::resolve_drops）
    pub fn resolve_drop(path: &str) -> Option<Custom> {
        let p = Path::new(path);
        if !p.exists() {
            return None;
        }
        let stem = p.file_stem().or_else(|| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string());
        let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if p.is_file() && ext == "url" {
            let text = std::fs::read_to_string(p).ok()?;
            let url = text.lines().find_map(|l| l.trim().strip_prefix("URL=")).map(|u| u.trim().to_string())?;
            return Some(Custom { name: stem, target: url, args: String::new() });
        }
        if p.is_file() && ext == "lnk" {
            let _com = Com::init();
            let it = item_at(path)?;
            let to = target(&it);
            // SAFETY: 属性键是系统定义的常量
            let args = it.cast::<IShellItem2>().ok().and_then(|i| unsafe { i.GetString(&PKEY_Link_Arguments) }.ok()).map(take).unwrap_or_default();
            // 指向的不是一个真实的文件（比如商店应用、控制面板里的东西）：只能记快捷方式本身
            if to.is_empty() || !Path::new(&to).exists() {
                return Some(Custom { name: stem, target: path.to_string(), args: String::new() });
            }
            return Some(Custom { name: stem, target: to, args });
        }
        Some(Custom { name: if p.is_dir() { p.file_name().map_or(stem.clone(), |n| n.to_string_lossy().into_owned()) } else { stem }, target: path.to_string(), args: String::new() })
    }

    /// 在资源管理器里打开它所在的文件夹，并选中它
    pub fn select(path: &str) -> Result<(), String> {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer.exe")
            .raw_arg(format!("/select,\"{path}\""))
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("打不开资源管理器：{e}"))
    }

    /// 选一个文件夹或文件，取消时为 None
    pub fn pick(folder: bool) -> Option<String> {
        let _com = Com::init();
        // SAFETY: COM 已初始化；对话框是模态的，选完才返回
        unsafe {
            let d: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
            if folder {
                let o = d.GetOptions().ok()?;
                d.SetOptions(o | FOS_PICKFOLDERS).ok()?;
            }
            d.Show(None).ok()?;
            d.GetResult().ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok().map(take)
        }
    }
}

#[cfg(not(windows))]
mod shell {
    use super::{Custom, Item};
    use std::path::Path;
    pub fn scan(_: &Path) -> Vec<Item> {
        Vec::new()
    }
    pub fn custom(c: &Custom, _: &Path) -> Item {
        Item { id: format!("path:{}", c.target), name: c.name.clone(), kind: "file", path: c.target.clone(), pys: Vec::new(), icon: String::new(), junk: false, missing: true, runs: 0 }
    }
    pub fn run(_: &str, _: Option<&str>) -> Result<(), String> {
        Err("只支持 Windows".into())
    }
    pub fn run_args(_: &str, _: Option<&str>, _: &str) -> Result<(), String> {
        Err("只支持 Windows".into())
    }
    pub fn resolve_drop(_: &str) -> Option<Custom> {
        None
    }
    pub fn select(_: &str) -> Result<(), String> {
        Err("只支持 Windows".into())
    }
    pub fn pick(_: bool) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinyin_of_names() {
        let v = |x: &[&[&str]]| x.iter().map(|r| r.iter().map(|s| s.to_string()).collect::<Vec<_>>()).collect::<Vec<_>>();
        // 多音字的第一个是常用读音（信 xin / shen）
        let wx = pinyin("微信");
        assert_eq!((wx.len(), wx[0][0].as_str(), wx[1][0].as_str()), (2, "wei", "xin"));
        let qq = pinyin("QQ音乐");
        assert_eq!(qq[..2], v(&[&["qq"], &["yin"]])[..]);
        assert!(qq[2].contains(&"yue".to_string()) && qq[2].contains(&"le".to_string()));
        assert_eq!(pinyin("Windows 终端"), v(&[&["windows"], &["zhong"], &["duan"]]));
        assert_eq!(pinyin("Xshell 8"), v(&[&["xshell"], &["8"]]));
        // 小写后面跟大写分开；连着的大写不分
        assert_eq!(pinyin("ToDesk"), v(&[&["to"], &["desk"]]));
        assert_eq!(pinyin("HBuilderX"), v(&[&["hbuilder"], &["x"]]));
        assert_eq!(pinyin("VMware Workstation"), v(&[&["vmware"], &["workstation"]]));
        assert_eq!(pinyin("XMind"), v(&[&["xmind"]]));
        assert_eq!(pinyin("WeChat微信")[..3], v(&[&["we"], &["chat"], &["wei"]])[..]);
    }

    #[test]
    fn junk_names() {
        assert!(is_junk("卸载 微信", ""));
        assert!(is_junk("Uninstall Xshell", ""));
        assert!(is_junk("WPS 帮助文档", ""));
        assert!(is_junk("官网", "https://example.com"));
        assert!(is_junk("Manual", r"C:\a\manual.pdf"));
        assert!(!is_junk("腾讯文档", r"C:\a\docs.exe"));
        assert!(!is_junk("微信", r"C:\a\WeChat.exe"));
    }

    #[test]
    fn peek_lists_newest_first() {
        let dir = std::env::temp_dir().join(format!("dingmi-peek-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.join("b.txt"), "b").unwrap();
        let p = peek(&dir.to_string_lossy(), 2).unwrap();
        assert_eq!(p.total, 3);
        assert_eq!(p.files.len(), 2);
        assert_eq!(p.files[0].name, "b.txt");
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(all(test, windows))]
mod scan_tests {
    /// 在这台电脑上真找一遍：cargo test --lib real_scan -- --ignored --nocapture
    #[test]
    #[ignore]
    fn real_scan() {
        let dir = std::env::temp_dir().join("dingmi-icons-test");
        for round in 0..2 {
            let t = std::time::Instant::now();
            let list = super::collect(&[], &dir);
            let icons = list.iter().filter(|i| !i.icon.is_empty()).count();
            println!("round {round}: {} items, {icons} icons, {:?}", list.len(), t.elapsed());
        }
        for i in super::collect(&[], &dir) {
            println!("{:5} {:5} {:4} {} | {} | {}", i.kind, if i.junk { "junk" } else { "" }, i.runs, i.name, i.id, i.path);
        }
    }
}
