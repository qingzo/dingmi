mod ai;
mod allow;
mod clock;
mod db;
mod desk;
mod diag;
mod export;
mod launcher;
mod links;
mod lunar;
mod onboard;
mod recur;
mod restore;
mod scheduler;
mod settings;
mod sync;
mod sys;
mod terms;
mod update;
mod win;

use chrono::{Datelike, Duration, Local};
use db::{Note, Reminder, Trash, ST_SEEN, ST_SHOW, ST_WAIT};
use rusqlite::Connection;
use serde_json::{json, Value};
use settings::Settings;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewWindow, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

pub struct AppState {
    pub db: Mutex<Connection>,
    pub settings: Mutex<Settings>,
    pub focus_manual: AtomicBool,
    pub focus_auto: AtomicBool,
    pub shortcuts: Mutex<HashMap<u32, &'static str>>,
    pub nav: Mutex<Option<Value>>,
    pub tray_focus: Mutex<Option<CheckMenuItem<Wry>>>,
    /// 托盘菜单里的同步状态和「立即同步」
    pub tray_sync: Mutex<Option<(MenuItem<Wry>, MenuItem<Wry>)>>,
    /// 托盘菜单里的「检查更新 / 重启更新到 x」
    pub tray_update: Mutex<Option<MenuItem<Wry>>>,
    /// 托盘提示上一次的内容，没变就不重设
    pub tray_tip: Mutex<String>,
    /// 托盘上现在是哪张图（状态、任务栏深浅、尺寸），没变就不重设
    pub tray_icon: Mutex<(usize, usize, usize)>,
    /// 按快捷框的快捷键时从别的程序里带过来的选中文字，快捷框打开后取走
    pub flash_text: Mutex<Option<String>>,
    pub last_ring: Mutex<Instant>,
    pub data_dir: PathBuf,
    /// 这台电脑的设备 ID，指定设备提醒时用
    pub device: String,
    pub sync: sync::SyncState,
    /// 快捷键注册失败或被自动改过时，给主窗口显示的提示
    pub hotkey_notice: Mutex<Option<String>>,
    /// 上次专注结束时一起放出来的提醒（提醒卡片把它们列成一张），处理掉一条就去掉一条
    pub focus_batch: Mutex<Vec<i64>>,
}

impl AppState {
    pub fn focus_active(&self) -> bool {
        self.focus_manual.load(Ordering::Relaxed) || self.focus_auto.load(Ordering::Relaxed)
    }
}

type Res<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// 数据变了：界面刷新，开启了同步的话也叫上传任务看一眼
pub fn notify_data(app: &AppHandle) {
    let _ = app.emit("data-changed", ());
    app.state::<AppState>().sync.poke();
}

pub fn notify_queue(app: &AppHandle) {
    let _ = app.emit("queue-changed", ());
    update_tray(app);
}

/// 调试用：调试版设置了 DINGMI_DATA_DIR 时用这个目录放数据（密钥也分开放），
/// 并且不限制只能开一个，可以和正在用的叮咪同时运行
pub fn dev_data_dir() -> Option<PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var("DINGMI_DATA_DIR").ok().filter(|d| !d.trim().is_empty()).map(PathBuf::from)
}

/// 别的电脑改了共用设置：重新读一遍，告诉各个窗口
pub fn reload_settings(app: &AppHandle) {
    let st = app.state::<AppState>();
    let Some(s) = settings::load(&st.db.lock().unwrap()) else { return };
    let changed = {
        let mut cur = st.settings.lock().unwrap();
        let changed = serde_json::to_string(&*cur).ok() != serde_json::to_string(&s).ok();
        *cur = s.clone();
        changed
    };
    if changed {
        let _ = app.emit("settings-changed", &s);
    }
}

/// "今天 15:00" 说成 "15:00"，明天的加"明天"，再往后写日期
fn when_label(at: &str) -> String {
    let today = Local::now().date_naive();
    let Some(d) = db::parse_dt(at) else { return at.to_string() };
    let hm = d.format("%H:%M");
    match (d.date() - today).num_days() {
        0 => hm.to_string(),
        1 => format!("明天 {hm}"),
        _ => format!("{}月{}日 {hm}", d.month(), d.day()),
    }
}

/// 托盘：鼠标停上去显示下一条提醒和同步状态；菜单里的同步状态跟着变
pub fn update_tray(app: &AppHandle) {
    let st = app.state::<AppState>();
    let sync_line = sync::tray_line(app);
    let next = db::next_reminder(&st.db.lock().unwrap(), &db::now_min(), &st.device);
    let mut tip = match next {
        Some((at, title)) => {
            let mut t: String = title.chars().take(24).collect();
            if title.chars().count() > 24 {
                t.push('…');
            }
            format!("叮咪 {} · 下一条：{} {t}", app.package_info().version, when_label(&at))
        }
        None => format!("叮咪 {} · 接下来没有提醒", app.package_info().version),
    };
    if dev_data_dir().is_some() {
        tip.insert_str(0, "【测试】");
    }
    if let Some(l) = &sync_line {
        tip.push('\n');
        tip.push_str(l);
    }
    {
        let mut last = st.tray_tip.lock().unwrap();
        if *last != tip {
            if let Some(t) = app.tray_by_id("dingmi") {
                let _ = t.set_tooltip(Some(&tip));
            }
            *last = tip;
        }
    }
    let items = st.tray_sync.lock().unwrap();
    if let Some((status, now)) = items.as_ref() {
        let _ = status.set_text(sync_line.as_deref().unwrap_or("同步：没有开启"));
        let _ = now.set_enabled(sync_line.is_some());
    }
    let upd = st.tray_update.lock().unwrap();
    if let Some(item) = upd.as_ref() {
        let _ = item.set_text(update::tray_text());
    }
    drop(upd);
    set_tray_icon(app);
}

/// 托盘图标的 4 种状态（U3），每种按浅色 / 深色任务栏、16 / 20 / 24 / 32 像素各一张，scripts/build-icons.mjs 生成
macro_rules! tray_pngs {
    ($s:literal, $t:literal) => {
        [
            include_bytes!(concat!("../icons/tray/", $s, "-", $t, "-16.png")),
            include_bytes!(concat!("../icons/tray/", $s, "-", $t, "-20.png")),
            include_bytes!(concat!("../icons/tray/", $s, "-", $t, "-24.png")),
            include_bytes!(concat!("../icons/tray/", $s, "-", $t, "-32.png")),
        ]
    };
}
static TRAY_PNGS: [[[&[u8]; 4]; 2]; 4] = [
    [tray_pngs!("normal", "light"), tray_pngs!("normal", "dark")],
    [tray_pngs!("remind", "light"), tray_pngs!("remind", "dark")],
    [tray_pngs!("focus", "light"), tray_pngs!("focus", "dark")],
    [tray_pngs!("bad", "light"), tray_pngs!("bad", "dark")],
];

/// 托盘图标跟着状态换（需求-0.2 第 7.2 节）：同步出问题 > 有没处理的提醒 > 专注中 > 平时
fn tray_icon_state(app: &AppHandle) -> usize {
    let st = app.state::<AppState>();
    if sync::has_problem(app) {
        3
    } else if db::queue(&st.db.lock().unwrap()).is_ok_and(|q| !q.is_empty()) {
        1
    } else if st.focus_active() {
        2
    } else {
        0
    }
}

fn set_tray_icon(app: &AppHandle) {
    let px = sys::tray_icon_px();
    let want = (tray_icon_state(app), sys::taskbar_dark() as usize, [16, 20, 24, 32].iter().position(|&s| s == px).unwrap_or(3));
    let st = app.state::<AppState>();
    let mut cur = st.tray_icon.lock().unwrap();
    if *cur == want {
        return;
    }
    let Some(tray) = app.tray_by_id("dingmi") else { return };
    if let Ok(img) = tauri::image::Image::from_bytes(TRAY_PNGS[want.0][want.1][want.2]) {
        if tray.set_icon(Some(img)).is_ok() {
            *cur = want;
        }
    }
}

// ---------- 专注模式 ----------

pub fn focus_info(app: &AppHandle) -> Value {
    let st = app.state::<AppState>();
    let held = db::held(&st.db.lock().unwrap()).map(|v| v.len()).unwrap_or(0);
    json!({
        "manual": st.focus_manual.load(Ordering::Relaxed),
        "auto": st.focus_auto.load(Ordering::Relaxed),
        "active": st.focus_active(),
        "held": held,
    })
}

/// 专注状态变了：结束专注时把攒着的提醒一起放出来。
pub fn focus_changed(app: &AppHandle, was: bool, now: bool) {
    let st = app.state::<AppState>();
    if was && !now {
        let (ids, released) = {
            let c = st.db.lock().unwrap();
            let ids: Vec<i64> = db::held(&c).unwrap_or_default().iter().map(|r| r.id).collect();
            (ids, db::release_held(&c).unwrap_or(0))
        };
        *st.focus_batch.lock().unwrap() = ids;
        if released > 0 {
            win::open_remind(app);
            notify_queue(app);
            notify_data(app);
        }
    }
    if let Some(item) = st.tray_focus.lock().unwrap().as_ref() {
        let _ = item.set_checked(now);
    }
    let _ = app.emit("focus-changed", focus_info(app));
    update_tray(app);
}

fn set_focus_manual(app: &AppHandle, on: bool) {
    let st = app.state::<AppState>();
    let was = st.focus_active();
    st.focus_manual.store(on, Ordering::Relaxed);
    focus_changed(app, was, st.focus_active());
}

// ---------- 快捷键 ----------

fn hotkey_label(name: &str) -> &'static str {
    match name {
        "flash" => "快捷框",
        "qnote" => "快速记笔记",
        "main" => "打开主窗口",
        _ => "开关专注模式",
    }
}

fn apply_hotkeys(app: &AppHandle, s: &Settings) -> Res<()> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let mut map = HashMap::new();
    let mut errs = vec![];
    for (name, acc) in [
        ("flash", &s.hk_flash),
        ("qnote", &s.hk_qnote),
        ("main", &s.hk_main),
        ("focus", &s.hk_focus),
    ] {
        let acc = acc.trim();
        if acc.is_empty() {
            continue;
        }
        match acc.parse::<Shortcut>() {
            Ok(sc) => match gs.register(sc) {
                Ok(_) => {
                    map.insert(sc.id(), name);
                }
                Err(_) => errs.push(format!(
                    "{}（{}）被其他软件占用了，换一个试试",
                    hotkey_label(name),
                    acc
                )),
            },
            Err(_) => errs.push(format!("{}（{}）不是有效的快捷键", hotkey_label(name), acc)),
        }
    }
    let st = app.state::<AppState>();
    *st.shortcuts.lock().unwrap() = map;
    if errs.is_empty() {
        *st.hotkey_notice.lock().unwrap() = None;
        Ok(())
    } else {
        let e = errs.join("；");
        diag::log("快捷键", &e);
        *st.hotkey_notice.lock().unwrap() = Some(e.clone());
        Err(e)
    }
}

fn hotkey_mut<'a>(s: &'a mut Settings, name: &str) -> &'a mut String {
    match name {
        "flash" => &mut s.hk_flash,
        "qnote" => &mut s.hk_qnote,
        "main" => &mut s.hk_main,
        _ => &mut s.hk_focus,
    }
}

/// 第一次运行时，默认快捷键常被 uTools、PowerToys 之类占用：被占的那几个自动换成能用的，再提示一句
fn first_run_hotkeys(app: &AppHandle, s: &mut Settings) {
    if apply_hotkeys(app, s).is_ok() {
        return;
    }
    let st = app.state::<AppState>();
    let failed = |name: &str| st.hotkey_notice.lock().unwrap().as_deref().is_some_and(|e| e.contains(hotkey_label(name)));
    let mut changed = vec![];
    for (name, alts) in [
        ("flash", ["Ctrl+Alt+Space", "Ctrl+Shift+Space", "Alt+Q"]),
        ("qnote", ["Ctrl+Alt+J", "Alt+Shift+N", "Ctrl+Alt+B"]),
        ("main", ["Ctrl+Alt+K", "Alt+Shift+H", "Ctrl+Alt+Y"]),
        ("focus", ["Ctrl+Alt+G", "Alt+Shift+F", "Ctrl+Alt+U"]),
    ] {
        if !failed(name) {
            continue;
        }
        let old = hotkey_mut(s, name).clone();
        let found = alts.into_iter().find(|alt| {
            *hotkey_mut(s, name) = alt.to_string();
            let _ = apply_hotkeys(app, s);
            !failed(name)
        });
        match found {
            Some(alt) => {
                if name == "flash" {
                    onboard::flash_moved(&old);
                }
                changed.push(format!("{} {} → {}", hotkey_label(name), old.replace('+', " + "), alt.replace('+', " + ")))
            }
            None => *hotkey_mut(s, name) = old,
        }
    }
    // 按最后定下来的重新登记一遍；还有换不掉的，提示里照旧写着
    let rest = apply_hotkeys(app, s).err();
    if changed.is_empty() {
        return;
    }
    let _ = settings::save_local(&st.db.lock().unwrap(), s);
    *st.settings.lock().unwrap() = s.clone();
    let mut notice = format!(
        "这几个默认快捷键被其他软件占用了，已经自动换成：{}。可以在「设置 → 快捷键」里改",
        changed.join("；")
    );
    if let Some(e) = rest {
        notice.push_str(&format!("。另外，{e}"));
    }
    *st.hotkey_notice.lock().unwrap() = Some(notice);
}

fn on_hotkey(app: &AppHandle, sc: &Shortcut) {
    let name = app
        .state::<AppState>()
        .shortcuts
        .lock()
        .unwrap()
        .get(&sc.id())
        .copied();
    match name {
        Some("flash") => open_flash(app),
        Some("qnote") => win::toggle_qnote(app),
        Some("main") => match app.get_webview_window("main") {
            Some(w) if w.is_focused().unwrap_or(false) => {
                let _ = w.close();
            }
            _ => win::open_main(app),
        },
        Some("focus") => {
            let on = !app.state::<AppState>().focus_manual.load(Ordering::Relaxed);
            set_focus_manual(app, on);
        }
        _ => {}
    }
}

/// 快捷框的快捷键：先把别的程序里选中的文字复制过来（不会改动剪贴板原来的内容），再打开快捷框。
/// 那个程序收到"复制"就打开，复制慢的话选中的文字随后再补进输入框（flash-text）
fn open_flash(app: &AppHandle) {
    let visible = app.get_webview_window("flash").and_then(|w| w.is_visible().ok()).unwrap_or(false);
    let grab = app.state::<AppState>().settings.lock().unwrap().flash_selection;
    if visible || !grab {
        win::toggle_flash(app);
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        if let Some(text) = sys::grab_selection(|| win::toggle_flash(&app)) {
            if !text.trim().is_empty() {
                onboard::tick(&app, "selection");
            }
            let _ = app.emit_to("flash", "flash-text", text);
        }
    });
}

fn apply_autostart(on: bool) {
    if cfg!(debug_assertions) {
        return;
    }
    sys::set_autostart(on);
}

// ---------- 命令：提醒 ----------

#[tauri::command]
fn list_reminders(st: tauri::State<AppState>) -> Res<Vec<Reminder>> {
    db::list_reminders(&st.db.lock().unwrap()).map_err(err)
}

#[tauri::command]
fn save_reminder(app: AppHandle, window: WebviewWindow, st: tauri::State<AppState>, reminder: Reminder) -> Res<Reminder> {
    let mut r = reminder;
    r.title = r.title.trim().to_string();
    if r.title.is_empty() {
        return Err("先写上要提醒什么".into());
    }
    let due = db::parse_dt(&r.due_at).ok_or("时间格式不对")?;
    let now = Local::now().naive_local();
    r.due_at = recur::normalize(&r.repeat, due, now)
        .format(db::FMT)
        .to_string();
    r.snoozed_until = None;
    let c = st.db.lock().unwrap();
    let old = if r.id > 0 { db::get_reminder(&c, r.id).map_err(err)? } else { None };
    let keep_done = !r.repeat.is_recurring()
        && old.as_ref().map(|o| o.done).unwrap_or(false)
        && db::parse_dt(&r.due_at).map(|d| d <= now).unwrap_or(false);
    if let Some(o) = &old {
        r.uid = o.uid.clone();
        r.snooze_count = o.snooze_count;
        r.last_done_at = o.last_done_at.clone();
        r.created_at = o.created_at.clone();
        // 说得含糊先定的时间，改过就不再算含糊
        if r.due_at != o.due_at {
            r.vague = false;
        }
    }
    r.done = keep_done;
    r.completed_at = if keep_done { old.and_then(|o| o.completed_at) } else { None };
    r.state = ST_WAIT;
    let id = db::upsert_reminder(&c, &r).map_err(err)?;
    let saved = db::get_reminder(&c, id).map_err(err)?.ok_or("保存失败")?;
    drop(c);
    notify_data(&app);
    notify_queue(&app);
    if window.label() == "flash" {
        onboard::tried(&app, "reminder", &saved.title, Some(saved.fire_at()));
    }
    Ok(saved)
}

#[tauri::command]
fn delete_reminder(app: AppHandle, st: tauri::State<AppState>, id: i64) -> Res<()> {
    db::delete_reminder(&st.db.lock().unwrap(), id).map_err(err)?;
    notify_data(&app);
    notify_queue(&app);
    Ok(())
}

fn complete(r: &mut Reminder) {
    let now = Local::now().naive_local();
    let now_s = now.format(db::FMT).to_string();
    r.snoozed_until = None;
    r.state = ST_WAIT;
    if r.repeat.is_recurring() {
        r.last_done_at = Some(now_s);
        if let Some(base) = db::parse_dt(&r.due_at) {
            if let Some(next) = recur::next_after(&r.repeat, base, base.max(now)) {
                r.due_at = next.format(db::FMT).to_string();
            }
        }
    } else {
        r.done = true;
        r.completed_at = Some(now_s);
    }
}

/// 提醒卡片或列表上的操作：done 完成 / snooze 稍后 / tomorrow 明天再说 / snooze_to 推迟到 until / undo 取消完成
#[tauri::command]
fn reminder_action(
    app: AppHandle,
    window: WebviewWindow,
    st: tauri::State<AppState>,
    id: i64,
    action: String,
    until: Option<String>,
) -> Res<Reminder> {
    let snooze_min = st.settings.lock().unwrap().snooze_min.max(1) as i64;
    st.focus_batch.lock().unwrap().retain(|x| *x != id);
    let c = st.db.lock().unwrap();
    let mut r = db::get_reminder(&c, id).map_err(err)?.ok_or("这条提醒已经不在了")?;
    let now = Local::now().naive_local();
    match action.as_str() {
        "done" => complete(&mut r),
        "snooze" => {
            r.snoozed_until = Some((now + Duration::minutes(snooze_min)).format(db::FMT).to_string());
            r.state = ST_WAIT;
            r.snooze_count += 1;
        }
        "tomorrow" => {
            let t = db::parse_dt(r.fire_at()).unwrap_or(now).time();
            let d = now.date().succ_opt().unwrap_or(now.date());
            r.snoozed_until = Some(d.and_time(t).format(db::FMT).to_string());
            r.state = ST_WAIT;
            r.snooze_count += 1;
        }
        "snooze_to" => {
            let at = until.as_deref().and_then(db::parse_dt).ok_or("时间格式不对")?;
            if at <= now {
                return Err("选一个以后的时间".into());
            }
            r.snoozed_until = Some(at.format(db::FMT).to_string());
            r.state = ST_WAIT;
            r.snooze_count += 1;
        }
        "undo" => {
            if !r.repeat.is_recurring() {
                r.done = false;
                r.completed_at = None;
                let past = db::parse_dt(r.fire_at()).map(|d| d <= now).unwrap_or(true);
                r.state = if past { ST_SEEN } else { ST_WAIT };
            }
        }
        _ => return Err("不认识的操作".into()),
    }
    db::upsert_reminder(&c, &r).map_err(err)?;
    // 从笔记待办设的提醒：笔记里那一行跟着勾上 / 取消
    let tick = match action.as_str() {
        "done" if !r.repeat.is_recurring() => Some(true),
        "undo" if !r.repeat.is_recurring() => Some(false),
        _ => None,
    };
    if let (Some(done), Some(note), Some(todo)) = (tick, &r.note_uid, &r.todo) {
        db::tick_todo(&c, note, todo, done).map_err(err)?;
    }
    let saved = db::get_reminder(&c, id).map_err(err)?.ok_or("保存失败")?;
    drop(c);
    notify_data(&app);
    notify_queue(&app);
    // 上手清单：在提醒卡片上处理了一条（完成、稍后都算）
    if window.label() == "remind" && action != "undo" {
        onboard::tick(&app, "card");
    }
    Ok(saved)
}

#[tauri::command]
fn get_queue(st: tauri::State<AppState>) -> Res<Vec<Reminder>> {
    db::queue(&st.db.lock().unwrap()).map_err(err)
}

#[tauri::command]
fn held_reminders(st: tauri::State<AppState>) -> Res<Vec<Reminder>> {
    db::held(&st.db.lock().unwrap()).map_err(err)
}

/// 上次专注结束时一起放出来、还没处理的提醒
#[tauri::command]
fn focus_batch(st: tauri::State<AppState>) -> Vec<i64> {
    st.focus_batch.lock().unwrap().clone()
}

/// 这条提醒最后是不是在别的电脑上改的：是的话回那台电脑的名字（不知道名字时是空字符串），提醒卡片写「公司电脑上已经完成了」
#[tauri::command]
fn reminder_changed_on(st: tauri::State<AppState>, id: i64) -> Res<Option<String>> {
    db::reminder_changed_on(&st.db.lock().unwrap(), id, &st.device).map_err(err)
}

// ---------- 命令：笔记 ----------

#[tauri::command]
fn list_notes(st: tauri::State<AppState>) -> Res<Vec<Note>> {
    db::list_notes(&st.db.lock().unwrap()).map_err(err)
}

#[tauri::command]
fn save_note(app: AppHandle, window: WebviewWindow, st: tauri::State<AppState>, note: Note) -> Res<Note> {
    let mut n = note;
    let new = n.id <= 0;
    n.title = n.title.trim().to_string();
    if n.title.is_empty() {
        let first = n
            .body
            .lines()
            .map(|l| l.trim().trim_start_matches(['#', '>', '-', ' ']).trim())
            .find(|l| !l.is_empty())
            .unwrap_or("");
        let mut t: String = first.chars().take(20).collect();
        if first.chars().count() > 20 {
            t.push('…');
        }
        n.title = if t.is_empty() { "无标题".into() } else { t };
    }
    let c = st.db.lock().unwrap();
    let id = db::upsert_note(&c, &n).map_err(err)?;
    let saved = db::get_note(&c, id).map_err(err)?.ok_or("保存失败")?;
    drop(c);
    notify_data(&app);
    if new {
        onboard::tick(&app, "note");
    }
    if window.label() == "flash" {
        onboard::tried(&app, "note", &saved.title, None);
    }
    Ok(saved)
}

/// 置顶 / 取消置顶一条笔记
#[tauri::command]
fn pin_note(app: AppHandle, st: tauri::State<AppState>, id: i64, on: bool) -> Res<()> {
    db::pin_note(&st.db.lock().unwrap(), id, on).map_err(err)?;
    notify_data(&app);
    Ok(())
}

#[tauri::command]
fn get_note(st: tauri::State<AppState>, id: i64) -> Res<Option<Note>> {
    db::get_note(&st.db.lock().unwrap(), id).map_err(err)
}

/// 在单独的窗口里编辑一条笔记（建窗口的命令要是 async，见上面 save_settings 的说明）
#[tauri::command]
async fn open_note_window(app: AppHandle, id: i64) -> Res<()> {
    win::open_note(&app, id);
    Ok(())
}

#[tauri::command]
fn note_windows(app: AppHandle) -> Vec<i64> {
    win::note_windows(&app)
}

#[tauri::command]
fn delete_note(app: AppHandle, st: tauri::State<AppState>, id: i64) -> Res<()> {
    db::delete_note(&st.db.lock().unwrap(), id).map_err(err)?;
    notify_data(&app);
    Ok(())
}

/// 删掉了一个分类：用到它的提醒和笔记（包括回收站里的）变成没有分类，返回改了几条
#[tauri::command]
fn clear_tag(app: AppHandle, st: tauri::State<AppState>, tag: String, notes: Option<bool>) -> Res<usize> {
    let n = db::clear_tag(&st.db.lock().unwrap(), &tag, notes.unwrap_or(false)).map_err(err)?;
    if n > 0 {
        notify_data(&app);
    }
    Ok(n)
}

// ---------- 命令：回收站 ----------

#[tauri::command]
fn list_trash(st: tauri::State<AppState>) -> Res<Trash> {
    db::list_trash(&st.db.lock().unwrap()).map_err(err)
}

/// kind：reminder / note；action：restore 恢复 / purge 彻底删除
#[tauri::command]
fn trash_action(app: AppHandle, st: tauri::State<AppState>, kind: String, id: i64, action: String) -> Res<()> {
    let c = st.db.lock().unwrap();
    let r = match (kind.as_str(), action.as_str()) {
        ("reminder", "restore") => db::restore_reminder(&c, id),
        ("reminder", "purge") => db::purge_reminder(&c, id),
        ("note", "restore") => db::restore_note(&c, id),
        ("note", "purge") => db::purge_note(&c, id),
        _ => return Err("不认识的操作".into()),
    };
    r.map_err(err)?;
    drop(c);
    notify_data(&app);
    notify_queue(&app);
    Ok(())
}

#[tauri::command]
fn empty_trash(app: AppHandle, st: tauri::State<AppState>) -> Res<usize> {
    let n = db::purge_trash(&st.db.lock().unwrap(), None).map_err(err)?;
    notify_data(&app);
    Ok(n)
}

// ---------- 命令：手机收件箱、图片、链接 ----------

#[tauri::command]
fn inbox_list(st: tauri::State<AppState>) -> Res<Vec<db::InboxEntry>> {
    db::list_inbox(&st.db.lock().unwrap()).map_err(err)
}

/// 存成笔记：里面的网址先抓一下标题，图片归到这条笔记下。返回新笔记的 id
#[tauri::command]
async fn inbox_to_note(app: AppHandle, id: i64) -> Res<i64> {
    let entry = {
        let st = app.state::<AppState>();
        let c = st.db.lock().unwrap();
        db::get_inbox(&c, id).map_err(err)?
    }
    .ok_or("这一条已经不在了")?;
    let mut titles = HashMap::new();
    for url in links::find_urls(&entry.text).into_iter().take(3) {
        if let Some(t) = links::fetch_title(&url).await {
            titles.insert(url, t);
        }
    }
    let st = app.state::<AppState>();
    let c = st.db.lock().unwrap();
    let files: HashMap<String, String> = db::list_attachments(&c)
        .map_err(err)?
        .into_iter()
        .filter(|a| !a.mime.starts_with("image/") && entry.images.contains(&a.file))
        .map(|a| (a.file, a.name))
        .collect();
    let (title, body) = links::compose_note(&entry, &titles, &files);
    let nid = db::upsert_note(&c, &Note { title, body, ..Default::default() }).map_err(err)?;
    let uid = db::get_note(&c, nid).map_err(err)?.map(|n| n.uid).unwrap_or_default();
    db::attach_to_note(&c, &entry.images, &uid).map_err(err)?;
    db::set_inbox_status(&c, id, db::INBOX_DONE).map_err(err)?;
    drop(c);
    notify_data(&app);
    Ok(nid)
}

/// 用「整理一下」处理完了（随手记那边已经存好）
#[tauri::command]
fn inbox_done(app: AppHandle, st: tauri::State<AppState>, id: i64) -> Res<()> {
    db::set_inbox_status(&st.db.lock().unwrap(), id, db::INBOX_DONE).map_err(err)?;
    notify_data(&app);
    Ok(())
}

#[tauri::command]
fn inbox_delete(app: AppHandle, st: tauri::State<AppState>, id: i64) -> Res<()> {
    db::delete_inbox(&st.db.lock().unwrap(), id).map_err(err)?;
    notify_data(&app);
    Ok(())
}

/// 笔记里粘贴、拖入图片或文件：原始字节直接放在请求体里，类型、原文件名、所属笔记放在请求头
/// x-mime / x-name（URL 编码过，请求头里不能直接放中文）/ x-note
#[tauri::command]
fn save_attachment(st: tauri::State<AppState>, request: tauri::ipc::Request<'_>) -> Res<String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("没收到文件".into());
    };
    let header = |k: &str| request.headers().get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
    let mime = header("x-mime").filter(|m| !m.is_empty()).unwrap_or_else(|| "application/octet-stream".into());
    let name = header("x-name").map(|n| links::percent_decode(&n)).unwrap_or_default();
    db::save_file(&st.db.lock().unwrap(), &st.data_dir, &mime, &name, bytes, header("x-note").filter(|n| !n.is_empty()).as_deref(), None)
}

/// 快速记笔记：先存了附件、后存的笔记，存好笔记再把这些附件归到它下面
#[tauri::command]
fn attach_files(app: AppHandle, st: tauri::State<AppState>, files: Vec<String>, note_uid: String) -> Res<()> {
    db::attach_to_note(&st.db.lock().unwrap(), &files, &note_uid).map_err(err)?;
    notify_data(&app);
    Ok(())
}

#[tauri::command]
fn list_attachments(st: tauri::State<AppState>) -> Res<Vec<db::AttachmentInfo>> {
    db::list_attachments(&st.db.lock().unwrap()).map_err(err)
}

#[tauri::command]
fn attachment_dir(st: tauri::State<AppState>) -> String {
    db::attachment_dir(&st.data_dir).display().to_string()
}

#[tauri::command]
fn open_url(url: String) -> Res<()> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("只能打开网址".into());
    }
    links::shell_open(&url)
}

/// 打开附件：图片用看图程序，文件用默认程序。文件还没下载的先从服务器下载（文件不自动下载，见需求文档 6.12）。
/// 文件复制一份用原文件名打开，看到的不是一串编号。
/// 只直接打开图片、PDF、文本、不带宏的 Office 文档和常见的音视频（links::safe_to_open）；别的（程序、脚本、快捷方式、
/// 压缩包、带宏的文档……）不替你打开，只在资源管理器里选中，回 true，界面提示一句
#[tauri::command]
async fn open_attachment(app: AppHandle, file: String) -> Res<bool> {
    let (path, info) = {
        let st = app.state::<AppState>();
        let path = db::att_path(&db::attachment_dir(&st.data_dir), &file).ok_or("找不到这个附件")?;
        let info = db::get_attachment(&st.db.lock().unwrap(), &file).map_err(err)?;
        (path, info)
    };
    if !path.exists() {
        let info = info.as_ref().ok_or("找不到这个附件")?;
        sync::download_attachment(&app, &info.uid, &file).await?;
        notify_data(&app);
    }
    // 用原文件名另存一份打开：编号（只取字母数字 -）当子文件夹，原文件名要是正经的文件名
    let named = info
        .filter(|i| !i.mime.starts_with("image/") && db::valid_att_uid(&i.uid))
        .and_then(|i| {
            let name = links::copy_name(&i.name)?;
            let dir = std::env::temp_dir().join("dingmi").join(i.uid.chars().take(13).collect::<String>());
            Some((dir.clone(), dir.join(name)))
        });
    let web = db::from_web(&path);
    let target = match named {
        Some((dir, copy)) => {
            std::fs::create_dir_all(&dir).map_err(err)?;
            std::fs::copy(&path, &copy).map_err(err)?;
            if web {
                db::mark_from_web(&copy);
            }
            copy
        }
        None => path.clone(),
    };
    // 存的文件和原文件名两个扩展名都要在白名单里（别的电脑传来的这两个是分开的）
    let ok = links::safe_to_open(&file) && links::safe_to_open(&target.display().to_string());
    let target = target.display().to_string();
    if !ok {
        links::shell_reveal(&target)?;
        return Ok(true);
    }
    links::shell_open(&target)?;
    Ok(false)
}

/// 附件另存为（笔记里「看大图」的另存为）：弹出「另存为」，默认用原文件名（图片用 图片.<扩展名>），存好回路径，取消时为 null。
/// 还没下载的先下载
#[tauri::command]
async fn save_attachment_as(app: AppHandle, window: tauri::WebviewWindow, file: String) -> Res<Option<String>> {
    let (path, info) = {
        let st = app.state::<AppState>();
        let path = db::att_path(&db::attachment_dir(&st.data_dir), &file).ok_or("找不到这个附件")?;
        let info = db::get_attachment(&st.db.lock().unwrap(), &file).map_err(err)?;
        (path, info)
    };
    if !path.exists() {
        let info = info.as_ref().ok_or("找不到这个附件")?;
        sync::download_attachment(&app, &info.uid, &file).await?;
        notify_data(&app);
    }
    let ext = file.rsplit_once('.').map(|(_, e)| e.to_string()).unwrap_or_default();
    // 图片默认存到「图片」，别的文件（笔记里的文件卡片「另存为…」）默认存到「下载」
    let image = info.as_ref().is_none_or(|i| i.mime.starts_with("image/"));
    let name = info.map(|i| i.name).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("{}.{ext}", if image { "图片" } else { "附件" }));
    let start = if image { app.path().picture_dir() } else { app.path().download_dir() }.or_else(|_| app.path().document_dir()).ok();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        use tauri_plugin_dialog::DialogExt;
        let mut d = app.dialog().file().set_title("存到哪").set_file_name(name).set_parent(&window);
        if !ext.is_empty() {
            d = d.add_filter(ext.to_uppercase(), &[ext.as_str()]);
        }
        if let Some(dir) = start {
            d = d.set_directory(dir);
        }
        d.blocking_save_file().and_then(|f| f.into_path().ok())
    })
    .await
    .map_err(err)?;
    let Some(to) = picked else {
        return Ok(None);
    };
    std::fs::copy(&path, &to).map_err(|e| format!("存不下：{e}"))?;
    Ok(Some(to.display().to_string()))
}

/// 同步过的电脑（指定设备提醒时选）
#[tauri::command]
fn list_devices(st: tauri::State<AppState>) -> Res<Vec<db::Device>> {
    db::list_devices(&st.db.lock().unwrap(), &st.device).map_err(err)
}

// ---------- 命令：导出全部、从导出包恢复（需求-0.1 第 4 节） ----------

/// 默认导出到哪：文档\叮咪导出\叮咪导出_<日期>_<时分>.zip
#[tauri::command]
fn export_default(app: AppHandle) -> Res<String> {
    let docs = app.path().document_dir().map_err(err)?;
    let p = export::default_path(&docs, Local::now().naive_local());
    // 一分钟里导了两次：后一个加 (2)，不盖掉前一个
    let p = (1..100)
        .map(|n| if n == 1 { p.clone() } else { p.with_file_name(format!("{} ({n}).zip", p.file_stem().unwrap_or_default().to_string_lossy())) })
        .find(|p| !p.exists())
        .unwrap_or(p);
    let p = p.display().to_string();
    allow::grant(&p);
    Ok(p)
}

/// 选文件的框放在哪个窗口上面：开着引导就放在引导上（引导里也能选同步文件夹、导出包），否则主窗口
fn dialog_parent(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("onboard").or_else(|| app.get_webview_window("main"))
}

/// 弹出选文件的框（在主窗口或引导上面）：`save` 为 true 是「另存为」，`path` 是默认的位置。取消时为 None
fn zip_dialog(app: &AppHandle, title: &str, path: &std::path::Path, save: bool) -> Option<PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    let mut d = app.dialog().file().set_title(title).add_filter("叮咪导出包（zip）", &["zip"]);
    // 从 `path` 所在的文件夹开始选；这个文件夹还没建的话，从再上一层开始
    if let Some(dir) = path.ancestors().skip(1).find(|p| p.is_dir()) {
        d = d.set_directory(dir);
    }
    if let Some(name) = path.file_name().filter(|_| save) {
        d = d.set_file_name(name.to_string_lossy());
    }
    if let Some(w) = dialog_parent(app) {
        d = d.set_parent(&w);
    }
    let picked = if save { d.blocking_save_file() } else { d.blocking_pick_file() };
    picked.and_then(|f| f.into_path().ok())
}

/// 换个导出位置：弹出「另存为」，取消时为 null
#[tauri::command]
async fn export_pick(app: AppHandle, path: String) -> Option<String> {
    let mut p = tauri::async_runtime::spawn_blocking(move || zip_dialog(&app, "导出到哪", std::path::Path::new(&path), true))
        .await
        .ok()
        .flatten()?;
    if !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
        let name = format!("{}.zip", p.file_name()?.to_string_lossy());
        p.set_file_name(name);
    }
    let p = p.display().to_string();
    allow::grant(&p);
    Some(p)
}

/// 导出全部：开着同步时先把还没下载到这台电脑的文件下载下来（下载不了的照常导出，在 说明.txt 里列出），
/// 再复制一份数据库、打包。进度用 export-progress 事件告诉主窗口
#[tauri::command]
async fn export_all(app: AppHandle, path: String) -> Res<export::Exported> {
    let out = PathBuf::from(path.trim());
    // 只导出到 export_default 给的、或者用户在「另存为」里选的位置
    if out.file_name().is_none() || !out.is_absolute() || !allow::granted(&path) {
        return Err("请选一个保存的位置".into());
    }
    let st = app.state::<AppState>();
    let data_dir = st.data_dir.clone();
    let att_dir = db::attachment_dir(&data_dir);
    let need: Vec<(String, String)> = export::wanted_attachments(&st.db.lock().unwrap())
        .map_err(err)?
        .into_iter()
        .filter(|(_, f)| !att_dir.join(f).exists())
        .collect();
    // 连不上服务器时就不一个个试了，免得等很久
    if !need.is_empty() && sync::sync_status(app.clone()).state == "online" {
        let mut got = false;
        for (i, (uid, file)) in need.iter().enumerate() {
            let _ = app.emit("export-progress", export::Progress { stage: "download", done: i, total: need.len() });
            got |= sync::download_attachment(&app, uid, file).await.is_ok();
        }
        if got {
            notify_data(&app);
        }
    }
    let copy = std::env::temp_dir().join(format!("dingmi-export-{}.db", uuid::Uuid::new_v4().simple()));
    export::copy_db(&st.db.lock().unwrap(), &copy)?;
    let version = app.package_info().version.to_string();
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut progress = |p: export::Progress| {
            let _ = app2.emit("export-progress", p);
        };
        let r = export::pack(&copy, &data_dir, &out, &version, Local::now().naive_local(), &mut progress);
        let _ = std::fs::remove_file(&copy);
        r
    })
    .await
    .map_err(err)?
}

/// 在资源管理器里打开所在文件夹，选中这个文件
#[tauri::command]
fn reveal_path(app: AppHandle, path: String) -> Res<()> {
    check_path(&app, &path, PathUse::Reveal)?;
    links::shell_reveal(&path)
}

// ---------- 命令：导出诊断信息（需求-0.3 第 12 节） ----------

/// 先把要放进诊断包的几样写到临时文件夹，列给用户看（「打开看看」用 open_path）
#[tauri::command]
async fn diag_prepare(app: AppHandle) -> Res<Vec<diag::Part>> {
    tauri::async_runtime::spawn_blocking(move || diag::prepare(&app)).await.map_err(err)?
}

/// 打成 zip 放到桌面，回 zip 的位置（之后「打开所在文件夹」用 reveal_path）
#[tauri::command]
async fn diag_export(app: AppHandle) -> Res<String> {
    let p = tauri::async_runtime::spawn_blocking(move || diag::export(&app).map(|p| p.display().to_string())).await.map_err(err)??;
    allow::grant(&p);
    Ok(p)
}

/// 选一个文件夹（网盘文件夹同步的「别的文件夹」、备份放在哪），取消时为 null
#[tauri::command]
async fn pick_folder(app: AppHandle, title: String) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        use tauri_plugin_dialog::DialogExt;
        let mut d = app.dialog().file().set_title(title);
        if let Some(w) = dialog_parent(&app) {
            d = d.set_parent(&w);
        }
        d.blocking_pick_folder().and_then(|f| f.into_path().ok())
    })
    .await
    .ok()
    .flatten()
    .map(|p| p.display().to_string())
    .inspect(|p| allow::grant(p))
}

/// 存一张图片（恢复卡）：弹出「另存为」，存好回路径，取消时为 null。`data` 是 PNG 的 base64
#[tauri::command]
async fn save_png(app: AppHandle, name: String, data: String) -> Res<Option<String>> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(err)?;
    let start = app.path().picture_dir().or_else(|_| app.path().document_dir()).ok();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        use tauri_plugin_dialog::DialogExt;
        let mut d = app.dialog().file().set_title("存到哪").add_filter("图片（PNG）", &["png"]).set_file_name(name);
        if let Some(dir) = start {
            d = d.set_directory(dir);
        }
        if let Some(w) = app.get_webview_window("main") {
            d = d.set_parent(&w);
        }
        d.blocking_save_file().and_then(|f| f.into_path().ok())
    })
    .await
    .map_err(err)?;
    let Some(mut path) = picked else {
        return Ok(None);
    };
    if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")) {
        path.set_extension("png");
    }
    std::fs::write(&path, bytes).map_err(|e| format!("存不下：{e}"))?;
    Ok(Some(path.display().to_string()))
}

/// 选一个导出包，取消时为 null
#[tauri::command]
async fn restore_pick(app: AppHandle) -> Option<String> {
    let start = app.path().document_dir().map(|d| d.join("叮咪导出").join("x.zip")).unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || zip_dialog(&app, "选择导出包", &start, false))
        .await
        .ok()
        .flatten()
        .map(|p| p.display().to_string())
        .inspect(|p| allow::grant(p))
}

/// 恢复只读用户在对话框里选的那个导出包
fn picked_package(path: &str) -> Res<&Path> {
    if allow::granted(path) {
        Ok(Path::new(path))
    } else {
        Err("请先选一个导出包".into())
    }
}

/// 核对选好的导出包，告诉界面里面有多少条、能不能替换
#[tauri::command]
fn restore_peek(app: AppHandle, path: String) -> Res<restore::Peek> {
    let manifest = restore::read_manifest(picked_package(&path)?)?;
    Ok(restore::Peek { manifest, sync_on: sync::sync_status(app.clone()).state != "off" })
}

/// 从导出包恢复：合并，或者（没开同步时）替换
#[tauri::command]
async fn restore_apply(app: AppHandle, path: String, replace: bool) -> Res<restore::Restored> {
    picked_package(&path)?;
    if replace && sync::sync_status(app.clone()).state != "off" {
        return Err("开着同步时只能合并：替换会把别的电脑上的数据也一起删掉。要替换，先在「设置 → 同步」里停止同步".into());
    }
    let app2 = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        let st = app2.state::<AppState>();
        // 解包（附件可能很多）时不拿数据库的锁，合并时才拿
        let u = restore::unpack(std::path::Path::new(&path), &st.data_dir)?;
        let c = st.db.lock().unwrap();
        restore::merge_in(&c, &st.data_dir, &u, replace)
    })
    .await
    .map_err(err)??;
    reload_settings(&app);
    notify_data(&app);
    notify_queue(&app);
    update_tray(&app);
    Ok(r)
}

// ---------- 命令：设置 ----------

#[tauri::command]
fn get_settings(st: tauri::State<AppState>) -> Settings {
    st.settings.lock().unwrap().clone()
}

// 下面几个命令可能会创建窗口，必须是 async：Windows 上在同步命令里建窗口会卡死
#[tauri::command]
async fn save_settings(app: AppHandle, st: tauri::State<'_, AppState>, settings: Settings) -> Res<()> {
    settings::save(&st.db.lock().unwrap(), &settings).map_err(err)?;
    let custom_changed = std::mem::replace(&mut *st.settings.lock().unwrap(), settings.clone()).launcher.custom != settings.launcher.custom;
    if custom_changed {
        launcher::scan(&app);
    }
    st.sync.poke();
    apply_autostart(settings.autostart);
    if !settings.auto_focus && st.focus_auto.load(Ordering::Relaxed) {
        let was = st.focus_active();
        st.focus_auto.store(false, Ordering::Relaxed);
        focus_changed(&app, was, st.focus_active());
    }
    if settings.widget {
        win::open_widget(&app);
    } else {
        win::close_widget(&app);
    }
    let r = apply_hotkeys(&app, &settings);
    let _ = app.emit("settings-changed", &settings);
    if !settings.ai_key.trim().is_empty() {
        onboard::tick(&app, "ai");
    }
    r
}

/// 主窗口里的搜索框（Ctrl + K）：打开快捷框，先放一个 "?"，就是搜索
#[tauri::command]
async fn open_flash_search(app: AppHandle, st: tauri::State<'_, AppState>) -> Res<()> {
    *st.flash_text.lock().unwrap() = Some("?".into());
    if app.get_webview_window("flash").and_then(|w| w.is_visible().ok()).unwrap_or(false) {
        let _ = app.emit_to("flash", "flash-open", ());
    } else {
        win::toggle_flash(&app);
    }
    Ok(())
}

/// 快捷框打开后，取走按快捷键时带过来的选中文字
#[tauri::command]
fn take_flash_text(st: tauri::State<AppState>) -> Option<String> {
    st.flash_text.lock().unwrap().take()
}

// ---------- 命令：每天收尾 ----------

fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// 今天在哪台电脑上收尾过了（几台电脑共用）
#[tauri::command]
fn wrap_status(st: tauri::State<AppState>) -> bool {
    db::shared_get(&st.db.lock().unwrap(), "wrapped").as_deref() == Some(today().as_str())
}

/// 收尾卡片处理完了：记下今天已收尾，别的电脑就不再弹，正开着的卡片也会收起来
#[tauri::command]
fn wrap_done(app: AppHandle, st: tauri::State<AppState>) -> Res<()> {
    let c = st.db.lock().unwrap();
    db::kv_set(&c, "last_wrap", &today()).map_err(err)?;
    db::shared_set(&c, "wrapped", &today()).map_err(err)?;
    drop(c);
    notify_data(&app);
    Ok(())
}

/// 调试用的测试副本（设了 DINGMI_DATA_DIR）：界面上标出来，免得和正在用的叮咪弄混
#[tauri::command]
fn is_test_copy() -> bool {
    dev_data_dir().is_some()
}

#[tauri::command]
fn hotkey_notice(st: tauri::State<AppState>) -> Option<String> {
    st.hotkey_notice.lock().unwrap().clone()
}

#[tauri::command]
fn data_path(st: tauri::State<AppState>) -> String {
    st.data_dir.join("dingmi.db").display().to_string()
}

/// 本机每天的自动备份：放在哪个文件夹、最近一次是什么时候（"YYYY-MM-DD HH:MM"，还没有时为 null）
#[tauri::command]
fn local_backups(st: tauri::State<AppState>) -> Value {
    let dir = st.data_dir.join("backups");
    let last = std::fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.starts_with("dingmi-") && n.ends_with(".db")))
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .max()
        .map(|t| chrono::DateTime::<Local>::from(t).format("%Y-%m-%d %H:%M").to_string());
    json!({ "dir": dir.display().to_string(), "last": last })
}

/// 桌面组件能不能开毛玻璃（Windows 11 才有；Windows 10 上设置里不显示这一行）
#[tauri::command]
fn widget_glass_ok() -> bool {
    desk::glass_supported()
}

// ---------- 命令：专注模式 ----------

#[tauri::command]
fn get_focus(app: AppHandle) -> Value {
    focus_info(&app)
}

#[tauri::command]
async fn set_focus(app: AppHandle, on: bool) -> Res<Value> {
    set_focus_manual(&app, on);
    Ok(focus_info(&app))
}

// ---------- 命令：AI ----------

fn weekday_cn(d: chrono::Weekday) -> &'static str {
    ["一", "二", "三", "四", "五", "六", "日"][d.num_days_from_monday() as usize]
}

/// devices：同步过的几台电脑的名字，用来理解"到公司提醒我"
#[tauri::command]
async fn ai_parse(st: tauri::State<'_, AppState>, text: String, devices: Option<Vec<String>>) -> Res<Value> {
    let s = st.settings.lock().unwrap().clone();
    let now = Local::now();
    let devices = devices.unwrap_or_default();
    let mut user = format!("现在是 {} 星期{}。\n", now.format("%Y-%m-%d %H:%M"), weekday_cn(now.weekday()));
    if devices.len() > 1 {
        user.push_str(&format!("用户的电脑：{}\n", devices.join("、")));
    }
    user.push_str(&format!("用户写的是：{}", text.trim()));
    let out = ai::chat(&s, &ai::parse_prompt(&s.tags, &s.note_cats), &user, true).await?;
    serde_json::from_str(&out).map_err(|_| "AI 返回的格式不对".into())
}

/// 整理一条笔记的格式，回 {title, body}。不动内容；附件行漏了的补回去
#[tauri::command]
async fn ai_tidy_note(st: tauri::State<'_, AppState>, title: String, body: String, ask: Option<String>) -> Res<Value> {
    let s = st.settings.lock().unwrap().clone();
    if body.chars().count() > 12_000 {
        return Err("笔记太长了（超过 1.2 万字），AI 整理不了，可以分成几条再整理".into());
    }
    // 用户这次的要求（比如"整理成表格"），放在最前面
    let ask: String = ask.unwrap_or_default().trim().chars().take(200).collect();
    let ask = if ask.is_empty() { String::new() } else { format!("整理要求：{ask}\n") };
    let user = format!("{ask}标题：{}\n正文：\n{}", title.trim(), body.trim());
    let out = ai::chat_timeout(&s, ai::TIDY_PROMPT, &user, true, 120).await?;
    let v: Value = serde_json::from_str(&out).map_err(|_| "AI 返回的格式不对，再试一次")?;
    let tidied = v["body"].as_str().filter(|b| !b.trim().is_empty()).ok_or("AI 没有返回整理后的内容，再试一次")?;
    let new_title = v["title"].as_str().map(str::trim).filter(|t| !t.is_empty()).unwrap_or(title.trim());
    Ok(json!({ "title": new_title, "body": ai::keep_attachments(&body, tidied) }))
}

#[tauri::command]
async fn ai_morning(st: tauri::State<'_, AppState>, context: String) -> Res<String> {
    let s = st.settings.lock().unwrap().clone();
    let now = Local::now();
    let user = format!(
        "今天是 {} 星期{}，现在 {}。\n{}",
        now.format("%m月%d日"),
        weekday_cn(now.weekday()),
        now.format("%H:%M"),
        context
    );
    ai::chat(&s, ai::MORNING_PROMPT, &user, false)
        .await
        .map(|t| t.trim().to_string())
}

#[tauri::command]
async fn ai_test(settings: Settings) -> Res<String> {
    ai::chat(&settings, "你是一个测试助手。", "请只回复：连接成功", false).await
}

// ---------- 命令：窗口 ----------

#[tauri::command]
async fn open_window(app: AppHandle, name: String) -> Res<()> {
    match name.as_str() {
        "main" => win::open_main(&app),
        "flash" => win::toggle_flash(&app),
        "qnote" => win::toggle_qnote(&app),
        "morning" => win::open_morning(&app),
        "wrap" => win::open_wrap(&app),
        _ => {}
    }
    Ok(())
}

/// 从别的窗口跳到主窗口的某个页面（比如开场白里点"看看"某条笔记、快捷框的搜索里打开一条提醒）
#[tauri::command]
async fn open_main_nav(
    app: AppHandle,
    st: tauri::State<'_, AppState>,
    view: String,
    note_id: Option<i64>,
    reminder_id: Option<i64>,
    sub: Option<String>,
    row: Option<String>,
) -> Res<()> {
    // sub：笔记本按哪个分类筛 / 设置的哪一页；row：设置里跳到那一行（闪一下）
    let nav = json!({ "view": view, "noteId": note_id, "reminderId": reminder_id, "sub": sub, "row": row });
    if app.get_webview_window("main").is_some() {
        let _ = app.emit_to("main", "nav", nav);
    } else {
        *st.nav.lock().unwrap() = Some(nav);
    }
    win::open_main(&app);
    Ok(())
}

#[tauri::command]
fn take_nav(st: tauri::State<AppState>) -> Option<Value> {
    st.nav.lock().unwrap().take()
}

#[tauri::command]
fn fit_window(app: AppHandle, window: WebviewWindow, height: f64) {
    win::fit(&app, &window, height);
}

/// 快捷框被拖到别处了：记下位置，下次在这块屏幕上就出现在那里
#[tauri::command]
fn flash_moved(app: AppHandle, window: WebviewWindow) {
    win::flash_moved(&app, &window);
}

/// 设置里「放回原位」
#[tauri::command]
fn flash_pos_reset(app: AppHandle) {
    win::flash_pos_reset(&app);
}

/// 拖进「自己加的」的文件变成一项一项（快捷方式解开成真实程序）
#[tauri::command]
fn launcher_resolve(paths: Vec<String>) -> Vec<launcher::Custom> {
    // 只解开真的拖进来的（sys::file_drops 拿到路径时记下了），解开的目标和参数也记下，launcher_save 才认
    let paths: Vec<String> = paths.into_iter().filter(|p| allow::granted(p)).collect();
    let out = launcher::resolve_drops(&paths);
    for c in &out {
        allow::grant_custom(&c.target, &c.args);
    }
    out
}

/// 设置里主窗口「恢复默认」：默认大小、屏幕中间
#[tauri::command]
fn main_reset(app: AppHandle) {
    win::main_reset(&app);
}

/// 显示出来但不抢焦点（提醒卡片、开场白、收尾）：正在别处打字的人不会被打断，点一下卡片才接收按键
#[tauri::command]
fn show_quiet(window: WebviewWindow) {
    desk::show(&window);
}

// ---------- 命令：启动器 ----------

/// 能打开的东西（启动器、桌面组件、设置里用）。列表变了会发 launcher-changed
#[tauri::command]
fn launcher_list() -> Vec<launcher::Item> {
    launcher::items()
}

#[tauri::command]
fn launcher_stats(app: AppHandle) -> launcher::Stats {
    launcher::stats(&app)
}

/// how：open / admin（管理员身份）/ dir（打开所在位置）；query 是当时输入框里的字
#[tauri::command]
async fn launcher_open(app: AppHandle, window: WebviewWindow, id: String, how: String, query: Option<String>) -> Res<()> {
    // 上手清单：用快捷框打开了一个程序（打开所在位置不算）
    let from_flash = window.label() == "flash" && how != "dir";
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || launcher::open(&app2, &id, &how, query.as_deref())).await.map_err(err)??;
    if from_flash {
        onboard::tick(&app, "launch");
    }
    Ok(())
}

/// 「重新找一遍」：图标也重新取
#[tauri::command]
fn launcher_rescan(app: AppHandle) {
    launcher::rescan(&app);
}

/// 只改设置里启动器那部分（只存在这台电脑上）；自己加的变了就重新找一遍
#[tauri::command]
fn launcher_save(app: AppHandle, st: tauri::State<AppState>, config: launcher::Config) -> Res<()> {
    let (s, custom_changed) = {
        let mut s = st.settings.lock().unwrap();
        // 新加的「自己加的」只能是网址，或者用户刚在对话框里选的、拖进来的（参数也要是快捷方式里本来就有的）：
        // 网页被塞了脚本也不能往启动器里加一个随便哪的程序再打开它
        if let Some(bad) = config.custom.iter().find(|c| !s.launcher.custom.iter().any(|o| o.target == c.target && o.args == c.args) && !launcher::custom_ok(c)) {
            diag::log("出错", format!("启动器要加一个没选过的位置，没有加：{}", bad.target));
            return Err("这个位置要从「选文件」「选文件夹」里选，或者拖进来".into());
        }
        let changed = s.launcher.custom != config.custom;
        s.launcher = config;
        (s.clone(), changed)
    };
    settings::save_local(&st.db.lock().unwrap(), &s).map_err(err)?;
    if custom_changed {
        launcher::scan(&app);
    }
    let _ = app.emit("settings-changed", &s);
    Ok(())
}

/// 界面拿来的路径能不能用（见 allow.rs）。`how`：
/// - Open：打开（相当于双击）：用户选过、拖进来的，启动器里自己加的（和它下面一层），叮咪的备份文件夹、诊断包、
///   同步和备份文件夹，附件文件夹里白名单里的类型
/// - Reveal：在资源管理器里选中：上面这些，加上数据文件夹里的
/// - Peek：看文件夹里有什么：用户选过的、启动器里的文件夹
#[derive(Clone, Copy, PartialEq)]
enum PathUse {
    Open,
    Reveal,
    Peek,
}

fn path_allowed(app: &AppHandle, p: &str, how: PathUse) -> bool {
    if allow::granted_or_child(p) && (how != PathUse::Peek || allow::granted(p)) {
        return true;
    }
    if !allow::local_abs(p) {
        return false;
    }
    let st = app.state::<AppState>();
    // 启动器里的：自己加的文件、文件夹，开始菜单、桌面上指向文件夹的；文件夹下面一层（桌面组件上点开文件夹看到的）
    let custom: Vec<String> = st.settings.lock().unwrap().launcher.custom.iter().map(|c| c.target.clone()).collect();
    let folders: Vec<String> =
        launcher::items().into_iter().filter(|i| i.kind == "folder" && !i.path.is_empty()).map(|i| i.path).collect();
    let listed = |t: &String| allow::norm(t) == allow::norm(p);
    if custom.iter().any(listed) || folders.iter().any(listed) {
        return true;
    }
    if how == PathUse::Peek {
        return false;
    }
    if custom.iter().chain(&folders).any(|d| allow::child_of(p, d)) {
        return true;
    }
    let att = db::attachment_dir(&st.data_dir);
    if allow::under(p, &att) {
        return allow::child_of(p, &att.display().to_string()) && (how == PathUse::Reveal || links::safe_to_open(p));
    }
    // 叮咪自己写的：本机备份、诊断包（里面的文件也能打开看看）
    if [st.data_dir.join("backups"), diag::temp_dir()].iter().any(|d| allow::under(p, d)) {
        return true;
    }
    // 同步、备份用的网盘文件夹：别人也能往里放东西，只打开文件夹本身
    let shared: Vec<String> =
        sync::sync_status(app.clone()).folder.into_iter().chain(sync::backup::backup_status(app.clone()).dir).collect();
    if shared.iter().any(|d| allow::norm(d) == allow::norm(p) || (how == PathUse::Reveal && allow::under(p, Path::new(d)))) {
        return true;
    }
    how == PathUse::Reveal && allow::under(p, &st.data_dir)
}

fn check_path(app: &AppHandle, p: &str, how: PathUse) -> Res<()> {
    if path_allowed(app, p, how) {
        Ok(())
    } else {
        diag::log("出错", format!("界面要用一个没选过的路径，没有理它：{p}"));
        Err("这个位置叮咪不能打开，请重新选一次".into())
    }
}

/// 文件夹里最新的几个（桌面组件上点文件夹的小箭头）
#[tauri::command]
fn folder_peek(app: AppHandle, path: String) -> Res<launcher::Peek> {
    check_path(&app, &path, PathUse::Peek)?;
    launcher::peek(&path, 6)
}

/// 打开一个文件、文件夹（相当于双击）
#[tauri::command]
async fn open_path(app: AppHandle, path: String) -> Res<()> {
    check_path(&app, &path, PathUse::Open)?;
    tauri::async_runtime::spawn_blocking(move || launcher::run(&path, None)).await.map_err(err)?
}

/// 选一个文件夹或文件，取消时为 null
#[tauri::command]
async fn pick_path(folder: bool) -> Option<String> {
    let p = tauri::async_runtime::spawn_blocking(move || launcher::pick(folder)).await.ok().flatten()?;
    allow::grant(&p);
    Some(p)
}

// ---------- 命令：自动更新 ----------

#[tauri::command]
fn update_status(app: AppHandle) -> update::Status {
    update::status(&app)
}

/// 「设置 → 关于」里的「检查更新」：连不上时回「连不上，过会儿再试」
#[tauri::command]
async fn update_check(app: AppHandle) -> Res<update::Status> {
    update::check(&app, true).await?;
    Ok(update::status(&app))
}

/// 点「更新」：在后台下载，进度和结果从 update-status 事件里来
#[tauri::command]
fn update_download(app: AppHandle) {
    update::download(&app)
}

/// 点 ✕：这个版本不再提示
#[tauri::command]
fn update_dismiss(app: AppHandle) {
    update::dismiss(&app)
}

#[tauri::command]
fn update_install(app: AppHandle) -> Res<()> {
    update::install(&app)
}

/// 更新后第一次打开时的提示
#[tauri::command]
fn update_notice(app: AppHandle) -> Option<String> {
    update::take_notice(&app)
}

/// 协议：「设置 → 关于」里哪份最近更新过、主窗口顶上要不要通知重大修改（terms.rs）
#[tauri::command]
fn terms_info(app: AppHandle) -> terms::View {
    terms::current(&app)
}

/// 协议通知点了「知道了」：这份协议的这次修改不再显示
#[tauri::command]
fn terms_seen(app: AppHandle, key: String, version: String) {
    terms::mark_seen(&app, &key, &version)
}

// ---------- 命令：桌面组件 ----------

/// 组件画好了：按设置上毛玻璃（或者换回不透明）、摆好位置，显示出来（不抢焦点）。换了外观也调一次。回是不是毛玻璃
#[tauri::command]
fn widget_show(app: AppHandle, window: WebviewWindow, tint: [u8; 4]) -> bool {
    let glass = win::widget_look(&app, &window, tint);
    desk::show(&window);
    glass
}

/// 移动位置：记住现在的位置；reset 时放回右上角
#[tauri::command]
fn widget_place(app: AppHandle, reset: bool) {
    win::widget_place(&app, reset);
}

/// 组件上的「关掉桌面组件」。只改这台电脑的设置
#[tauri::command]
async fn widget_off(app: AppHandle, st: tauri::State<'_, AppState>) -> Res<()> {
    let s = {
        let mut s = st.settings.lock().unwrap();
        s.widget = false;
        s.clone()
    };
    settings::save_local(&st.db.lock().unwrap(), &s).map_err(err)?;
    let _ = app.emit("settings-changed", &s);
    win::close_widget(&app);
    Ok(())
}

// ---------- 托盘 ----------

/// 点托盘、再打开一次叮咪：引导还没走完（比如装好后开机自动启动的，引导没弹）就打开引导，否则打开主窗口
fn open_home(app: &AppHandle) {
    let pending = app.try_state::<AppState>().is_some_and(|st| onboard::pending(&st.db.lock().unwrap()));
    if pending {
        win::open_onboard(app);
    } else {
        win::open_main(app);
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "打开叮咪", true, None::<&str>)?;
    let flash = MenuItem::with_id(app, "flash", "快捷框", true, None::<&str>)?;
    let qnote = MenuItem::with_id(app, "qnote", "快速记笔记", true, None::<&str>)?;
    let focus = CheckMenuItem::with_id(app, "focus", "专注模式", true, false, None::<&str>)?;
    let wrap = MenuItem::with_id(app, "wrap", "每天收尾", true, None::<&str>)?;
    let sync_status = MenuItem::with_id(app, "sync_status", "同步：没有开启", false, None::<&str>)?;
    let sync_now = MenuItem::with_id(app, "sync_now", "立即同步", false, None::<&str>)?;
    let upd = MenuItem::with_id(app, "update", update::tray_text(), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open,
            &flash,
            &qnote,
            &PredefinedMenuItem::separator(app)?,
            &focus,
            &wrap,
            &PredefinedMenuItem::separator(app)?,
            &sync_status,
            &sync_now,
            &PredefinedMenuItem::separator(app)?,
            &upd,
            &quit,
        ],
    )?;
    let st = app.state::<AppState>();
    *st.tray_focus.lock().unwrap() = Some(focus);
    *st.tray_sync.lock().unwrap() = Some((sync_status, sync_now));
    *st.tray_update.lock().unwrap() = Some(upd);
    let mut tb = TrayIconBuilder::with_id("dingmi")
        .tooltip("叮咪")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| match e.id.as_ref() {
            "open" => open_home(app),
            "flash" => win::toggle_flash(app),
            "qnote" => win::toggle_qnote(app),
            "wrap" => win::open_wrap(app),
            "sync_now" => sync::sync_now(app.clone()),
            // 下载好了就装；有新版本就开始下载（进度在主窗口顶上）；没有就打开「设置 → 关于」检查一次
            "update" => match update::state().as_str() {
                "ready" => {
                    let _ = update::install(app);
                }
                "available" | "downloading" => {
                    update::download(app);
                    win::open_main(app);
                }
                _ => {
                    let nav = json!({ "view": "settings", "sub": "about", "row": "update" });
                    if app.get_webview_window("main").is_some() {
                        let _ = app.emit_to("main", "nav", nav);
                    } else {
                        *app.state::<AppState>().nav.lock().unwrap() = Some(nav);
                    }
                    win::open_main(app);
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        let _ = update::check(&app, true).await;
                    });
                }
            },
            "focus" => {
                let on = !app.state::<AppState>().focus_manual.load(Ordering::Relaxed);
                set_focus_manual(app, on);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                open_home(tray.app_handle());
            }
        });
    // 托盘只有 16～32 像素，用专门加粗过的那一版
    tb = tb.icon(tauri::image::Image::from_bytes(TRAY_PNGS[0][sys::taskbar_dark() as usize][3])?);
    tb.build(app)?;
    update_tray(app);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if cfg!(windows) && !sys::webview2_ready() {
        sys::alert_no_webview2();
        return;
    }
    win::init();
    let mut builder = tauri::Builder::default();
    if dev_data_dir().is_none() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            open_home(app);
        }));
    }
    builder
        .plugin(tauri_plugin_dialog::init())
        // 拖进窗口的文件（桌面组件上拖进来加到「常用」）：路径是系统给的，记下来，之后能打开、能加到启动器
        .on_webview_event(|_, e| {
            if let tauri::WebviewEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = e {
                paths.iter().for_each(|p| allow::grant(&p.display().to_string()));
            }
        })
        .on_window_event(|_, e| {
            if let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = e {
                paths.iter().for_each(|p| allow::grant(&p.display().to_string()));
            }
        })
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, sc, ev| {
                    if ev.state() == ShortcutState::Pressed {
                        on_hotkey(app, sc);
                    }
                })
                .build(),
        )
        .setup(|app| {
            let handle = app.handle().clone();
            let data_dir = dev_data_dir()
                .or_else(|| app.path().app_data_dir().ok())
                .unwrap_or_else(|| PathBuf::from("."));
            diag::init(&data_dir);
            diag::log("启动", format!("叮咪 {} 启动了", app.package_info().version));
            let conn = db::open(&data_dir).inspect_err(|e| diag::log("出错", format!("打不开数据库：{e}")))?;
            // 笔记里的图片：配置里只放行了 $APPDATA 下的，调试时数据目录可能在别处
            let _ = app.asset_protocol_scope().allow_directory(db::attachment_dir(&data_dir), true);
            let device = db::device_id(&conn)?;
            let saved = settings::load(&conn);
            let first_run = saved.is_none();
            // 第一次用叮咪（要在下面存设置之前看）：打开首次引导，不打开主窗口
            onboard::init(&conn, onboard::fresh(&conn))?;
            let show_onboard = onboard::pending(&conn);
            let mut settings: Settings = saved.unwrap_or_default();
            settings::ensure_shared(&conn, &settings, &device)?;
            settings::upgrade_note_cats(&conn, &mut settings)?;
            // 上次退出时攒着的提醒，这次直接弹出来
            let _ = conn.execute(
                "UPDATE reminders SET state=?1 WHERE state=2 AND done=0",
                [ST_SHOW],
            );
            app.manage(AppState {
                db: Mutex::new(conn),
                settings: Mutex::new(settings.clone()),
                focus_manual: AtomicBool::new(false),
                focus_auto: AtomicBool::new(false),
                shortcuts: Mutex::new(HashMap::new()),
                nav: Mutex::new(None),
                tray_focus: Mutex::new(None),
                tray_sync: Mutex::new(None),
                tray_update: Mutex::new(None),
                tray_tip: Mutex::new(String::new()),
                tray_icon: Mutex::new((usize::MAX, 0, 0)),
                flash_text: Mutex::new(None),
                last_ring: Mutex::new(Instant::now()),
                data_dir,
                device,
                sync: sync::SyncState::new(),
                hotkey_notice: Mutex::new(None),
                focus_batch: Mutex::new(Vec::new()),
            });
            build_tray(&handle)?;
            if first_run {
                first_run_hotkeys(&handle, &mut settings);
            } else {
                let _ = apply_hotkeys(&handle, &settings);
            }
            apply_autostart(settings.autostart);
            let from_autostart = std::env::args().any(|a| a == "--autostart");
            if show_onboard && !from_autostart {
                win::open_onboard(&handle);
            } else if !from_autostart {
                win::open_main(&handle);
            }
            if settings.widget {
                win::open_widget(&handle);
            }
            win::warm_flash(&handle);
            launcher::start(handle.clone());
            sync::start(handle.clone());
            sync::backup::start(handle.clone());
            update::start(handle.clone());
            sync::start_pusher(handle.clone());
            scheduler::start(handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_reminders,
            save_reminder,
            delete_reminder,
            reminder_action,
            get_queue,
            held_reminders,
            focus_batch,
            reminder_changed_on,
            list_notes,
            save_note,
            delete_note,
            pin_note,
            get_note,
            open_note_window,
            note_windows,
            clear_tag,
            list_trash,
            trash_action,
            empty_trash,
            sync::sync_status,
            sync::sync_pair,
            sync::sync_unlock,
            sync::sync_new_recovery,
            sync::sync_change_password,
            sync::sync_folder_candidates,
            sync::sync_folder_probe,
            sync::sync_folder_start,
            sync::sync_folder_devices,
            sync::sync_stop,
            sync::sync_invite,
            sync::sync_now,
            sync::official_redeem,
            sync::official_beta,
            sync::official_beta_info,
            sync::official_email,
            sync::official_recover,
            sync::official_join,
            sync::official_join_on,
            sync::official_join_off,
            sync::official_phone_start,
            sync::official_phone_poll,
            sync::official_phone_join,
            sync::official_delete,
            sync::official_renew,
            sync::official_support_code,
            sync::official_account,
            sync::backup::backup_status,
            sync::backup::backup_start,
            sync::backup::backup_now,
            sync::backup::backup_off,
            sync::phone_list,
            sync::phone_add,
            sync::phone_revoke,
            sync::pc_list,
            sync::pc_revoke,
            inbox_list,
            inbox_to_note,
            inbox_done,
            inbox_delete,
            save_attachment,
            attachment_dir,
            open_url,
            open_attachment,
            list_attachments,
            attach_files,
            export_default,
            export_pick,
            export_all,
            reveal_path,
            diag_prepare,
            diag_export,
            restore_pick,
            pick_folder,
            save_png,
            save_attachment_as,
            restore_peek,
            restore_apply,
            list_devices,
            get_settings,
            save_settings,
            data_path,
            local_backups,
            widget_glass_ok,
            hotkey_notice,
            is_test_copy,
            take_flash_text,
            open_flash_search,
            wrap_status,
            wrap_done,
            get_focus,
            set_focus,
            ai_parse,
            ai_morning,
            ai_tidy_note,
            ai_test,
            open_window,
            open_main_nav,
            take_nav,
            fit_window,
            show_quiet,
            flash_moved,
            flash_pos_reset,
            main_reset,
            launcher_resolve,
            widget_show,
            widget_place,
            widget_off,
            update_status,
            update_check,
            update_install,
            update_notice,
            update_download,
            update_dismiss,
            terms_info,
            terms_seen,
            launcher_list,
            launcher_stats,
            launcher_open,
            launcher_rescan,
            launcher_save,
            folder_peek,
            open_path,
            pick_path,
            onboard::onboard_info,
            onboard::onboard_open,
            onboard::onboard_done,
            onboard::onboard_guide_note,
            onboard::checklist_get,
            onboard::checklist_hide,
        ])
        .build(tauri::generate_context!())
        .expect("叮咪启动失败")
        .run(|_app, event| {
            // 关掉所有窗口后继续在托盘里运行，只有菜单里的"退出"才真正退出
            match event {
                RunEvent::ExitRequested { api, code, .. } if code.is_none() => api.prevent_exit(),
                RunEvent::Exit => diag::log("启动", "退出了"),
                _ => {}
            }
        });
}
