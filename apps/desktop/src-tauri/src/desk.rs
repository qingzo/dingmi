//! 桌面组件贴在桌面上：窗口挂在桌面（Progman）下面、压在所有窗口底下，按 Win+D 时跟着桌面一起露出来；
//! 设置里开了「毛玻璃」时背景用 Windows 的毛玻璃（Acrylic），不管窗口是不是在前面都一直有效；默认不透明，卡片由网页自己画。

use tauri::WebviewWindow;

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetWindowLongPtrW, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWLP_HWNDPARENT,
        HWND_BOTTOM, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_SHOWNOACTIVATE, WA_ACTIVE, WINDOWPOS,
        WM_ACTIVATE, WM_WINDOWPOSCHANGING,
    };

    fn progman() -> HWND {
        let cls: Vec<u16> = "Progman".encode_utf16().chain([0]).collect();
        // SAFETY: 类名是以 0 结尾的宽字符串
        unsafe { FindWindowW(cls.as_ptr(), std::ptr::null()) }
    }

    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _data: usize) -> LRESULT {
        match msg {
            // 不管谁要把它往前挪，都放回最底下（挂在桌面下面，所以是"桌面之上、所有窗口之下"）
            WM_WINDOWPOSCHANGING => {
                let pos = &mut *(lp as *mut WINDOWPOS);
                pos.hwndInsertAfter = HWND_BOTTOM;
                pos.flags &= !SWP_NOZORDER;
            }
            // 不是点它而被激活的（比如按 Win+D 后系统把焦点给了它）：交还给桌面，
            // 不然再按一次 Win+D 恢复不了刚才的窗口
            WM_ACTIVATE if (wp & 0xffff) as u32 == WA_ACTIVE => {
                let mouse = GetAsyncKeyState(VK_LBUTTON as i32) < 0 || GetAsyncKeyState(VK_RBUTTON as i32) < 0;
                if !mouse {
                    let r = DefSubclassProc(hwnd, msg, wp, lp);
                    let desk = progman();
                    if !desk.is_null() {
                        SetForegroundWindow(desk);
                    }
                    return r;
                }
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }

    /// 挂到桌面下面、放到最底下。要在窗口所在的线程（主线程）里调用。可以重复调用（资源管理器重启后桌面换了）
    pub fn pin(hwnd: HWND) {
        let desk = progman();
        if desk.is_null() {
            return;
        }
        // SAFETY: hwnd 是本程序的窗口；子类化的回调在整个窗口生命期内有效
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, desk as isize);
            SetWindowSubclass(hwnd, Some(proc), 0x4865_794d, 0);
            SetWindowPos(hwnd, HWND_BOTTOM, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }

    /// 还挂在现在的桌面下面吗（资源管理器重启后桌面是新的窗口）
    pub fn pinned(hwnd: HWND) -> bool {
        let desk = progman();
        // SAFETY: 只读窗口属性
        desk.is_null() || unsafe { GetWindowLongPtrW(hwnd, GWLP_HWNDPARENT) } == desk as isize
    }

    #[repr(C)]
    struct AccentPolicy {
        state: u32,
        flags: u32,
        color: u32,
        animation: u32,
    }

    #[repr(C)]
    struct CompositionData {
        attrib: u32,
        data: *mut AccentPolicy,
        size: usize,
    }

    type SetWca = unsafe extern "system" fn(HWND, *mut CompositionData) -> BOOL;

    /// 窗口背后的效果：state 4 是毛玻璃（ACCENT_ENABLE_ACRYLICBLURBEHIND），0 是关掉；color 是叠在上面的颜色 (r, g, b, 不透明度)。
    /// 用的是老办法 SetWindowCompositionAttribute：Windows 11 新的那种（DWMSBT）窗口不在前面时会变成纯色，组件几乎总是不在前面
    fn accent(hwnd: HWND, state: u32, tint: [u8; 4]) -> bool {
        use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
        let user32: Vec<u16> = "user32.dll".encode_utf16().chain([0]).collect();
        // SAFETY: user32 一定已经加载；函数签名和系统里的一致
        unsafe {
            let m = GetModuleHandleW(user32.as_ptr());
            let Some(f) = GetProcAddress(m, c"SetWindowCompositionAttribute".as_ptr().cast()) else {
                return false;
            };
            let f: SetWca = std::mem::transmute(f);
            let [r, g, b, a] = tint;
            let mut policy = AccentPolicy {
                state,
                flags: 0,
                color: r as u32 | (g as u32) << 8 | (b as u32) << 16 | (a.max(1) as u32) << 24,
                animation: 0,
            };
            let mut data = CompositionData {
                // WCA_ACCENT_POLICY
                attrib: 19,
                data: &mut policy,
                size: std::mem::size_of::<AccentPolicy>(),
            };
            let ok = f(hwnd, &mut data) != 0;
            // 毛玻璃时要系统的圆角（DWMWA_WINDOW_CORNER_PREFERENCE = 33，DWMWCP_ROUND = 2），关掉时回到默认（0）
            let round: u32 = if state == 0 { 0 } else { 2 };
            DwmSetWindowAttribute(hwnd, 33, (&round as *const u32).cast(), 4);
            ok
        }
    }

    pub fn acrylic(hwnd: HWND, tint: [u8; 4]) -> bool {
        win11() && accent(hwnd, 4, tint)
    }

    pub fn clear(hwnd: HWND) {
        accent(hwnd, 0, [0; 4]);
    }

    /// Windows 11（10.0.22000 起）。Windows 10 上毛玻璃拖动时卡、也没有圆角，退成不透明的（需求-0.2 第 9.2 节）
    pub fn win11() -> bool {
        use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
        use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;
        type GetVersion = unsafe extern "system" fn(*mut OSVERSIONINFOW) -> i32;
        let ntdll: Vec<u16> = "ntdll.dll".encode_utf16().chain([0]).collect();
        // SAFETY: ntdll 一定已经加载；RtlGetVersion 的签名和系统里的一致，结构体大小已经填好
        unsafe {
            let Some(f) = GetProcAddress(GetModuleHandleW(ntdll.as_ptr()), c"RtlGetVersion".as_ptr().cast()) else {
                return false;
            };
            let f: GetVersion = std::mem::transmute(f);
            let mut v: OSVERSIONINFOW = std::mem::zeroed();
            v.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
            f(&mut v) == 0 && v.dwBuildNumber >= 22000
        }
    }

    pub fn show(hwnd: HWND) {
        // SAFETY: 本程序的窗口
        unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
    }
}

#[cfg(windows)]
fn hwnd(w: &WebviewWindow) -> Option<windows_sys::Win32::Foundation::HWND> {
    w.hwnd().ok().map(|h| h.0 as _)
}

/// 挂到桌面下面（在主线程里做）
pub fn pin(w: &WebviewWindow) {
    #[cfg(windows)]
    if let Some(h) = hwnd(w) {
        let h = h as isize;
        let _ = w.run_on_main_thread(move || imp::pin(h as _));
    }
}

/// 资源管理器重启过的话重新挂一次。后台每 10 秒看一眼
pub fn repin(w: &WebviewWindow) {
    #[cfg(windows)]
    if let Some(h) = hwnd(w) {
        if !imp::pinned(h) {
            pin(w);
        }
    }
}

/// 上毛玻璃，tint 是叠在上面的颜色 (r, g, b, 不透明度)；Windows 10 和不支持的系统上返回 false
pub fn glass(w: &WebviewWindow, tint: [u8; 4]) -> bool {
    #[cfg(windows)]
    if let Some(h) = hwnd(w) {
        return imp::acrylic(h, tint);
    }
    let _ = (w, tint);
    false
}

/// 这台电脑上能不能用毛玻璃（Windows 11 才有）
pub fn glass_supported() -> bool {
    #[cfg(windows)]
    return imp::win11();
    #[cfg(not(windows))]
    false
}

/// 去掉毛玻璃（换回不透明）
pub fn clear(w: &WebviewWindow) {
    #[cfg(windows)]
    if let Some(h) = hwnd(w) {
        imp::clear(h);
    }
    let _ = w;
}

/// 显示出来但不抢焦点
pub fn show(w: &WebviewWindow) {
    #[cfg(windows)]
    if let Some(h) = hwnd(w) {
        imp::show(h);
        return;
    }
    let _ = w.show();
}
