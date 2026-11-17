//! 快捷框里打「f 加空格」搜电脑上的文件（需求-1.x 第 21、42.7 节）。
//! 有 Everything 在运行就问它（几乎瞬间、整台电脑都有）：用户自己装的，或者叮咪一键开启的那个（见 everything.rs）；
//! 都没有就用 Windows 自带的搜索索引（只有建了索引的地方，默认是用户文件夹）。
//! 搜的字、文件名都不进日志（需求-0.3 第 12.1 节）。

use serde::Serialize;
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

/// 最多列几个
pub const MAX: u32 = 50;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Hit {
    pub path: String,
    pub name: String,
    /// 所在的文件夹
    pub dir: String,
    pub folder: bool,
    /// 字节；文件夹、拿不到的是 None
    pub size: Option<u64>,
    /// "YYYY-MM-DD HH:MM"
    pub modified: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct Found {
    /// everything 用户自己装的 / mine 叮咪带的 / index Windows 搜索索引 / 空的是都用不了
    pub engine: &'static str,
    pub hits: Vec<Hit>,
    /// 装了 Everything 但没开着：它在哪（界面上给「打开 Everything」）
    pub idle: Option<String>,
}

/// 最近一次搜出来的：只有这些能从快捷框里打开、选中（界面万一被塞进了别的脚本，也打不开别的路径）
static LAST: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

pub fn was_found(p: &str) -> bool {
    LAST.lock().unwrap().contains(&crate::allow::norm(p))
}

pub fn search(q: &str) -> Found {
    let q = q.trim();
    let mut f = if q.is_empty() {
        Found { engine: engine(), hits: Vec::new(), idle: None }
    } else if let Some((who, hwnd)) = everything::window() {
        match everything::query(hwnd, q, MAX) {
            Some(hits) => Found { engine: who, hits, idle: None },
            None => index_or_nothing(q),
        }
    } else {
        index_or_nothing(q)
    };
    if f.engine != "everything" && f.engine != "mine" {
        f.idle = everything::installed();
    }
    *LAST.lock().unwrap() = f.hits.iter().map(|h| crate::allow::norm(&h.path)).collect();
    f
}

/// 现在会用哪一种搜（设置里显示）
pub fn engine() -> &'static str {
    match everything::window() {
        Some((who, _)) => who,
        None if index::available() => "index",
        None => "",
    }
}

fn index_or_nothing(q: &str) -> Found {
    match index::search(q, MAX) {
        Some(hits) => Found { engine: "index", hits, idle: None },
        None => Found { engine: "", hits: Vec::new(), idle: None },
    }
}

/// FILETIME（1601 年起的 100 纳秒）→ 本地时间 "YYYY-MM-DD HH:MM"
fn filetime(ft: u64) -> Option<String> {
    let secs = (ft / 10_000_000).checked_sub(11_644_473_600)?;
    let t = chrono::DateTime::from_timestamp(secs as i64, 0)?;
    Some(t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
}

fn hit(path: String, folder: bool, size: Option<u64>, modified: Option<String>) -> Hit {
    let p = std::path::Path::new(&path);
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone());
    let dir = p.parent().map(|d| d.display().to_string()).unwrap_or_default();
    Hit { path, name, dir, folder, size, modified }
}

/// Everything 的查询接口（WM_COPYDATA，SDK 里的 everything_ipc.h，1.4 的 QUERY2）
pub mod everything {
    use super::Hit;

    /// 用户自己装的 Everything 窗口的类名；叮咪带的那个用了实例名 dingmi，类名后面多「_(dingmi)」
    pub const CLASS: &str = "EVERYTHING_TASKBAR_NOTIFICATION";
    pub const INSTANCE: &str = "dingmi";
    const QUERY2: usize = 18;
    /// 回信时 dwData 带着这个，认出是这次查询的回信
    const REPLY: u32 = 0x444D_5146;
    const REQ_NAME: u32 = 0x01;
    const REQ_PATH: u32 = 0x02;
    const REQ_SIZE: u32 = 0x10;
    const REQ_MODIFIED: u32 = 0x40;
    const SORT_MODIFIED_DESC: u32 = 14;
    const ITEM_FOLDER: u32 = 0x01;

    /// 查询的请求：QUERY2 的头 + 以 0 结尾的 UTF-16 搜索词
    pub fn request(reply_hwnd: u32, q: &str, max: u32) -> Vec<u8> {
        let mut b = Vec::new();
        for v in [reply_hwnd, REPLY, 0, 0, max, REQ_NAME | REQ_PATH | REQ_SIZE | REQ_MODIFIED, SORT_MODIFIED_DESC] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        for u in q.encode_utf16().chain([0]) {
            b.extend_from_slice(&u.to_le_bytes());
        }
        b
    }

    /// 解开回信（LIST2）：头 5 个 DWORD，再是每一项的 flags、数据位置；每一项的数据按请求的位从低到高排：
    /// 名字、路径是「DWORD 长度 + 那么多个 UTF-16 + 结尾的 0」，大小、修改时间各 8 字节
    pub fn parse(b: &[u8]) -> Option<Vec<Hit>> {
        let dw = |o: usize| b.get(o..o + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap()));
        let qw = |o: usize| b.get(o..o + 8).map(|s| u64::from_le_bytes(s.try_into().unwrap()));
        let num = dw(4)? as usize;
        let flags = dw(12)?;
        let mut out = Vec::new();
        for i in 0..num.min(1000) {
            let item = 20 + i * 8;
            let (iflags, mut o) = (dw(item)?, dw(item + 4)? as usize);
            let mut text = || -> Option<String> {
                let len = dw(o)? as usize;
                let s = b.get(o + 4..o + 4 + len * 2)?;
                o += 4 + (len + 1) * 2;
                Some(String::from_utf16_lossy(&s.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect::<Vec<_>>()))
            };
            let name = if flags & REQ_NAME != 0 { text()? } else { String::new() };
            let dir = if flags & REQ_PATH != 0 { text()? } else { String::new() };
            let folder = iflags & ITEM_FOLDER != 0;
            let size = if flags & REQ_SIZE != 0 {
                let v = qw(o)?;
                o += 8;
                // 文件夹的大小 Everything 可能没算（全 1）
                (!folder && v != u64::MAX).then_some(v)
            } else {
                None
            };
            let modified = if flags & REQ_MODIFIED != 0 { qw(o).and_then(|v| (v != u64::MAX).then(|| super::filetime(v)).flatten()) } else { None };
            let path = if dir.is_empty() { name.clone() } else { format!("{}\\{name}", dir.trim_end_matches('\\')) };
            out.push(Hit { path, name, dir, folder, size, modified });
        }
        Some(out)
    }

    #[cfg(windows)]
    mod win {
        use super::*;
        use std::cell::RefCell;
        use std::time::{Duration, Instant};
        use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
        use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
        use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows_sys::Win32::UI::WindowsAndMessaging::*;

        thread_local!(static GOT: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) });

        unsafe extern "system" fn proc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
            if msg == WM_COPYDATA && l != 0 {
                // SAFETY: WM_COPYDATA 的 lParam 是系统给的 COPYDATASTRUCT，处理这条消息期间有效
                let cds = unsafe { &*(l as *const COPYDATASTRUCT) };
                if cds.dwData == REPLY as usize && !cds.lpData.is_null() {
                    // SAFETY: 同上，lpData 指向 cbData 个字节
                    let data = unsafe { std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize) }.to_vec();
                    GOT.with(|g| *g.borrow_mut() = Some(data));
                    return 1;
                }
            }
            // SAFETY: 别的消息照常交给系统
            unsafe { DefWindowProcW(h, msg, w, l) }
        }

        fn wide(s: &str) -> Vec<u16> {
            s.encode_utf16().chain([0]).collect()
        }

        /// 正在运行的 Everything：用户自己装的优先，再是叮咪带的
        pub fn window() -> Option<(&'static str, HWND)> {
            let find = |class: &str| {
                let c = wide(class);
                // SAFETY: 类名以 0 结尾
                let h = unsafe { FindWindowW(c.as_ptr(), std::ptr::null()) };
                (!h.is_null()).then_some(h)
            };
            find(CLASS).map(|h| ("everything", h)).or_else(|| find(&format!("{CLASS}_({INSTANCE})")).map(|h| ("mine", h)))
        }

        /// 问 Everything；它没回（没开好、卡住）就是 None
        pub fn query(everything: HWND, q: &str, max: u32) -> Option<Vec<Hit>> {
            // SAFETY: 下面都是普通的 Win32 调用，字符串以 0 结尾，窗口在这个函数里建、用完就毁掉
            unsafe {
                let class = wide("DingmiEverythingReply");
                let inst = GetModuleHandleW(std::ptr::null());
                let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: inst, lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
                RegisterClassW(&wc);
                let me = CreateWindowExW(0, class.as_ptr(), std::ptr::null(), 0, 0, 0, 0, 0, HWND_MESSAGE, std::ptr::null_mut(), inst, std::ptr::null());
                if me.is_null() {
                    return None;
                }
                GOT.with(|g| *g.borrow_mut() = None);
                let req = request(me as usize as u32, q, max);
                let cds = COPYDATASTRUCT { dwData: QUERY2, cbData: req.len() as u32, lpData: req.as_ptr() as *mut _ };
                let mut res = 0usize;
                let sent = SendMessageTimeoutW(everything, WM_COPYDATA, me as WPARAM, &cds as *const _ as LPARAM, SMTO_ABORTIFHUNG, 2000, &mut res);
                let mut got = None;
                if sent != 0 && res != 0 {
                    let end = Instant::now() + Duration::from_secs(3);
                    let mut msg: MSG = std::mem::zeroed();
                    loop {
                        got = GOT.with(|g| g.borrow_mut().take());
                        if got.is_some() {
                            break;
                        }
                        let left = end.saturating_duration_since(Instant::now());
                        if left.is_zero() {
                            break;
                        }
                        while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                            TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                        if GOT.with(|g| g.borrow().is_none()) {
                            MsgWaitForMultipleObjects(0, std::ptr::null(), 0, left.as_millis().min(100) as u32, QS_ALLINPUT);
                        }
                    }
                }
                DestroyWindow(me);
                parse(&got?)
            }
        }
    }

    #[cfg(windows)]
    pub fn window() -> Option<(&'static str, usize)> {
        win::window().map(|(who, h)| (who, h as usize))
    }

    #[cfg(windows)]
    pub fn query(hwnd: usize, q: &str, max: u32) -> Option<Vec<Hit>> {
        win::query(hwnd as _, q, max)
    }

    #[cfg(not(windows))]
    pub fn window() -> Option<(&'static str, usize)> {
        None
    }

    #[cfg(not(windows))]
    pub fn query(_: usize, _: &str, _: u32) -> Option<Vec<Hit>> {
        None
    }

    /// 用户自己装了 Everything（安装版的老地方），只是没开着
    pub fn installed() -> Option<String> {
        ["ProgramFiles", "ProgramFiles(x86)"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|d| std::path::Path::new(&d).join("Everything").join("Everything.exe"))
            .find(|p| p.is_file())
            .map(|p| p.display().to_string())
    }
}

/// Windows 自带的搜索索引（WinRT 的 Windows.Storage.Search）：只查建了索引的地方，从用户文件夹往下找。
/// 只按名字找：索引会连正文、作者这些一起配，多拿一些回来，名字里没有这几个字的去掉
mod index {
    use super::Hit;

    #[cfg(windows)]
    pub fn search(q: &str, max: u32) -> Option<Vec<Hit>> {
        use windows::core::HSTRING;
        use windows::Storage::Search::{FolderDepth, IndexerOption, QueryOptions, SortEntry};
        use windows::Storage::{StorageFolder, StorageItemTypes};
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
        let home = std::env::var_os("USERPROFILE")?;
        let words: Vec<String> = q.split_whitespace().map(|w| w.to_lowercase()).collect();
        // SAFETY: 这个线程用 COM（多线程模式）；已经初始化过的返回 S_FALSE / 模式不同的错误，都不影响下面用
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let run = || -> windows::core::Result<Vec<Hit>> {
            let folder = StorageFolder::GetFolderFromPathAsync(&HSTRING::from(home.to_string_lossy().as_ref()))?.join()?;
            let opts = QueryOptions::new()?;
            opts.SetFolderDepth(FolderDepth::Deep)?;
            opts.SetIndexerOption(IndexerOption::OnlyUseIndexer)?;
            // 只按文件名找（~= 是「包含」），程序自己的缓存（AppData）不要；下面再按名字核对一遍
            let mut aqs: Vec<String> = words.iter().map(|w| format!("System.FileName:~=\"{}\"", w.replace('"', ""))).collect();
            aqs.push("NOT System.ItemFolderPathDisplay:~=\"\\AppData\\\"".into());
            opts.SetApplicationSearchFilter(&HSTRING::from(aqs.join(" AND ")))?;
            let sort = opts.SortOrder()?;
            sort.Clear()?;
            sort.Append(&SortEntry { PropertyName: HSTRING::from("System.DateModified"), AscendingOrder: false })?;
            let items = folder.CreateItemQueryWithOptions(&opts)?.GetItemsAsync(0, max * 4)?.join()?;
            let mut out = Vec::new();
            for it in items {
                let name = it.Name()?.to_string();
                let low = name.to_lowercase();
                if !words.iter().all(|w| low.contains(w.as_str())) {
                    continue;
                }
                let path = it.Path()?.to_string();
                // 程序自己的缓存、设置（AppData、「.」开头的文件夹）用户一般不找，不列出来
                if path.split('\\').any(|part| part.starts_with('.') || part.eq_ignore_ascii_case("AppData")) {
                    continue;
                }
                let folder = it.IsOfType(StorageItemTypes::Folder)?;
                let meta = std::fs::metadata(&path).ok();
                let size = meta.as_ref().filter(|m| m.is_file()).map(|m| m.len());
                let modified = meta.and_then(|m| m.modified().ok()).map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M").to_string());
                out.push(super::hit(path, folder, size, modified));
                if out.len() >= max as usize {
                    break;
                }
            }
            Ok(out)
        };
        run().ok()
    }

    #[cfg(not(windows))]
    pub fn search(_: &str, _: u32) -> Option<Vec<Hit>> {
        None
    }

    /// Windows Search 服务开着（关了的话索引用不了）
    #[cfg(windows)]
    pub fn available() -> bool {
        std::process::Command::new("sc")
            .args(["query", "WSearch"])
            .creation_flags(0x0800_0000)
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("RUNNING"))
    }

    #[cfg(not(windows))]
    pub fn available() -> bool {
        false
    }

    #[cfg(windows)]
    use std::os::windows::process::CommandExt;
}

#[cfg(test)]
mod tests {
    use super::everything::*;

    /// 照 Everything 的格式拼一封回信
    fn reply(items: &[(&str, &str, bool, u64, u64)]) -> Vec<u8> {
        let flags = 0x01 | 0x02 | 0x10 | 0x40;
        let mut head = Vec::new();
        for v in [items.len() as u32, items.len() as u32, 0, flags, 14] {
            head.extend_from_slice(&v.to_le_bytes());
        }
        let mut data = Vec::new();
        let base = 20 + items.len() * 8;
        let mut idx = Vec::new();
        for (name, dir, folder, size, ft) in items {
            idx.extend_from_slice(&(*folder as u32).to_le_bytes());
            idx.extend_from_slice(&((base + data.len()) as u32).to_le_bytes());
            for s in [name, dir] {
                let w: Vec<u16> = s.encode_utf16().collect();
                data.extend_from_slice(&(w.len() as u32).to_le_bytes());
                for u in w.iter().chain([&0]) {
                    data.extend_from_slice(&u.to_le_bytes());
                }
            }
            data.extend_from_slice(&size.to_le_bytes());
            data.extend_from_slice(&ft.to_le_bytes());
        }
        [head, idx, data].concat()
    }

    #[test]
    fn parses_list2() {
        // 2026-11-17 12:00 UTC
        let ft = (1_794_744_000u64 + 11_644_473_600) * 10_000_000;
        let b = reply(&[("合同 v2.pdf", r"D:\工作\合同", false, 12345, ft), ("合同", r"D:\工作", true, u64::MAX, ft)]);
        let hits = parse(&b).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!((hits[0].path.as_str(), hits[0].name.as_str(), hits[0].dir.as_str()), (r"D:\工作\合同\合同 v2.pdf", "合同 v2.pdf", r"D:\工作\合同"));
        assert_eq!((hits[0].size, hits[0].folder), (Some(12345), false));
        assert!(hits[0].modified.as_deref().is_some_and(|m| m.starts_with("2026-11-1")));
        assert_eq!((hits[1].folder, hits[1].size), (true, None));
        // 坏的回信不崩
        assert!(parse(&b[..30]).is_none());
        assert!(parse(&[]).is_none());
    }

    /// 手动跑：cargo test --lib filesearch::tests::index_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn index_live() {
        let t = std::time::Instant::now();
        let q = std::env::var("Q").unwrap_or_else(|_| "readme".into());
        let f = super::search(&q);
        println!("{} 条，用的 {}，{:?}，{:?}", f.hits.len(), f.engine, t.elapsed(), f.idle);
        for h in f.hits.iter().take(5) {
            println!("  {} | {:?} | {:?}", h.path, h.size, h.modified);
        }
        println!("现在用 {}", super::engine());
    }

    #[test]
    fn builds_query2() {
        let r = request(0x1234, "ext:pdf 合同", 50);
        assert_eq!(&r[0..4], &0x1234u32.to_le_bytes());
        assert_eq!(&r[16..20], &50u32.to_le_bytes());
        assert_eq!(&r[r.len() - 2..], &[0, 0]);
        assert_eq!((r.len() - 28) / 2, "ext:pdf 合同".encode_utf16().count() + 1);
    }
}
