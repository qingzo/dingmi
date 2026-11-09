//! 电脑之间同步的记录：本机改过哪些要上传、别的电脑传来的怎么合并进来。只碰数据库，方便测试。
//!
//! 每条记录上传时是 {"k": 类型, "d": 内容} 的 JSON，再用数据密钥加密（附加数据是 "编号|版本"）。
//! 合并规则：版本号（见 clock.rs）大的为准，整条覆盖；笔记两边都改过、本机的还没上传时，
//! 先把本机的另存成「（冲突副本）」再覆盖。提醒的弹窗状态只属于本机，不同步。

use crate::db::{self, Note, Repeat};
use crate::settings;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SReminder {
    pub uid: String,
    pub title: String,
    pub detail: String,
    pub tag: Option<String>,
    pub urgent: bool,
    pub due_at: String,
    pub repeat: Repeat,
    pub snoozed_until: Option<String>,
    pub done: bool,
    pub snooze_count: i64,
    pub last_done_at: Option<String>,
    pub completed_at: Option<String>,
    pub only_on: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
    pub purged: bool,
    /// 从笔记待办设的提醒（后加的，旧版传来的没有）
    #[serde(default)]
    pub note_uid: Option<String>,
    #[serde(default)]
    pub todo: Option<String>,
    /// 说得含糊、时间是先定的（0.2 加的，旧版传来的没有）
    #[serde(default)]
    pub vague: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SNote {
    pub uid: String,
    pub title: String,
    pub body: String,
    pub tag: Option<String>,
    pub reminder_uid: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
    pub purged: bool,
    /// 置顶时间（v0.3.5 加的，旧版本传来的没有）
    #[serde(default)]
    pub pinned_at: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SAttachment {
    pub uid: String,
    pub file: String,
    pub mime: String,
    pub size: i64,
    pub sha256: String,
    pub note_uid: Option<String>,
    pub created_at: String,
    pub deleted_at: Option<String>,
    pub purged: bool,
    /// 文件附件的原文件名（后加的，图片和旧版传来的没有）
    #[serde(default)]
    pub name: String,
    /// 叮咪同步的套餐不传它（超过单个附件的大小，或者图片压不下来，需求-1.0 第 32.1 节第 5 条）：
    /// 原文件只在这台设备上（设备编号，和提醒的 only_on 一样）。别的设备看到记录、知道文件在哪，不去下载；
    /// 后来放得下了（换了套餐、换了同步方式）那台设备会清掉它、把文件传上去
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only_on: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SInbox {
    pub uid: String,
    pub phone: String,
    pub received_at: String,
    pub sent_at: Option<String>,
    pub text: String,
    pub images: Vec<String>,
    pub status: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SDevice {
    pub uid: String,
    pub name: String,
}

/// 只有一条的共用记录：共用设置（uid "settings"）、今天收尾过没有（uid "wrapped"，内容是日期）
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SShared {
    pub uid: String,
    pub value: String,
}

// 一批最多一百来条、用完就丢，各种记录大小不一不要紧，不值得为此装箱
#[allow(clippy::large_enum_variant)]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "k", content = "d", rename_all = "snake_case")]
pub enum Payload {
    Reminder(SReminder),
    Note(SNote),
    Attachment(SAttachment),
    Inbox(SInbox),
    Device(SDevice),
    Shared(SShared),
}

impl Payload {
    pub fn uid(&self) -> &str {
        match self {
            Payload::Reminder(x) => &x.uid,
            Payload::Note(x) => &x.uid,
            Payload::Attachment(x) => &x.uid,
            Payload::Inbox(x) => &x.uid,
            Payload::Device(x) => &x.uid,
            Payload::Shared(x) => &x.uid,
        }
    }
}

/// 本机改过、还没上传的一条
pub struct Dirty {
    pub table: &'static str,
    pub uid: String,
    pub ver: String,
    pub payload: Payload,
}

fn opt_bool(r: &Row, i: usize) -> rusqlite::Result<bool> {
    Ok(r.get::<_, i64>(i)? != 0)
}

fn rows<T>(c: &Connection, sql: &str, limit: usize, f: impl Fn(&Row) -> rusqlite::Result<T>) -> rusqlite::Result<Vec<T>> {
    let mut st = c.prepare(sql)?;
    let out = st.query_map([limit as i64], f)?.collect();
    out
}

/// 最多 `limit` 条本机改过、还没上传的记录
pub fn dirty(c: &Connection, limit: usize) -> rusqlite::Result<Vec<Dirty>> {
    collect(c, "dirty=1", "", limit)
}

/// 同上，但不要还得传原文件的附件（叮咪同步的空间满了：附件先留在这台电脑上，文字照常传；
/// 彻底删除的附件记录照样要，删掉它能腾出地方）
/// （不传原文件、只在一台设备上的附件记录也照样要，它用不着传文件）
pub fn dirty_without_files(c: &Connection, limit: usize) -> rusqlite::Result<Vec<Dirty>> {
    collect(c, "dirty=1", " AND (COALESCE(purged,0)=1 OR only_on IS NOT NULL)", limit)
}

/// 还没传上去的附件有几个（空间满了时「先只在这台电脑上」的那些；超过套餐大小、本来就不传的不算）
pub fn held_files(c: &Connection) -> rusqlite::Result<u32> {
    c.query_row("SELECT COUNT(*) FROM attachments WHERE dirty=1 AND COALESCE(purged,0)=0 AND only_on IS NULL", [], |r| r.get(0))
}

/// 叮咪同步的套餐对单个图片、单个附件的限额（Account.max_image / max_file），None 不限；me 是这台电脑的设备编号
#[derive(Clone, Debug, Default)]
pub struct Limits {
    pub max_image: Option<u64>,
    pub max_file: Option<u64>,
    pub me: String,
}

impl Limits {
    /// 原文件不压就放得下（图片按图片的、别的按附件的）
    pub fn fits(&self, mime: &str, size: u64) -> bool {
        let max = if mime.starts_with("image/") { self.max_image } else { self.max_file };
        max.is_none_or(|m| size <= m)
    }
}

/// 超过套餐的大小、不传的附件有几个（只在这台电脑上，还没删的）
pub fn only_here(c: &Connection, me: &str) -> rusqlite::Result<u32> {
    c.query_row(
        "SELECT COUNT(*) FROM attachments WHERE only_on=?1 AND COALESCE(purged,0)=0 AND deleted_at IS NULL",
        [me],
        |r| r.get(0),
    )
}

/// 这个附件不传原文件、只留在这台电脑上：记下来、改版本，记录照样传（别的设备知道它在哪）
pub fn mark_only_here(c: &Connection, uid: &str, me: &str) -> rusqlite::Result<()> {
    c.execute("UPDATE attachments SET only_on=?2, ver=?3, dirty=1 WHERE uid=?1", params![uid, me, crate::clock::next()])?;
    Ok(())
}

/// 以前因为太大只留在这台电脑上、现在放得下了（换了大套餐、换成不限大小的同步方式）：清掉记号，原文件跟着记录传上去。
/// 回清掉了几个
pub fn unmark_fitting(c: &Connection, limits: &Limits) -> rusqlite::Result<usize> {
    let rows: Vec<(String, String, i64)> = {
        let mut st = c.prepare("SELECT uid, mime, size FROM attachments WHERE only_on=?1 AND COALESCE(purged,0)=0")?;
        let out = st.query_map([&limits.me], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
        out
    };
    let mut n = 0;
    for (uid, mime, size) in rows {
        if limits.fits(&mime, size.max(0) as u64) {
            c.execute("UPDATE attachments SET only_on=NULL, ver=?2, dirty=1 WHERE uid=?1", params![uid, crate::clock::next()])?;
            n += 1;
        }
    }
    Ok(n)
}

/// 本机的全部记录（网盘文件夹同步写快照时用）
pub fn all(c: &Connection) -> rusqlite::Result<Vec<Dirty>> {
    collect(c, "1=1", "", i64::MAX as usize)
}

/// 按 `filter` 取最多 `limit` 条记录，几张表加起来；`att` 是附件表另外加的条件
fn collect(c: &Connection, filter: &str, att: &str, limit: usize) -> rusqlite::Result<Vec<Dirty>> {
    let mut out = rows(
        c,
        &format!("SELECT uid,title,detail,tag,urgent,due_at,repeat,snoozed_until,done,snooze_count,last_done_at,completed_at,
                only_on,created_at,updated_at,deleted_at,purged,ver,note_uid,todo,vague
         FROM reminders WHERE {filter} LIMIT ?1"),
        limit,
        |r| {
            let repeat: String = r.get(6)?;
            let only_on: String = r.get(12)?;
            Ok(Dirty {
                table: "reminders",
                uid: r.get(0)?,
                ver: r.get(17)?,
                payload: Payload::Reminder(SReminder {
                    uid: r.get(0)?,
                    title: r.get(1)?,
                    detail: r.get(2)?,
                    tag: r.get(3)?,
                    urgent: opt_bool(r, 4)?,
                    due_at: r.get(5)?,
                    repeat: serde_json::from_str(&repeat).unwrap_or_default(),
                    snoozed_until: r.get(7)?,
                    done: opt_bool(r, 8)?,
                    snooze_count: r.get(9)?,
                    last_done_at: r.get(10)?,
                    completed_at: r.get(11)?,
                    only_on: serde_json::from_str(&only_on).unwrap_or_default(),
                    created_at: r.get(13)?,
                    updated_at: r.get(14)?,
                    deleted_at: r.get(15)?,
                    purged: opt_bool(r, 16)?,
                    note_uid: r.get(18)?,
                    todo: r.get(19)?,
                    vague: opt_bool(r, 20)?,
                }),
            })
        },
    )?;
    let left = |out: &Vec<Dirty>| limit.saturating_sub(out.len());
    out.extend(rows(
        c,
        &format!("SELECT uid,title,body,tag,reminder_uid,created_at,updated_at,deleted_at,purged,ver,pinned_at FROM notes WHERE {filter} LIMIT ?1"),
        left(&out),
        |r| {
            Ok(Dirty {
                table: "notes",
                uid: r.get(0)?,
                ver: r.get(9)?,
                payload: Payload::Note(SNote {
                    uid: r.get(0)?,
                    title: r.get(1)?,
                    body: r.get(2)?,
                    tag: r.get(3)?,
                    reminder_uid: r.get(4)?,
                    created_at: r.get(5)?,
                    updated_at: r.get(6)?,
                    deleted_at: r.get(7)?,
                    purged: opt_bool(r, 8)?,
                    pinned_at: r.get(10)?,
                }),
            })
        },
    )?);
    out.extend(rows(
        c,
        &format!("SELECT uid,file,mime,size,sha256,note_uid,created_at,deleted_at,purged,ver,name,only_on FROM attachments WHERE {filter}{att} LIMIT ?1"),
        left(&out),
        |r| {
            Ok(Dirty {
                table: "attachments",
                uid: r.get(0)?,
                ver: r.get(9)?,
                payload: Payload::Attachment(SAttachment {
                    uid: r.get(0)?,
                    file: r.get(1)?,
                    mime: r.get(2)?,
                    size: r.get(3)?,
                    sha256: r.get(4)?,
                    note_uid: r.get(5)?,
                    created_at: r.get(6)?,
                    deleted_at: r.get(7)?,
                    purged: opt_bool(r, 8)?,
                    name: r.get(10)?,
                    only_on: r.get(11)?,
                }),
            })
        },
    )?);
    out.extend(rows(
        c,
        &format!("SELECT uid,phone,received_at,sent_at,text,images,status,ver FROM inbox WHERE {filter} LIMIT ?1"),
        left(&out),
        |r| {
            let images: String = r.get(5)?;
            Ok(Dirty {
                table: "inbox",
                uid: r.get(0)?,
                ver: r.get(7)?,
                payload: Payload::Inbox(SInbox {
                    uid: r.get(0)?,
                    phone: r.get(1)?,
                    received_at: r.get(2)?,
                    sent_at: r.get(3)?,
                    text: r.get(4)?,
                    images: serde_json::from_str(&images).unwrap_or_default(),
                    status: r.get(6)?,
                }),
            })
        },
    )?);
    out.extend(rows(c, &format!("SELECT uid,name,ver FROM devices WHERE {filter} LIMIT ?1"), left(&out), |r| {
        Ok(Dirty {
            table: "devices",
            uid: r.get(0)?,
            ver: r.get(2)?,
            payload: Payload::Device(SDevice { uid: r.get(0)?, name: r.get(1)? }),
        })
    })?);
    out.extend(rows(c, &format!("SELECT uid,value,ver FROM shared WHERE {filter} LIMIT ?1"), left(&out), |r| {
        Ok(Dirty {
            table: "shared",
            uid: r.get(0)?,
            ver: r.get(2)?,
            payload: Payload::Shared(SShared { uid: r.get(0)?, value: r.get(1)? }),
        })
    })?);
    Ok(out)
}

/// 上传成功：版本没变（上传期间没再改过）才算传完
pub fn clear_dirty(c: &Connection, table: &str, uid: &str, ver: &str) -> rusqlite::Result<()> {
    c.execute(&format!("UPDATE {table} SET dirty=0 WHERE uid=?1 AND ver=?2"), params![uid, ver])?;
    Ok(())
}


/// 合并一条别的电脑传来的记录（`ver` 比本机新才覆盖）。返回本机有没有变化
pub fn apply(c: &Connection, data_dir: &Path, ver: &str, p: &Payload) -> rusqlite::Result<bool> {
    let n = match p {
        Payload::Reminder(r) => c.execute(
            // 时间、完成、删除变了的话，本机正在弹的卡片收起来，按新的情况重新算
            "INSERT INTO reminders(uid,title,detail,tag,urgent,due_at,repeat,snoozed_until,done,state,snooze_count,
                last_done_at,completed_at,only_on,created_at,updated_at,deleted_at,purged,ver,note_uid,todo,vague,dirty)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,0,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,0)
             ON CONFLICT(uid) DO UPDATE SET
                title=excluded.title, detail=excluded.detail, tag=excluded.tag, urgent=excluded.urgent,
                due_at=excluded.due_at, repeat=excluded.repeat, snoozed_until=excluded.snoozed_until, done=excluded.done,
                snooze_count=excluded.snooze_count, last_done_at=excluded.last_done_at, completed_at=excluded.completed_at,
                only_on=excluded.only_on, created_at=excluded.created_at, updated_at=excluded.updated_at,
                deleted_at=excluded.deleted_at, purged=excluded.purged, ver=excluded.ver, dirty=0,
                note_uid=excluded.note_uid, todo=excluded.todo, vague=excluded.vague,
                state=CASE WHEN excluded.done != reminders.done
                             OR excluded.deleted_at IS NOT reminders.deleted_at
                             OR COALESCE(excluded.snoozed_until, excluded.due_at) != COALESCE(reminders.snoozed_until, reminders.due_at)
                           THEN 0 ELSE reminders.state END
             WHERE excluded.ver > reminders.ver",
            params![
                r.uid,
                r.title,
                r.detail,
                r.tag,
                r.urgent as i64,
                r.due_at,
                serde_json::to_string(&r.repeat).unwrap_or_else(|_| "{}".into()),
                r.snoozed_until,
                r.done as i64,
                r.snooze_count,
                r.last_done_at,
                r.completed_at,
                serde_json::to_string(&r.only_on).unwrap_or_else(|_| "[]".into()),
                r.created_at,
                r.updated_at,
                r.deleted_at,
                r.purged as i64,
                ver,
                r.note_uid,
                r.todo,
                r.vague as i64
            ],
        )?,
        Payload::Note(n) => {
            keep_conflict_copy(c, ver, n)?;
            c.execute(
                "INSERT INTO notes(uid,title,body,tag,reminder_id,reminder_uid,created_at,updated_at,deleted_at,purged,ver,dirty,pinned_at)
                 VALUES(?1,?2,?3,?4,(SELECT id FROM reminders WHERE uid=?5),?5,?6,?7,?8,?9,?10,0,?11)
                 ON CONFLICT(uid) DO UPDATE SET
                    title=excluded.title, body=excluded.body, tag=excluded.tag, reminder_id=excluded.reminder_id,
                    reminder_uid=excluded.reminder_uid, created_at=excluded.created_at, updated_at=excluded.updated_at,
                    deleted_at=excluded.deleted_at, purged=excluded.purged, ver=excluded.ver, dirty=0,
                    pinned_at=excluded.pinned_at
                 WHERE excluded.ver > notes.ver",
                params![
                    n.uid,
                    n.title,
                    n.body,
                    n.tag,
                    n.reminder_uid,
                    n.created_at,
                    n.updated_at,
                    n.deleted_at,
                    n.purged as i64,
                    ver,
                    n.pinned_at
                ],
            )?
        }
        Payload::Attachment(a) => {
            // 文件名和编号都会拼进路径（附件文件夹、网盘文件夹的 files/<编号>.dfile、服务器上的文件），不合规矩的不收
            if !db::valid_att_file(&a.file) || !db::valid_att_uid(&a.uid) {
                return Ok(false);
            }
            let path = db::attachment_dir(data_dir).join(&a.file);
            let n = c.execute(
                "INSERT INTO attachments(uid,file,mime,size,sha256,note_uid,created_at,deleted_at,purged,ver,dirty,local_ok,name,only_on)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,0,?11,?12,?13)
                 ON CONFLICT(uid) DO UPDATE SET
                    file=excluded.file, mime=excluded.mime, size=excluded.size, sha256=excluded.sha256,
                    note_uid=excluded.note_uid, created_at=excluded.created_at, deleted_at=excluded.deleted_at,
                    purged=excluded.purged, ver=excluded.ver, dirty=0, name=excluded.name, only_on=excluded.only_on
                 WHERE excluded.ver > attachments.ver",
                params![
                    a.uid,
                    a.file,
                    a.mime,
                    a.size,
                    a.sha256,
                    a.note_uid,
                    a.created_at,
                    a.deleted_at,
                    a.purged as i64,
                    ver,
                    path.exists() as i64,
                    db::clean_file_name(&a.name),
                    a.only_on
                ],
            )?;
            if n > 0 && a.purged {
                db::clean_attachments(c, data_dir)?;
            }
            n
        }
        Payload::Inbox(i) => c.execute(
            "INSERT INTO inbox(uid,phone,received_at,sent_at,text,images,status,ver,dirty)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,0)
             ON CONFLICT(uid) DO UPDATE SET
                phone=excluded.phone, received_at=excluded.received_at, sent_at=excluded.sent_at, text=excluded.text,
                images=excluded.images, status=excluded.status, ver=excluded.ver, dirty=0
             WHERE excluded.ver > inbox.ver",
            params![
                i.uid,
                i.phone,
                i.received_at,
                i.sent_at,
                i.text,
                // 附件的文件名之后会拼进路径、写进笔记：不合规矩的去掉
                serde_json::to_string(&i.images.iter().filter(|f| db::valid_att_file(f)).collect::<Vec<_>>()).unwrap_or_else(|_| "[]".into()),
                i.status,
                ver
            ],
        )?,
        Payload::Device(d) => c.execute(
            "INSERT INTO devices(uid,name,ver,dirty) VALUES(?1,?2,?3,0)
             ON CONFLICT(uid) DO UPDATE SET name=excluded.name, ver=excluded.ver, dirty=0 WHERE excluded.ver > devices.ver",
            params![d.uid, d.name, ver],
        )?,
        Payload::Shared(s) => {
            let n = c.execute(
                "INSERT INTO shared(uid,value,ver,dirty) VALUES(?1,?2,?3,0)
                 ON CONFLICT(uid) DO UPDATE SET value=excluded.value, ver=excluded.ver, dirty=0 WHERE excluded.ver > shared.ver",
                params![s.uid, s.value, ver],
            )?;
            if n > 0 && s.uid == settings::SHARED_UID {
                settings::apply_shared(c, &s.value)?;
            }
            n
        }
    };
    Ok(n > 0)
}

/// 本机这条笔记现在的样子：(版本, 是否没上传, 标题, 正文, 分类, 删除时间)
type LocalNote = (String, i64, String, String, Option<String>, Option<String>);

/// 本机这条笔记也改过、还没上传，内容又和传来的不一样：先另存一份，免得本机改的丢了
fn keep_conflict_copy(c: &Connection, ver: &str, n: &SNote) -> rusqlite::Result<()> {
    let local: Option<LocalNote> = c
        .query_row(
            "SELECT ver, dirty, title, body, tag, deleted_at FROM notes WHERE uid=?1",
            [&n.uid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .optional()?;
    let Some((lver, dirty, title, body, tag, deleted)) = local else {
        return Ok(());
    };
    if dirty == 1 && deleted.is_none() && ver > lver.as_str() && (title != n.title || body != n.body) {
        db::upsert_note(c, &Note { title: format!("{title}（冲突副本）"), body, tag, ..Default::default() })?;
    }
    Ok(())
}

/// 一批合并完后，笔记关联的提醒换算成本机的编号（提醒可能比笔记后到）
pub fn fix_links(c: &Connection) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE notes SET reminder_id=(SELECT id FROM reminders WHERE uid=notes.reminder_uid) WHERE reminder_uid IS NOT NULL",
        [],
    )?;
    Ok(())
}

/// 记录到了、原图还没下载的图片：(编号, 文件名)。文件附件不在这里，点开时才下载
pub fn missing_files(c: &Connection) -> rusqlite::Result<Vec<(String, String)>> {
    // 只在别的设备上的（太大、叮咪同步不传）不去下载
    let mut st =
        c.prepare("SELECT uid, file FROM attachments WHERE purged=0 AND local_ok=0 AND mime LIKE 'image/%' AND only_on IS NULL")?;
    let out = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect();
    out
}

pub fn file_ok(c: &Connection, uid: &str) -> rusqlite::Result<()> {
    c.execute("UPDATE attachments SET local_ok=1 WHERE uid=?1", [uid])?;
    Ok(())
}

pub fn has_dirty(c: &Connection) -> bool {
    dirty(c, 1).is_ok_and(|d| !d.is_empty())
}

/// 全部记录的 (uid, 版本号)，按 uid 排好。备份时用来看有没有变化，比取全部内容轻得多
pub fn versions(c: &Connection) -> rusqlite::Result<Vec<(String, String)>> {
    let sql = ["reminders", "notes", "attachments", "inbox", "devices", "shared"]
        .map(|t| format!("SELECT uid, ver FROM {t}"))
        .join(" UNION ALL ")
        + " ORDER BY 1, 2";
    let mut st = c.prepare(&sql)?;
    let out = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect();
    out
}

/// 全部附件：(uid, 本机文件名, 是否彻底删除了)。备份附件时用
pub fn attachment_files(c: &Connection) -> rusqlite::Result<Vec<(String, String, bool)>> {
    let mut st = c.prepare("SELECT uid, file, purged FROM attachments")?;
    let out = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, opt_bool(r, 2)?)))?.collect();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{get_note, get_reminder, list_notes, list_reminders, upsert_note, upsert_reminder, Reminder};

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("dingmi-sync-{name}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// 把 from 里改过的都"传"给 to，返回传了几条
    fn sync(from: &Connection, to: &Connection, to_dir: &Path) -> usize {
        let batch = dirty(from, 1000).unwrap();
        for d in &batch {
            // 走一遍 JSON，和真的上传一样
            let p: Payload = serde_json::from_str(&serde_json::to_string(&d.payload).unwrap()).unwrap();
            apply(to, to_dir, &d.ver, &p).unwrap();
            clear_dirty(from, d.table, &d.uid, &d.ver).unwrap();
        }
        fix_links(to).unwrap();
        batch.len()
    }

    #[test]
    fn new_repeat_and_vague_sync() {
        let (da, db_) = (temp("a"), temp("b"));
        let a = db::open(&da).unwrap();
        let b = db::open(&db_).unwrap();
        let repeat = Repeat { kind: "lunar_yearly".into(), month: 8, monthday: 15, ..Default::default() };
        let r = Reminder { title: "改天去看奶奶".into(), due_at: "2099-01-01 09:00".into(), repeat: repeat.clone(), vague: true, ..Default::default() };
        upsert_reminder(&a, &r).unwrap();
        assert_eq!(sync(&a, &b, &db_), 1);
        let got = &list_reminders(&b).unwrap()[0];
        assert!(got.vague);
        assert_eq!(got.repeat, repeat);
        // 旧版本传来的记录没有 vague、repeat 里没有 month / nth
        let old: SReminder = serde_json::from_str(
            r#"{"uid":"x","title":"旧","detail":"","tag":null,"urgent":false,"due_at":"2099-01-01 09:00",
                "repeat":{"kind":"monthly","days":[],"monthday":5,"every":0},"snoozed_until":null,"done":false,
                "snooze_count":0,"last_done_at":null,"completed_at":null,"only_on":[],"created_at":"","updated_at":"",
                "deleted_at":null,"purged":false}"#,
        )
        .unwrap();
        assert!(!old.vague);
        assert_eq!((old.repeat.month, old.repeat.nth), (0, 0));
    }

    #[test]
    fn two_devices_converge() {
        let (da, db_) = (temp("a"), temp("b"));
        let a = db::open(&da).unwrap();
        let b = db::open(&db_).unwrap();
        let rid = upsert_reminder(&a, &Reminder { title: "交报销".into(), due_at: "2099-01-01 09:00".into(), ..Default::default() }).unwrap();
        upsert_note(&a, &Note { title: "发票".into(), body: "三张".into(), reminder_id: Some(rid), ..Default::default() }).unwrap();
        db::set_own_device(&a, "deviceaaaaaa", "公司电脑").unwrap();
        assert_eq!(sync(&a, &b, &db_), 3);
        assert_eq!(sync(&a, &b, &db_), 0);

        let r = &list_reminders(&b).unwrap()[0];
        assert_eq!(r.title, "交报销");
        let n = &list_notes(&b).unwrap()[0];
        assert_eq!(n.reminder_id, Some(r.id));
        assert_eq!(db::list_devices(&b, "x").unwrap()[0].name, "公司电脑");
        // 收到的记录在 B 上不算"改过"，不会又传回去
        assert!(!has_dirty(&b));

        // B 改了提醒，传回 A；A 上旧的版本不会把它覆盖回去
        let mut r2 = r.clone();
        r2.title = "交报销（带发票）".into();
        upsert_reminder(&b, &r2).unwrap();
        let stale = dirty(&a, 10).unwrap();
        assert!(stale.is_empty());
        assert_eq!(sync(&b, &a, &da), 1);
        assert_eq!(get_reminder(&a, rid).unwrap().unwrap().title, "交报销（带发票）");
        let old = Payload::Reminder(SReminder {
            uid: r.uid.clone(),
            title: "很旧".into(),
            detail: String::new(),
            tag: None,
            urgent: false,
            due_at: r.due_at.clone(),
            repeat: Repeat::default(),
            snoozed_until: None,
            done: false,
            snooze_count: 0,
            last_done_at: None,
            completed_at: None,
            only_on: vec![],
            created_at: r.created_at.clone(),
            updated_at: r.updated_at.clone(),
            deleted_at: None,
            purged: false,
            note_uid: None,
            todo: None,
            vague: false,
        });
        assert!(!apply(&a, &da, "0000000000001-0000-old", &old).unwrap());
        assert_eq!(get_reminder(&a, rid).unwrap().unwrap().title, "交报销（带发票）");
    }

    #[test]
    fn done_elsewhere_dismisses_card() {
        let (da, db_) = (temp("a"), temp("b"));
        let a = db::open(&da).unwrap();
        let b = db::open(&db_).unwrap();
        upsert_reminder(&a, &Reminder { title: "喝水".into(), due_at: "2000-01-01 09:00".into(), ..Default::default() }).unwrap();
        sync(&a, &b, &db_);
        // 两台都弹出来了
        let (ia, ib) = (list_reminders(&a).unwrap()[0].id, list_reminders(&b).unwrap()[0].id);
        db::set_state(&a, ia, db::ST_SHOW).unwrap();
        db::set_state(&b, ib, db::ST_SHOW).unwrap();
        // A 上点了完成
        let mut r = get_reminder(&a, ia).unwrap().unwrap();
        r.done = true;
        r.state = db::ST_WAIT;
        upsert_reminder(&a, &r).unwrap();
        sync(&a, &b, &db_);
        let rb = get_reminder(&b, ib).unwrap().unwrap();
        assert!(rb.done);
        assert_eq!(rb.state, db::ST_WAIT);
        assert!(db::queue(&b).unwrap().is_empty());
        // 只改了标题、时间没变的话，B 上正在弹的卡片不动
        db::set_state(&b, ib, db::ST_SHOW).unwrap();
        let mut r = get_reminder(&a, ia).unwrap().unwrap();
        r.title = "多喝水".into();
        upsert_reminder(&a, &r).unwrap();
        sync(&a, &b, &db_);
        assert_eq!(get_reminder(&b, ib).unwrap().unwrap().state, db::ST_SHOW);
    }

    #[test]
    fn note_conflict_keeps_a_copy() {
        let (da, db_) = (temp("a"), temp("b"));
        let a = db::open(&da).unwrap();
        let b = db::open(&db_).unwrap();
        let na = upsert_note(&a, &Note { title: "方案".into(), body: "v1".into(), ..Default::default() }).unwrap();
        sync(&a, &b, &db_);
        let nb = list_notes(&b).unwrap()[0].id;
        // 两边离线各改各的，A 后改（版本更新）
        let mut n = get_note(&b, nb).unwrap().unwrap();
        n.body = "B 改的".into();
        upsert_note(&b, &n).unwrap();
        let mut n = get_note(&a, na).unwrap().unwrap();
        n.body = "A 改的".into();
        upsert_note(&a, &n).unwrap();
        sync(&a, &b, &db_);
        let notes = list_notes(&b).unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(get_note(&b, nb).unwrap().unwrap().body, "A 改的");
        let copy = notes.iter().find(|x| x.id != nb).unwrap();
        assert_eq!((copy.title.as_str(), copy.body.as_str()), ("方案（冲突副本）", "B 改的"));
        // 副本是 B 上的新笔记，会传给 A
        assert_eq!(sync(&b, &a, &da), 1);
        assert_eq!(list_notes(&a).unwrap().len(), 2);
    }

    #[test]
    fn shared_settings_and_wrapped() {
        let (da, db_) = (temp("a"), temp("b"));
        let a = db::open(&da).unwrap();
        let b = db::open(&db_).unwrap();
        // B 是新装的：默认设置不会传出去
        settings::save_local(&b, &settings::Settings { hk_flash: "Alt+Q".into(), ..Default::default() }).unwrap();
        settings::ensure_shared(&b, &settings::load(&b).unwrap(), "devb").unwrap();
        assert!(!has_dirty(&b));
        // A 改过设置
        let s = settings::Settings { ai_key: "sk-a".into(), snooze_min: 30, hk_flash: "Ctrl+Space".into(), ..Default::default() };
        settings::save(&a, &s).unwrap();
        db::shared_set(&a, "wrapped", "2026-10-31").unwrap();
        assert_eq!(sync(&a, &b, &db_), 2);
        let got = settings::load(&b).unwrap();
        assert_eq!((got.ai_key.as_str(), got.snooze_min, got.hk_flash.as_str()), ("sk-a", 30, "Alt+Q"));
        assert_eq!(db::shared_get(&b, "wrapped").as_deref(), Some("2026-10-31"));
        // 只改了快捷键：共用的部分没变，不用同步
        settings::save(&a, &settings::Settings { hk_flash: "Alt+W".into(), ..s }).unwrap();
        assert!(!has_dirty(&a));
    }

    #[test]
    fn attachments_and_inbox() {
        let (da, db_) = (temp("a"), temp("b"));
        let a = db::open(&da).unwrap();
        let b = db::open(&db_).unwrap();
        let f = db::save_file(&a, &da, "image/png", "", b"png", None, None).unwrap();
        db::add_inbox(&a, "srv1", "iPhone", None, "看看", std::slice::from_ref(&f)).unwrap();
        sync(&a, &b, &db_);
        // B 上记录到了，原图还要下载
        let uid = f.trim_end_matches(".png").to_string();
        assert_eq!(missing_files(&b).unwrap(), vec![(uid.clone(), f.clone())]);
        std::fs::create_dir_all(db::attachment_dir(&db_)).unwrap();
        std::fs::write(db::attachment_dir(&db_).join(&f), b"png").unwrap();
        file_ok(&b, &uid).unwrap();
        assert!(missing_files(&b).unwrap().is_empty());
        assert_eq!(db::list_inbox(&b).unwrap()[0].images, vec![f.clone()]);

        // 文件附件：原文件名跟着同步过去，但不自动下载（点开时才下）
        let pdf = db::save_file(&a, &da, "application/pdf", "合同[终版].pdf", b"%PDF", None, None).unwrap();
        sync(&a, &b, &db_);
        let got = db::get_attachment(&b, &pdf).unwrap().unwrap();
        assert_eq!((got.name.as_str(), got.local), ("合同_终版_.pdf", false));
        assert!(missing_files(&b).unwrap().is_empty());

        // A 上删掉这条收件箱（图片没进笔记，跟着删），B 上的图片文件也删掉
        let id = db::list_inbox(&a).unwrap()[0].id;
        db::delete_inbox(&a, id).unwrap();
        sync(&a, &b, &db_);
        assert!(db::list_inbox(&b).unwrap().is_empty());
        assert!(!db::attachment_dir(&db_).join(&f).exists());

        // 文件名不像样的不收
        let bad = Payload::Attachment(SAttachment {
            uid: "x".into(),
            file: "../../evil.exe".into(),
            mime: "image/png".into(),
            size: 1,
            sha256: String::new(),
            note_uid: None,
            created_at: String::new(),
            deleted_at: None,
            purged: false,
            name: String::new(),
            only_on: None,
        });
        assert!(!apply(&b, &db_, "9", &bad).unwrap());
    }
}
