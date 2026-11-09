//! 首次引导和上手清单（需求-0.2 第 5 节）。
//!
//! 第一次打开：启动时 kv 里还没有设置、也没有一条提醒或笔记（包括回收站里的），就算这台电脑第一次用叮咪，
//! kv 里记 onboard = "pending"，打开引导窗口而不是主窗口；走完或者点「跳过」改成 "done"。
//! 走到一半退出了叮咪，下次打开还是引导。
//!
//! 上手清单：7 项各自做过没有，记在 kv 的 onboard_checklist（只在这台电脑，不同步），格式
//! `{"done":["flash","card",...],"hidden":false}`；done 按做到的先后排。全部做完自动 hidden。

use crate::{db, AppState};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

const KV_STATE: &str = "onboard";
const KV_LIST: &str = "onboard_checklist";
/// 「叮咪怎么用」放过的话记下它的 uid，以后不再放、也不再改
const KV_NOTE: &str = "onboard_note";

/// 上手清单的 7 项，顺序就是「今天」页上的顺序：
/// flash 按快捷键记一句话 / card 处理一张提醒卡片 / selection 选中文字再按快捷键 / launch 用快捷键打开程序 /
/// note 记一条笔记 / sync 设置同步 / ai 填 AI Key
pub const ITEMS: [&str; 7] = ["flash", "card", "selection", "launch", "note", "sync", "ai"];

#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
#[serde(default)]
struct Saved {
    done: Vec<String>,
    hidden: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Item {
    pub id: &'static str,
    pub done: bool,
}

/// 给「今天」页的上手清单
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Checklist {
    pub items: Vec<Item>,
    /// 做过几项 / 一共几项
    pub done: usize,
    pub total: usize,
    /// 点过 ✕ 或者全做完了：不再显示（「设置 → 关于」里能找回来）
    pub hidden: bool,
}

/// 这台电脑是不是第一次用叮咪：还没存过设置，也没有任何提醒和笔记。要在启动时、存设置之前调用
pub fn fresh(c: &Connection) -> bool {
    let count = |t: &str| c.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get::<_, i64>(0)).unwrap_or(1);
    db::kv_get(c, "settings").is_none() && count("reminders") == 0 && count("notes") == 0
}

/// 启动时调用：第一次用就记下要走引导。清单第一次建时，老用户（不是第一次用）默认不显示
pub fn init(c: &Connection, fresh: bool) -> rusqlite::Result<()> {
    if fresh {
        db::kv_set(c, KV_STATE, "pending")?;
    }
    if db::kv_get(c, KV_LIST).is_none() {
        write(c, &Saved { done: vec![], hidden: !fresh })?;
    }
    Ok(())
}

/// 引导还没走完（启动时打开引导，不打开主窗口）
pub fn pending(c: &Connection) -> bool {
    db::kv_get(c, KV_STATE).as_deref() == Some("pending")
}

pub fn finish(c: &Connection) -> rusqlite::Result<()> {
    db::kv_set(c, KV_STATE, "done")
}

fn read(c: &Connection) -> Saved {
    db::kv_get(c, KV_LIST).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write(c: &Connection, s: &Saved) -> rusqlite::Result<()> {
    db::kv_set(c, KV_LIST, &serde_json::to_string(s).unwrap_or_default())
}

fn view(s: &Saved) -> Checklist {
    let items: Vec<Item> = ITEMS.iter().map(|&id| Item { id, done: s.done.iter().any(|d| d == id) }).collect();
    Checklist { done: items.iter().filter(|i| i.done).count(), total: ITEMS.len(), items, hidden: s.hidden }
}

pub fn checklist(c: &Connection) -> Checklist {
    view(&read(c))
}

/// 打勾；以前就勾过（或者不认识的）回 None，新勾上回新的清单。全部做完就不再显示
pub fn mark(c: &Connection, id: &str) -> Option<Checklist> {
    if !ITEMS.contains(&id) {
        return None;
    }
    let mut s = read(c);
    if s.done.iter().any(|d| d == id) {
        return None;
    }
    s.done.push(id.to_string());
    if s.done.len() >= ITEMS.len() {
        s.hidden = true;
    }
    write(c, &s).ok()?;
    Some(view(&s))
}

pub fn set_hidden(c: &Connection, hidden: bool) -> rusqlite::Result<Checklist> {
    let mut s = read(c);
    s.hidden = hidden;
    write(c, &s)?;
    Ok(view(&s))
}

/// 同步开过的话就不用每次都去读数据库
static SYNC_TICKED: AtomicBool = AtomicBool::new(false);

/// 做了清单上的某件事：打勾，告诉「今天」页（checklist-changed）。调用时不能拿着数据库的锁
pub fn tick(app: &AppHandle, id: &str) {
    let st = app.state::<AppState>();
    let Some(list) = mark(&st.db.lock().unwrap(), id) else { return };
    let _ = app.emit("checklist-changed", list);
}

/// 同步状态变了（sync::publish_as 里调）：开着同步就勾上「设置同步」。那里可能拿着别的锁，放到后台去勾
pub fn sync_on(app: &AppHandle) {
    if SYNC_TICKED.swap(true, Ordering::Relaxed) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move { tick(&app, "sync") });
}

// ---------- 引导第 2 步用的 ----------

/// 第一次运行时快捷框的快捷键被占、自动换过的话，原来是哪个（只记这次运行）
static FLASH_MOVED: Mutex<Option<String>> = Mutex::new(None);

pub fn flash_moved(from: &str) {
    *FLASH_MOVED.lock().unwrap() = Some(from.to_string());
}

/// 快捷框里记下了一条（save_reminder / save_note 里调）：开着引导的话告诉它，第 2 步打勾
pub fn tried(app: &AppHandle, kind: &str, title: &str, due_at: Option<&str>) {
    tick(app, "flash");
    if app.get_webview_window("onboard").is_some() {
        let _ = app.emit_to("onboard", "onboard-tried", serde_json::json!({ "kind": kind, "title": title, "due_at": due_at }));
    }
}

#[derive(Serialize)]
pub struct Info {
    /// 这台电脑第一次用、引导还没走完（不是从设置里重新打开的）
    pub first: bool,
    /// 快捷框现在的快捷键，"Alt+Space" 这种写法
    pub flash: String,
    /// 这个快捷键登记上了没有（被别的程序占了、又没换成的话是 false）
    pub flash_ok: bool,
    /// 默认的被占了、自动换过的话，原来是哪个
    pub moved_from: Option<String>,
}

// ---------- 命令 ----------

#[tauri::command]
pub fn onboard_info(st: tauri::State<AppState>) -> Info {
    let flash = st.settings.lock().unwrap().hk_flash.clone();
    let flash_ok = st.shortcuts.lock().unwrap().values().any(|n| *n == "flash");
    Info {
        first: pending(&st.db.lock().unwrap()),
        flash,
        flash_ok,
        moved_from: FLASH_MOVED.lock().unwrap().clone(),
    }
}

/// 「设置 → 关于 → 再看一遍引导」（建窗口的命令要是 async，见 lib.rs save_settings 的说明）
#[tauri::command]
pub async fn onboard_open(app: AppHandle) -> Result<(), String> {
    crate::win::open_onboard(&app);
    Ok(())
}

/// 引导走完或者点了「跳过」：以后启动不再打开引导
#[tauri::command]
pub fn onboard_done(st: tauri::State<AppState>) -> Result<(), String> {
    finish(&st.db.lock().unwrap()).map_err(|e| e.to_string())
}

/// 放「叮咪怎么用」笔记（分类「资料」）。只放一次：放过的（哪怕用户已经删了）不再放，回 None
#[tauri::command]
pub fn onboard_guide_note(app: AppHandle, st: tauri::State<AppState>, title: String, body: String) -> Result<Option<i64>, String> {
    let tag = st.settings.lock().unwrap().note_cats.iter().any(|t| t.id == "ref").then(|| "ref".to_string());
    let c = st.db.lock().unwrap();
    let Some(id) = place_note(&c, &title, &body, tag).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    drop(c);
    crate::notify_data(&app);
    Ok(Some(id))
}

fn place_note(c: &Connection, title: &str, body: &str, tag: Option<String>) -> rusqlite::Result<Option<i64>> {
    if db::kv_get(c, KV_NOTE).is_some() {
        return Ok(None);
    }
    let id = db::upsert_note(c, &db::Note { title: title.into(), body: body.into(), tag, ..Default::default() })?;
    let uid = db::get_note(c, id)?.map(|n| n.uid).unwrap_or_default();
    db::kv_set(c, KV_NOTE, &uid)?;
    Ok(Some(id))
}

/// 读上手清单。同步开着、AI Key 填了的（比如别的电脑同步过来的）顺便勾上
#[tauri::command]
pub fn checklist_get(app: AppHandle, st: tauri::State<AppState>) -> Checklist {
    let has_key = !st.settings.lock().unwrap().ai_key.trim().is_empty();
    let sync_on = crate::sync::sync_status(app.clone()).state != "off";
    let c = st.db.lock().unwrap();
    if has_key {
        mark(&c, "ai");
    }
    if sync_on {
        mark(&c, "sync");
    }
    checklist(&c)
}

/// 「今天」页上点 ✕（hidden = true），或者「设置 → 关于 → 找回上手清单」（hidden = false）
#[tauri::command]
pub fn checklist_hide(app: AppHandle, st: tauri::State<AppState>, hidden: bool) -> Result<Checklist, String> {
    let list = set_hidden(&st.db.lock().unwrap(), hidden).map_err(|e| e.to_string())?;
    let _ = app.emit("checklist-changed", &list);
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings;

    fn temp_db() -> Connection {
        let dir = std::env::temp_dir().join(format!("dingmi-onboard-{}", uuid::Uuid::new_v4().simple()));
        db::open(&dir).unwrap()
    }

    #[test]
    fn first_open_only_without_settings_or_data() {
        // 新装：没有设置、没有数据
        let c = temp_db();
        assert!(fresh(&c));
        init(&c, true).unwrap();
        assert!(pending(&c));
        assert!(!checklist(&c).hidden);
        // 启动时存过设置以后就不算第一次了；引导没走完，下次还是打开引导
        settings::save_local(&c, &settings::Settings::default()).unwrap();
        assert!(!fresh(&c));
        init(&c, fresh(&c)).unwrap();
        assert!(pending(&c));
        finish(&c).unwrap();
        assert!(!pending(&c));

        // 有数据、没有设置（比如很早的版本）：不算第一次，清单默认不显示
        let c2 = temp_db();
        db::upsert_note(&c2, &db::Note { title: "x".into(), ..Default::default() }).unwrap();
        assert!(!fresh(&c2));
        init(&c2, false).unwrap();
        assert!(!pending(&c2));
        assert!(checklist(&c2).hidden);
        // 删进回收站的也算有过数据
        let c3 = temp_db();
        let id = db::upsert_reminder(&c3, &db::Reminder { title: "x".into(), due_at: "2099-01-01 09:00".into(), ..Default::default() }).unwrap();
        db::delete_reminder(&c3, id).unwrap();
        assert!(!fresh(&c3));
        // 老用户：有设置
        let c4 = temp_db();
        settings::save(&c4, &settings::Settings::default()).unwrap();
        assert!(!fresh(&c4));
    }

    #[test]
    fn checklist_ticks_once_and_hides_when_all_done() {
        let c = temp_db();
        init(&c, true).unwrap();
        let l = checklist(&c);
        assert_eq!((l.done, l.total, l.hidden), (0, 7, false));
        assert_eq!(l.items.iter().map(|i| i.id).collect::<Vec<_>>(), ITEMS);
        // 勾上一次就够了，再做一次不算新的
        let l = mark(&c, "card").unwrap();
        assert_eq!(l.done, 1);
        assert!(l.items[1].done && !l.items[0].done);
        assert!(mark(&c, "card").is_none());
        assert!(mark(&c, "nope").is_none());
        // 点 ✕ 不显示，找回来接着显示，勾过的还在
        assert!(set_hidden(&c, true).unwrap().hidden);
        let l = set_hidden(&c, false).unwrap();
        assert!(!l.hidden && l.done == 1);
        // 全做完自动不显示
        for id in ITEMS {
            mark(&c, id);
        }
        let l = checklist(&c);
        assert_eq!(l.done, 7);
        assert!(l.hidden);
        // 存的是按做到的先后排的 id
        let saved: Saved = serde_json::from_str(&db::kv_get(&c, KV_LIST).unwrap()).unwrap();
        assert_eq!(saved.done[0], "card");
        // 已经有清单的不会被 init 重置
        init(&c, false).unwrap();
        assert_eq!(checklist(&c).done, 7);
    }

    #[test]
    fn guide_note_only_once() {
        let c = temp_db();
        let id = place_note(&c, "叮咪怎么用", "# 叮咪怎么用", Some("ref".into())).unwrap().unwrap();
        let n = db::get_note(&c, id).unwrap().unwrap();
        assert_eq!((n.title.as_str(), n.tag.as_deref()), ("叮咪怎么用", Some("ref")));
        // 再打开引导、或者用户删了它：都不再放
        db::delete_note(&c, id).unwrap();
        assert!(place_note(&c, "叮咪怎么用", "x", None).unwrap().is_none());
    }
}
