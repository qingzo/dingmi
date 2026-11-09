//! 一键导出（需求-0.1 第 4 节）：提醒、笔记、图片和文件打包成一个 zip，离开叮咪也能看，也能再导回叮咪。
//! 笔记一条一个 Markdown（带文件头，Obsidian、Typora 都认），按笔记分类分文件夹；提醒是一个清单 `提醒.md`；
//! `数据/dingmi.db` 是去掉了 AI Key、同步配对信息、发布私钥和只属于这台电脑的东西的完整数据库，恢复时只用它和 `附件/`（见 restore.rs）。
//! 回收站里的不导出。

use crate::db::{self, Device, Note, Reminder, Repeat};
use crate::lunar;
use crate::settings::{self, TagDef};
use chrono::{Datelike, NaiveDateTime, Timelike};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// 导出包的格式版本。包里的结构或数据库变得旧版叮咪读不懂时加一
pub const FORMAT: u32 = 1;
pub const DB_PATH: &str = "数据/dingmi.db";
pub const MANIFEST_PATH: &str = "数据/manifest.json";
pub const ATT_DIR: &str = "附件";
/// 认出叮咪导出包用的
pub const APP: &str = "dingmi";

/// 数据库里要带走的表；别的表（只属于这台电脑的同步状态这类）一律不带
const KEEP_TABLES: [&str; 7] = ["reminders", "notes", "kv", "attachments", "inbox", "devices", "shared"];

/// 要导出的附件：没彻底删除、不在回收站里，也不属于回收站里的笔记
const KEEP_ATT: &str = "purged=0 AND deleted_at IS NULL
    AND (note_uid IS NULL OR note_uid NOT IN (SELECT uid FROM notes WHERE deleted_at IS NOT NULL))";

/// `数据/manifest.json`：恢复时先看它
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Manifest {
    /// 固定是 "dingmi"
    pub app: String,
    pub format: u32,
    /// 导出时的叮咪版本
    pub version: String,
    pub exported_at: String,
    pub reminders: usize,
    pub notes: usize,
    pub attachments: usize,
}

/// 导出完的结果，给界面显示
#[derive(Serialize, Clone, Debug)]
pub struct Exported {
    pub path: String,
    pub reminders: usize,
    pub notes: usize,
    pub attachments: usize,
    /// 不在这台电脑上、也没能下载下来的附件（说明.txt 里也列了）
    pub missing: Vec<String>,
}

/// 导出进度：stage 是 download 先下载缺的文件 / pack 打包
#[derive(Serialize, Clone, Debug)]
pub struct Progress {
    pub stage: &'static str,
    pub done: usize,
    pub total: usize,
}

/// 标题变成 Windows 能用的文件名：去掉不允许的字符，避开 CON 这类保留名，太长截断
pub fn safe_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| if c.is_control() || r#"\/:*?"<>|"#.contains(c) { ' ' } else { c })
        .collect();
    let mut name: String = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    name = name.chars().take(60).collect();
    let name = name.trim_matches(|c: char| c == '.' || c == ' ').to_string();
    if name.is_empty() {
        return "无标题".into();
    }
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1",
        "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        format!("_{name}")
    } else {
        name
    }
}

/// 同名的加上 (2)、(3)…
fn unique(name: String, used: &mut HashSet<String>) -> String {
    let mut n = 1;
    let mut candidate = name.clone();
    while !used.insert(candidate.to_lowercase()) {
        n += 1;
        candidate = format!("{name} ({n})");
    }
    candidate
}

/// 笔记的文件名：用标题；没标题的用正文开头十几个字
pub fn note_file_stem(n: &Note) -> String {
    if !n.title.trim().is_empty() {
        return safe_name(&n.title);
    }
    let first = n
        .body
        .lines()
        .filter(|l| !l.contains("](att:"))
        .map(|l| {
            let t = l.trim().trim_start_matches(['#', '>', '-', '*', '+', ' ']);
            let t = ["[ ]", "[x]", "[X]"].iter().find_map(|m| t.strip_prefix(m)).unwrap_or(t);
            t.trim().to_string()
        })
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    safe_name(&first.chars().take(15).collect::<String>())
}

/// 文件头里的值：普通文字原样写；会被 YAML 误会的（带冒号、井号、引号开头、像数字或 true 的）用双引号括起来
fn yaml(s: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let plain = !s.is_empty()
        && s == s.trim()
        && !s.starts_with(|c: char| "-?:,[]{}#&*!|>'\"%@`".contains(c))
        && !s.contains(": ")
        && !s.contains(" #")
        && !s.ends_with(':')
        && !s.contains(['\n', '\r', '\t'])
        && !matches!(lower.as_str(), "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "~")
        && s.parse::<f64>().is_err();
    if plain {
        s.to_string()
    } else {
        serde_json::to_string(s).unwrap_or_default()
    }
}

/// "2026-11-03 14:20:05" 只留到分钟
fn minute(s: &str) -> &str {
    s.get(..16).unwrap_or(s)
}

/// 图片还是文件，决定放在 附件/images 还是 附件/files
fn sub_dir(mime: &str) -> &'static str {
    if mime.starts_with("image/") {
        "images"
    } else {
        "files"
    }
}

/// 正文里的 `](att:文件名)` 改成指向 附件/ 的相对路径（笔记在 笔记/<分类>/ 下，往上两层）；别的一个字都不动
fn link_attachments(body: &str, subs: &HashMap<String, &'static str>) -> String {
    const MARK: &str = "](att:";
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(i) = rest.find(MARK) {
        out.push_str(&rest[..i + 2]);
        let after = &rest[i + MARK.len()..];
        let len = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')).unwrap_or(after.len());
        let file = &after[..len];
        if len > 0 && after[len..].starts_with(')') {
            let sub = subs.get(file).copied().unwrap_or(if rest[..i].contains("![") { "images" } else { "files" });
            out.push_str(&format!("../../{ATT_DIR}/{sub}/{file}"));
        } else {
            out.push_str("att:");
            out.push_str(file);
        }
        rest = &after[len..];
    }
    out.push_str(rest);
    out
}

/// 一条笔记的 Markdown：文件头（标题、时间、分类、关联的提醒），再是原样的正文，只改图片和文件的路径
pub fn note_markdown(n: &Note, cat: Option<&str>, reminder: Option<&Reminder>, subs: &HashMap<String, &'static str>) -> String {
    let mut head = vec!["---".to_string()];
    if !n.title.trim().is_empty() {
        head.push(format!("title: {}", yaml(&n.title)));
    }
    head.push(format!("created: {}", minute(&n.created_at)));
    head.push(format!("updated: {}", minute(&n.updated_at)));
    if let Some(c) = cat {
        head.push(format!("category: {}", yaml(c)));
    }
    if let Some(r) = reminder {
        head.push(format!("reminder: {}", yaml(&format!("{} {}", r.due_at, r.title))));
    }
    head.push("---".into());
    format!("{}\n\n{}\n", head.join("\n"), link_attachments(&n.body, subs).trim_end())
}

const WD: [&str; 8] = ["", "一", "二", "三", "四", "五", "六", "日"];

/// 周期提醒怎么重复，说成一句话（和界面上的说法一样）
fn repeat_label(r: &Repeat, due: &str) -> String {
    let d = db::parse_dt(due);
    match r.kind.as_str() {
        "daily" => "每天".into(),
        "weekdays" => "每个工作日".into(),
        "weekly" => {
            let mut days = if r.days.is_empty() {
                d.map(|d| vec![d.weekday().number_from_monday()]).unwrap_or_default()
            } else {
                r.days.clone()
            };
            days.sort();
            days.dedup();
            let n = r.every.max(1);
            if days.len() == 7 && n == 1 {
                return "每天".into();
            }
            let names: Vec<&str> = days.iter().filter_map(|&x| WD.get(x as usize).copied()).collect();
            match n {
                1 => format!("每周{}", names.join("、")),
                2 => format!("每两周的周{}", names.join("、")),
                _ => format!("每 {n} 周的周{}", names.join("、")),
            }
        }
        "monthly" => {
            let day = if r.monthday > 0 { r.monthday } else { d.map(|d| d.day()).unwrap_or(1) };
            format!("每月 {day} 号")
        }
        "interval" => format!("每 {} 天", r.every.max(1)),
        "yearly" => {
            let m = if r.month > 0 { r.month } else { d.map(|d| d.month()).unwrap_or(1) };
            let day = if r.monthday > 0 { r.monthday } else { d.map(|d| d.day()).unwrap_or(1) };
            format!("每年 {m} 月 {day} 号")
        }
        "month_last" => "每月最后一天".into(),
        "month_nth" => {
            let nth = if r.nth != 0 { r.nth } else { d.map(|d| (d.day() as i32 + 6) / 7).unwrap_or(1) };
            let wd = r.days.first().copied().or(d.map(|d| d.weekday().number_from_monday())).unwrap_or(1);
            let which = ["最后一个", "第一个", "第二个", "第三个", "第四个", "第五个"].get(nth.max(0) as usize).copied().unwrap_or("");
            format!("每月{which}周{}", WD.get(wd as usize).copied().unwrap_or(""))
        }
        "lunar_yearly" => {
            let l = d.and_then(|d| lunar::to_lunar(d.date()));
            let m = if r.month > 0 { r.month } else { l.map(|l| l.month).unwrap_or(1) };
            let day = if r.monthday > 0 { r.monthday } else { l.map(|l| l.day).unwrap_or(1) };
            // 和界面上一样，同时写上这次是公历几号
            let next = d.map(|d| format!(" · {}月{}日", d.month(), d.day())).unwrap_or_default();
            format!("每年农历{}{next}", lunar::label(m, day))
        }
        _ => String::new(),
    }
}

/// 提醒后面跟的：分类、紧急、只在哪几台电脑上提醒
fn reminder_extras(r: &Reminder, tags: &[TagDef], devices: &[Device]) -> String {
    let mut parts = Vec::new();
    if let Some(t) = tags.iter().find(|t| r.tag.as_deref() == Some(t.id.as_str())) {
        parts.push(t.name.clone());
    }
    if r.urgent {
        parts.push("紧急".into());
    }
    if !r.only_on.is_empty() {
        let names: Vec<&str> = r
            .only_on
            .iter()
            .map(|id| devices.iter().find(|d| &d.uid == id).map(|d| d.name.as_str()).unwrap_or(id.as_str()))
            .collect();
        parts.push(format!("只在：{}", names.join("、")));
    }
    parts.iter().map(|p| format!(" · {p}")).collect()
}

/// `提醒.md`：还没做的、周期提醒、做完的 三段
pub fn reminders_markdown(rs: &[Reminder], tags: &[TagDef], devices: &[Device]) -> String {
    let live = rs.iter().filter(|r| r.deleted_at.is_none());
    let mut todo: Vec<&Reminder> = live.clone().filter(|r| !r.repeat.is_recurring() && !r.done).collect();
    let mut cycle: Vec<&Reminder> = live.clone().filter(|r| r.repeat.is_recurring()).collect();
    let mut done: Vec<&Reminder> = live.filter(|r| !r.repeat.is_recurring() && r.done).collect();
    todo.sort_by(|a, b| a.due_at.cmp(&b.due_at));
    cycle.sort_by(|a, b| a.due_at.cmp(&b.due_at));
    done.sort_by(|a, b| b.completed_at.cmp(&a.completed_at));
    let none = || vec!["（没有）".to_string()];
    let mut out = vec!["# 提醒".to_string(), String::new(), "## 还没做的".into()];
    if todo.is_empty() {
        out.extend(none());
    }
    for r in todo {
        out.push(format!("- [ ] {} {}{}", r.due_at, r.title, reminder_extras(r, tags, devices)));
        out.extend(r.detail.lines().filter(|l| !l.trim().is_empty()).map(|l| format!("  {}", l.trim_end())));
    }
    out.extend([String::new(), "## 周期提醒".into()]);
    if cycle.is_empty() {
        out.extend(none());
    }
    for r in cycle {
        let time = r.due_at.get(11..16).unwrap_or("");
        out.push(format!("- {} {time} {}{}", repeat_label(&r.repeat, &r.due_at), r.title, reminder_extras(r, tags, devices)));
        out.extend(r.detail.lines().filter(|l| !l.trim().is_empty()).map(|l| format!("  {}", l.trim_end())));
    }
    out.extend([String::new(), "## 做完的".into()]);
    if done.is_empty() {
        out.extend(none());
    }
    for r in done {
        let at = r.completed_at.as_deref().and_then(|c| c.get(5..16)).map(|c| format!("（{c} 做完）")).unwrap_or_default();
        out.push(format!("- [x] {} {}{}{at}", r.due_at, r.title, reminder_extras(r, tags, devices)));
    }
    out.join("\n") + "\n"
}

/// 默认存到哪：文档\叮咪导出\叮咪导出_2026-11-05_1430.zip
pub fn default_path(documents: &Path, now: NaiveDateTime) -> PathBuf {
    documents.join("叮咪导出").join(format!("叮咪导出_{}.zip", now.format("%Y-%m-%d_%H%M")))
}

/// 要导出的附件：(编号, 文件名)。导出前看看哪些还没下载到这台电脑
pub fn wanted_attachments(c: &Connection) -> rusqlite::Result<Vec<(String, String)>> {
    let mut st = c.prepare(&format!("SELECT uid, file FROM attachments WHERE {KEEP_ATT}"))?;
    let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

/// 第一步（拿着数据库的锁，很快）：整个库复制一份到临时文件，后面都对着这一份做
pub fn copy_db(c: &Connection, to: &Path) -> Result<(), String> {
    let _ = std::fs::remove_file(to);
    c.execute("VACUUM INTO ?1", [to.to_string_lossy().to_string()]).map_err(|e| format!("复制数据库没成功：{e}"))?;
    Ok(())
}

/// 设置的 JSON 里去掉一些项：`blank` 里的清空，`drop` 里的删掉
fn strip_json(text: &str, blank: &[&str], drop: &[&str], empty: serde_json::Value) -> Option<String> {
    let mut v: serde_json::Value = serde_json::from_str(text).ok()?;
    let o = v.as_object_mut()?;
    for k in blank {
        if o.contains_key(*k) {
            o.insert((*k).into(), empty.clone());
        }
    }
    for k in drop {
        o.remove(*k);
    }
    serde_json::to_string(&v).ok()
}

/// 导出包里的数据库去掉不导出的（需求-0.1 第 4.3 节）：回收站里的；AI Key、同步配对信息、发布私钥；
/// 只属于这台电脑的（启动器的常用、别名、用过几次，桌面组件的位置）。最后整理一遍，删掉的内容不留在文件里
pub fn scrub(c: &Connection) -> rusqlite::Result<()> {
    let tables: Vec<String> = {
        let mut st = c.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")?;
        let rows = st.query_map([], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for t in tables.iter().filter(|t| !KEEP_TABLES.contains(&t.as_str())) {
        c.execute_batch(&format!("DROP TABLE \"{}\"", t.replace('"', "\"\"")))?;
    }
    // kv 里除了设置都是这台电脑自己的：设备 ID、同步配对信息（sync、sync_seq）、启动器用过几次、桌面组件位置……
    let local = db::kv_get(c, "settings");
    c.execute_batch(&format!(
        "DELETE FROM kv;
         DELETE FROM attachments WHERE NOT ({KEEP_ATT});
         DELETE FROM reminders WHERE deleted_at IS NOT NULL;
         DELETE FROM notes WHERE deleted_at IS NOT NULL;
         DELETE FROM inbox WHERE status={};",
        db::INBOX_DELETED
    ))?;
    if let Some(s) = local.and_then(|t| strip_json(&t, &["ai_key"], &["launcher"], "".into())) {
        db::kv_set(c, "settings", &s)?;
    }
    // 共用设置里也有 AI Key 和发布私钥；清成 null，恢复时本机保留自己的
    if let Some(s) = db::shared_get(c, settings::SHARED_UID)
        .and_then(|t| strip_json(&t, &["ai_key"], &[], serde_json::Value::Null))
    {
        c.execute("UPDATE shared SET value=?2 WHERE uid=?1", rusqlite::params![settings::SHARED_UID, s])?;
    }
    c.execute_batch("VACUUM")
}

/// 往 zip 里写一个文件
fn put(z: &mut ZipWriter<std::fs::File>, name: &str, bytes: &[u8], opt: SimpleFileOptions) -> Result<(), String> {
    z.start_file(name, opt).map_err(|e| format!("写不进导出包：{e}"))?;
    z.write_all(bytes).map_err(|e| format!("写不进导出包：{e}"))
}

/// 第二步：从复制出来的库（会先去掉不导出的）打包成 zip，写到 `out`。先写到 .part，写完再改名，中途出错不留半个包
pub fn pack(
    copy: &Path,
    data_dir: &Path,
    out: &Path,
    version: &str,
    now: NaiveDateTime,
    progress: &mut dyn FnMut(Progress),
) -> Result<Exported, String> {
    let bad = |e: rusqlite::Error| format!("读数据库没成功：{e}");
    let (reminders, notes, atts, devices, s) = {
        let c = Connection::open(copy).map_err(bad)?;
        c.execute_batch("PRAGMA journal_mode=DELETE").map_err(bad)?;
        scrub(&c).map_err(bad)?;
        let s = settings::load(&c).unwrap_or_default();
        (
            db::list_reminders(&c).map_err(bad)?,
            db::list_notes(&c).map_err(bad)?,
            db::list_attachments(&c).map_err(bad)?,
            db::list_devices(&c, "").map_err(bad)?,
            s,
        )
    };
    let db_bytes = std::fs::read(copy).map_err(|e| format!("读数据库没成功：{e}"))?;
    let name = out.file_name().ok_or("请选一个保存的位置")?.to_string_lossy().to_string();
    let dir = out.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| format!("建不了文件夹 {}：{e}", dir.display()))?;
    let part = dir.join(format!("{name}.part"));
    let file = std::fs::File::create(&part).map_err(|e| format!("写不进 {}：{e}。换个位置试试", dir.display()))?;
    let r = write_zip(file, data_dir, version, now, &reminders, &notes, &atts, &devices, &s, &db_bytes, progress);
    match r {
        Ok((n_atts, missing)) => {
            std::fs::rename(&part, out).map_err(|e| {
                let _ = std::fs::remove_file(&part);
                format!("存不成 {}：{e}。可能是同名的文件正开着，关掉再试", out.display())
            })?;
            Ok(Exported { path: out.display().to_string(), reminders: reminders.len(), notes: notes.len(), attachments: n_atts, missing })
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn write_zip(
    file: std::fs::File,
    data_dir: &Path,
    version: &str,
    now: NaiveDateTime,
    reminders: &[Reminder],
    notes: &[Note],
    atts: &[db::AttachmentInfo],
    devices: &[Device],
    s: &settings::Settings,
    db_bytes: &[u8],
    progress: &mut dyn FnMut(Progress),
) -> Result<(usize, Vec<String>), String> {
    let when = zip::DateTime::from_date_and_time(
        now.year().clamp(1980, 2107) as u16,
        now.month() as u8,
        now.day() as u8,
        now.hour() as u8,
        now.minute() as u8,
        (now.second() as u8).min(58),
    )
    .unwrap_or_default();
    let text = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).last_modified_time(when);
    // 图片、PDF 这类本来就压缩过，原样放进去更快
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored).last_modified_time(when).large_file(true);
    let mut z = ZipWriter::new(file);
    let total = notes.len() + atts.len() + 1;
    let mut done = 0;
    // 条数多时不用每条都告诉界面，大约每 1% 说一次
    let every = (total / 100).max(1);
    let mut tick = |done: &mut usize| {
        *done += 1;
        if done.is_multiple_of(every) || *done == total {
            progress(Progress { stage: "pack", done: *done, total });
        }
    };

    // 笔记：按分类分文件夹，同一个文件夹里重名的加 (2)
    let subs: HashMap<String, &'static str> = atts.iter().map(|a| (a.file.clone(), sub_dir(&a.mime))).collect();
    let by_id: HashMap<i64, &Reminder> = reminders.iter().map(|r| (r.id, r)).collect();
    let mut used: HashMap<String, HashSet<String>> = HashMap::new();
    // 按记下的先后起名，先记的那条不带 (2)，每次导出都一样
    let mut ordered: Vec<&Note> = notes.iter().collect();
    ordered.sort_by_key(|n| n.id);
    for n in ordered {
        let cat = s.note_cats.iter().find(|t| n.tag.as_deref() == Some(t.id.as_str()));
        let folder = cat.map(|t| safe_name(&t.name)).unwrap_or_else(|| "未分类".into());
        let names = used.entry(folder.to_lowercase()).or_default();
        let stem = unique(note_file_stem(n), names);
        let rem = n.reminder_id.and_then(|id| by_id.get(&id).copied());
        let md = note_markdown(n, cat.map(|t| t.name.as_str()), rem, &subs);
        put(&mut z, &format!("笔记/{folder}/{stem}.md"), md.as_bytes(), text)?;
        tick(&mut done);
    }

    // 附件：本机有的放进去；没有的记下来写进说明
    let att_dir = db::attachment_dir(data_dir);
    let mut missing = Vec::new();
    let mut n_atts = 0;
    for a in atts {
        let src = db::att_path(&att_dir, &a.file).ok_or(());
        match src.and_then(|p| std::fs::File::open(p).map_err(|_| ())) {
            Ok(mut f) => {
                z.start_file(format!("{ATT_DIR}/{}/{}", sub_dir(&a.mime), a.file), stored).map_err(|e| format!("写不进导出包：{e}"))?;
                std::io::copy(&mut f, &mut z).map_err(|e| format!("复制 {} 没成功：{e}", a.file))?;
                n_atts += 1;
            }
            Err(_) => missing.push(describe_missing(a, notes)),
        }
        tick(&mut done);
    }

    put(&mut z, "提醒.md", reminders_markdown(reminders, &s.tags, devices).as_bytes(), text)?;
    put(&mut z, DB_PATH, db_bytes, text)?;
    let m = Manifest {
        app: APP.into(),
        format: FORMAT,
        version: version.into(),
        exported_at: now.format(db::FMT).to_string(),
        reminders: reminders.len(),
        notes: notes.len(),
        attachments: n_atts,
    };
    put(&mut z, MANIFEST_PATH, serde_json::to_string_pretty(&m).unwrap_or_default().as_bytes(), text)?;
    put(&mut z, "说明.txt", readme(&m, &missing).as_bytes(), text)?;
    z.finish().map_err(|e| format!("写不进导出包：{e}"))?;
    tick(&mut done);
    Ok((n_atts, missing))
}

/// 缺了的附件怎么写：原文件名（图片没有，写「图片」），在哪条笔记里
fn describe_missing(a: &db::AttachmentInfo, notes: &[Note]) -> String {
    let what = if a.name.is_empty() { format!("图片 {}", a.file) } else { a.name.clone() };
    let link = format!("(att:{})", a.file);
    match notes.iter().find(|n| n.body.contains(&link)) {
        Some(n) => format!("{what}（笔记「{}」）", note_file_stem(n)),
        None => what,
    }
}

/// `说明.txt`：导出时间、版本、各有多少条、缺了的文件、怎么用这个包。记事本打开，用 Windows 的换行
pub fn readme(m: &Manifest, missing: &[String]) -> String {
    let mut lines = vec![
        "叮咪导出包".to_string(),
        String::new(),
        format!("导出时间：{}", m.exported_at),
        format!("叮咪版本：{}", m.version),
        format!("提醒 {} 条、笔记 {} 条、附件 {} 个（回收站里的不在里面）", m.reminders, m.notes, m.attachments),
        String::new(),
        "里面有什么".into(),
        "- 提醒.md：全部提醒，分成「还没做的」「周期提醒」「做完的」三段".into(),
        "- 笔记\\：一条笔记一个 Markdown 文件，按笔记分类分文件夹，没分类的在「未分类」。整个文件夹拖进 Obsidian、Typora 就能看，图片也在".into(),
        "- 附件\\：笔记里的图片（images）和文件（files）".into(),
        "- 数据\\：完整的数据库和核对用的 manifest.json，给叮咪恢复用，不用打开".into(),
        String::new(),
        "怎么用".into(),
        "- 直接看：解压后打开 提醒.md 和 笔记 文件夹".into(),
        "- 导回叮咪：在叮咪里「设置 → 数据 → 从导出包恢复」选这个 zip，不用解压。可以「合并」，也可以「替换」".into(),
        "- 恢复时只用 数据\\dingmi.db 和 附件\\，不读 Markdown：在 Markdown 里改过的不会导回叮咪".into(),
        String::new(),
        "注意".into(),
        "- 导出包里是全部内容的明文，没有加密，请注意保管".into(),
        "- 包里没有 AI Key、同步密码和配对信息；快捷框的常用、别名，桌面组件的位置只属于原来那台电脑，也不在里面".into(),
    ];
    if !missing.is_empty() {
        lines.extend([String::new(), format!("缺了 {} 个文件（不在导出的这台电脑上，也没能下载下来）：", missing.len())]);
        lines.extend(missing.iter().map(|m| format!("- {m}")));
    }
    lines.join("\r\n") + "\r\n"
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::db::{save_file, upsert_note, upsert_reminder};
    use std::io::Read;

    pub fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("dingmi-export-{name}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    /// 和界面上点「导出全部」一样：复制一份库，打包
    pub fn export_to(c: &Connection, data_dir: &Path, out: &Path) -> Exported {
        let copy = data_dir.join("copy.db");
        copy_db(c, &copy).unwrap();
        let mut steps = 0;
        let r = pack(&copy, data_dir, out, "0.1.0", at("2026-11-05 14:30"), &mut |p| {
            assert_eq!(p.stage, "pack");
            steps = p.done;
        })
        .unwrap();
        assert!(steps > 0);
        std::fs::remove_file(copy).unwrap();
        r
    }

    fn read(zip: &Path, name: &str) -> Vec<u8> {
        let mut z = zip::ZipArchive::new(std::fs::File::open(zip).unwrap()).unwrap();
        let mut f = z.by_name(name).unwrap_or_else(|_| panic!("包里没有 {name}"));
        let mut v = Vec::new();
        f.read_to_end(&mut v).unwrap();
        v
    }

    fn names(zip: &Path) -> Vec<String> {
        let z = zip::ZipArchive::new(std::fs::File::open(zip).unwrap()).unwrap();
        let mut v: Vec<String> = z.file_names().map(|n| n.unwrap().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn safe_names() {
        assert_eq!(safe_name("预算: 20万 / 方案?"), "预算 20万 方案");
        assert_eq!(safe_name("  ...  "), "无标题");
        assert_eq!(safe_name("con"), "_con");
        assert_eq!(safe_name("Com1.txt"), "_Com1.txt");
        assert_eq!(safe_name(&"长".repeat(100)).chars().count(), 60);
        let mut used = HashSet::new();
        assert_eq!(unique("周报".into(), &mut used), "周报");
        assert_eq!(unique("周报".into(), &mut used), "周报 (2)");
        assert_eq!(unique("周报".into(), &mut used), "周报 (3)");
    }

    #[test]
    fn file_name_from_title_or_body() {
        let note = |title: &str, body: &str| Note { title: title.into(), body: body.into(), ..Default::default() };
        assert_eq!(note_file_stem(&note("周四评审", "正文")), "周四评审");
        // 没标题：用正文第一行有字的开头十几个字，去掉标题、待办这些记号，跳过图片
        assert_eq!(note_file_stem(&note("", "\n![](att:a.png)\n## 下周要带的材料和报价单还有合同原件")), "下周要带的材料和报价单还有合同");
        assert_eq!(note_file_stem(&note(" ", "- [ ] 买菜")), "买菜");
        assert_eq!(note_file_stem(&note("", "")), "无标题");
    }

    #[test]
    fn front_matter() {
        let n = Note {
            title: "周四评审要带的材料".into(),
            body: "- [ ] 方案\n| a | b<br>c |\n\n".into(),
            created_at: "2026-11-03 14:20:05".into(),
            updated_at: "2026-11-05 09:12:00".into(),
            ..Default::default()
        };
        let r = Reminder { title: "周四评审".into(), due_at: "2026-11-06 15:00".into(), ..Default::default() };
        let md = note_markdown(&n, Some("资料"), Some(&r), &HashMap::new());
        assert_eq!(
            md,
            "---\ntitle: 周四评审要带的材料\ncreated: 2026-11-03 14:20\nupdated: 2026-11-05 09:12\ncategory: 资料\nreminder: 2026-11-06 15:00 周四评审\n---\n\n- [ ] 方案\n| a | b<br>c |\n"
        );
        // 没分类、没关联提醒的不写这两行；会被 YAML 误会的标题加引号
        let n = Note { title: "预算: 20万 #急".into(), created_at: "2026-11-03 14:20:05".into(), updated_at: "2026-11-03 14:20:05".into(), ..Default::default() };
        let md = note_markdown(&n, None, None, &HashMap::new());
        assert!(md.starts_with("---\ntitle: \"预算: 20万 #急\"\ncreated: 2026-11-03 14:20\nupdated: 2026-11-03 14:20\n---\n"));
        assert_eq!(yaml("true"), "\"true\"");
        assert_eq!(yaml("2026"), "\"2026\"");
        assert_eq!(yaml("周报：本周"), "周报：本周");
    }

    #[test]
    fn attachment_links() {
        let subs = HashMap::from([("a.png".to_string(), "images"), ("c.pdf".to_string(), "files")]);
        let body = "看这张\n![](att:a.png)\n  ![说明](att:b.jpg)  \n![](https://x/y.png)\n[报价.pdf](att:c.pdf)\n文字里也有 ![](att:a.png) 一张\n[网址](https://x)\n[坏的](att:)";
        assert_eq!(
            link_attachments(body, &subs),
            "看这张\n![](../../附件/images/a.png)\n  ![说明](../../附件/images/b.jpg)  \n![](https://x/y.png)\n[报价.pdf](../../附件/files/c.pdf)\n文字里也有 ![](../../附件/images/a.png) 一张\n[网址](https://x)\n[坏的](att:)"
        );
    }

    #[test]
    fn reminder_list() {
        let r = |title: &str, due: &str| Reminder { title: title.into(), due_at: due.into(), ..Default::default() };
        let mut a = r("周四评审", "2026-11-06 15:00");
        a.tag = Some("work".into());
        a.urgent = true;
        a.detail = "带上方案和报价单\n\n".into();
        let mut b = r("交报销", "2026-11-07 09:00");
        b.tag = Some("work".into());
        b.only_on = vec!["office".into()];
        let mut c = r("看一下邮件", "2026-11-06 09:30");
        c.tag = Some("work".into());
        c.repeat = Repeat { kind: "weekdays".into(), ..Default::default() };
        let mut d = r("交房租", "2026-12-01 10:00");
        d.tag = Some("life".into());
        d.repeat = Repeat { kind: "monthly".into(), ..Default::default() };
        let mut e = r("取快递", "2026-11-04 18:00");
        e.done = true;
        e.completed_at = Some("2026-11-04 18:20".into());
        let mut gone = r("删掉的", "2026-11-04 18:00");
        gone.deleted_at = Some("2026-11-04 18:00:00".into());
        let devices = vec![Device { uid: "office".into(), name: "公司电脑".into(), me: false }];
        let md = reminders_markdown(&[b, e, d, c, a, gone], &settings::default_tags(), &devices);
        assert_eq!(
            md,
            "# 提醒\n\n## 还没做的\n- [ ] 2026-11-06 15:00 周四评审 · 工作 · 紧急\n  带上方案和报价单\n- [ ] 2026-11-07 09:00 交报销 · 工作 · 只在：公司电脑\n\n## 周期提醒\n- 每个工作日 09:30 看一下邮件 · 工作\n- 每月 1 号 10:00 交房租 · 生活\n\n## 做完的\n- [x] 2026-11-04 18:00 取快递（11-04 18:20 做完）\n"
        );
        assert_eq!(reminders_markdown(&[], &[], &[]), "# 提醒\n\n## 还没做的\n（没有）\n\n## 周期提醒\n（没有）\n\n## 做完的\n（没有）\n");
        assert_eq!(repeat_label(&Repeat { kind: "weekly".into(), days: vec![3, 1], ..Default::default() }, ""), "每周一、三");
        assert_eq!(repeat_label(&Repeat { kind: "weekly".into(), ..Default::default() }, "2026-11-05 09:00"), "每周四");
        assert_eq!(repeat_label(&Repeat { kind: "interval".into(), every: 3, ..Default::default() }, ""), "每 3 天");
        assert_eq!(repeat_label(&Repeat { kind: "weekly".into(), days: vec![5], every: 2, ..Default::default() }, ""), "每两周的周五");
        assert_eq!(repeat_label(&Repeat { kind: "yearly".into(), month: 3, monthday: 5, ..Default::default() }, ""), "每年 3 月 5 号");
        assert_eq!(repeat_label(&Repeat { kind: "month_last".into(), ..Default::default() }, ""), "每月最后一天");
        assert_eq!(repeat_label(&Repeat { kind: "month_nth".into(), nth: -1, days: vec![5], ..Default::default() }, ""), "每月最后一个周五");
        assert_eq!(repeat_label(&Repeat { kind: "month_nth".into(), ..Default::default() }, "2026-11-05 09:00"), "每月第一个周四");
        assert_eq!(repeat_label(&Repeat { kind: "lunar_yearly".into(), month: 8, monthday: 15, ..Default::default() }, ""), "每年农历八月十五");
        assert_eq!(repeat_label(&Repeat { kind: "lunar_yearly".into(), ..Default::default() }, "2026-09-25 09:00"), "每年农历八月十五 · 9月25日");
    }

    #[test]
    fn default_location() {
        let p = default_path(Path::new(r"C:\Users\me\Documents"), at("2026-11-05 14:30"));
        assert_eq!(p, Path::new(r"C:\Users\me\Documents").join("叮咪导出").join("叮咪导出_2026-11-05_1430.zip"));
    }

    #[test]
    fn exports_everything_but_secrets() {
        let dir = temp("all");
        let c = db::open(&dir).unwrap();
        let mut s = settings::Settings { ai_key: "sk-secret-a".into(), ..Default::default() };
        s.launcher.aliases.push(crate::launcher::Alias { alias: "wx".into(), id: "app:wechat".into() });
        settings::save(&c, &s).unwrap();
        for (k, v) in [("sync", "{\"host\":\"h\"}"), ("sync_seq", "42"), ("launch_stats", "{}"), ("widget_pos", "1,2")] {
            db::kv_set(&c, k, v).unwrap();
        }
        c.execute_batch("CREATE TABLE folder_sync(secret TEXT); INSERT INTO folder_sync VALUES('KEY-IN-TABLE');").unwrap();

        let img = save_file(&c, &dir, "image/png", "", b"png", None, None).unwrap();
        let pdf = save_file(&c, &dir, "application/pdf", "报价.pdf", b"%PDF", None, None).unwrap();
        // 别的电脑上传的文件，还没下载到这台电脑
        let far = save_file(&c, &dir, "application/zip", "合同.zip", b"zip", None, None).unwrap();
        std::fs::remove_file(db::attachment_dir(&dir).join(&far)).unwrap();
        let rid = upsert_reminder(&c, &Reminder { title: "周四评审".into(), due_at: "2026-11-06 15:00".into(), ..Default::default() }).unwrap();
        let note = |title: &str, body: &str, tag: Option<&str>| Note { title: title.into(), body: body.into(), tag: tag.map(Into::into), ..Default::default() };
        upsert_note(&c, &Note { reminder_id: Some(rid), ..note("周报", &format!("![](att:{img})\n[报价.pdf](att:{pdf})\n[合同.zip](att:{far})"), Some("ref")) }).unwrap();
        upsert_note(&c, &note("周报", "第二条", Some("ref"))).unwrap();
        upsert_note(&c, &note("", "没标题的笔记正文开头", None)).unwrap();
        // 回收站里的不导出，它的图片也不导出
        let trash_img = save_file(&c, &dir, "image/png", "", b"old", None, None).unwrap();
        let tid = upsert_note(&c, &note("扔掉的", &format!("![](att:{trash_img})"), None)).unwrap();
        let tuid = db::get_note(&c, tid).unwrap().unwrap().uid;
        db::attach_to_note(&c, std::slice::from_ref(&trash_img), &tuid).unwrap();
        db::delete_note(&c, tid).unwrap();
        let gone = upsert_reminder(&c, &Reminder { title: "删掉的提醒".into(), due_at: "2026-11-06 15:00".into(), ..Default::default() }).unwrap();
        db::delete_reminder(&c, gone).unwrap();

        let out = dir.join("out").join("叮咪导出_2026-11-05_1430.zip");
        let r = export_to(&c, &dir, &out);
        assert_eq!((r.reminders, r.notes, r.attachments), (1, 3, 2));
        assert_eq!(r.missing, vec!["合同.zip（笔记「周报」）".to_string()]);
        assert!(!dir.join("out").join("叮咪导出_2026-11-05_1430.zip.part").exists());

        assert_eq!(
            names(&out),
            [
                "提醒.md".to_string(),
                "数据/dingmi.db".into(),
                "数据/manifest.json".into(),
                "笔记/未分类/没标题的笔记正文开头.md".into(),
                "笔记/资料/周报 (2).md".into(),
                "笔记/资料/周报.md".into(),
                "说明.txt".into(),
                format!("附件/files/{pdf}"),
                format!("附件/images/{img}"),
            ]
        );
        let md = String::from_utf8(read(&out, "笔记/资料/周报.md")).unwrap();
        assert!(md.contains("category: 资料\nreminder: 2026-11-06 15:00 周四评审\n---\n"), "{md}");
        assert!(md.contains(&format!("![](../../附件/images/{img})\n[报价.pdf](../../附件/files/{pdf})")));
        assert_eq!(read(&out, &format!("附件/images/{img}")), b"png");
        assert!(String::from_utf8(read(&out, "提醒.md")).unwrap().contains("- [ ] 2026-11-06 15:00 周四评审\n"));
        let readme = String::from_utf8(read(&out, "说明.txt")).unwrap();
        assert!(readme.contains("叮咪版本：0.1.0\r\n") && readme.contains("- 合同.zip（笔记「周报」）\r\n"));

        let m: Manifest = serde_json::from_slice(&read(&out, MANIFEST_PATH)).unwrap();
        assert_eq!(
            m,
            Manifest { app: "dingmi".into(), format: FORMAT, version: "0.1.0".into(), exported_at: "2026-11-05 14:30".into(), reminders: 1, notes: 3, attachments: 2 }
        );

        // 包里的数据库：密钥、只属于这台电脑的、回收站里的都去掉了，文件里也找不到
        let bytes = read(&out, DB_PATH);
        for secret in ["sk-secret-a", "RELEASE-SECRET", "KEY-IN-TABLE", "app:wechat", "扔掉的", "删掉的提醒"] {
            assert!(!bytes.windows(secret.len()).any(|w| w == secret.as_bytes()), "包里还有 {secret}");
        }
        let unpacked = dir.join("unpacked.db");
        std::fs::write(&unpacked, &bytes).unwrap();
        let p = Connection::open(&unpacked).unwrap();
        let keys: Vec<String> = p.prepare("SELECT key FROM kv").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        assert_eq!(keys, ["settings"]);
        let got = settings::load(&p).unwrap();
        assert_eq!((got.ai_key.as_str(), got.launcher.aliases.len()), ("", 0));
        let shared: serde_json::Value = serde_json::from_str(&db::shared_get(&p, settings::SHARED_UID).unwrap()).unwrap();
        assert!(shared["ai_key"].is_null() && shared["snooze_min"] == 10);
        let n: i64 = p.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name='folder_sync'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
        assert_eq!(db::list_trash(&p).unwrap().notes.len(), 0);
        // 本机的库没被动过
        assert_eq!(settings::load(&c).unwrap().ai_key, "sk-secret-a");
        assert_eq!(db::kv_get(&c, "sync_seq").as_deref(), Some("42"));
    }
}
