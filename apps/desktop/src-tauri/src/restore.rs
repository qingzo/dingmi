//! 从导出包恢复（需求-0.1 第 4.2 节）：只读包里的 `数据/dingmi.db` 和 `附件/`，不读 Markdown。
//! 合并：和同步一样一条条合进来，同一条按版本号大的（最后改的）为准（见 sync/records.rs）；
//! 替换：先把本机备份到 backups/，清空后再合进来。数据库连接一直开着，不换文件，都在一个事务里做。
//! 恢复进来的改动算本机改过的（dirty），开着同步时会传到别的电脑。

use crate::export::{Manifest, APP, ATT_DIR, DB_PATH, FORMAT, MANIFEST_PATH};
use crate::sync::records;
use crate::{clock, db};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use zip::ZipArchive;

const NOT_OURS: &str = "这不是叮咪的导出包。请选「导出全部」得到的那个 zip";
const TOO_NEW: &str = "这个导出包是新版叮咪导出的，这台电脑上的叮咪读不懂。请先更新叮咪再恢复";

/// 选好包后先给用户看的：哪天导出的、各有多少条
#[derive(Serialize, Clone, Debug)]
pub struct Peek {
    #[serde(flatten)]
    pub manifest: Manifest,
    /// 开着同步，只能合并
    pub sync_on: bool,
}

/// 恢复完的结果：合进来（或改了）几条
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Restored {
    pub reminders: usize,
    pub notes: usize,
    pub attachments: usize,
    /// 替换前本机数据备份到哪了
    pub backup: Option<String>,
}

fn open_zip(path: &Path) -> Result<ZipArchive<std::fs::File>, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("打不开 {}：{e}", path.display()))?;
    ZipArchive::new(f).map_err(|_| NOT_OURS.to_string())
}

/// 核对 `数据/manifest.json`：是不是叮咪的包、格式这台电脑读不读得懂
pub fn read_manifest(path: &Path) -> Result<Manifest, String> {
    let mut z = open_zip(path)?;
    let mut text = String::new();
    z.by_name(MANIFEST_PATH)
        .map_err(|_| NOT_OURS.to_string())?
        .take(1 << 20)
        .read_to_string(&mut text)
        .map_err(|_| NOT_OURS.to_string())?;
    let m: Manifest = serde_json::from_str(&text).map_err(|_| NOT_OURS.to_string())?;
    if m.app != APP {
        return Err(NOT_OURS.into());
    }
    if m.format > FORMAT {
        return Err(TOO_NEW.into());
    }
    if z.by_name(DB_PATH).is_err() {
        return Err(NOT_OURS.into());
    }
    Ok(m)
}

/// 包里的附件放进 attachments/：本机已经有的（同一个编号就是同一个文件）不动。返回包里有哪些附件
fn unpack_attachments(z: &mut ZipArchive<std::fs::File>, data_dir: &Path) -> Result<Vec<String>, String> {
    let dir = db::attachment_dir(data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("建不了附件文件夹：{e}"))?;
    let prefix = format!("{ATT_DIR}/");
    let mut files = Vec::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| format!("导出包坏了：{e}"))?;
        // 名字不是 UTF-8 的（不是叮咪导出的）不认
        let Ok(name) = f.name().map(|n| n.into_owned()) else { continue };
        let Some((sub, file)) = name.strip_prefix(&prefix).and_then(|r| r.split_once('/')) else { continue };
        // 文件名会拼进路径：只认附件文件名的规矩（db::valid_att_file），zip 里的「../」、绝对路径、设备名都不认
        if !matches!(sub, "images" | "files") || !db::valid_att_file(file) {
            continue;
        }
        files.push(file.to_string());
        let Some(dst) = db::att_path(&dir, file) else { continue };
        if dst.exists() {
            continue;
        }
        // 先写临时文件再改名，写一半不会留下坏文件；单个附件不会超过 50 MB
        let part = dir.join(format!("{file}.part"));
        let write = |f: &mut dyn Read| -> std::io::Result<()> {
            let mut out = std::fs::File::create(&part)?;
            std::io::copy(&mut f.take(db::MAX_ATTACHMENT as u64 + 1), &mut out)?;
            Ok(())
        };
        if let Err(e) = write(&mut f).and_then(|_| std::fs::rename(&part, &dst)) {
            let _ = std::fs::remove_file(&part);
            return Err(format!("附件放不进去：{e}"));
        }
        // 导出包可能是别人给的：解出来的附件也标上「来自网络」
        db::mark_from_web(&dst);
    }
    Ok(files)
}

/// 用完就删的临时文件夹
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 解开的导出包：数据库在临时文件夹里（用完自动删），附件已经放进 attachments/
pub struct Unpacked {
    tmp: TempDir,
    files: Vec<String>,
}

/// 第一步（不用拿数据库的锁）：解开导出包。附件先放好，合并时才知道哪些原文件在这台电脑上
pub fn unpack(path: &Path, data_dir: &Path) -> Result<Unpacked, String> {
    read_manifest(path)?;
    let tmp = TempDir(std::env::temp_dir().join(format!("dingmi-restore-{}", uuid::Uuid::new_v4().simple())));
    std::fs::create_dir_all(&tmp.0).map_err(|e| format!("建不了临时文件夹：{e}"))?;
    let mut z = open_zip(path)?;
    {
        let mut f = z.by_name(DB_PATH).map_err(|_| NOT_OURS.to_string())?;
        let mut out = std::fs::File::create(tmp.0.join("dingmi.db")).map_err(|e| format!("解不开导出包：{e}"))?;
        std::io::copy(&mut f, &mut out).map_err(|e| format!("解不开导出包：{e}"))?;
    }
    let files = unpack_attachments(&mut z, data_dir)?;
    Ok(Unpacked { tmp, files })
}

/// 解开再合进来（测试用；界面上分两步，解包时不拿数据库的锁）
#[cfg(test)]
pub fn restore(c: &Connection, data_dir: &Path, path: &Path, replace: bool) -> Result<Restored, String> {
    merge_in(c, data_dir, &unpack(path, data_dir)?, replace)
}

/// 第二步：合进来。`replace` 为 true 时先备份本机再清空（开着同步时不能替换，由调用的地方把关）
pub fn merge_in(c: &Connection, data_dir: &Path, u: &Unpacked, replace: bool) -> Result<Restored, String> {
    let broken = |e: rusqlite::Error| format!("导出包里的数据坏了，恢复不了：{e}");
    let pkg = Connection::open(u.tmp.0.join("dingmi.db")).map_err(broken)?;
    db::add_new_columns(&pkg).map_err(broken)?;
    // 包里每一条都当成「要合进来的」，合完一条划掉一条
    db::mark_all_dirty(&pkg).map_err(broken)?;

    let mut out = Restored::default();
    if replace {
        let name = format!("恢复前-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"));
        let file = db::snapshot(c, data_dir, &name).map_err(|e| format!("备份这台电脑的数据没成功，所以没有替换：{e}"))?;
        out.backup = Some(file.display().to_string());
    }
    let err = |e: rusqlite::Error| format!("恢复没成功，这台电脑上的数据没有动：{e}");
    let tx = c.unchecked_transaction().map_err(err)?;
    if replace {
        db::wipe_synced(&tx).map_err(err)?;
    }
    let mut reminders = Vec::new();
    loop {
        let batch = records::dirty(&pkg, 500).map_err(broken)?;
        if batch.is_empty() {
            break;
        }
        for d in &batch {
            clock::observe(&d.ver);
            if records::apply(&tx, data_dir, &d.ver, &d.payload).map_err(err)? {
                // 合进来的算本机的改动，等着同步出去
                tx.execute(&format!("UPDATE {} SET dirty=1 WHERE uid=?1", d.table), [&d.uid]).map_err(err)?;
                match d.table {
                    "reminders" => reminders.push(d.uid.clone()),
                    "notes" => out.notes += 1,
                    "attachments" => out.attachments += 1,
                    _ => {}
                }
            }
            records::clear_dirty(&pkg, d.table, &d.uid, &d.ver).map_err(broken)?;
        }
    }
    records::fix_links(&tx).map_err(err)?;
    // 本机原来缺的原文件，包里带来了
    for f in &u.files {
        tx.execute("UPDATE attachments SET local_ok=1 WHERE file=?1 AND purged=0", [f]).map_err(err)?;
    }
    // 已经过点的一次性提醒恢复后不立刻弹出来（和从回收站恢复一样）
    let now = db::now_min();
    for uid in &reminders {
        tx.execute(
            "UPDATE reminders SET state=?2 WHERE uid=?1 AND state=?3 AND done=0 AND deleted_at IS NULL
               AND COALESCE(json_extract(repeat, '$.kind'), 'none') IN ('', 'none')
               AND COALESCE(snoozed_until, due_at) <= ?4",
            params![uid, db::ST_SEEN, db::ST_WAIT, now],
        )
        .map_err(err)?;
    }
    tx.commit().map_err(err)?;
    out.reminders = reminders.len();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{get_note, list_notes, list_reminders, save_file, upsert_note, upsert_reminder, Note, Reminder};
    use crate::export::tests::{export_to, temp};
    use crate::settings;
    use std::io::Write;

    fn dirty_count(c: &Connection, table: &str) -> i64 {
        c.query_row(&format!("SELECT COUNT(*) FROM {table} WHERE dirty=1"), [], |r| r.get(0)).unwrap()
    }

    fn note_by_title(c: &Connection, title: &str) -> Note {
        list_notes(c).unwrap().into_iter().find(|n| n.title == title).unwrap_or_else(|| panic!("没有「{title}」"))
    }

    /// 电脑 A：一条提醒、一条带图片的笔记、改过的设置（有 AI Key）
    fn computer_a() -> (PathBuf, Connection) {
        let dir = temp("a");
        let c = db::open(&dir).unwrap();
        let mut s = settings::Settings { ai_key: "sk-a".into(), snooze_min: 30, ..Default::default() };
        s.note_cats[0].name = "以后看".into();
        settings::save(&c, &s).unwrap();
        let img = save_file(&c, &dir, "image/png", "", b"png-a", None, None).unwrap();
        upsert_reminder(&c, &Reminder { title: "交报销".into(), due_at: "2099-01-01 09:00".into(), ..Default::default() }).unwrap();
        // 过点了还没做的一次性提醒，恢复后不立刻弹出来
        upsert_reminder(&c, &Reminder { title: "过点的".into(), due_at: "2000-01-01 09:00".into(), ..Default::default() }).unwrap();
        upsert_note(&c, &Note { title: "方案".into(), body: format!("v1\n![](att:{img})"), tag: Some("toread".into()), ..Default::default() }).unwrap();
        (dir, c)
    }

    /// 攻击：别人给的导出包里附件的名字带 ../、绝对路径、设备名、备用数据流，想把文件写到附件文件夹外面。
    /// 叮咪不用 zip 库的 extract（RUSTSEC-2026-0367/0368/0369/0429 都在 extract 里），每个名字自己核对；解出来的标上「来自网络」
    #[test]
    fn evil_package_cannot_write_outside() {
        let base = temp("evil-pkg");
        let (adir, a) = computer_a();
        let good = base.join("好的.zip");
        export_to(&a, &adir, &good);
        // 把好的包抄一遍，再加几个坏名字
        let mut src = ZipArchive::new(std::fs::File::open(&good).unwrap()).unwrap();
        let evil = base.join("坏的.zip");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&evil).unwrap());
        for i in 0..src.len() {
            let f = src.by_index(i).unwrap();
            z.raw_copy_file(f).unwrap();
        }
        let bad_names = [
            "附件/images/../../../escaped1.png",
            "附件/files/..\\..\\escaped2.exe",
            "附件/files/C:\\escaped3.exe",
            "附件/files/CON.txt",
            "附件/files/a.txt:Zone.Identifier",
            "附件/images/sub/escaped4.png",
        ];
        for n in bad_names {
            z.start_file(n, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(b"MZ").unwrap();
        }
        z.finish().unwrap();
        let bdir = base.join("b");
        let b = db::open(&bdir).unwrap();
        restore(&b, &bdir, &evil, false).unwrap();
        let att = db::attachment_dir(&bdir);
        let mut names: Vec<String> = std::fs::read_dir(&att).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        // 只有好的那张图片
        assert_eq!(names.len(), 1, "{names:?}");
        for n in ["escaped1.png", "escaped2.exe", "escaped3.exe", "escaped4.png"] {
            assert!(!base.join(n).exists() && !bdir.join(n).exists() && !Path::new(r"C:\").join(n).exists(), "{n}");
        }
        if cfg!(windows) {
            assert!(db::from_web(&att.join(&names[0])));
        }
    }

    #[test]
    fn checks_manifest() {
        let dir = temp("peek");
        let (adir, a) = computer_a();
        let pkg = dir.join("包.zip");
        export_to(&a, &adir, &pkg);
        let m = read_manifest(&pkg).unwrap();
        assert_eq!((m.reminders, m.notes, m.attachments), (2, 1, 1));

        // 不是 zip、不是叮咪的 zip
        std::fs::write(dir.join("x.zip"), b"hello").unwrap();
        assert_eq!(read_manifest(&dir.join("x.zip")).unwrap_err(), NOT_OURS);
        let write_zip = |name: &str, files: &[(&str, &str)]| {
            let p = dir.join(name);
            let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
            for (n, text) in files {
                z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
                z.write_all(text.as_bytes()).unwrap();
            }
            z.finish().unwrap();
            p
        };
        assert_eq!(read_manifest(&write_zip("other.zip", &[("readme.txt", "hi")])).unwrap_err(), NOT_OURS);
        let other_app = serde_json::to_string(&Manifest { app: "uTools".into(), ..m.clone() }).unwrap();
        assert_eq!(read_manifest(&write_zip("app.zip", &[(MANIFEST_PATH, &other_app), (DB_PATH, "")])).unwrap_err(), NOT_OURS);
        // 新版叮咪导出的包
        let newer = serde_json::to_string(&Manifest { format: FORMAT + 1, ..m.clone() }).unwrap();
        assert_eq!(read_manifest(&write_zip("new.zip", &[(MANIFEST_PATH, &newer), (DB_PATH, "")])).unwrap_err(), TOO_NEW);
        // 没有数据库的
        let ok = serde_json::to_string(&m).unwrap();
        assert_eq!(read_manifest(&write_zip("nodb.zip", &[(MANIFEST_PATH, &ok)])).unwrap_err(), NOT_OURS);
    }

    #[test]
    fn merge_keeps_both_and_newest_wins() {
        let (adir, a) = computer_a();
        let p1 = adir.join("p1.zip");
        export_to(&a, &adir, &p1);

        // 电脑 B 有自己的数据和 AI Key
        let bdir = temp("b");
        let b = db::open(&bdir).unwrap();
        settings::save(&b, &settings::Settings { ai_key: "sk-b".into(), ..Default::default() }).unwrap();
        upsert_note(&b, &Note { title: "B 自己的".into(), ..Default::default() }).unwrap();
        c_clean(&b);

        let r = restore(&b, &bdir, &p1, false).unwrap();
        assert_eq!((r.reminders, r.notes, r.attachments, r.backup), (2, 1, 1, None));
        assert_eq!(list_notes(&b).unwrap().len(), 2);
        assert_eq!(list_reminders(&b).unwrap().len(), 2);
        // 图片放进来了，记录上也知道原图在
        let img = db::list_attachments(&b).unwrap().remove(0);
        assert!(img.local);
        assert_eq!(std::fs::read(db::attachment_dir(&bdir).join(&img.file)).unwrap(), b"png-a");
        // 恢复进来的算本机改过的，开着同步时会传出去
        assert_eq!((dirty_count(&b, "notes"), dirty_count(&b, "reminders"), dirty_count(&b, "attachments")), (1, 2, 1));
        // 共用设置也按谁后改的为准：B 的设置比包里的晚改，留着 B 的
        let s = settings::load(&b).unwrap();
        assert_eq!((s.ai_key.as_str(), s.snooze_min, s.note_cats[0].name.as_str()), ("sk-b", 10, "待看"));
        // 过点的一次性提醒不弹
        let past = list_reminders(&b).unwrap().into_iter().find(|r| r.title == "过点的").unwrap();
        assert_eq!(past.state, db::ST_SEEN);
        assert!(db::due_now(&b, &db::now_min(), "any").unwrap().is_empty());

        // B 后来又改了这条笔记：再导一次旧包，B 改的留着（本机的更新）
        let mut n = note_by_title(&b, "方案");
        n.body = "B 改的".into();
        upsert_note(&b, &n).unwrap();
        c_clean(&b);
        // 图片原文件丢了（像是同步过来还没下载）：再导一次同一个包，记录不用改，原文件补回来
        std::fs::remove_file(db::attachment_dir(&bdir).join(&img.file)).unwrap();
        b.execute("UPDATE attachments SET local_ok=0", []).unwrap();
        let r = restore(&b, &bdir, &p1, false).unwrap();
        assert_eq!((r.reminders, r.notes, r.attachments), (0, 0, 0));
        assert_eq!(get_note(&b, n.id).unwrap().unwrap().body, "B 改的");
        assert_eq!(dirty_count(&b, "notes"), 0);
        assert!(db::list_attachments(&b).unwrap()[0].local);
        assert!(db::attachment_dir(&bdir).join(&img.file).exists());

        // A 上更晚改的：包里的新，盖过 B 的
        let mut an = note_by_title(&a, "方案");
        an.body = "A 后来改的".into();
        upsert_note(&a, &an).unwrap();
        settings::save(&a, &settings::Settings { snooze_min: 15, ..settings::load(&a).unwrap() }).unwrap();
        let p2 = adir.join("p2.zip");
        export_to(&a, &adir, &p2);
        let r = restore(&b, &bdir, &p2, false).unwrap();
        assert_eq!(r.notes, 1);
        assert_eq!(get_note(&b, n.id).unwrap().unwrap().body, "A 后来改的");
        assert_eq!(dirty_count(&b, "notes"), 1);
        // A 后改的设置合进来了，AI Key 还是 B 自己的（包里没有 AI Key）
        let s = settings::load(&b).unwrap();
        assert_eq!((s.ai_key.as_str(), s.snooze_min, s.note_cats[0].name.as_str()), ("sk-b", 15, "以后看"));
    }

    #[test]
    fn replace_backs_up_then_matches_package() {
        let (adir, a) = computer_a();
        let pkg = adir.join("p.zip");
        export_to(&a, &adir, &pkg);

        let bdir = temp("b");
        let b = db::open(&bdir).unwrap();
        settings::save(&b, &settings::Settings { ai_key: "sk-b".into(), hk_flash: "Alt+Q".into(), ..Default::default() }).unwrap();
        upsert_note(&b, &Note { title: "B 自己的".into(), ..Default::default() }).unwrap();
        upsert_reminder(&b, &Reminder { title: "B 的提醒".into(), due_at: "2099-01-01 09:00".into(), ..Default::default() }).unwrap();
        db::kv_set(&b, "launch_stats", "{\"n\":{}}").unwrap();

        let r = restore(&b, &bdir, &pkg, true).unwrap();
        let backup = PathBuf::from(r.backup.clone().unwrap());
        assert!(backup.exists() && backup.parent().unwrap().ends_with("backups"));
        // 这台电脑变成包里的样子
        let titles: Vec<String> = list_notes(&b).unwrap().into_iter().map(|n| n.title).collect();
        assert_eq!(titles, ["方案"]);
        let mut rs: Vec<String> = list_reminders(&b).unwrap().into_iter().map(|r| r.title).collect();
        rs.sort();
        assert_eq!(rs, ["交报销", "过点的"]);
        assert_eq!((r.reminders, r.notes, r.attachments), (2, 1, 1));
        assert_eq!((dirty_count(&b, "notes"), dirty_count(&b, "reminders")), (1, 2));
        // 只属于这台电脑的（快捷键、启动器用过几次）和 AI Key 不动
        let s = settings::load(&b).unwrap();
        assert_eq!((s.ai_key.as_str(), s.hk_flash.as_str(), s.snooze_min), ("sk-b", "Alt+Q", 30));
        assert!(db::kv_get(&b, "launch_stats").is_some());
        // 备份里还是原来的
        let old = Connection::open(&backup).unwrap();
        assert_eq!(list_notes(&old).unwrap()[0].title, "B 自己的");

        // 导出 → 替换 → 再导出，内容一样
        let again = bdir.join("again.zip");
        let r2 = export_to(&b, &bdir, &again);
        assert_eq!((r2.reminders, r2.notes, r2.attachments), (2, 1, 1));
    }

    /// 模拟已经同步过：都不算本机改过
    fn c_clean(c: &Connection) {
        c.execute_batch("UPDATE reminders SET dirty=0; UPDATE notes SET dirty=0; UPDATE attachments SET dirty=0; UPDATE shared SET dirty=0; UPDATE devices SET dirty=0;").unwrap();
    }
}
