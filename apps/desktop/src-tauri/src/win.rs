//! 各个窗口的创建和摆放。窗口都是用到时才创建：快捷框建了就一直留着；主窗口和快速记笔记关掉时先藏起来，
//! 再打开是瞬间的，藏了 10 分钟没再用才销毁（省内存）；提醒卡片这类关掉就销毁。
//! 窗口先隐藏着创建，等网页画好了再显示，免得先闪一下白底。桌面组件打开后一直留着，贴在桌面上。

use crate::{db, AppState};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::window::Color;
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, Monitor, PhysicalPosition, PhysicalSize, Size, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder, WindowEvent, Wry,
};

/// 关掉的主窗口、快速记笔记藏着留多久，过了还没再打开就销毁
const KEEP_HIDDEN: Duration = Duration::from_secs(600);

/// 每个窗口的"第几次显示或藏起来"：藏起来时记下当时的数，到时候数没变（期间没再打开过）才销毁
static GEN: LazyLock<Mutex<HashMap<String, u64>>> = LazyLock::new(Default::default);

fn bump(label: &str) -> u64 {
    let mut m = GEN.lock().unwrap();
    let g = m.entry(label.to_string()).or_default();
    *g += 1;
    *g
}

fn gen_of(label: &str) -> u64 {
    GEN.lock().unwrap().get(label).copied().unwrap_or(0)
}

/// 藏起来，过一会儿还没再打开就销毁
pub fn park(w: &WebviewWindow) {
    if w.label() == "main" {
        save_main_rect(w);
    }
    let _ = w.hide();
    let gen = bump(w.label());
    let w = w.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(KEEP_HIDDEN).await;
        if gen_of(w.label()) == gen && !w.is_visible().unwrap_or(true) {
            let _ = w.destroy();
        }
    });
}

/// 点关闭（或者 Alt+F4）只是藏起来
fn hide_on_close(w: &WebviewWindow) {
    let me = w.clone();
    w.on_window_event(move |e| {
        if let WindowEvent::CloseRequested { api, .. } = e {
            api.prevent_close();
            park(&me);
        }
    });
}

/// 主窗口、笔记窗口在网页画好之前的底色，和暖云母的窗口底（tokens.css 的 --bg）一样
fn window_color() -> Color {
    if crate::sys::dark_mode() {
        Color(0x17, 0x13, 0x0f, 0xff)
    } else {
        Color(0xf6, 0xf1, 0xec, 0xff)
    }
}

// 所有窗口共用同一组 WebView2 参数（参数不同会导致创建失败）。允许无手势播放提示音。
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required";

fn builder<'a>(app: &'a AppHandle, label: &'a str, url: &str) -> WebviewWindowBuilder<'a, Wry, AppHandle> {
    // 调试用：设置环境变量 DINGMI_WEBVIEW_ARGS="--remote-debugging-port=9333" 可以用浏览器调试各个窗口
    let args = match std::env::var("DINGMI_WEBVIEW_ARGS") {
        Ok(extra) if !extra.trim().is_empty() => format!("{BROWSER_ARGS} {}", extra.trim()),
        _ => BROWSER_ARGS.to_string(),
    };
    let b = WebviewWindowBuilder::new(app, label, WebviewUrl::App(url.into())).additional_browser_args(&args);
    // 测试副本（DINGMI_DATA_DIR）用自己的 WebView2 目录：不和装好的叮咪共用网页存储（比如快速记笔记的草稿），
    // 装好的叮咪开着时，带调试参数的测试副本也能启动（同一个目录下参数不同会创建失败）
    match crate::dev_data_dir() {
        Some(d) => b.data_directory(d.join("EBWebView")),
        None => b,
    }
}

// ---------- WebView2 在叮咪开着时自己更新了 ----------
// 之后新建的窗口用的是新版本，和已经在跑的旧版本对不上，建不出来（主窗口、笔记窗口、提醒卡片都出不来，也不报错），
// 开着的快捷框、桌面组件倒是照常能用。只有重启叮咪才好：发现更新了、又没在用叮咪时悄悄重启；
// 万一已经有窗口建不出来了，也马上重启。

/// 重启出来的叮咪带着这个参数：先等旧的退干净，免得被当成第二个叮咪
const RELAUNCH: &str = "--relaunch";
static STARTED: LazyLock<Instant> = LazyLock::new(Instant::now);
static WEBVIEW2_AT_START: LazyLock<String> = LazyLock::new(crate::sys::webview2_version);

/// 启动时最先调用：记下启动时间和这时装着的 WebView2 版本
pub fn init() {
    LazyLock::force(&STARTED);
    LazyLock::force(&WEBVIEW2_AT_START);
    if std::env::args().any(|a| a == RELAUNCH) {
        std::thread::sleep(Duration::from_millis(1500));
    }
}

/// 后台每次计时都看一眼：WebView2 更新了、除了桌面组件没有窗口开着（没在用叮咪），就悄悄重启，主窗口不打开
pub fn restart_if_webview2_updated(app: &AppHandle) {
    if crate::sys::webview2_version() == *WEBVIEW2_AT_START {
        return;
    }
    let in_use = app.webview_windows().iter().any(|(label, w)| label != "widget" && w.is_visible().unwrap_or(false));
    if !in_use {
        relaunch(app, false);
    }
}

/// 建窗口的结果：建好了，按 Alt 不弹系统菜单，右键、F5 这些不当浏览器用。建不出来就重启叮咪：open_main 是用户点了要开的窗口，重启后打开主窗口，让他知道发生了什么；
/// 提醒卡片这类自己弹的，重启后会接着弹
fn made(app: &AppHandle, open_main: bool, r: tauri::Result<WebviewWindow>) -> Option<WebviewWindow> {
    match r {
        Ok(w) => {
            crate::sys::no_alt_menu(&w);
            crate::sys::not_a_browser(&w);
            Some(w)
        }
        // 同时点了两下：另一下已经在建了，不是坏了
        Err(tauri::Error::WindowLabelAlreadyExists(_) | tauri::Error::WebviewLabelAlreadyExists(_)) => None,
        Err(e) => {
            crate::diag::log("窗口", format!("窗口没建成：{e}"));
            relaunch(app, open_main);
            None
        }
    }
}

fn relaunch(app: &AppHandle, open_main: bool) {
    // 刚启动就建不出窗口，多半是 WebView2 本身坏了，重启也没用，不来回重启
    if STARTED.elapsed() < Duration::from_secs(120) {
        return;
    }
    static ONCE: AtomicBool = AtomicBool::new(false);
    if ONCE.swap(true, Ordering::SeqCst) {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let mut cmd = std::process::Command::new(exe);
    cmd.arg(RELAUNCH);
    if !open_main {
        cmd.arg("--autostart");
    }
    if cmd.spawn().is_err() {
        return;
    }
    app.cleanup_before_exit();
    std::process::exit(0);
}

/// 一块屏幕的工作区（去掉任务栏），物理像素：(x, y, w, h, scale)
fn area_of(m: &Monitor) -> (f64, f64, f64, f64, f64) {
    let wa = m.work_area();
    (wa.position.x as f64, wa.position.y as f64, wa.size.width as f64, wa.size.height as f64, m.scale_factor())
}

/// 主显示器的工作区
fn work_area(app: &AppHandle) -> (f64, f64, f64, f64, f64) {
    match app.primary_monitor() {
        Ok(Some(m)) => area_of(&m),
        _ => (0.0, 0.0, 1920.0, 1040.0, 1.0),
    }
}

/// 鼠标所在那块屏幕的工作区；拿不到时用主显示器的
fn cursor_area(app: &AppHandle) -> (f64, f64, f64, f64, f64) {
    let m = app.cursor_position().ok().and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten());
    m.map(|m| area_of(&m)).unwrap_or_else(|| work_area(app))
}

fn reveal(w: &WebviewWindow) {
    bump(w.label());
    let _ = w.unminimize();
    let _ = w.show();
    let _ = w.set_focus();
}

// ---------- 主窗口（UI设计.md U6）：默认大小跟着屏幕定，最小 760 × 520，记住上次的大小和位置 ----------

/// 默认大小的下限、上限（逻辑像素）：一般在工作区的 50% × 66%，夹在这两个之间（2026-11-06 试用后改，原来固定 1040 × 680；
/// 上限先定过 1360 × 900，4K 屏上太大）
const MAIN_W: f64 = 1040.0;
const MAIN_H: f64 = 680.0;
const MAIN_MAX_W: f64 = 1200.0;
const MAIN_MAX_H: f64 = 780.0;
const MAIN_MIN_W: f64 = 760.0;
const MAIN_MIN_H: f64 = 520.0;
/// 上次藏起来时的位置和大小，记在 kv 里（物理像素 "x,y,宽,高"）。只属于这台电脑
const MAIN_KV: &str = "main_rect";

/// 默认大小：工作区的 50% × 66%，夹在 1040 × 680 和 1200 × 780 之间；
/// 屏幕小（比如 1366 × 768 再放大 125%）时不超过工作区的九成，但不小于最小大小
fn main_size(app: &AppHandle) -> (f64, f64) {
    let (_, _, ww, wh, sf) = work_area(app);
    fit_size((ww / sf, wh / sf))
}

fn fit_size((ww, wh): (f64, f64)) -> (f64, f64) {
    let w = (ww * 0.5).clamp(MAIN_W, MAIN_MAX_W);
    let h = (wh * 0.66).clamp(MAIN_H, MAIN_MAX_H);
    (w.min(ww * 0.92).max(MAIN_MIN_W).round(), h.min(wh * 0.9).max(MAIN_MIN_H).round())
}

/// 设置里「恢复默认」：忘掉记下的位置和大小，开着的主窗口马上变回默认大小、放到屏幕中间
pub fn main_reset(app: &AppHandle) {
    {
        let st = app.state::<AppState>();
        let c = st.db.lock().unwrap();
        let _ = db::kv_del(&c, MAIN_KV);
    }
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unmaximize();
        let (mw, mh) = main_size(app);
        let _ = w.set_size(Size::Logical(LogicalSize::new(mw, mh)));
        let _ = w.center();
    }
}

/// "x,y,宽,高" → 数字；不对就是 None
fn parse_rect(s: &str) -> Option<(i32, i32, u32, u32)> {
    let v: Vec<i64> = s.split(',').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
    match v[..] {
        [x, y, w, h] if w > 0 && h > 0 => Some((x as i32, y as i32, w as u32, h as u32)),
        _ => None,
    }
}

/// 上次的位置和大小；标题栏那一块不在任何一块屏幕上（拔了显示器、改了分辨率）就不用
fn main_rect(app: &AppHandle) -> Option<(i32, i32, u32, u32)> {
    let saved = {
        let st = app.state::<AppState>();
        let c = st.db.lock().unwrap();
        db::kv_get(&c, MAIN_KV)
    }?;
    let (x, y, w, h) = parse_rect(&saved)?;
    let (cx, cy) = (x + 80, y + 16);
    let on_screen = app.available_monitors().unwrap_or_default().iter().any(|m| {
        let (p, s) = (m.position(), m.size());
        cx >= p.x && cy >= p.y && cx < p.x + s.width as i32 && cy < p.y + s.height as i32
    });
    on_screen.then_some((x, y, w, h))
}

/// 藏起来时记下位置和大小（最大化、最小化着的不记，下次还是原来正常时的样子）
fn save_main_rect(w: &WebviewWindow) {
    if w.is_maximized().unwrap_or(false) || w.is_minimized().unwrap_or(false) {
        return;
    }
    let (Ok(p), Ok(s)) = (w.outer_position(), w.inner_size()) else {
        return;
    };
    let app = w.app_handle();
    let st = app.state::<AppState>();
    let c = st.db.lock().unwrap();
    let _ = db::kv_set(&c, MAIN_KV, &format!("{},{},{},{}", p.x, p.y, s.width, s.height));
}

pub fn open_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        reveal(&w);
        return;
    }
    let (mw, mh) = main_size(app);
    // 先藏着，网页读完数据、画好第一屏后自己显示出来（main.ts）
    let r = builder(app, "main", "index.html")
        .title(if crate::dev_data_dir().is_some() { "叮咪测试" } else { "叮咪" })
        .inner_size(mw, mh)
        .min_inner_size(MAIN_MIN_W, MAIN_MIN_H)
        // 让网页自己收拖进来的文件（笔记里拖入图片）
        .disable_drag_drop_handler()
        .decorations(false)
        .center()
        .background_color(window_color())
        .visible(false)
        .build();
    let Some(w) = made(app, true, r) else {
        return;
    };
    if let Some((x, y, pw, ph)) = main_rect(app) {
        let _ = w.set_size(PhysicalSize::new(pw, ph));
        let _ = w.set_position(PhysicalPosition::new(x, y));
    }
    // 设置 → 快捷框 → 自己加的：拖进来的文件要真实路径
    crate::sys::file_drops(&w);
    hide_on_close(&w);
    // 万一网页出错没能自己显示，2 秒后也显示出来（期间用户已经关掉、又打开过的不管）
    let gen = bump("main");
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if gen_of("main") == gen && !w.is_visible().unwrap_or(true) {
            reveal(&w);
        }
    });
}

// 弹出窗口的宽度 = 卡片宽度 + 两边各 14 像素留给阴影
/// 快捷框：框宽 640，两边各 24 留给阴影（U4；和 quickbox.css 的 .stage 对上）
const FLASH_W: f64 = 688.0;
/// 快捷框窗口顶上留给阴影的那一点（.stage 的上边距）：框的顶边在屏幕高度 22% 处
const FLASH_TOP: f64 = 10.0;
/// 快速记笔记：卡片宽 460，两边留给阴影（U7）
const QNOTE_W: f64 = 488.0;
/// 开场白、收尾：卡片宽 420，两边各 28 留给阴影（U5）
const CARD_W: f64 = 476.0;

pub fn toggle_flash(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("flash") {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
        } else {
            place_flash(app, &w);
            reveal(&w);
            let _ = w.emit_to("flash", "flash-open", ());
            crate::launcher::refresh_if_old(app);
        }
        return;
    }
    // 还没建好（一般启动时就建好了）：先藏着建，网页准备好后自己显示（flash.ts）
    build_flash(app, "flash.html#open");
}

/// 启动时先把快捷框藏着建好，按快捷键时只要显示出来，不用等网页加载
pub fn warm_flash(app: &AppHandle) {
    if app.get_webview_window("flash").is_none() {
        build_flash(app, "flash.html");
    }
}

fn build_flash(app: &AppHandle, url: &str) {
    let r = builder(app, "flash", url)
        .title("快捷框")
        .inner_size(FLASH_W, 200.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build();
    if let Some(w) = made(app, false, r) {
        place_flash(app, &w);
    }
}

/// 快捷框被拖到别处后记下的位置：kv 里 "工作区左上角 x,y,离工作区左上角 dx,dy"（物理像素）。
/// 工作区左上角用来认是哪块屏幕：只在同一块屏幕上用，别的屏幕照旧默认位置。只属于这台电脑
const FLASH_KV: &str = "flash_pos";

/// 快捷框：鼠标所在那块屏幕，水平居中，框的顶边在屏幕高度 22% 处；拖到别处过就用拖到的位置。结果往下展开，顶边不动（U4）
fn place_flash(app: &AppHandle, w: &WebviewWindow) {
    let (x, y, ww, wh, sf) = cursor_area(app);
    let saved = {
        let st = app.state::<AppState>();
        let c = st.db.lock().unwrap();
        db::kv_get(&c, FLASH_KV)
    };
    let mine = saved.and_then(|t| {
        let v: Vec<f64> = t.split(',').filter_map(|n| n.trim().parse().ok()).collect();
        (v.len() == 4 && v[0] == x && v[1] == y).then(|| (v[2], v[3]))
    });
    let (px, py) = match mine {
        // 屏幕分辨率、缩放变过的话，夹回屏幕里面
        Some((dx, dy)) => ((x + dx).min(x + ww - FLASH_W * sf).max(x), (y + dy).min(y + wh - 120.0 * sf).max(y)),
        None => (x + (ww - FLASH_W * sf) / 2.0, y + wh * 0.22 - FLASH_TOP * sf),
    };
    let _ = w.set_position(PhysicalPosition::new(px.round() as i32, py.round() as i32));
}

/// 快捷框拖完了：记下它在哪块屏幕的什么位置
pub fn flash_moved(app: &AppHandle, w: &WebviewWindow) {
    let Ok(p) = w.outer_position() else { return };
    let sf = w.scale_factor().unwrap_or(1.0);
    let Some(m) = app.monitor_from_point(p.x as f64 + FLASH_W * sf / 2.0, p.y as f64 + 40.0 * sf).ok().flatten() else { return };
    let (x, y, ..) = area_of(&m);
    let st = app.state::<AppState>();
    let c = st.db.lock().unwrap();
    let _ = db::kv_set(&c, FLASH_KV, &format!("{x},{y},{},{}", p.x as f64 - x, p.y as f64 - y));
}

/// 设置里「放回原位」：忘掉拖过的位置，开着的话马上挪回去
pub fn flash_pos_reset(app: &AppHandle) {
    {
        let st = app.state::<AppState>();
        let c = st.db.lock().unwrap();
        let _ = db::kv_del(&c, FLASH_KV);
    }
    if let Some(w) = app.get_webview_window("flash").filter(|w| w.is_visible().unwrap_or(false)) {
        place_flash(app, &w);
    }
}

/// 快速记笔记：和快捷框同一个位置（鼠标所在那块屏幕，顶边在屏幕高度 22% 处），变高时顶边不动往下长（U7）
fn place_top(app: &AppHandle, w: &WebviewWindow, width: f64) {
    let (x, y, ww, wh, sf) = cursor_area(app);
    let px = x + (ww - width * sf) / 2.0;
    let py = y + wh * 0.22 - FLASH_TOP * sf;
    let _ = w.set_position(PhysicalPosition::new(px.round() as i32, py.round() as i32));
}

fn place_center(app: &AppHandle, w: &WebviewWindow, width: f64, height: f64) {
    let (x, y, ww, wh, sf) = work_area(app);
    let px = x + (ww - width * sf) / 2.0;
    let py = y + ((wh - height * sf) / 2.0).max(0.0);
    let _ = w.set_position(PhysicalPosition::new(px as i32, py as i32));
}

/// 屏幕中间偏上：卡片中心在工作区高度的 40% 处（开场白、收尾；U5 原来定的偏下，试用后改的）
fn place_upper(app: &AppHandle, w: &WebviewWindow, width: f64, height: f64) {
    let (x, y, ww, wh, sf) = work_area(app);
    let px = x + (ww - width * sf) / 2.0;
    let py = (y + wh * 0.4 - height * sf / 2.0).min(y + wh - height * sf).max(y);
    let _ = w.set_position(PhysicalPosition::new(px as i32, py as i32));
}

fn place_bottom_right(app: &AppHandle, w: &WebviewWindow, width: f64, height: f64) {
    let (x, y, ww, wh, sf) = work_area(app);
    let px = x + ww - width * sf;
    let py = y + wh - height * sf;
    let _ = w.set_position(PhysicalPosition::new(px as i32, py as i32));
}

pub fn toggle_qnote(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("qnote") {
        if w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false) {
            park(&w);
        } else {
            place_top(app, &w, QNOTE_W);
            reveal(&w);
            let _ = w.emit_to("qnote", "qnote-open", ());
        }
        return;
    }
    let r = builder(app, "qnote", "qnote.html")
        .title("快速记笔记")
        .inner_size(QNOTE_W, 330.0)
        // 让网页自己收拖进来的文件
        .disable_drag_drop_handler()
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build();
    if let Some(w) = made(app, true, r) {
        // 网页量好高度后自己显示（qnote.ts）。和快捷框同一个位置（U7）
        place_top(app, &w, QNOTE_W);
        hide_on_close(&w);
    }
}

/// 提醒卡片宽 360；左边留 28 给阴影，右边 12 是离屏幕边的距离（U5；网页里 cards.css 的 .stage 留同样的边）
const REMIND_W: f64 = 400.0;

/// 打开提醒卡片。不抢焦点，免得打断正在打字的人。
pub fn open_remind(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("remind") {
        crate::desk::show(&w);
        return;
    }
    let r = builder(app, "remind", "remind.html")
        .title("叮咪提醒")
        .inner_size(REMIND_W, 260.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build();
    if let Some(w) = made(app, false, r) {
        place_bottom_right(app, &w, REMIND_W, 260.0);
    }
}

pub fn open_morning(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("morning") {
        reveal(&w);
        return;
    }
    let r = builder(app, "morning", "morning.html")
        .title("早上好")
        .inner_size(CARD_W, 520.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build();
    if let Some(w) = made(app, false, r) {
        place_upper(app, &w, CARD_W, 520.0);
    }
}

/// 每天收尾的卡片，和开场白一样放在屏幕中间偏上
pub fn open_wrap(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("wrap") {
        reveal(&w);
        return;
    }
    let r = builder(app, "wrap", "wrap.html")
        .title("今天收个尾")
        .inner_size(CARD_W, 480.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build();
    if let Some(w) = made(app, false, r) {
        place_upper(app, &w, CARD_W, 480.0);
    }
}

// 首次引导：卡片宽 520，两边各 14 像素留给阴影
const ONBOARD_W: f64 = 548.0;

/// 首次引导（需求-0.2 第 5 节）：屏幕中间，在任务栏上有按钮（这时候还没有主窗口）。关掉就销毁
pub fn open_onboard(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("onboard") {
        reveal(&w);
        return;
    }
    let r = builder(app, "onboard", "onboard.html")
        .title(if crate::dev_data_dir().is_some() { "叮咪测试" } else { "叮咪" })
        .inner_size(ONBOARD_W, 520.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .visible(false)
        .build();
    let Some(w) = made(app, true, r) else {
        return;
    };
    // 网页量好高度后自己显示（onboard.ts）；万一网页出错，2 秒后也显示出来
    place_center(app, &w, ONBOARD_W, 520.0);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if !w.is_visible().unwrap_or(true) {
            reveal(&w);
        }
    });
}

/// 页面量好自己的高度后调用，窗口跟着内容变高变矮。
pub fn fit(app: &AppHandle, w: &WebviewWindow, height: f64) {
    let width = w
        .inner_size()
        .ok()
        .map(|s| s.width as f64 / w.scale_factor().unwrap_or(1.0))
        .unwrap_or(400.0);
    let _ = w.set_size(Size::Logical(LogicalSize::new(width, height)));
    match w.label() {
        "remind" => place_bottom_right(app, w, width, height),
        // 开场白、收尾能拖：已经显示出来的（可能被拖到别处了）只改高度，不挪回去
        "morning" | "wrap" if !w.is_visible().unwrap_or(false) => place_upper(app, w, width, height),
        "onboard" => place_center(app, w, width, height),
        // 快速记笔记：顶边不动，往下长
        "qnote" => place_top(app, w, width),
        _ => {}
    }
}

// ---------- 桌面组件 ----------

/// 卡片的宽度（UI设计.md U8）
const WIDGET_W: f64 = 300.0;
/// 不透明时窗口比卡片大一圈透明的边：左右给阴影，上面给团子猫探头（关掉团子猫时只留一点给阴影）。
/// 和 widget.css 里 .wrap 的 padding 一样；毛玻璃时窗口就是卡片，没有边
const WIDGET_SIDE: f64 = 16.0;
const WIDGET_PEEK: f64 = 40.0;
const WIDGET_TOP: f64 = 12.0;
/// 默认位置离屏幕右边、上边多远（算到窗口边上）
const WIDGET_GAP: f64 = 24.0;
/// 拖到哪里记在 kv 里（卡片左上角，物理像素 "x,y"）。只属于这台电脑，不放进设置，免得主窗口存设置时把它盖掉
const WIDGET_KV: &str = "widget_pos";
/// 现在窗口比卡片多出来的边（左、上，逻辑像素），widget_look 时定
static WIDGET_PAD: Mutex<(f64, f64)> = Mutex::new((WIDGET_SIDE, WIDGET_PEEK));
/// 现在是不是毛玻璃
static WIDGET_GLASS: AtomicBool = AtomicBool::new(false);

/// 卡片左上角默认放在主屏幕右上角（窗口离屏幕边 WIDGET_GAP）
fn widget_home(app: &AppHandle) -> PhysicalPosition<i32> {
    let (x, y, ww, _, sf) = work_area(app);
    let (side, top) = *WIDGET_PAD.lock().unwrap();
    PhysicalPosition::new((x + ww - (WIDGET_W + side + WIDGET_GAP) * sf) as i32, (y + (top + WIDGET_GAP) * sf) as i32)
}

/// 上次放的位置（卡片左上角）；那块屏幕不在了（拔了显示器、改了分辨率）就回到右上角
fn widget_pos(app: &AppHandle) -> PhysicalPosition<i32> {
    let saved = {
        let st = app.state::<AppState>();
        let c = st.db.lock().unwrap();
        db::kv_get(&c, WIDGET_KV)
    };
    let xy = saved.and_then(|s| {
        let (x, y) = s.split_once(',')?;
        Some((x.trim().parse::<i32>().ok()?, y.trim().parse::<i32>().ok()?))
    });
    if let Some((x, y)) = xy {
        // 卡片左上角往里一点的地方要在某块屏幕上
        let (cx, cy) = (x + 40, y + 20);
        let on_screen = app.available_monitors().unwrap_or_default().iter().any(|m| {
            let (p, s) = (m.position(), m.size());
            cx >= p.x && cy >= p.y && cx < p.x + s.width as i32 && cy < p.y + s.height as i32
        });
        if on_screen {
            return PhysicalPosition::new(x, y);
        }
    }
    widget_home(app)
}

/// 窗口比卡片多出来的边（左右、上）
fn widget_pad(s: &crate::settings::Settings, glass: bool) -> (f64, f64) {
    // 团子猫没手动改过就跟着风格：简洁默认关（和 lib/look.ts 的 mascotOn 一样）
    let mascot = s.mascot.unwrap_or(s.style != "simple");
    match (glass, mascot) {
        (true, _) => (0.0, 0.0),
        (false, true) => (WIDGET_SIDE, WIDGET_PEEK),
        (false, false) => (WIDGET_SIDE, WIDGET_TOP),
    }
}

/// 把窗口摆到卡片该在的地方
fn widget_move(w: &WebviewWindow, card: PhysicalPosition<i32>) {
    let (side, top) = *WIDGET_PAD.lock().unwrap();
    let sf = w.scale_factor().unwrap_or(1.0);
    let _ = w.set_position(PhysicalPosition::new(card.x - (side * sf).round() as i32, card.y - (top * sf).round() as i32));
}

/// 组件的底：设置里开了毛玻璃（而且是 Windows 11）就上毛玻璃，不然是网页自己画的不透明卡片。
/// 两种样子窗口大小不一样，卡片在屏幕上的位置不变。回是不是毛玻璃
pub fn widget_look(app: &AppHandle, w: &WebviewWindow, tint: [u8; 4]) -> bool {
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    let glass = s.widget_glass && crate::desk::glass(w, tint);
    // 上过毛玻璃又关掉时才去掉（一直不透明的窗口不去动它）
    if WIDGET_GLASS.swap(glass, Ordering::Relaxed) && !glass {
        crate::desk::clear(w);
    }
    // 毛玻璃时窗口就是卡片，用系统的细边框和圆角；不透明时卡片的边和阴影网页自己画
    let _ = w.set_shadow(glass);
    let pad = widget_pad(&s, glass);
    *WIDGET_PAD.lock().unwrap() = pad;
    let sf = w.scale_factor().unwrap_or(1.0);
    let h = w.inner_size().map(|s| s.height as f64 / sf).unwrap_or(400.0);
    let _ = w.set_size(Size::Logical(LogicalSize::new(WIDGET_W + pad.0 * 2.0, h)));
    widget_move(w, widget_pos(app));
    glass
}

/// 单独的笔记编辑窗口：每条笔记一个（标签 note-<id>），已经开着就切到前面。关掉就销毁（关之前网页先存好）
pub fn open_note(app: &AppHandle, id: i64) {
    let label = format!("note-{id}");
    if let Some(w) = app.get_webview_window(&label) {
        reveal(&w);
        return;
    }
    // 先藏着，网页读好笔记、按上次的大小摆好后自己显示（note.ts）
    let r = builder(app, &label, &format!("note.html?id={id}"))
        .title("叮咪笔记")
        .inner_size(980.0, 680.0)
        .min_inner_size(560.0, 400.0)
        .disable_drag_drop_handler()
        .decorations(false)
        .center()
        .background_color(window_color())
        .visible(false)
        .build();
    let Some(w) = made(app, true, r) else {
        return;
    };
    let me = app.clone();
    w.on_window_event(move |e| {
        if let WindowEvent::Destroyed = e {
            let _ = me.emit("note-windows", note_windows(&me));
        }
    });
    let _ = app.emit("note-windows", note_windows(app));
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if !w.is_visible().unwrap_or(true) {
            reveal(&w);
        }
    });
}

/// 哪些笔记开着单独的编辑窗口（主窗口里这些笔记不再就地编辑，免得两边互相覆盖）
pub fn note_windows(app: &AppHandle) -> Vec<i64> {
    app.webview_windows()
        .keys()
        .filter_map(|l| l.strip_prefix("note-")?.parse().ok())
        .collect()
}

/// 打开桌面组件：挂在桌面上，压在所有窗口下面（见 desk.rs）。一直留着，关掉组件时才销毁
pub fn open_widget(app: &AppHandle) {
    if app.get_webview_window("widget").is_some() {
        return;
    }
    let r = builder(app, "widget", "widget.html")
        .title("叮咪桌面组件")
        .inner_size(WIDGET_W + WIDGET_SIDE * 2.0, 400.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .build();
    let Some(w) = made(app, false, r) else {
        return;
    };
    crate::desk::pin(&w);
    // 网页画好后自己摆好、显示（widget.ts 调 widget_show）；万一网页出错，3 秒后也显示出来
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(3)).await;
        if !w.is_visible().unwrap_or(true) {
            let tint = if crate::sys::dark_mode() { [35, 30, 26, 0xc0] } else { [255, 252, 248, 0xb8] };
            widget_look(&app, &w, tint);
            crate::desk::show(&w);
        }
    });
}

pub fn close_widget(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("widget") {
        WIDGET_GLASS.store(false, Ordering::Relaxed);
        let _ = w.destroy();
    }
}

/// 移动位置后记住卡片在哪（reset 时放回右上角，组件没开着的话下次打开时在右上角）
pub fn widget_place(app: &AppHandle, reset: bool) {
    let w = app.get_webview_window("widget");
    if reset {
        {
            let st = app.state::<AppState>();
            let c = st.db.lock().unwrap();
            let _ = db::kv_del(&c, WIDGET_KV);
        }
        if let Some(w) = w {
            widget_move(&w, widget_home(app));
        }
    } else if let Some((w, p)) = w.and_then(|w| w.outer_position().ok().map(|p| (w, p))) {
        let (side, top) = *WIDGET_PAD.lock().unwrap();
        let sf = w.scale_factor().unwrap_or(1.0);
        let card = format!("{},{}", p.x + (side * sf).round() as i32, p.y + (top * sf).round() as i32);
        let st = app.state::<AppState>();
        let c = st.db.lock().unwrap();
        let _ = db::kv_set(&c, WIDGET_KV, &card);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_size_fits_small_screens() {
        // 1920 × 1040 的工作区：一半宽是 960，用下限；三分之二高是 686
        assert_eq!(fit_size((1920.0, 1040.0)), (1040.0, 686.0));
        // 4K 放大 150%（2560 × 1392）：到上限
        assert_eq!(fit_size((2560.0, 1392.0)), (MAIN_MAX_W, MAIN_MAX_H));
        // 1080p 放大 125%（1536 × 824）：下限 1040 × 680
        assert_eq!(fit_size((1536.0, 824.0)), (MAIN_W, MAIN_H));
        // 1366 × 768 放大 125% 后只剩 1093 × 584：不超过九成，但不小于最小大小
        let (w, h) = fit_size((1093.0, 584.0));
        assert!((MAIN_MIN_W..MAIN_W).contains(&w));
        assert!((MAIN_MIN_H..MAIN_H).contains(&h));
        // 再小也不小于最小大小
        assert_eq!(fit_size((800.0, 500.0)), (MAIN_MIN_W, MAIN_MIN_H));
    }

    #[test]
    fn main_rect_parses() {
        assert_eq!(parse_rect("-8,20,1040,680"), Some((-8, 20, 1040, 680)));
        assert_eq!(parse_rect(" 10, 20 , 30,40"), Some((10, 20, 30, 40)));
        assert_eq!(parse_rect("10,20,0,40"), None);
        assert_eq!(parse_rect("10,20,30"), None);
        assert_eq!(parse_rect("a,b,c,d"), None);
    }
}
