//! SQLite 存储：提醒、笔记和键值设置。时间统一存成本地时间字符串 "YYYY-MM-DD HH:MM"，可以直接按字符串比较大小。
//!
//! 为多设备同步做准备：提醒和笔记都有跨设备唯一的 uid、版本号 ver（见 clock.rs）和 dirty（本机改过还没上传）。
//! 删除先进回收站（deleted_at），彻底删除时清空内容、只留 purged 标记，好让以后别的电脑也跟着删。

use crate::clock;
use chrono::{Local, NaiveDateTime};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const FMT: &str = "%Y-%m-%d %H:%M";

pub fn now_min() -> String {
    Local::now().format(FMT).to_string()
}

pub fn now_sec() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

pub fn parse_dt(s: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s.trim(), FMT).ok()
}

fn new_uid() -> String {
    Uuid::now_v7().to_string()
}

/// 提醒状态：0 等待到点；1 正在弹窗；2 专注模式下攒着；3 已看过（取消完成后不再立刻弹出）。只属于这台电脑，不同步。
pub const ST_WAIT: i64 = 0;
pub const ST_SHOW: i64 = 1;
pub const ST_HELD: i64 = 2;
pub const ST_SEEN: i64 = 3;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Repeat {
    /// none / daily / weekdays / weekly / monthly / interval / yearly / month_last / month_nth / lunar_yearly，
    /// 格式见 docs/方案/同步方案.md 4.2
    pub kind: String,
    /// 每周的哪几天，1=周一 … 7=周日；month_nth 只用第一个
    pub days: Vec<u32>,
    /// 每月几号（农历时是农历的几日），0 表示沿用开始日期
    pub monthday: u32,
    /// 每几天一次；weekly 时是每几周（0、1 每周，2 每两周）
    pub every: u32,
    /// 几月（农历时是农历的几月），0 表示沿用开始日期（0.2 加）
    pub month: u32,
    /// 每月第几个周几：1–5，-1 最后一个，0 表示沿用开始日期（0.2 加）
    pub nth: i32,
}

impl Repeat {
    pub fn is_recurring(&self) -> bool {
        !matches!(self.kind.as_str(), "" | "none")
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Reminder {
    pub id: i64,
    pub uid: String,
    pub title: String,
    pub detail: String,
    pub tag: Option<String>,
    pub urgent: bool,
    pub due_at: String,
    pub repeat: Repeat,
    pub snoozed_until: Option<String>,
    pub done: bool,
    pub state: i64,
    pub snooze_count: i64,
    pub last_done_at: Option<String>,
    pub completed_at: Option<String>,
    /// 只在这几台电脑上提醒（设备 ID），空表示所有电脑
    pub only_on: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
    /// 从笔记里的待办设的提醒：哪条笔记、哪一行待办。完成时把那一行勾上
    pub note_uid: Option<String>,
    pub todo: Option<String>,
    /// 一句话说得含糊（"改天""过几天"），时间是先定的；改过时间后变回 false
    pub vague: bool,
}

impl Reminder {
    pub fn fire_at(&self) -> &str {
        self.snoozed_until.as_deref().unwrap_or(&self.due_at)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Note {
    pub id: i64,
    pub uid: String,
    pub title: String,
    pub body: String,
    pub tag: Option<String>,
    pub reminder_id: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
    /// 置顶的时间；没置顶是 None。置顶的排在最前面，后置顶的在上面
    pub pinned_at: Option<String>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Trash {
    pub reminders: Vec<Reminder>,
    pub notes: Vec<Note>,
}

pub fn open(dir: &Path) -> rusqlite::Result<Connection> {
    std::fs::create_dir_all(dir).ok();
    let old = set_aside_v0(dir)?;
    let conn = Connection::open(dir.join("dingmi.db"))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         CREATE TABLE IF NOT EXISTS reminders(
            id INTEGER PRIMARY KEY,
            uid TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL,
            detail TEXT NOT NULL DEFAULT '',
            tag TEXT,
            urgent INTEGER NOT NULL DEFAULT 0,
            due_at TEXT NOT NULL,
            repeat TEXT NOT NULL DEFAULT '{}',
            snoozed_until TEXT,
            done INTEGER NOT NULL DEFAULT 0,
            state INTEGER NOT NULL DEFAULT 0,
            snooze_count INTEGER NOT NULL DEFAULT 0,
            last_done_at TEXT,
            completed_at TEXT,
            only_on TEXT NOT NULL DEFAULT '[]',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            deleted_at TEXT,
            purged INTEGER NOT NULL DEFAULT 0,
            ver TEXT NOT NULL,
            dirty INTEGER NOT NULL DEFAULT 1
         );
         CREATE TABLE IF NOT EXISTS notes(
            id INTEGER PRIMARY KEY,
            uid TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL,
            body TEXT NOT NULL DEFAULT '',
            tag TEXT,
            reminder_id INTEGER,
            reminder_uid TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            deleted_at TEXT,
            purged INTEGER NOT NULL DEFAULT 0,
            ver TEXT NOT NULL,
            dirty INTEGER NOT NULL DEFAULT 1
         );
         CREATE TABLE IF NOT EXISTS kv(key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS attachments(
            id INTEGER PRIMARY KEY,
            uid TEXT NOT NULL UNIQUE,
            file TEXT NOT NULL,
            mime TEXT NOT NULL,
            size INTEGER NOT NULL,
            sha256 TEXT NOT NULL,
            note_uid TEXT,
            created_at TEXT NOT NULL,
            deleted_at TEXT,
            purged INTEGER NOT NULL DEFAULT 0,
            ver TEXT NOT NULL,
            dirty INTEGER NOT NULL DEFAULT 1,
            local_ok INTEGER NOT NULL DEFAULT 1
         );
         CREATE TABLE IF NOT EXISTS inbox(
            id INTEGER PRIMARY KEY,
            uid TEXT NOT NULL UNIQUE,
            phone TEXT NOT NULL DEFAULT '',
            received_at TEXT NOT NULL,
            sent_at TEXT,
            text TEXT NOT NULL DEFAULT '',
            images TEXT NOT NULL DEFAULT '[]',
            status INTEGER NOT NULL DEFAULT 0,
            ver TEXT NOT NULL,
            dirty INTEGER NOT NULL DEFAULT 1
         );
         CREATE TABLE IF NOT EXISTS devices(
            uid TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            ver TEXT NOT NULL,
            dirty INTEGER NOT NULL DEFAULT 1
         );
         CREATE TABLE IF NOT EXISTS shared(
            uid TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            ver TEXT NOT NULL,
            dirty INTEGER NOT NULL DEFAULT 1
         );",
    )?;
    add_new_columns(&conn)?;
    if let Some(old) = old {
        // 设置（快捷键、AI Key 等）的格式没变，搬过来；搬不成也不影响启动
        if conn.execute("ATTACH DATABASE ?1 AS old", [old.to_string_lossy().to_string()]).is_ok() {
            let _ = conn.execute_batch("INSERT OR IGNORE INTO kv SELECT key, value FROM old.kv");
            let _ = conn.execute_batch("DETACH DATABASE old");
        }
    }
    clock::init(&device_id(&conn)?);
    let seen: Option<String> = conn.query_row(
        "SELECT MAX(v) FROM (SELECT MAX(ver) AS v FROM reminders UNION ALL SELECT MAX(ver) FROM notes)",
        [],
        |r| r.get(0),
    )?;
    if let Some(v) = seen {
        clock::observe(&v);
    }
    Ok(conn)
}

/// 后来加的列：已经装过的库补上。从旧版导出包恢复时，包里的库也用它补齐
pub fn add_new_columns(conn: &Connection) -> rusqlite::Result<()> {
    // v0.2 开发中加的列
    add_column(conn, "reminders", "note_uid", "TEXT")?;
    add_column(conn, "reminders", "todo", "TEXT")?;
    // 文件附件的原文件名（图片没有）
    add_column(conn, "attachments", "name", "TEXT NOT NULL DEFAULT ''")?;
    // 笔记置顶（v0.3.5）
    add_column(conn, "notes", "pinned_at", "TEXT")?;
    // 说得含糊、时间是先定的提醒（0.2）
    add_column(conn, "reminders", "vague", "INTEGER NOT NULL DEFAULT 0")?;
    // 叮咪同步的套餐不传、原文件只在哪台设备上（1.0，需求-1.0 第 32.1 节第 5 条）
    add_column(conn, "attachments", "only_on", "TEXT")?;
    Ok(())
}

fn add_column(c: &Connection, table: &str, col: &str, ty: &str) -> rusqlite::Result<()> {
    let has: i64 = c.query_row(
        &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name=?1"),
        [col],
        |r| r.get(0),
    )?;
    if has == 0 {
        c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {col} {ty}"))?;
    }
    Ok(())
}

/// v0.1 的数据库没有 uid 等同步用的列，也没人正式用过，不做升级：
/// 把它改名成 dingmi-v0.1-<时间>.db 留在原地（想找回内容还能找），返回改名后的路径，由调用方新建一个。
fn set_aside_v0(dir: &Path) -> rusqlite::Result<Option<PathBuf>> {
    let path = dir.join("dingmi.db");
    if !path.exists() {
        return Ok(None);
    }
    let is_v0 = {
        let c = Connection::open(&path)?;
        let count = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0));
        count("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='reminders'")? > 0
            && count("SELECT COUNT(*) FROM pragma_table_info('reminders') WHERE name='uid'")? == 0
    };
    if !is_v0 {
        return Ok(None);
    }
    // 上面的连接已经关掉，WAL 里的内容会合并回主文件；万一还有残留，-wal / -shm 一起改名
    let stem = format!("dingmi-v0.1-{}", Local::now().format("%Y%m%d-%H%M%S"));
    for suffix in ["", "-wal", "-shm"] {
        let from = dir.join(format!("dingmi.db{suffix}"));
        if from.exists() {
            std::fs::rename(&from, dir.join(format!("{stem}.db{suffix}")))
                .map_err(|_| rusqlite::Error::InvalidPath(from.clone()))?;
        }
    }
    Ok(Some(dir.join(format!("{stem}.db"))))
}

/// 这台电脑的设备 ID，第一次打开时生成
pub fn device_id(c: &Connection) -> rusqlite::Result<String> {
    if let Some(d) = kv_get(c, "device_id") {
        return Ok(d);
    }
    let d = Uuid::new_v4().simple().to_string()[..12].to_string();
    kv_set(c, "device_id", &d)?;
    Ok(d)
}

const R_COLS: &str = "id,title,detail,tag,urgent,due_at,repeat,snoozed_until,done,state,snooze_count,last_done_at,completed_at,created_at,uid,only_on,updated_at,deleted_at,note_uid,todo,vague";

fn row_reminder(r: &Row) -> rusqlite::Result<Reminder> {
    let repeat: String = r.get(6)?;
    let only_on: String = r.get(15)?;
    Ok(Reminder {
        id: r.get(0)?,
        title: r.get(1)?,
        detail: r.get(2)?,
        tag: r.get(3)?,
        urgent: r.get::<_, i64>(4)? != 0,
        due_at: r.get(5)?,
        repeat: serde_json::from_str(&repeat).unwrap_or_default(),
        snoozed_until: r.get(7)?,
        done: r.get::<_, i64>(8)? != 0,
        state: r.get(9)?,
        snooze_count: r.get(10)?,
        last_done_at: r.get(11)?,
        completed_at: r.get(12)?,
        created_at: r.get(13)?,
        uid: r.get(14)?,
        only_on: serde_json::from_str(&only_on).unwrap_or_default(),
        updated_at: r.get(16)?,
        deleted_at: r.get(17)?,
        note_uid: r.get(18)?,
        todo: r.get(19)?,
        vague: r.get::<_, i64>(20)? != 0,
    })
}

fn query_reminders(c: &Connection, sql_tail: &str, p: impl rusqlite::Params) -> rusqlite::Result<Vec<Reminder>> {
    let mut st = c.prepare(&format!("SELECT {R_COLS} FROM reminders {sql_tail}"))?;
    let rows = st.query_map(p, row_reminder)?;
    rows.collect()
}

pub fn list_reminders(c: &Connection) -> rusqlite::Result<Vec<Reminder>> {
    query_reminders(c, "WHERE deleted_at IS NULL ORDER BY COALESCE(snoozed_until, due_at)", [])
}

pub fn get_reminder(c: &Connection, id: i64) -> rusqlite::Result<Option<Reminder>> {
    c.query_row(
        &format!("SELECT {R_COLS} FROM reminders WHERE id=?1 AND deleted_at IS NULL"),
        [id],
        row_reminder,
    )
    .optional()
}

pub fn queue(c: &Connection) -> rusqlite::Result<Vec<Reminder>> {
    query_reminders(
        c,
        "WHERE state=?1 AND done=0 AND deleted_at IS NULL ORDER BY COALESCE(snoozed_until, due_at)",
        [ST_SHOW],
    )
}

pub fn held(c: &Connection) -> rusqlite::Result<Vec<Reminder>> {
    query_reminders(
        c,
        "WHERE state=?1 AND done=0 AND deleted_at IS NULL ORDER BY COALESCE(snoozed_until, due_at)",
        [ST_HELD],
    )
}

/// 到点但还没处理的提醒：(id, urgent)。指定了设备的，只在那几台电脑上算到点。
pub fn due_now(c: &Connection, now: &str, device: &str) -> rusqlite::Result<Vec<(i64, bool)>> {
    let mut st = c.prepare(
        "SELECT id, urgent FROM reminders
         WHERE done=0 AND state=0 AND deleted_at IS NULL AND COALESCE(snoozed_until, due_at) <= ?1
           AND (only_on='[]' OR EXISTS(SELECT 1 FROM json_each(only_on) WHERE value=?2))",
    )?;
    let rows = st.query_map(params![now, device], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)))?;
    rows.collect()
}

pub fn set_state(c: &Connection, id: i64, state: i64) -> rusqlite::Result<()> {
    c.execute("UPDATE reminders SET state=?2 WHERE id=?1", params![id, state])?;
    Ok(())
}

/// 这条提醒最后是在哪台电脑上改的（看版本号里的设备 ID）：别的电脑改的回那台的名字，不知道名字时是空字符串；
/// 这台电脑改的、或者找不到这条时回 None
pub fn reminder_changed_on(c: &Connection, id: i64, me: &str) -> rusqlite::Result<Option<String>> {
    let ver: Option<String> = c.query_row("SELECT ver FROM reminders WHERE id=?1", [id], |r| r.get(0)).optional()?;
    let Some(dev) = ver.as_deref().and_then(|v| v.splitn(3, '-').nth(2)).filter(|d| !d.is_empty() && *d != me) else {
        return Ok(None);
    };
    let name: Option<String> = c.query_row("SELECT name FROM devices WHERE uid=?1", [dev], |r| r.get(0)).optional()?;
    Ok(Some(name.unwrap_or_default()))
}

pub fn release_held(c: &Connection) -> rusqlite::Result<usize> {
    c.execute(
        "UPDATE reminders SET state=?1 WHERE state=?2",
        params![ST_SHOW, ST_HELD],
    )
}

pub fn upsert_reminder(c: &Connection, r: &Reminder) -> rusqlite::Result<i64> {
    let repeat = serde_json::to_string(&r.repeat).unwrap_or_else(|_| "{}".into());
    let only_on = serde_json::to_string(&r.only_on).unwrap_or_else(|_| "[]".into());
    let now = now_sec();
    if r.id > 0 {
        c.execute(
            "UPDATE reminders SET title=?2, detail=?3, tag=?4, urgent=?5, due_at=?6, repeat=?7,
                snoozed_until=?8, done=?9, state=?10, snooze_count=?11, last_done_at=?12, completed_at=?13,
                only_on=?14, updated_at=?15, ver=?16, note_uid=?17, todo=?18, vague=?19, dirty=1
             WHERE id=?1",
            params![
                r.id,
                r.title,
                r.detail,
                r.tag,
                r.urgent as i64,
                r.due_at,
                repeat,
                r.snoozed_until,
                r.done as i64,
                r.state,
                r.snooze_count,
                r.last_done_at,
                r.completed_at,
                only_on,
                now,
                clock::next(),
                r.note_uid,
                r.todo,
                r.vague as i64
            ],
        )?;
        Ok(r.id)
    } else {
        let uid = if r.uid.is_empty() { new_uid() } else { r.uid.clone() };
        c.execute(
            "INSERT INTO reminders(title,detail,tag,urgent,due_at,repeat,snoozed_until,done,state,snooze_count,last_done_at,completed_at,
                only_on,created_at,updated_at,uid,ver,note_uid,todo,vague,dirty)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?14,?15,?16,?17,?18,?19,1)",
            params![
                r.title,
                r.detail,
                r.tag,
                r.urgent as i64,
                r.due_at,
                repeat,
                r.snoozed_until,
                r.done as i64,
                r.state,
                r.snooze_count,
                r.last_done_at,
                r.completed_at,
                only_on,
                now,
                uid,
                clock::next(),
                r.note_uid,
                r.todo,
                r.vague as i64
            ],
        )?;
        Ok(c.last_insert_rowid())
    }
}

/// 移到回收站。关联的笔记不动：提醒恢复后关联还在。
pub fn delete_reminder(c: &Connection, id: i64) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE reminders SET deleted_at=?2, state=?3, ver=?4, dirty=1 WHERE id=?1 AND deleted_at IS NULL",
        params![id, now_sec(), ST_WAIT, clock::next()],
    )?;
    Ok(())
}

pub fn restore_reminder(c: &Connection, id: i64) -> rusqlite::Result<()> {
    let r = c
        .query_row(
            &format!("SELECT {R_COLS} FROM reminders WHERE id=?1 AND deleted_at IS NOT NULL AND purged=0"),
            [id],
            row_reminder,
        )
        .optional()?;
    let Some(r) = r else { return Ok(()) };
    // 已经过点的一次性提醒恢复后不立刻弹出，跟"取消完成"一样；周期提醒照常补弹
    let past = r.fire_at() <= now_min().as_str();
    let state = if !r.done && past && !r.repeat.is_recurring() { ST_SEEN } else { ST_WAIT };
    c.execute(
        "UPDATE reminders SET deleted_at=NULL, state=?2, ver=?3, dirty=1 WHERE id=?1",
        params![id, state, clock::next()],
    )?;
    Ok(())
}

/// 彻底删除：清空内容，只留删除标记；关联它的笔记解除关联
pub fn purge_reminder(c: &Connection, id: i64) -> rusqlite::Result<()> {
    let ver = clock::next();
    let n = c.execute(
        "UPDATE reminders SET purged=1, title='', detail='', tag=NULL, repeat='{}', snoozed_until=NULL, ver=?2, dirty=1
         WHERE id=?1 AND deleted_at IS NOT NULL AND purged=0",
        params![id, ver],
    )?;
    if n > 0 {
        c.execute(
            "UPDATE notes SET reminder_id=NULL, reminder_uid=NULL, ver=?2, dirty=1 WHERE reminder_id=?1",
            params![id, ver],
        )?;
    }
    Ok(())
}

/// 分类删掉了：用到它的提醒和笔记都改成没有分类（记一笔，同步给别的电脑）
/// 删掉一个分类：`notes` 为 false 时清提醒上的（提醒分类），为 true 时清笔记上的（笔记分类）
pub fn clear_tag(c: &Connection, tag: &str, notes: bool) -> rusqlite::Result<usize> {
    let table = if notes { "notes" } else { "reminders" };
    c.execute(
        &format!("UPDATE {table} SET tag=NULL, ver=?2, dirty=1 WHERE tag=?1 AND purged=0"),
        params![tag, clock::next()],
    )
}

const N_COLS: &str = "id,title,body,tag,reminder_id,created_at,updated_at,uid,deleted_at,pinned_at";

fn row_note(r: &Row) -> rusqlite::Result<Note> {
    Ok(Note {
        id: r.get(0)?,
        title: r.get(1)?,
        body: r.get(2)?,
        tag: r.get(3)?,
        reminder_id: r.get(4)?,
        created_at: r.get(5)?,
        updated_at: r.get(6)?,
        uid: r.get(7)?,
        deleted_at: r.get(8)?,
        pinned_at: r.get(9)?,
    })
}

pub fn list_notes(c: &Connection) -> rusqlite::Result<Vec<Note>> {
    let mut st = c.prepare(&format!(
        "SELECT {N_COLS} FROM notes WHERE deleted_at IS NULL ORDER BY pinned_at IS NULL, pinned_at DESC, updated_at DESC, id DESC"
    ))?;
    let rows = st.query_map([], row_note)?;
    rows.collect()
}

pub fn get_note(c: &Connection, id: i64) -> rusqlite::Result<Option<Note>> {
    c.query_row(
        &format!("SELECT {N_COLS} FROM notes WHERE id=?1 AND deleted_at IS NULL"),
        [id],
        row_note,
    )
    .optional()
}

pub fn upsert_note(c: &Connection, n: &Note) -> rusqlite::Result<i64> {
    let now = now_sec();
    if n.id > 0 {
        c.execute(
            "UPDATE notes SET title=?2, body=?3, tag=?4, reminder_id=?5,
                reminder_uid=(SELECT uid FROM reminders WHERE id=?5), updated_at=?6, ver=?7, dirty=1
             WHERE id=?1",
            params![n.id, n.title, n.body, n.tag, n.reminder_id, now, clock::next()],
        )?;
        Ok(n.id)
    } else {
        let uid = if n.uid.is_empty() { new_uid() } else { n.uid.clone() };
        c.execute(
            "INSERT INTO notes(title,body,tag,reminder_id,reminder_uid,created_at,updated_at,uid,ver,dirty)
             VALUES(?1,?2,?3,?4,(SELECT uid FROM reminders WHERE id=?4),?5,?5,?6,?7,1)",
            params![n.title, n.body, n.tag, n.reminder_id, now, uid, clock::next()],
        )?;
        Ok(c.last_insert_rowid())
    }
}

/// 置顶 / 取消置顶：不算改了内容，修改时间不变，只记一笔等着同步
pub fn pin_note(c: &Connection, id: i64, on: bool) -> rusqlite::Result<()> {
    let at = on.then(now_sec);
    c.execute(
        "UPDATE notes SET pinned_at=?2, ver=?3, dirty=1 WHERE id=?1 AND deleted_at IS NULL",
        params![id, at, clock::next()],
    )?;
    Ok(())
}

pub fn delete_note(c: &Connection, id: i64) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE notes SET deleted_at=?2, ver=?3, dirty=1 WHERE id=?1 AND deleted_at IS NULL",
        params![id, now_sec(), clock::next()],
    )?;
    Ok(())
}

pub fn restore_note(c: &Connection, id: i64) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE notes SET deleted_at=NULL, ver=?2, dirty=1 WHERE id=?1 AND deleted_at IS NOT NULL AND purged=0",
        params![id, clock::next()],
    )?;
    Ok(())
}

pub fn purge_note(c: &Connection, id: i64) -> rusqlite::Result<()> {
    // 笔记里的图片一起删；文件由每天的清理删掉
    c.execute(
        "UPDATE attachments SET purged=1, deleted_at=?2, ver=?3, dirty=1
         WHERE purged=0 AND note_uid=(SELECT uid FROM notes WHERE id=?1 AND deleted_at IS NOT NULL AND purged=0)",
        params![id, now_sec(), clock::next()],
    )?;
    c.execute(
        "UPDATE notes SET purged=1, title='', body='', tag=NULL, reminder_id=NULL, reminder_uid=NULL, ver=?2, dirty=1
         WHERE id=?1 AND deleted_at IS NOT NULL AND purged=0",
        params![id, clock::next()],
    )?;
    Ok(())
}

// ---------- 回收站 ----------

pub fn list_trash(c: &Connection) -> rusqlite::Result<Trash> {
    let reminders = query_reminders(
        c,
        "WHERE deleted_at IS NOT NULL AND purged=0 ORDER BY deleted_at DESC",
        [],
    )?;
    let mut st = c.prepare(&format!(
        "SELECT {N_COLS} FROM notes WHERE deleted_at IS NOT NULL AND purged=0 ORDER BY deleted_at DESC"
    ))?;
    let notes = st.query_map([], row_note)?.collect::<rusqlite::Result<_>>()?;
    Ok(Trash { reminders, notes })
}

/// 彻底删除回收站里在 `before` 之前删掉的东西；`before` 为 None 时清空整个回收站。返回删了几条。
pub fn purge_trash(c: &Connection, before: Option<&str>) -> rusqlite::Result<usize> {
    let before = before.unwrap_or("9999");
    let ids = |table: &str| -> rusqlite::Result<Vec<i64>> {
        let mut st = c.prepare(&format!(
            "SELECT id FROM {table} WHERE deleted_at IS NOT NULL AND purged=0 AND deleted_at < ?1"
        ))?;
        let rows = st.query_map([before], |r| r.get(0))?;
        rows.collect()
    };
    let (rs, ns) = (ids("reminders")?, ids("notes")?);
    let tx = c.unchecked_transaction()?;
    for id in &rs {
        purge_reminder(&tx, *id)?;
    }
    for id in &ns {
        purge_note(&tx, *id)?;
    }
    tx.commit()?;
    Ok(rs.len() + ns.len())
}

// ---------- 图片（附件） ----------

/// 附件文件放在数据目录的 attachments/ 下，文件名是 <uid>.<扩展名>
pub fn attachment_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("attachments")
}

/// 图片和文件附件单个最大多大（和手机网页、服务器的限制一致）
pub const MAX_ATTACHMENT: usize = 50 * 1024 * 1024;

/// 附件的编号会拼进文件名：本机生成的是 UUID（36 位带 -），手机发来的是「服务器上那一条的编号（32 位十六进制）-序号」。
/// 只认字母数字和 -，不长于 64
pub fn valid_att_uid(uid: &str) -> bool {
    (1..=64).contains(&uid.len()) && uid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// 附件存在本机的文件名 `<编号>.<扩展名>` 会拼进路径（别的电脑、网盘文件夹、导出包传来的也是）：
/// 只认字母数字 . - _，开头是字母数字，要有扩展名，不能有 ..，也不能是 Windows 的设备名（CON、NUL、COM1……）
pub fn valid_att_file(f: &str) -> bool {
    let Some((stem, _)) = f.split_once('.') else { return false };
    let device = |s: &str| {
        let s = s.to_ascii_uppercase();
        matches!(s.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
            || ((s.starts_with("COM") || s.starts_with("LPT")) && s.len() == 4 && s.as_bytes()[3].is_ascii_digit())
    };
    f.len() <= 120
        && f.starts_with(|c: char| c.is_ascii_alphanumeric())
        && !f.ends_with('.')
        && !f.contains("..")
        && f.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        && !device(stem)
}

/// 附件文件在本机的完整路径：文件名不合规矩、拼出来不是正好在附件文件夹里面的，回 None
pub fn att_path(att_dir: &Path, file: &str) -> Option<PathBuf> {
    if !valid_att_file(file) {
        return None;
    }
    let p = att_dir.join(file);
    (p.parent() == Some(att_dir) && p.file_name().and_then(|n| n.to_str()) == Some(file)).then_some(p)
}

/// 从别处来的文件（手机发来的、别的电脑同步来的、导出包里的）写好后标上「来自网络」（Zone.Identifier 备用数据流，ZoneId=3），
/// 双击打开时 Windows 的 SmartScreen、Office 的受保护视图会把它当下载的文件看。不是 NTFS（U 盘、网盘的特殊文件夹）标不上就算了
pub fn mark_from_web(path: &Path) {
    if cfg!(windows) {
        let mut ads = path.as_os_str().to_owned();
        ads.push(":Zone.Identifier");
        let _ = std::fs::write(PathBuf::from(ads), "[ZoneTransfer]\r\nZoneId=3\r\n");
    }
}

/// 这个文件标着「来自网络」吗（复制一份用原文件名打开时，副本也要标上）
pub fn from_web(path: &Path) -> bool {
    let mut ads = path.as_os_str().to_owned();
    ads.push(":Zone.Identifier");
    cfg!(windows) && std::fs::metadata(PathBuf::from(ads)).is_ok()
}

/// 存在本机的文件名用的扩展名：图片按类型，别的文件沿用原文件名的扩展名（只留字母数字）
fn ext_for(mime: &str, name: &str) -> String {
    let known = match mime {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/heic" => "heic",
        "image/bmp" => "bmp",
        _ => "",
    };
    if !known.is_empty() {
        return known.into();
    }
    let ext: String = name
        .rsplit_once('.')
        .map(|(_, e)| e.chars().filter(|c| c.is_ascii_alphanumeric()).take(10).collect::<String>())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if ext.is_empty() { "bin".into() } else { ext }
}

/// 存一个附件（图片或文件），返回存在本机的文件名 `<编号>.<扩展名>`。
/// 文件在笔记里写成 `[原文件名](att:<文件名>)`，图片写成 `![](att:<文件名>)`。
/// `uid` 一般不填；手机发来的用固定的编号，几台电脑同时取到也只会是同一个
pub fn save_file(
    c: &Connection,
    data_dir: &Path,
    mime: &str,
    name: &str,
    bytes: &[u8],
    note_uid: Option<&str>,
    uid: Option<&str>,
) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    if bytes.len() > MAX_ATTACHMENT {
        return Err("文件不能超过 50 MB".into());
    }
    let name = clean_file_name(name);
    let uid = match uid {
        Some(u) if valid_att_uid(u) => u.to_string(),
        Some(_) => return Err("附件的编号不对，没有存".into()),
        None => new_uid(),
    };
    let file = format!("{uid}.{}", ext_for(mime, &name));
    let dir = attachment_dir(data_dir);
    let path = att_path(&dir, &file).ok_or("附件的文件名不对，没有存")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(&path, bytes).map_err(|e| format!("存不下：{e}"))?;
    let sha: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
    c.execute(
        "INSERT OR IGNORE INTO attachments(uid, file, mime, size, sha256, note_uid, created_at, ver, name)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![uid, file, mime, bytes.len() as i64, sha, note_uid, now_sec(), clock::next(), name],
    )
    .map_err(|e| e.to_string())?;
    Ok(file)
}

/// 原文件名只留最后一段，去掉 Windows 文件名里不能用的字符和 Markdown 会误会的方括号
pub fn clean_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let s: String = base
        .chars()
        .map(|c| if c.is_control() || "<>:\"|?*[]".contains(c) { '_' } else { c })
        .collect();
    let s = s.trim().trim_end_matches('.').to_string();
    s.chars().take(120).collect()
}

/// 一个附件的情况，给界面显示文件卡片、决定点开时要不要先下载
#[derive(Serialize, Clone, Debug)]
pub struct AttachmentInfo {
    pub uid: String,
    pub file: String,
    pub name: String,
    pub mime: String,
    pub size: i64,
    /// 原文件在不在本机
    pub local: bool,
    /// 叮咪同步的套餐不传它（太大）：原文件只在这台设备上（设备编号）；是这台电脑自己的话界面说「只在这台电脑上」
    pub only_on: Option<String>,
}

pub fn list_attachments(c: &Connection) -> rusqlite::Result<Vec<AttachmentInfo>> {
    let mut st = c.prepare("SELECT uid, file, name, mime, size, local_ok, only_on FROM attachments WHERE purged=0")?;
    let rows = st.query_map([], |r| {
        Ok(AttachmentInfo {
            uid: r.get(0)?,
            file: r.get(1)?,
            name: r.get(2)?,
            mime: r.get(3)?,
            size: r.get(4)?,
            local: r.get::<_, i64>(5)? != 0,
            only_on: r.get(6)?,
        })
    })?;
    rows.collect()
}

pub fn get_attachment(c: &Connection, file: &str) -> rusqlite::Result<Option<AttachmentInfo>> {
    Ok(list_attachments(c)?.into_iter().find(|a| a.file == file))
}

/// 把这些图片归到一条笔记下（彻底删除笔记时会一起删掉）
pub fn attach_to_note(c: &Connection, files: &[String], note_uid: &str) -> rusqlite::Result<()> {
    for f in files {
        c.execute(
            "UPDATE attachments SET note_uid=?2, ver=?3, dirty=1 WHERE file=?1",
            params![f, note_uid, clock::next()],
        )?;
    }
    Ok(())
}

/// 不属于任何笔记的这些图片标记为彻底删除
fn drop_loose_attachments(c: &Connection, files: &[String]) -> rusqlite::Result<()> {
    for f in files {
        c.execute(
            "UPDATE attachments SET purged=1, deleted_at=?2, ver=?3, dirty=1 WHERE file=?1 AND note_uid IS NULL AND purged=0",
            params![f, now_sec(), clock::next()],
        )?;
    }
    Ok(())
}

/// 删掉已经彻底删除的图片文件，返回删了几个
pub fn clean_attachments(c: &Connection, data_dir: &Path) -> rusqlite::Result<usize> {
    let files: Vec<String> = {
        let mut st = c.prepare("SELECT file FROM attachments WHERE purged=1 AND local_ok=1")?;
        let rows = st.query_map([], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let dir = attachment_dir(data_dir);
    for f in &files {
        let _ = std::fs::remove_file(dir.join(f));
        c.execute("UPDATE attachments SET local_ok=0 WHERE file=?1", [f])?;
    }
    Ok(files.len())
}

// ---------- 手机收件箱 ----------

pub const INBOX_PENDING: i64 = 0;
pub const INBOX_DONE: i64 = 1;
pub const INBOX_DELETED: i64 = 2;

#[derive(Serialize, Clone, Debug)]
pub struct InboxEntry {
    pub id: i64,
    pub uid: String,
    pub phone: String,
    pub received_at: String,
    pub sent_at: Option<String>,
    pub text: String,
    /// 图片的文件名
    pub images: Vec<String>,
}

fn row_inbox(r: &Row) -> rusqlite::Result<InboxEntry> {
    let images: String = r.get(6)?;
    Ok(InboxEntry {
        id: r.get(0)?,
        uid: r.get(1)?,
        phone: r.get(2)?,
        received_at: r.get(3)?,
        sent_at: r.get(4)?,
        text: r.get(5)?,
        images: serde_json::from_str(&images).unwrap_or_default(),
    })
}

const I_COLS: &str = "id, uid, phone, received_at, sent_at, text, images";

pub fn has_inbox(c: &Connection, uid: &str) -> bool {
    c.query_row("SELECT 1 FROM inbox WHERE uid=?1", [uid], |_| Ok(())).is_ok()
}

pub fn add_inbox(
    c: &Connection,
    uid: &str,
    phone: &str,
    sent_at: Option<&str>,
    text: &str,
    images: &[String],
) -> rusqlite::Result<()> {
    let images = serde_json::to_string(images).unwrap_or_else(|_| "[]".into());
    c.execute(
        "INSERT OR IGNORE INTO inbox(uid, phone, received_at, sent_at, text, images, ver)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![uid, phone, now_sec(), sent_at, text, images, clock::next()],
    )?;
    Ok(())
}

/// 还没处理的
pub fn list_inbox(c: &Connection) -> rusqlite::Result<Vec<InboxEntry>> {
    let mut st = c.prepare(&format!("SELECT {I_COLS} FROM inbox WHERE status=?1 ORDER BY received_at, id"))?;
    let rows = st.query_map([INBOX_PENDING], row_inbox)?;
    rows.collect()
}

pub fn get_inbox(c: &Connection, id: i64) -> rusqlite::Result<Option<InboxEntry>> {
    c.query_row(&format!("SELECT {I_COLS} FROM inbox WHERE id=?1"), [id], row_inbox).optional()
}

pub fn set_inbox_status(c: &Connection, id: i64, status: i64) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE inbox SET status=?2, ver=?3, dirty=1 WHERE id=?1",
        params![id, status, clock::next()],
    )?;
    Ok(())
}

/// 删掉一条：没存进笔记的图片也一起删
pub fn delete_inbox(c: &Connection, id: i64) -> rusqlite::Result<()> {
    if let Some(e) = get_inbox(c, id)? {
        drop_loose_attachments(c, &e.images)?;
    }
    set_inbox_status(c, id, INBOX_DELETED)
}

// ---------- 设备 ----------

#[derive(Serialize, Clone, Debug)]
pub struct Device {
    pub uid: String,
    pub name: String,
    /// 是不是这台电脑
    pub me: bool,
}

/// 记下这台电脑的名字（同步给别的电脑，指定设备提醒时显示）；名字没变就不动
pub fn set_own_device(c: &Connection, device: &str, name: &str) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO devices(uid, name, ver) VALUES(?1, ?2, ?3)
         ON CONFLICT(uid) DO UPDATE SET name=excluded.name, ver=excluded.ver, dirty=1 WHERE devices.name != excluded.name",
        params![device, name, clock::next()],
    )?;
    Ok(())
}

/// 一台设备的名字（同步过来的设备列表里有的话）
pub fn device_name(c: &Connection, uid: &str) -> Option<String> {
    c.query_row("SELECT name FROM devices WHERE uid=?1", [uid], |r| r.get(0)).ok()
}

pub fn list_devices(c: &Connection, me: &str) -> rusqlite::Result<Vec<Device>> {
    let mut st = c.prepare("SELECT uid, name FROM devices ORDER BY name")?;
    let rows = st.query_map([], |r| {
        let uid: String = r.get(0)?;
        Ok(Device { me: uid == me, uid, name: r.get(1)? })
    })?;
    rows.collect()
}

/// 刚配对到一个空间时，本机所有数据都要传上去（和服务器上已有的合在一起）
pub fn mark_all_dirty(c: &Connection) -> rusqlite::Result<()> {
    c.execute_batch(
        "UPDATE reminders SET dirty=1; UPDATE notes SET dirty=1; UPDATE attachments SET dirty=1;
         UPDATE inbox SET dirty=1; UPDATE devices SET dirty=1; UPDATE shared SET dirty=1;",
    )
}

/// 用服务器上的数据替换本机：清空所有同步的内容（调用前先备份）。只属于本机的设置、配对信息不动
pub fn wipe_synced(c: &Connection) -> rusqlite::Result<()> {
    c.execute_batch(
        "DELETE FROM reminders; DELETE FROM notes; DELETE FROM attachments; DELETE FROM inbox;
         DELETE FROM devices; DELETE FROM shared;",
    )
}

// ---------- 几台电脑共用的单条记录（设置、今天收尾过没有） ----------

pub fn shared_get(c: &Connection, uid: &str) -> Option<String> {
    c.query_row("SELECT value FROM shared WHERE uid=?1", [uid], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
}

/// 本机改了一条共用记录；内容没变就不动。返回有没有改
pub fn shared_set(c: &Connection, uid: &str, value: &str) -> rusqlite::Result<bool> {
    let n = c.execute(
        "INSERT INTO shared(uid, value, ver) VALUES(?1, ?2, ?3)
         ON CONFLICT(uid) DO UPDATE SET value=excluded.value, ver=excluded.ver, dirty=1 WHERE shared.value != excluded.value",
        params![uid, value, clock::next()],
    )?;
    Ok(n > 0)
}

/// 还没有这条时用指定的版本号建一条（升级时补建用，版本号很小，别的电脑上真改过的会盖过它）
pub fn shared_init(c: &Connection, uid: &str, value: &str, ver: &str) -> rusqlite::Result<()> {
    c.execute(
        "INSERT OR IGNORE INTO shared(uid, value, ver) VALUES(?1, ?2, ?3)",
        params![uid, value, ver],
    )?;
    Ok(())
}

// ---------- 托盘提示、每天收尾 ----------

/// 这台电脑上接下来要提醒的一条：(时间, 标题)
pub fn next_reminder(c: &Connection, now: &str, device: &str) -> Option<(String, String)> {
    c.query_row(
        "SELECT COALESCE(snoozed_until, due_at) AS f, title FROM reminders
         WHERE done=0 AND deleted_at IS NULL AND f > ?1
           AND (only_on='[]' OR EXISTS(SELECT 1 FROM json_each(only_on) WHERE value=?2))
         ORDER BY f LIMIT 1",
        params![now, device],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .ok()
    .flatten()
}

/// 今天完成了几件、还剩几件（今天和之前没处理完的）
pub fn today_summary(c: &Connection, day: &str) -> (i64, i64) {
    let count = |sql: &str| c.query_row(sql, [day], |r| r.get::<_, i64>(0)).unwrap_or(0);
    let done = count(
        "SELECT COUNT(*) FROM reminders WHERE deleted_at IS NULL
           AND ((done=1 AND substr(completed_at,1,10)=?1) OR substr(last_done_at,1,10)=?1)",
    );
    let left = count(
        "SELECT COUNT(*) FROM reminders WHERE deleted_at IS NULL AND done=0
           AND substr(COALESCE(snoozed_until, due_at),1,10) <= ?1",
    );
    (done, left)
}

// ---------- 笔记里的待办 ----------

/// 把正文里第一行文字是 `todo` 的待办勾上（done）或取消勾选，返回改好的正文；找不到就是 None
pub fn toggle_todo_line(body: &str, todo: &str, done: bool) -> Option<String> {
    let want = todo.trim();
    let mut lines: Vec<String> = body.split('\n').map(str::to_string).collect();
    for line in lines.iter_mut() {
        let lead = line.len() - line.trim_start().len();
        let rest = &line[lead..];
        if !(rest.starts_with("- ") || rest.starts_with("* ")) {
            continue;
        }
        let after_dash = rest[2..].trim_start();
        let mark_at = lead + (rest.len() - after_dash.len());
        let checked = after_dash.starts_with("[x]") || after_dash.starts_with("[X]");
        let open = after_dash.starts_with("[ ]");
        if !(checked || open) || after_dash[3..].trim() != want {
            continue;
        }
        if checked == done {
            return None;
        }
        line.replace_range(mark_at..mark_at + 3, if done { "[x]" } else { "[ ]" });
        return Some(lines.join("\n"));
    }
    None
}

/// 提醒完成（或取消完成）时，把它来自的那条笔记里的待办跟着勾上（或取消）。返回笔记有没有变
pub fn tick_todo(c: &Connection, note_uid: &str, todo: &str, done: bool) -> rusqlite::Result<bool> {
    let row: Option<(i64, String)> = c
        .query_row("SELECT id, body FROM notes WHERE uid=?1 AND deleted_at IS NULL", [note_uid], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?;
    let Some((id, body)) = row else { return Ok(false) };
    let Some(body) = toggle_todo_line(&body, todo, done) else { return Ok(false) };
    c.execute(
        "UPDATE notes SET body=?2, updated_at=?3, ver=?4, dirty=1 WHERE id=?1",
        params![id, body, now_sec(), clock::next()],
    )?;
    Ok(true)
}

// ---------- 备份 ----------

/// 做一次性的整库备份 backups/<name>.db（比如用服务器上的数据替换本机之前）
pub fn snapshot(c: &Connection, dir: &Path, name: &str) -> Result<PathBuf, String> {
    let bdir = dir.join("backups");
    std::fs::create_dir_all(&bdir).map_err(|e| e.to_string())?;
    let file = bdir.join(format!("{name}.db"));
    let _ = std::fs::remove_file(&file);
    c.execute("VACUUM INTO ?1", [file.to_string_lossy().to_string()]).map_err(|e| e.to_string())?;
    Ok(file)
}

/// 把整个数据库备份成 backups/dingmi-<day>.db，只留最近 `keep` 份
pub fn backup(c: &Connection, dir: &Path, day: &str, keep: usize) -> Result<(), String> {
    let bdir = dir.join("backups");
    std::fs::create_dir_all(&bdir).map_err(|e| e.to_string())?;
    let file = bdir.join(format!("dingmi-{day}.db"));
    let _ = std::fs::remove_file(&file);
    let path = file.to_string_lossy().to_string();
    c.execute("VACUUM INTO ?1", [&path]).map_err(|e| e.to_string())?;
    let mut old: Vec<_> = std::fs::read_dir(&bdir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("dingmi-") && n.ends_with(".db"))
                .unwrap_or(false)
        })
        .collect();
    // 文件名里带日期，按名字排就是按时间排
    old.sort();
    let extra = old.len().saturating_sub(keep);
    for p in old.into_iter().take(extra) {
        let _ = std::fs::remove_file(p);
    }
    Ok(())
}

pub fn kv_get(c: &Connection, key: &str) -> Option<String> {
    c.query_row("SELECT value FROM kv WHERE key=?1", [key], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
}

pub fn kv_set(c: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn kv_del(c: &Connection, key: &str) -> rusqlite::Result<()> {
    c.execute("DELETE FROM kv WHERE key=?1", [key])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("dingmi-test-{name}-{}", Uuid::new_v4().simple()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn reminder(title: &str, due: &str) -> Reminder {
        Reminder { title: title.into(), due_at: due.into(), ..Default::default() }
    }

    #[test]
    fn sets_aside_v0_database() {
        let dir = temp_dir("v0");
        {
            let c = Connection::open(dir.join("dingmi.db")).unwrap();
            c.execute_batch(
                "PRAGMA journal_mode=WAL;
                 CREATE TABLE reminders(id INTEGER PRIMARY KEY, title TEXT NOT NULL, due_at TEXT NOT NULL, created_at TEXT NOT NULL);
                 CREATE TABLE kv(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO reminders(title, due_at, created_at) VALUES('试用时记的', '2026-01-01 10:00', '2026-01-01 09:00:00');
                 INSERT INTO kv VALUES('settings', '{\"ai_key\":\"sk-test\"}');",
            )
            .unwrap();
        }
        let c = open(&dir).unwrap();
        assert!(list_reminders(&c).unwrap().is_empty());
        assert_eq!(kv_get(&c, "settings").as_deref(), Some("{\"ai_key\":\"sk-test\"}"));
        upsert_reminder(&c, &reminder("新的", "2099-01-01 09:00")).unwrap();
        let kept: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with("dingmi-v0.1-") && n.ends_with(".db"))
            .collect();
        assert_eq!(kept.len(), 1);
        drop(c);
        // 新库再打开不会被当成旧版挪走
        let c = open(&dir).unwrap();
        assert_eq!(list_reminders(&c).unwrap().len(), 1);
    }

    #[test]
    fn new_records_get_uid_and_version() {
        let dir = temp_dir("uid");
        let c = open(&dir).unwrap();
        let rid = upsert_reminder(&c, &reminder("喝水", "2099-01-01 10:00")).unwrap();
        let nid = upsert_note(&c, &Note { title: "预算".into(), reminder_id: Some(rid), ..Default::default() }).unwrap();
        let r = get_reminder(&c, rid).unwrap().unwrap();
        assert_eq!(r.uid.len(), 36);
        assert!(r.only_on.is_empty());
        assert_eq!(r.updated_at, r.created_at);
        let (note_uid, rem_uid, ver): (String, String, String) = c
            .query_row("SELECT uid, reminder_uid, ver FROM notes WHERE id=?1", [nid], |x| {
                Ok((x.get(0)?, x.get(1)?, x.get(2)?))
            })
            .unwrap();
        assert_eq!(note_uid.len(), 36);
        assert_eq!(rem_uid, r.uid);
        drop(c);
        // 重新打开后，新的版本号仍然比库里已有的大
        let c = open(&dir).unwrap();
        upsert_note(&c, &get_note(&c, nid).unwrap().unwrap()).unwrap();
        let ver2: String = c.query_row("SELECT ver FROM notes WHERE id=?1", [nid], |x| x.get(0)).unwrap();
        assert!(ver2 > ver);
    }

    /// 附件编号、文件名拼路径前的白名单；拼出来不在附件文件夹里的不认
    #[test]
    fn attachment_names() {
        for ok in ["019a2b3c-0000-7000-8000-000000000001.png", "0123456789abcdef0123456789abcdef-00.pdf", "a_b.tar.gz"] {
            assert!(valid_att_file(ok), "{ok}");
        }
        for bad in [
            "",
            "noext",
            r"..\..\Start Menu\Programs\Startup\x.exe",
            "../x.png",
            r"C:\x.png",
            "a..png",
            ".hidden.png",
            "a.png.",
            "a.png:Zone.Identifier",
            "CON.txt",
            "nul.png",
            "COM1.pdf",
            "a b.png",
            "图片.png",
        ] {
            assert!(!valid_att_file(bad), "{bad}");
        }
        assert!(valid_att_uid("0123456789abcdef0123456789abcdef-00") && valid_att_uid("019a2b3c-0000-7000-8000-000000000001"));
        for bad in ["", r"..\x", "a/b", "a.b", &"a".repeat(65)] {
            assert!(!valid_att_uid(bad), "{bad}");
        }
        let dir = Path::new(r"C:\data\attachments");
        assert_eq!(att_path(dir, "a.png"), Some(dir.join("a.png")));
        assert_eq!(att_path(dir, r"..\a.png"), None);
    }

    /// 攻击：拿带 ..\ 的编号存附件（修之前收件箱就是这样把服务器给的编号交给 save_file 的），文件会写到附件文件夹外面。
    /// 现在编号不对直接不存
    #[test]
    fn save_file_refuses_escaping_uid() {
        let dir = temp_dir("escape");
        let c = open(&dir).unwrap();
        for uid in [r"..\..\escaped", "../escaped", r"C:\escaped", "x.exe:ads"] {
            assert!(save_file(&c, &dir, "application/octet-stream", "x.exe", b"MZ", None, Some(uid)).is_err(), "{uid}");
        }
        assert!(!dir.join("escaped.exe").exists() && !dir.parent().unwrap().join("escaped.exe").exists());
        let n: i64 = c.query_row("SELECT COUNT(*) FROM attachments", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
        // 正常的编号照常存
        let f = save_file(&c, &dir, "image/png", "", b"png", None, Some("0123456789abcdef0123456789abcdef-00")).unwrap();
        assert_eq!(f, "0123456789abcdef0123456789abcdef-00.png");
    }

    #[test]
    fn inbox_and_attachments() {
        let dir = temp_dir("inbox");
        let c = open(&dir).unwrap();
        let a = save_file(&c, &dir, "image/png", "", b"png-bytes", None, None).unwrap();
        let b = save_file(&c, &dir, "image/jpeg", "", b"jpg-bytes", None, None).unwrap();
        assert!(a.ends_with(".png") && b.ends_with(".jpg"));
        assert!(attachment_dir(&dir).join(&a).exists());
        // 文件：扩展名沿用原文件名，原文件名去掉路径和不能用的字符；超过 50 MB 不收
        let f = save_file(&c, &dir, "application/pdf", r"C:\x\报价 v2.PDF", b"%PDF", None, None).unwrap();
        assert!(f.ends_with(".pdf"));
        let info = get_attachment(&c, &f).unwrap().unwrap();
        assert_eq!((info.name.as_str(), info.size, info.local), ("报价 v2.PDF", 4, true));
        assert!(save_file(&c, &dir, "", "没有扩展名", b"x", None, None).unwrap().ends_with(".bin"));
        assert!(save_file(&c, &dir, "", "坏.e x/e", b"x", None, None).unwrap().ends_with(".bin"));
        assert!(save_file(&c, &dir, "", "a.zip", &vec![0u8; MAX_ATTACHMENT + 1], None, None).is_err());
        assert_eq!(clean_file_name("a<b>:c?.txt."), "a_b__c_.txt");

        add_inbox(&c, "srv1", "iPhone", Some("2026-10-31 10:00:00"), "看看这个", &[a.clone(), b.clone()]).unwrap();
        // 同一条取两次只存一份
        add_inbox(&c, "srv1", "iPhone", None, "看看这个", &[]).unwrap();
        let list = list_inbox(&c).unwrap();
        assert_eq!(list.len(), 1);
        assert!(has_inbox(&c, "srv1"));
        assert_eq!(list[0].images, vec![a.clone(), b.clone()]);

        // 第一张存进笔记，删掉这条后，只有没进笔记的第二张被清掉
        let nid = upsert_note(&c, &Note { title: "截图".into(), ..Default::default() }).unwrap();
        let note_uid = get_note(&c, nid).unwrap().unwrap().uid;
        attach_to_note(&c, std::slice::from_ref(&a), &note_uid).unwrap();
        delete_inbox(&c, list[0].id).unwrap();
        assert!(list_inbox(&c).unwrap().is_empty());
        assert_eq!(clean_attachments(&c, &dir).unwrap(), 1);
        assert!(attachment_dir(&dir).join(&a).exists());
        assert!(!attachment_dir(&dir).join(&b).exists());

        // 彻底删除笔记时，它的图片也跟着删
        delete_note(&c, nid).unwrap();
        purge_note(&c, nid).unwrap();
        assert_eq!(clean_attachments(&c, &dir).unwrap(), 1);
        assert!(!attachment_dir(&dir).join(&a).exists());
    }

    #[test]
    fn pinned_notes_first() {
        let dir = temp_dir("pin");
        let c = open(&dir).unwrap();
        let a = upsert_note(&c, &Note { title: "早的".into(), ..Default::default() }).unwrap();
        upsert_note(&c, &Note { title: "新的".into(), ..Default::default() }).unwrap();
        c.execute("UPDATE notes SET updated_at='2000-01-01 00:00:00', dirty=0 WHERE id=?1", [a]).unwrap();
        assert_eq!(list_notes(&c).unwrap()[0].title, "新的");
        // 置顶：排到最前面，修改时间不变，等着同步
        pin_note(&c, a, true).unwrap();
        let first = &list_notes(&c).unwrap()[0];
        assert_eq!((first.title.as_str(), first.updated_at.as_str()), ("早的", "2000-01-01 00:00:00"));
        assert!(first.pinned_at.is_some());
        let dirty: i64 = c.query_row("SELECT dirty FROM notes WHERE id=?1", [a], |r| r.get(0)).unwrap();
        assert_eq!(dirty, 1);
        // 改内容不会把置顶弄丢
        upsert_note(&c, &Note { id: a, title: "早的".into(), body: "改了".into(), ..Default::default() }).unwrap();
        assert!(get_note(&c, a).unwrap().unwrap().pinned_at.is_some());
        pin_note(&c, a, false).unwrap();
        assert!(get_note(&c, a).unwrap().unwrap().pinned_at.is_none());
    }

    #[test]
    fn trash_roundtrip() {
        let dir = temp_dir("trash");
        let c = open(&dir).unwrap();
        let rid = upsert_reminder(&c, &reminder("交报销", "2099-01-01 09:00")).unwrap();
        let nid = upsert_note(&c, &Note { title: "发票".into(), reminder_id: Some(rid), ..Default::default() }).unwrap();
        let ver_before: String = c.query_row("SELECT ver FROM reminders WHERE id=?1", [rid], |x| x.get(0)).unwrap();

        delete_reminder(&c, rid).unwrap();
        delete_note(&c, nid).unwrap();
        assert!(list_reminders(&c).unwrap().is_empty());
        assert!(list_notes(&c).unwrap().is_empty());
        assert!(get_reminder(&c, rid).unwrap().is_none());
        let t = list_trash(&c).unwrap();
        assert_eq!((t.reminders.len(), t.notes.len()), (1, 1));
        let ver_after: String = c.query_row("SELECT ver FROM reminders WHERE id=?1", [rid], |x| x.get(0)).unwrap();
        assert!(ver_after > ver_before);

        restore_reminder(&c, rid).unwrap();
        restore_note(&c, nid).unwrap();
        assert_eq!(list_reminders(&c).unwrap().len(), 1);
        assert_eq!(get_note(&c, nid).unwrap().unwrap().reminder_id, Some(rid));

        delete_reminder(&c, rid).unwrap();
        assert_eq!(purge_trash(&c, None).unwrap(), 1);
        assert!(list_trash(&c).unwrap().reminders.is_empty());
        let note = get_note(&c, nid).unwrap().unwrap();
        assert_eq!(note.reminder_id, None);
        let title: String = c.query_row("SELECT title FROM reminders WHERE id=?1", [rid], |x| x.get(0)).unwrap();
        assert_eq!(title, "");
    }

    #[test]
    fn restore_past_one_off_does_not_pop() {
        let dir = temp_dir("restore");
        let c = open(&dir).unwrap();
        let id = upsert_reminder(&c, &reminder("过期的", "2000-01-01 09:00")).unwrap();
        delete_reminder(&c, id).unwrap();
        restore_reminder(&c, id).unwrap();
        assert_eq!(get_reminder(&c, id).unwrap().unwrap().state, ST_SEEN);
        assert!(due_now(&c, &now_min(), "any").unwrap().is_empty());
    }

    #[test]
    fn only_on_limits_devices() {
        let dir = temp_dir("only-on");
        let c = open(&dir).unwrap();
        let mut r = reminder("到公司交报销", "2000-01-01 09:00");
        r.only_on = vec!["office".into()];
        upsert_reminder(&c, &r).unwrap();
        upsert_reminder(&c, &reminder("喝水", "2000-01-01 09:00")).unwrap();
        assert_eq!(due_now(&c, &now_min(), "office").unwrap().len(), 2);
        assert_eq!(due_now(&c, &now_min(), "home").unwrap().len(), 1);
    }

    #[test]
    fn changed_on_other_device() {
        let dir = temp_dir("changed-on");
        let c = open(&dir).unwrap();
        let me = device_id(&c).unwrap();
        let id = upsert_reminder(&c, &reminder("交周报", "2099-01-01 09:00")).unwrap();
        // 版本号里的设备直接写好：版本时钟是全局的，别的测试同时打开别的库会把它换掉
        c.execute("UPDATE reminders SET ver=?2 WHERE id=?1", params![id, format!("9999999999999-0000-{me}")]).unwrap();
        assert_eq!(reminder_changed_on(&c, id, &me).unwrap(), None);
        c.execute("UPDATE reminders SET ver='9999999999999-0000-office' WHERE id=?1", [id]).unwrap();
        assert_eq!(reminder_changed_on(&c, id, &me).unwrap(), Some(String::new()));
        c.execute("INSERT INTO devices(uid, name, ver) VALUES('office', '公司电脑', '1')", []).unwrap();
        assert_eq!(reminder_changed_on(&c, id, &me).unwrap().as_deref(), Some("公司电脑"));
        assert_eq!(reminder_changed_on(&c, id + 99, &me).unwrap(), None);
    }

    #[test]
    fn purge_trash_respects_cutoff() {
        let dir = temp_dir("cutoff");
        let c = open(&dir).unwrap();
        let old = upsert_reminder(&c, &reminder("旧的", "2099-01-01 09:00")).unwrap();
        let new = upsert_reminder(&c, &reminder("新的", "2099-01-01 09:00")).unwrap();
        delete_reminder(&c, old).unwrap();
        delete_reminder(&c, new).unwrap();
        c.execute("UPDATE reminders SET deleted_at='2000-01-01 00:00:00' WHERE id=?1", [old]).unwrap();
        assert_eq!(purge_trash(&c, Some("2020-01-01 00:00:00")).unwrap(), 1);
        let left = list_trash(&c).unwrap().reminders;
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, new);
    }

    #[test]
    fn todo_lines() {
        let body = "# 周末\n- [ ] 买菜\n  - [ ] 交电费 \n- [x] 洗车\n* [ ] 买菜";
        let ticked = toggle_todo_line(body, "交电费", true).unwrap();
        assert_eq!(ticked, "# 周末\n- [ ] 买菜\n  - [x] 交电费 \n- [x] 洗车\n* [ ] 买菜");
        // 同名的只改第一行；已经是这个状态、找不到的都不动
        assert!(toggle_todo_line(&ticked, "买菜", true).unwrap().starts_with("# 周末\n- [x] 买菜"));
        assert_eq!(toggle_todo_line(&ticked, "交电费", true), None);
        assert_eq!(toggle_todo_line(body, "不存在", true), None);
        assert_eq!(toggle_todo_line(body, "洗车", false).unwrap().lines().nth(3), Some("- [ ] 洗车"));
    }

    #[test]
    fn done_reminder_ticks_its_todo() {
        let dir = temp_dir("todo");
        let c = open(&dir).unwrap();
        let nid = upsert_note(&c, &Note { title: "周末".into(), body: "- [ ] 交电费\n- [ ] 买菜".into(), ..Default::default() }).unwrap();
        let note_uid = get_note(&c, nid).unwrap().unwrap().uid;
        c.execute("UPDATE notes SET dirty=0", []).unwrap();
        assert!(tick_todo(&c, &note_uid, "交电费", true).unwrap());
        let n = get_note(&c, nid).unwrap().unwrap();
        assert_eq!(n.body, "- [x] 交电费\n- [ ] 买菜");
        let dirty: i64 = c.query_row("SELECT dirty FROM notes WHERE id=?1", [nid], |r| r.get(0)).unwrap();
        assert_eq!(dirty, 1);
        assert!(tick_todo(&c, &note_uid, "交电费", false).unwrap());
        assert!(!tick_todo(&c, "没有这条", "交电费", true).unwrap());
        // 提醒上记着来自哪条待办
        let mut r = reminder("交电费", "2099-01-01 09:00");
        r.note_uid = Some(note_uid.clone());
        r.todo = Some("交电费".into());
        let rid = upsert_reminder(&c, &r).unwrap();
        let got = get_reminder(&c, rid).unwrap().unwrap();
        assert_eq!((got.note_uid.as_deref(), got.todo.as_deref()), (Some(note_uid.as_str()), Some("交电费")));
    }

    #[test]
    fn summary_and_next() {
        let dir = temp_dir("summary");
        let c = open(&dir).unwrap();
        let day = "2026-10-31";
        upsert_reminder(&c, &reminder("早上的", "2026-10-31 09:00")).unwrap();
        upsert_reminder(&c, &reminder("昨天没做", "2026-10-30 09:00")).unwrap();
        upsert_reminder(&c, &reminder("明天的", "2026-11-01 09:00")).unwrap();
        let mut d = reminder("做完了", "2026-10-31 08:00");
        d.done = true;
        d.completed_at = Some("2026-10-31 08:05".into());
        upsert_reminder(&c, &d).unwrap();
        assert_eq!(today_summary(&c, day), (1, 2));
        assert_eq!(next_reminder(&c, "2026-10-31 10:00", "me"), Some(("2026-11-01 09:00".into(), "明天的".into())));
        let mut other = reminder("只在公司", "2026-10-31 11:00");
        other.only_on = vec!["office".into()];
        upsert_reminder(&c, &other).unwrap();
        assert_eq!(next_reminder(&c, "2026-10-31 10:00", "home").unwrap().1, "明天的");
        assert_eq!(next_reminder(&c, "2026-10-31 10:00", "office").unwrap().1, "只在公司");
    }

    #[test]
    fn adds_new_columns_to_existing_db() {
        let dir = temp_dir("cols");
        drop(open(&dir).unwrap());
        {
            // 模拟加列之前装的版本建的库
            let c = Connection::open(dir.join("dingmi.db")).unwrap();
            c.execute_batch("ALTER TABLE reminders DROP COLUMN note_uid; ALTER TABLE reminders DROP COLUMN todo;").unwrap();
        }
        let c = open(&dir).unwrap();
        let id = upsert_reminder(&c, &reminder("喝水", "2099-01-01 09:00")).unwrap();
        assert_eq!(get_reminder(&c, id).unwrap().unwrap().note_uid, None);
    }

    #[test]
    fn backup_keeps_latest() {
        let dir = temp_dir("backup");
        let c = open(&dir).unwrap();
        for d in ["2026-01-01", "2026-01-02", "2026-01-03"] {
            backup(&c, &dir, d, 2).unwrap();
        }
        let mut names: Vec<String> = std::fs::read_dir(dir.join("backups"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, ["dingmi-2026-01-02.db", "dingmi-2026-01-03.db"]);
    }
}
