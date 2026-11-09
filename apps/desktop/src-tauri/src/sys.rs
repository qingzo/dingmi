//! 和 Windows 打交道的小工具：判断是否在全屏/演示、用户多久没碰键盘鼠标、取别的程序里选中的文字。

#[cfg(windows)]
pub fn is_busy() -> bool {
    use windows_sys::Win32::UI::Shell::SHQueryUserNotificationState;
    let mut s = 0;
    // SAFETY: 传入有效的输出指针
    let hr = unsafe { SHQueryUserNotificationState(&mut s) };
    if hr != 0 {
        return false;
    }
    // QUNS_BUSY(2) 全屏程序；QUNS_RUNNING_D3D_FULL_SCREEN(3) 全屏游戏；QUNS_PRESENTATION_MODE(4) 演示模式
    matches!(s, 2..=4)
}

#[cfg(not(windows))]
pub fn is_busy() -> bool {
    false
}

/// 读「个性化」里的一个开关（1 浅色 / 0 深色）
#[cfg(windows)]
fn personalize(name: &str) -> Option<u32> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide(name);
    let mut v: u32 = 1;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: 输出缓冲区是一个 u32，大小已经告诉了系统
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut v as *mut u32).cast(),
            &mut size,
        )
    };
    (r == 0).then_some(v)
}

/// 系统设置里「应用模式」是不是深色。窗口在网页画好之前先用这个底色，免得闪一下白
#[cfg(windows)]
pub fn dark_mode() -> bool {
    personalize("AppsUseLightTheme") == Some(0)
}

/// 任务栏是不是深色（「Windows 模式」，和应用的深浅色分开设）：托盘图标的描边跟着换。读不到按深色（Win10 默认）
#[cfg(windows)]
pub fn taskbar_dark() -> bool {
    personalize("SystemUsesLightTheme") != Some(1)
}

#[cfg(not(windows))]
pub fn taskbar_dark() -> bool {
    false
}

/// 托盘图标用多大的图：按系统缩放挑 16 / 20 / 24 / 32，免得系统拿大图缩小了发糊
#[cfg(windows)]
pub fn tray_icon_px() -> u32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON};
    // SAFETY: 只读一个系统尺寸
    let px = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as u32;
    [16, 20, 24, 32].into_iter().find(|&s| s >= px).unwrap_or(32)
}

#[cfg(not(windows))]
pub fn tray_icon_px() -> u32 {
    32
}

#[cfg(not(windows))]
pub fn dark_mode() -> bool {
    false
}

/// 按 Alt、Alt + 空格时不在窗口左上角弹出系统菜单（还原、移动、最小化、关闭那个）。叮咪的窗口都没有标题栏，
/// 网页里的按键还会交给窗口，Windows 就把系统菜单弹在左上角。Alt + F4 关窗口不走这里，照旧。
/// Alt + 空格网页里收不到（WebView2 直接交给窗口当成打开系统菜单），这里转给网页：发 alt-space，带上按着的组合键，
/// 比如 "Alt+Space"、"Ctrl+Alt+Space"（录快捷键时用）
#[cfg(windows)]
pub fn no_alt_menu(w: &tauri::WebviewWindow) {
    use tauri::Emitter;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_LWIN, VK_RWIN, VK_SHIFT};
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{SC_KEYMENU, WM_NCDESTROY, WM_SYSCOMMAND};
    const ID: usize = 0x416c_744d;
    // data 是 Box<WebviewWindow> 的指针，窗口销毁时释放
    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, data: usize) -> LRESULT {
        // 用键盘打开菜单都是 SC_KEYMENU（wParam 的低 4 位是系统自己用的），lParam 是和 Alt 一起按的那个字
        if msg == WM_SYSCOMMAND && (wp & 0xfff0) as u32 == SC_KEYMENU {
            if lp == 0x20 {
                let w = &*(data as *const tauri::WebviewWindow);
                let down = |vk: u16| GetKeyState(vk as i32) < 0;
                let mut combo = Vec::new();
                if down(VK_CONTROL) {
                    combo.push("Ctrl");
                }
                combo.push("Alt");
                if down(VK_SHIFT) {
                    combo.push("Shift");
                }
                if down(VK_LWIN) || down(VK_RWIN) {
                    combo.push("Super");
                }
                combo.push("Space");
                let _ = w.emit_to(w.label(), "alt-space", combo.join("+"));
            }
            return 0;
        }
        if msg == WM_NCDESTROY {
            RemoveWindowSubclass(hwnd, Some(proc), ID);
            drop(Box::from_raw(data as *mut tauri::WebviewWindow));
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
    let Ok(h) = w.hwnd() else { return };
    let h = h.0 as isize;
    let data = Box::into_raw(Box::new(w.clone())) as usize;
    // 子类化要在窗口所在的线程（主线程）里做
    let _ = w.run_on_main_thread(move || {
        // SAFETY: h 是本程序的窗口；回调是静态函数，data 在窗口销毁（WM_NCDESTROY）前一直有效
        unsafe { SetWindowSubclass(h as HWND, Some(proc), ID, data) };
    });
}

#[cfg(not(windows))]
pub fn no_alt_menu(_: &tauri::WebviewWindow) {}

/// 网页里拖进来的文件拿到真实路径。主窗口关了 Tauri 的拖放（笔记编辑器要自己收拖进来的图片），
/// 网页的 drop 事件只有文件内容没有路径；网页把文件交给 WebView2（chrome.webview.postMessageWithAdditionalObjects，
/// 消息是 "dingmi-drop:用途"），这里拿到路径发回这个窗口：files-dropped {tag, paths}
#[cfg(windows)]
pub fn file_drops(w: &tauri::WebviewWindow) {
    use tauri::{Emitter, Manager};
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2File, ICoreWebView2WebMessageReceivedEventArgs2};
    use webview2_com::{take_pwstr, WebMessageReceivedEventHandler};
    use windows::core::{Interface, PWSTR};
    let label = w.label().to_string();
    let app = w.app_handle().clone();
    let _ = w.with_webview(move |pv| {
        // SAFETY: WebView2 的接口都在它自己的线程里调（with_webview 就在那个线程）
        unsafe {
            let Ok(core) = pv.controller().CoreWebView2() else { return };
            let handler = WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut msg = PWSTR::null();
                if args.TryGetWebMessageAsString(&mut msg).is_err() {
                    return Ok(());
                }
                let text = take_pwstr(msg);
                let Some(tag) = text.strip_prefix("dingmi-drop:") else { return Ok(()) };
                let mut paths = Vec::new();
                if let Ok(objs) = args.cast::<ICoreWebView2WebMessageReceivedEventArgs2>().and_then(|a| a.AdditionalObjects()) {
                    let mut n = 0u32;
                    objs.Count(&mut n)?;
                    for i in 0..n {
                        if let Ok(f) = objs.GetValueAtIndex(i).and_then(|o| o.cast::<ICoreWebView2File>()) {
                            let mut p = PWSTR::null();
                            if f.Path(&mut p).is_ok() {
                                paths.push(take_pwstr(p));
                            }
                        }
                    }
                }
                // 真的拖进来的（路径是 WebView2 给的）：记下来，之后 launcher_resolve、打开时才认（见 allow.rs）
                paths.iter().for_each(|p| crate::allow::grant(p));
                let _ = app.emit_to(label.as_str(), "files-dropped", serde_json::json!({ "tag": tag, "paths": paths }));
                Ok(())
            }));
            let mut token = 0i64;
            let _ = core.add_WebMessageReceived(&handler, &mut token);
        }
    });
}

#[cfg(not(windows))]
pub fn file_drops(_: &tauri::WebviewWindow) {}

/// 叮咪的窗口不是浏览器：右键不弹 WebView2 自带的菜单（后退、刷新、另存为、打印……），能打字的地方照样有剪切、复制、粘贴；
/// 网页自己做的右键菜单（快捷框）在网页里 preventDefault，不走到这里。
/// 浏览器的快捷键（F5 / Ctrl+R 刷新、Ctrl+P 打印、Alt+← 后退、Ctrl+F 网页查找、F12……）也关掉，复制粘贴、撤销这些编辑的照旧，
/// 网页自己的快捷键（主窗口的 Ctrl+F）照样收得到。开发版留着，右键「检查」调试用
#[cfg(windows)]
pub fn not_a_browser(w: &tauri::WebviewWindow) {
    use webview2_com::ContextMenuRequestedEventHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2Settings3, ICoreWebView2_11};
    use windows::core::{Interface, BOOL};
    if cfg!(debug_assertions) {
        return;
    }
    let _ = w.with_webview(|pv| {
        // SAFETY: WebView2 的接口都在它自己的线程里调（with_webview 就在那个线程）
        unsafe {
            let Ok(core) = pv.controller().CoreWebView2() else { return };
            if let Ok(s) = core.Settings().and_then(|s| s.cast::<ICoreWebView2Settings3>()) {
                let _ = s.SetAreBrowserAcceleratorKeysEnabled(false);
            }
            let Ok(core) = core.cast::<ICoreWebView2_11>() else { return };
            let handler = ContextMenuRequestedEventHandler::create(Box::new(|_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut editable = BOOL(0);
                args.ContextMenuTarget()?.IsEditable(&mut editable)?;
                if !editable.as_bool() {
                    args.SetHandled(true)?;
                }
                Ok(())
            }));
            let mut token = 0i64;
            let _ = core.add_ContextMenuRequested(&handler, &mut token);
        }
    });
}

#[cfg(not(windows))]
pub fn not_a_browser(_: &tauri::WebviewWindow) {}

/// 有没有装 WebView2（叮咪的界面靠它画）。Win11 自带；Win10 上安装包会装，装不上（比如没网）时这里是 false
pub fn webview2_ready() -> bool {
    webview2_version().split_whitespace().any(|v| v != "0.0.0.0")
}

/// 没有 WebView2 时打不开任何窗口：用系统的对话框说清楚，点「是」打开微软的下载页
#[cfg(windows)]
pub fn alert_no_webview2() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONWARNING, MB_YESNO};
    const URL: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let text = wide(&format!(
        "叮咪的界面要用到微软的 WebView2，这台电脑上还没有装（一般是安装时没连上网）。\n\n\
         点「是」打开微软的下载页，装好后再打开叮咪就行。你的数据不受影响。\n\n下载地址：{URL}"
    ));
    let title = wide("叮咪：还差一个组件");
    // SAFETY: 两个字符串都以 0 结尾，在调用期间一直有效
    let r = unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_YESNO | MB_ICONWARNING) };
    if r == IDYES {
        let _ = std::process::Command::new("explorer").arg(URL).spawn();
    }
}

#[cfg(not(windows))]
pub fn alert_no_webview2() {}

/// 装着的 WebView2 版本（给所有用户装的、只给这个用户装的）。新版本装好后这里才变，拿它判断 WebView2 是不是更新了（见 win.rs）
#[cfg(windows)]
pub fn webview2_version() -> String {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    const CLIENT: &str = r"Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let read = |root: HKEY, key: String| {
        let key = wide(&key);
        let name = wide("pv");
        let mut buf = [0u16; 64];
        let mut size = std::mem::size_of_val(&buf) as u32;
        // SAFETY: 输出缓冲区大小已经告诉了系统，放不下时返回错误、不会写出界
        let r = unsafe {
            RegGetValueW(root, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size)
        };
        if r != 0 {
            return String::new();
        }
        // size 是字节数，算上了结尾的 0
        String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)])
    };
    format!(
        "{} {}",
        read(HKEY_LOCAL_MACHINE, format!(r"SOFTWARE\WOW6432Node\{CLIENT}")),
        read(HKEY_CURRENT_USER, format!(r"Software\{CLIENT}"))
    )
}

#[cfg(not(windows))]
pub fn webview2_version() -> String {
    String::new()
}

/// 开机自动启动：写在当前用户的 Run 里，指向现在运行的这个 dingmi.exe，路径不对就改过来（从别的文件夹运行过、
/// 装到了别处，开机打开的会是那个旧的）。所有用户的 Run 里也有的话删掉：以管理员身份运行过时会写到那里，
/// 普通身份运行的新版改不了它（不是管理员时删不掉，只能等下次以管理员身份运行）。
/// 在任务管理器「启动应用」里关掉的，照旧不开
#[cfg(windows)]
pub fn set_autostart(on: bool) {
    use windows_sys::Win32::System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_SZ, RRF_RT_REG_SZ,
    };
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Run");
    let name = wide("dingmi");
    let read = |root: HKEY| {
        let mut buf = [0u16; 1024];
        let mut size = std::mem::size_of_val(&buf) as u32;
        // SAFETY: 输出缓冲区大小已经告诉了系统，放不下时返回错误、不会写出界
        let r = unsafe {
            RegGetValueW(root, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size)
        };
        // size 是字节数，算上了结尾的 0
        (r == 0).then(|| String::from_utf16_lossy(&buf[..(size as usize / 2).saturating_sub(1)]))
    };
    // SAFETY: 键名、值名都是以 0 结尾的字符串
    let delete = |root: HKEY| unsafe { RegDeleteKeyValueW(root, key.as_ptr(), name.as_ptr()) };
    if read(HKEY_LOCAL_MACHINE).is_some() {
        delete(HKEY_LOCAL_MACHINE);
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let want = format!("\"{}\" --autostart", exe.display());
    // 装好的（旁边有 uninstall.exe）说了算；单独拷出来运行的（比如 release 里的）不抢，除非原来指的那个已经不在了
    let installed = exe.with_file_name("uninstall.exe").exists();
    match read(HKEY_CURRENT_USER) {
        Some(v) if !on || (v != want && (installed || !std::path::Path::new(run_target(&v)).exists())) => {
            delete(HKEY_CURRENT_USER);
        }
        _ => {}
    }
    if on && read(HKEY_CURRENT_USER).is_none() {
        let data = wide(&want);
        // SAFETY: data 以 0 结尾，长度按字节算、含结尾的 0
        unsafe {
            RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32)
        };
    }
}

#[cfg(not(windows))]
pub fn set_autostart(_: bool) {}

/// Run 里那一行指的程序：`"D:\叮咪\dingmi.exe" --autostart`，以前没加引号：`D:\叮咪\dingmi.exe --autostart`
#[cfg_attr(not(windows), allow(dead_code))]
fn run_target(v: &str) -> &str {
    match v.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(rest),
        None => v.split(" --").next().unwrap_or(v).trim(),
    }
}

/// 距离上次键盘或鼠标操作过去了多少秒
#[cfg(windows)]
pub fn idle_secs() -> u64 {
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // SAFETY: 结构体大小已设置
    let ok = unsafe { GetLastInputInfo(&mut info) };
    if ok == 0 {
        return 0;
    }
    let now = unsafe { GetTickCount() };
    (now.wrapping_sub(info.dwTime) / 1000) as u64
}

#[cfg(not(windows))]
pub fn idle_secs() -> u64 {
    0
}

/// 别的程序里选中的文字：模拟按一下"复制"（Ctrl+Insert，在命令行里不会像 Ctrl+C 那样中断程序），
/// 读出来后把剪贴板恢复成原来的样子。没选中、或者那个程序不支持时返回 None。
/// `pressed` 在那个程序收到"复制"之后马上调用（不用复制时也会调用，只调一次）：这时候可以把自己的窗口切到前台了，
/// 不用等到确定没选中文字（没选中时剪贴板不会变，要等到超时才知道）
#[cfg(windows)]
pub fn grab_selection(pressed: impl FnOnce()) -> Option<String> {
    let mut pressed = Some(pressed);
    let text = grab(&mut || {
        if let Some(f) = pressed.take() {
            f()
        }
    });
    if let Some(f) = pressed.take() {
        f()
    }
    text
}

#[cfg(windows)]
fn grab(pressed: &mut dyn FnMut()) -> Option<String> {
    use std::thread::sleep;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow};

    let fg = unsafe { GetForegroundWindow() };
    if fg.is_null() {
        return None;
    }
    // 老式命令行窗口不碰
    let mut cls = [0u16; 64];
    // SAFETY: 缓冲区长度和传入的一致
    let n = unsafe { GetClassNameW(fg, cls.as_mut_ptr(), cls.len() as i32) };
    let cls = String::from_utf16_lossy(&cls[..n.max(0) as usize]);
    if matches!(cls.as_str(), "ConsoleWindowClass" | "PseudoConsoleWindow" | "mintty") {
        return None;
    }
    // 等快捷键松开，不然按下去的会变成 Ctrl+Alt+Insert 之类
    let held = || {
        [VK_MENU, VK_CONTROL, VK_SHIFT, VK_LWIN, VK_RWIN]
            .iter()
            .any(|&k| unsafe { GetAsyncKeyState(k as i32) } < 0)
    };
    let start = Instant::now();
    while held() && start.elapsed() < Duration::from_millis(800) {
        sleep(Duration::from_millis(5));
    }
    if held() {
        return None;
    }
    let saved = clip::save()?;
    let before = unsafe { GetClipboardSequenceNumber() };
    clip::press_copy();
    let start = Instant::now();
    while unsafe { GetClipboardSequenceNumber() } == before {
        // 这时候那个程序早就收到"复制"了，还没复制好多半是没选中：先切过去，后面接着等复制慢一点的程序
        if start.elapsed() > Duration::from_millis(30) {
            pressed();
        }
        if start.elapsed() > Duration::from_millis(300) {
            return None;
        }
        sleep(Duration::from_millis(5));
    }
    // 那个程序已经复制好了，可以切过去了。有的程序分几次往剪贴板里写，稍等一下再读
    pressed();
    sleep(Duration::from_millis(40));
    let text = clip::text();
    clip::restore(&saved);
    text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

#[cfg(not(windows))]
pub fn grab_selection(pressed: impl FnOnce()) -> Option<String> {
    pressed();
    None
}

#[cfg(windows)]
mod clip {
    use std::thread::sleep;
    use std::time::Duration;
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VK_CONTROL, VK_INSERT,
    };

    const CF_UNICODETEXT: u32 = 13;
    /// 剪贴板里原来的东西太大（比如很大的图片）就不动它，免得复制慢、或者恢复不回去
    const MAX_SAVE: usize = 64 * 1024 * 1024;

    fn open() -> bool {
        for _ in 0..20 {
            // SAFETY: 不关联窗口打开剪贴板
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return true;
            }
            sleep(Duration::from_millis(10));
        }
        false
    }

    /// 内容是一块普通内存（HGLOBAL）的格式才能原样存下来；位图、图元文件这类句柄格式都有等价的内存格式，跳过它们
    fn plain_memory(f: u32) -> bool {
        !matches!(f, 2 | 3 | 9 | 14 | 0x80 | 0x82 | 0x83 | 0x8E) && !(0x200..=0x3FF).contains(&f)
    }

    /// 把剪贴板里所有格式的内容拷一份。打不开或者太大时返回 None
    pub fn save() -> Option<Vec<(u32, Vec<u8>)>> {
        if !open() {
            return None;
        }
        let mut out = Vec::new();
        let mut total = 0;
        let mut f = 0;
        let mut ok = true;
        loop {
            // SAFETY: 剪贴板已打开
            f = unsafe { EnumClipboardFormats(f) };
            if f == 0 {
                break;
            }
            if !plain_memory(f) {
                continue;
            }
            let h = unsafe { GetClipboardData(f) };
            if h.is_null() {
                continue;
            }
            let size = unsafe { GlobalSize(h) };
            total += size;
            if total > MAX_SAVE {
                ok = false;
                break;
            }
            let p = unsafe { GlobalLock(h) } as *const u8;
            if p.is_null() {
                continue;
            }
            // SAFETY: GlobalLock 返回的指针至少有 GlobalSize 个字节
            out.push((f, unsafe { std::slice::from_raw_parts(p, size) }.to_vec()));
            unsafe { GlobalUnlock(h) };
        }
        unsafe { CloseClipboard() };
        ok.then_some(out)
    }

    pub fn restore(saved: &[(u32, Vec<u8>)]) {
        if !open() {
            return;
        }
        unsafe { EmptyClipboard() };
        for (f, bytes) in saved {
            // SAFETY: 新分配的内存，拷完交给剪贴板；交不出去就自己释放
            unsafe {
                let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1));
                if h.is_null() {
                    continue;
                }
                let p = GlobalLock(h) as *mut u8;
                if p.is_null() {
                    GlobalFree(h);
                    continue;
                }
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
                GlobalUnlock(h);
                if SetClipboardData(*f, h).is_null() {
                    GlobalFree(h);
                }
            }
        }
        unsafe { CloseClipboard() };
    }

    pub fn text() -> Option<String> {
        if !open() {
            return None;
        }
        let mut out = None;
        // SAFETY: 剪贴板已打开；按 UTF-16 读到第一个 0 为止，不超过这块内存的大小
        unsafe {
            let h = GetClipboardData(CF_UNICODETEXT);
            if !h.is_null() {
                let p = GlobalLock(h) as *const u16;
                if !p.is_null() {
                    let len = GlobalSize(h) / 2;
                    let s = std::slice::from_raw_parts(p, len);
                    let end = s.iter().position(|&c| c == 0).unwrap_or(len);
                    out = Some(String::from_utf16_lossy(&s[..end]));
                    GlobalUnlock(h);
                }
            }
            CloseClipboard();
        }
        out
    }

    /// 按一下 Ctrl+Insert（复制）
    pub fn press_copy() {
        let key = |vk: u16, up: bool, ext: bool| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: (if up { KEYEVENTF_KEYUP } else { 0 }) | (if ext { KEYEVENTF_EXTENDEDKEY } else { 0 }),
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let inputs = [
            key(VK_CONTROL, false, false),
            key(VK_INSERT, false, true),
            key(VK_INSERT, true, true),
            key(VK_CONTROL, true, false),
        ];
        // SAFETY: 数组长度和结构体大小都正确
        unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// 会短暂改动剪贴板再恢复，平时跳过：cargo test --lib -- --ignored clipboard_roundtrip
    #[test]
    #[ignore]
    fn clipboard_roundtrip() {
        let before = clip::save().expect("打不开剪贴板");
        let text: Vec<u8> = "dingmi 测试\0".encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
        clip::restore(&[(13, text)]);
        assert_eq!(clip::text().as_deref(), Some("dingmi 测试"));
        clip::restore(&before);
        assert_eq!(clip::save().unwrap(), before);
    }

    #[test]
    fn run_target_path() {
        assert_eq!(run_target(r#""D:\My Apps\dingmi.exe" --autostart"#), r"D:\My Apps\dingmi.exe");
        assert_eq!(run_target(r"D:\work\dingmi\release\dingmi.exe --autostart"), r"D:\work\dingmi\release\dingmi.exe");
    }
}
