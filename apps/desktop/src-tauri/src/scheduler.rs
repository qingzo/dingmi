//! 后台计时：每 10 秒看一眼有没有到点的提醒、是否在全屏、要不要显示每日开场白和每天收尾；每天备份一次、清理回收站。

use crate::{db, desk, focus_changed, notify_data, notify_queue, sys, update_tray, win, AppState};
use chrono::{Local, TimeDelta, Timelike};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

const TICK: Duration = Duration::from_secs(10);
/// 回收站里放多少天后彻底删除
const TRASH_DAYS: i64 = 30;
/// 自动备份保留几份（每天一份）
const BACKUP_KEEP: usize = 7;

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tick(&app);
            tokio::time::sleep(TICK).await;
        }
    });
}

fn tick(app: &AppHandle) {
    let st = app.state::<AppState>();
    let s = st.settings.lock().unwrap().clone();

    // 桌面组件：资源管理器重启后桌面换了，重新挂上；跟着旧桌面一起没了的话重新打开
    if s.widget {
        match app.get_webview_window("widget") {
            Some(w) => desk::repin(&w),
            None => win::open_widget(app),
        }
    }

    win::restart_if_webview2_updated(app);

    // 全屏、游戏、演示时自动专注
    let was = st.focus_active();
    st.focus_auto
        .store(s.auto_focus && sys::is_busy(), Ordering::Relaxed);
    let active = st.focus_active();
    if was != active {
        focus_changed(app, was, active);
    }

    // 到点的提醒：专注时不紧急的先攒着
    let now = db::now_min();
    let (fired, queue_len) = {
        let c = st.db.lock().unwrap();
        let due = db::due_now(&c, &now, &st.device).unwrap_or_default();
        for (id, urgent) in &due {
            let state = if active && !urgent { db::ST_HELD } else { db::ST_SHOW };
            let _ = db::set_state(&c, *id, state);
        }
        (!due.is_empty(), db::queue(&c).map(|q| q.len()).unwrap_or(0))
    };
    if fired {
        notify_data(app);
        if active {
            let _ = app.emit("focus-changed", crate::focus_info(app));
        }
    }

    if queue_len > 0 {
        let mut last = st.last_ring.lock().unwrap();
        if app.get_webview_window("remind").is_none() {
            // 新开的卡片自己会响一声
            win::open_remind(app);
            *last = Instant::now();
        } else if fired {
            notify_queue(app);
            let _ = app.emit_to("remind", "ring", ());
            *last = Instant::now();
        } else if s.ring_every_min > 0
            && last.elapsed() >= Duration::from_secs(s.ring_every_min as u64 * 60)
        {
            let _ = app.emit_to("remind", "ring", ());
            *last = Instant::now();
        }
    }

    let local = Local::now();
    let today = local.format("%Y-%m-%d").to_string();

    // 每天一次：备份数据库，彻底删除回收站里放了 30 天以上的
    let purged = {
        let c = st.db.lock().unwrap();
        if db::kv_get(&c, "last_daily").as_deref() != Some(today.as_str()) {
            let _ = db::kv_set(&c, "last_daily", &today);
            let _ = db::backup(&c, &st.data_dir, &today, BACKUP_KEEP);
            let before = (local - TimeDelta::days(TRASH_DAYS)).format("%Y-%m-%d %H:%M:%S").to_string();
            let n = db::purge_trash(&c, Some(before.as_str())).unwrap_or(0);
            let _ = db::clean_attachments(&c, &st.data_dir);
            n
        } else {
            0
        }
    };
    if purged > 0 {
        notify_data(app);
    }

    // 引导开着（或者第一次用、还没走完引导）时，开场白和收尾都先等着，引导走完下一轮再出
    let onboarding = app.get_webview_window("onboard").is_some() || crate::onboard::pending(&st.db.lock().unwrap());

    // 每天第一次开电脑（并且人在电脑前）时显示开场白
    if s.morning && !onboarding && local.hour() >= 5 && sys::idle_secs() < 120 {
        let c = st.db.lock().unwrap();
        if db::kv_get(&c, "last_morning").as_deref() != Some(today.as_str()) {
            let _ = db::kv_set(&c, "last_morning", &today);
            drop(c);
            win::open_morning(app);
        }
    }

    // 每天收尾：到了设定的时间、人在电脑前，今天在哪台电脑上都还没收尾过，今天也确实有事
    let hm = local.format("%H:%M").to_string();
    if s.wrap
        && !onboarding
        && hm >= s.wrap_time
        && sys::idle_secs() < 120
        && app.get_webview_window("morning").is_none()
        && app.get_webview_window("wrap").is_none()
    {
        let c = st.db.lock().unwrap();
        let shown = db::kv_get(&c, "last_wrap").as_deref() == Some(today.as_str());
        let wrapped = db::shared_get(&c, "wrapped").as_deref() == Some(today.as_str());
        if !shown && !wrapped {
            let _ = db::kv_set(&c, "last_wrap", &today);
            let (done, left) = db::today_summary(&c, &today);
            drop(c);
            if done + left > 0 {
                win::open_wrap(app);
            }
        }
    }

    update_tray(app);
}
