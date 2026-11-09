//! 另存一份备份（一主一备的「备」）。需求见需求-0.1 第 3.5 节。
//!
//! 备份文件夹就是一个和网盘文件夹同步一样格式的「叮咪同步」文件夹（见 folder.rs），只往里写、不从里面读。
//! 主的坏了，在任意一台电脑上把同步方式换成「网盘文件夹」、选这个文件夹、输密码，它就变成主的了。
//!
//! - 备份有自己的主密钥（凭据管理器里的 BACKUP_KEY），和同步的分开：换同步方式、停止同步都不影响备份。
//!   space.json 的盒子 1 用备份密码锁着它。备份密码就是同步密码：开着同步时先核对；不同步时设一个。
//! - 每次写一份完整快照（全部记录），写完删掉旧的；全部记录的 (uid, 版本号) 算一个指纹，没变化就不写。
//!   有改动时最多每 30 分钟写一次；附件每 2 分钟看一次，新的马上写。
//! - 设置在 kv 的 "sync_backup"：文件夹位置、上次备份的时间、指纹。

use super::crypto::{self, DataKey};
use super::folder::{self, Folder, SpaceFile};
use super::records;
use super::secret::{self, BACKUP_KEY, DEVICE_KEY};
use super::err;
use crate::{db, AppState};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use chrono::{Local, NaiveDateTime};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Manager};

/// 隔这么久看一眼：新附件马上写，记录有改动、离上次写满 30 分钟就写快照
const CHECK_EVERY: Duration = Duration::from_secs(2 * 60);
/// 有改动时最多每这么多分钟写一份快照
const SNAP_EVERY_MIN: i64 = 30;
const WRONG_PASSWORD: &str = "同步密码不对";
const MISSING: &str = "找不到备份文件夹：U 盘拔了、网盘没登录或者文件夹被移走了。找回来以后会自动接着备份";

/// 同一时间只做一件事：后台的一轮、「现在备份」、开关备份、换密码不会撞在一起
static BUSY: Mutex<()> = Mutex::new(());

fn busy() -> std::sync::MutexGuard<'static, ()> {
    BUSY.lock().unwrap_or_else(|e| e.into_inner())
}

/// 备份的设置，存在 kv 表的 "sync_backup"；主密钥在 Windows 凭据管理器里
#[derive(Serialize, Deserialize, Clone, Default)]
struct Cfg {
    /// 「叮咪同步」文件夹的完整路径
    dir: String,
    /// 上次写快照的时间
    #[serde(default)]
    last: Option<String>,
    /// 上次写的快照里全部记录的指纹，没变化就不再写
    #[serde(default)]
    mark: Option<String>,
    /// 上一轮出的错，设置里显示
    #[serde(default)]
    error: Option<String>,
    /// 换了同步密码时备份文件夹不在（U 盘拔了）：新的 space.json 先记在这，下一轮再写
    #[serde(default)]
    space: Option<SpaceFile>,
}

#[derive(Serialize, Clone, Default)]
pub struct BackupStatus {
    pub on: bool,
    /// 「叮咪同步」文件夹的位置
    pub dir: Option<String>,
    /// 上次备份的时间
    pub last: Option<String>,
    pub error: Option<String>,
}

#[derive(PartialEq, Clone, Copy)]
enum When {
    /// 「现在备份」：不管有没有变化都写一份
    Now,
    /// 后台：记录变了、离上次写满 30 分钟才写
    IfChanged,
}

fn load(c: &Connection) -> Option<Cfg> {
    serde_json::from_str(&db::kv_get(c, "sync_backup")?).ok()
}

fn save(c: &Connection, cfg: &Cfg) -> Result<(), String> {
    db::kv_set(c, "sync_backup", &serde_json::to_string(cfg).map_err(err)?).map_err(err)
}

fn key() -> Option<DataKey> {
    secret::get(BACKUP_KEY).and_then(|k| k.try_into().ok())
}

/// 两个「叮咪同步」文件夹是不是同一个（大小写、末尾的斜杠、写法不同都算同一个）
fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        let p = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        p.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase()
    };
    norm(a) == norm(b)
}

/// 同步方式是网盘文件夹、而且就是这个备份文件夹（拿备份接着用了）：它已经是主的了
fn is_sync_dir(app: &AppHandle, dir: &Path) -> bool {
    super::load_folder(app).is_some_and(|fc| same_dir(dir, Path::new(&fc.dir)))
}

/// 备份里显示的名字：「<这台电脑的名字> 的备份」，开着同步就用同步时起的名字
fn device_name(app: &AppHandle) -> String {
    let name = super::load_folder(app)
        .map(|c| c.name)
        .or_else(|| super::load_config(app).map(|c| c.name))
        .unwrap_or_else(super::default_name);
    format!("{name} 的备份")
}

/// 备份写在单独的设备编号下，不用本机的编号：拿备份当同步接着用时，同一台电脑也能把它读回来
/// （网盘文件夹同步不读本机编号下的东西）
fn backup_id(device: &str) -> String {
    format!("{device}-backup")
}

/// 全部记录的 (uid, 版本号) 算出来的指纹：有一条改了、加了、删了都会变
fn fingerprint(c: &Connection) -> Result<String, String> {
    let mut h = Sha256::new();
    for (uid, ver) in records::versions(c).map_err(err)? {
        h.update(format!("{uid}|{ver}\n"));
    }
    Ok(format!("{:x}", h.finalize()))
}

/// 离上次写快照满 30 分钟了没有（还没写过算满了）
fn due(cfg: &Cfg) -> bool {
    let Some(last) = cfg.last.as_deref().and_then(|t| NaiveDateTime::parse_from_str(t, "%Y-%m-%d %H:%M:%S").ok()) else {
        return true;
    };
    (Local::now().naive_local() - last).num_minutes() >= SNAP_EVERY_MIN
}

/// 选好的备份文件夹：还没有叮咪的数据就建一个（用这个密码锁一把新的主密钥）；
/// 已经有了（比如以前的备份）就用这个密码打开，沿用里面的主密钥
fn open_or_create(dir: &Path, password: &str) -> Result<DataKey, String> {
    match folder::probe(dir)? {
        folder::Probe::Empty => {
            if password.chars().count() < 8 {
                return Err("密码至少 8 位".into());
            }
            let ns = crypto::new_space(password, false)?;
            folder::create(dir, &SpaceFile { v: 1, salt: ns.salt, box1: ns.box1 })?;
            Ok(ns.master)
        }
        folder::Probe::Space { .. } => {
            let s = folder::read_space(dir)?;
            crypto::unlock(password, &s.salt, &s.box1, None).map_err(|e| {
                if e == WRONG_PASSWORD {
                    "这个文件夹里已经有叮咪的数据（比如以前的备份），用这个密码打不开。输当时设的密码，或者换一个文件夹".to_string()
                } else {
                    e
                }
            })
        }
    }
}

/// 写一份快照。IfChanged 时：记录和上次写的一样、文件夹里那份快照也还在，就不写。返回写了没有
fn snapshot(db: &Mutex<Connection>, f: &Folder, name: &str, cfg: &mut Cfg, when: When) -> Result<bool, String> {
    let mark = fingerprint(&db.lock().unwrap())?;
    if when == When::IfChanged && cfg.mark.as_deref() == Some(mark.as_str()) && folder::last_snapshot(f).is_some() {
        return Ok(false);
    }
    // 先算指纹再取记录：中间又改了的话，下一轮指纹对不上会再写一次，不会漏
    folder::backup(db, f)?;
    folder::heartbeat(f, name)?;
    cfg.mark = Some(mark);
    cfg.last = Some(db::now_sec());
    Ok(true)
}

/// 一轮备份：先补写换过密码的 space.json，附件随时写，快照到时候了才写。返回写没写快照
fn round(db: &Mutex<Connection>, data_dir: &Path, f: &Folder, name: &str, cfg: &mut Cfg, when: When) -> Result<bool, String> {
    if !f.dir.join(folder::SPACE_FILE).exists() {
        return Err(MISSING.into());
    }
    if let Some(s) = &cfg.space {
        folder::write_space(f.dir, s)?;
        cfg.space = None;
    }
    folder::backup_files(db, data_dir, f)?;
    match when {
        When::Now => snapshot(db, f, name, cfg, When::Now),
        When::IfChanged if due(cfg) => snapshot(db, f, name, cfg, When::IfChanged),
        When::IfChanged => Ok(false),
    }
}

/// 关掉备份：删掉本机的备份设置和密钥，文件夹里已经写进去的不动
fn turn_off(c: &Connection) {
    let _ = db::kv_del(c, "sync_backup");
    secret::delete(BACKUP_KEY);
}

/// 在后台线程里跑一轮，结果（出的错）记进设置。没开备份时什么也不做
async fn run(app: &AppHandle, when: When) -> Result<bool, String> {
    let (app2, name) = (app.clone(), device_name(app));
    tokio::task::spawn_blocking(move || {
        let _g = busy();
        let st = app2.state::<AppState>();
        let Some(mut cfg) = load(&st.db.lock().unwrap()) else {
            return Ok(false);
        };
        let dir = PathBuf::from(&cfg.dir);
        // 同步方式换成了这个文件夹（主的坏了，拿备份接着用）：它已经是主的了，不用再备份
        if is_sync_dir(&app2, &dir) {
            turn_off(&st.db.lock().unwrap());
            return Ok(false);
        }
        let r = match key() {
            Some(key) => {
                let me = backup_id(&st.device);
                round(&st.db, &st.data_dir, &Folder { dir: &dir, me: &me, key: &key }, &name, &mut cfg, when)
            }
            None => Err("备份的密钥不见了：关掉备份，再重新开一次".into()),
        };
        cfg.error = r.as_ref().err().cloned();
        save(&st.db.lock().unwrap(), &cfg)?;
        r
    })
    .await
    .map_err(err)?
}

/// 开着同步时，备份密码就是同步密码：先核对输的确实是现在的同步密码
async fn check_sync_password(app: &AppHandle, password: &str) -> Result<(), String> {
    let (salt, box1, space_key) = if let Some(fc) = super::load_folder(app) {
        let s = folder::read_space(Path::new(&fc.dir)).map_err(|e| format!("核对不了同步密码：{e}"))?;
        (s.salt, s.box1, None)
    } else if let Some(cfg) = super::load_config(app) {
        let device = secret::get(DEVICE_KEY).ok_or("这台电脑的设备密钥不见了，核对不了同步密码。需要停止同步后重新配对")?;
        let server_public = B64.decode(&cfg.server_public).map_err(err)?;
        let (_, welcome) =
            super::dial(&cfg.host, cfg.port, &cfg.web, &device, &server_public, &cfg.name, None, None, super::SESSION_REJECTED)
                .await
                .map_err(|e| format!("要连上服务器才能核对同步密码：{e}"))?;
        let k = super::space_key().ok_or("这台电脑还没有空间密钥，核对不了同步密码。先在上面解锁")?;
        let salt = welcome.space.salt.ok_or("服务器上还没有设同步密码")?;
        (salt, welcome.space.box1.unwrap_or_default(), Some(k))
    } else {
        return Ok(());
    };
    let pw = password.to_string();
    tokio::task::spawn_blocking(move || crypto::unlock(&pw, &salt, &box1, space_key.as_ref()))
        .await
        .map_err(err)?
        .map(|_| ())
}

/// 换了同步密码：备份的盒子 1 也换成新密码锁（备份密码就是同步密码）。备份文件夹这时不在的话，下一轮再写
pub(super) async fn relock(app: &AppHandle, password: String) {
    let Some(key) = key() else {
        return;
    };
    let Ok(Ok((salt, box1))) = tokio::task::spawn_blocking(move || crypto::lock(&password, &key, None)).await else {
        return;
    };
    let app2 = app.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let _g = busy();
        let st = app2.state::<AppState>();
        let c = st.db.lock().unwrap();
        let Some(mut cfg) = load(&c) else {
            return;
        };
        let space = SpaceFile { v: 1, salt, box1 };
        let dir = PathBuf::from(&cfg.dir);
        if !dir.join(folder::SPACE_FILE).exists() || folder::write_space(&dir, &space).is_err() {
            cfg.space = Some(space);
        }
        let _ = save(&c, &cfg);
    })
    .await;
}

// ---------- 命令 ----------

#[tauri::command]
pub fn backup_status(app: AppHandle) -> BackupStatus {
    let st = app.state::<AppState>();
    let cfg = load(&st.db.lock().unwrap());
    match cfg {
        // 已经拿它当同步文件夹了：下一轮会自己关掉备份，这里先当没开
        Some(c) if !is_sync_dir(&app, Path::new(&c.dir)) => BackupStatus { on: true, dir: Some(c.dir), last: c.last, error: c.error },
        _ => BackupStatus::default(),
    }
}

/// 开备份：选的文件夹里建（或者沿用）一个「叮咪同步」文件夹，马上写第一份。
/// 开着同步时 password 要是现在的同步密码；不同步时是新设的密码（文件夹里已经有数据时是当时设的密码）
#[tauri::command]
pub async fn backup_start(app: AppHandle, path: String, password: String) -> Result<BackupStatus, String> {
    let dir = super::sync_dir(&path);
    if path.trim().is_empty() || !dir.parent().is_some_and(|p| p.is_dir()) {
        return Err("找不到这个文件夹".into());
    }
    if is_sync_dir(&app, &dir) {
        return Err("备份要选另一个文件夹：这个是现在网盘文件夹同步用的".into());
    }
    let syncing = super::load_folder(&app).is_some() || super::load_config(&app).is_some();
    if syncing {
        check_sync_password(&app, &password).await?;
    }
    let d2 = dir.clone();
    let key = tokio::task::spawn_blocking(move || open_or_create(&d2, &password)).await.map_err(err)?.map_err(|e| {
        if syncing && e.starts_with("这个文件夹里已经有叮咪的数据") {
            "这个文件夹里已经有叮咪的数据，用现在的同步密码打不开，请换一个文件夹".to_string()
        } else {
            e
        }
    })?;
    let app2 = app.clone();
    tokio::task::spawn_blocking(move || {
        let _g = busy();
        secret::set(BACKUP_KEY, &key)?;
        let st = app2.state::<AppState>();
        let c = st.db.lock().unwrap();
        save(&c, &Cfg { dir: dir.display().to_string(), ..Default::default() })
    })
    .await
    .map_err(err)??;
    // 第一份马上写；没写上（比如文件夹没有写入权限）也算开好了，错误显示在设置里，下一轮再试
    let _ = run(&app, When::Now).await;
    Ok(backup_status(app))
}

/// 现在备份：不等 30 分钟，马上写一份
#[tauri::command]
pub async fn backup_now(app: AppHandle) -> Result<BackupStatus, String> {
    run(&app, When::Now).await?;
    Ok(backup_status(app))
}

/// 关掉备份：只停止往里写，已经写进去的不删
#[tauri::command]
pub async fn backup_off(app: AppHandle) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let _g = busy();
        turn_off(&app.state::<AppState>().db.lock().unwrap());
    })
    .await
    .map_err(err)
}

/// 后台任务：每 2 分钟看一眼
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(CHECK_EVERY).await;
            let _ = run(&app, When::IfChanged).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Note, Reminder};

    struct Pc {
        db: Mutex<Connection>,
        data: PathBuf,
        id: String,
    }

    fn pc(base: &Path, n: u8) -> Pc {
        let data = base.join(format!("pc{n}"));
        Pc { db: Mutex::new(db::open(&data).unwrap()), data, id: format!("device-{n}-0000-aaaa") }
    }

    fn base(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("dingmi-backup-{name}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn add_reminder(p: &Pc, title: &str) {
        let c = p.db.lock().unwrap();
        let r = Reminder { title: title.into(), due_at: "2026-11-06 09:00".into(), ..Default::default() };
        db::upsert_reminder(&c, &r).unwrap();
    }

    fn titles(p: &Pc) -> Vec<String> {
        let mut t: Vec<String> = db::list_reminders(&p.db.lock().unwrap()).unwrap().into_iter().map(|r| r.title).collect();
        t.sort();
        t
    }

    /// 带一张图片的笔记，回图片的文件名
    fn add_image_note(p: &Pc) -> String {
        let c = p.db.lock().unwrap();
        let nid = db::upsert_note(&c, &Note { title: "装修".into(), ..Default::default() }).unwrap();
        let mut n = db::get_note(&c, nid).unwrap().unwrap();
        let name = db::save_file(&c, &p.data, "image/png", "", b"\x89PNG fake", Some(&n.uid), None).unwrap();
        n.body = format!("![](att:{name})");
        db::upsert_note(&c, &n).unwrap();
        name
    }

    fn snaps(dir: &Path, p: &Pc) -> usize {
        std::fs::read_dir(dir.join("devices").join(backup_id(&p.id)).join("snap")).map_or(0, |rd| rd.count())
    }

    #[test]
    fn backup_opens_as_a_sync_folder_elsewhere() {
        let b = base("restore");
        let dir = b.join("NAS").join(folder::DIR_NAME);
        std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
        let a = pc(&b, 1);
        add_reminder(&a, "周四评审");
        let image = add_image_note(&a);
        // 密码太短不行；空文件夹：建一个，锁一把新的主密钥
        assert!(open_or_create(&dir, "short").is_err());
        let key = open_or_create(&dir, "password1").unwrap();
        let bid = backup_id(&a.id);
        let f = Folder { dir: &dir, me: &bid, key: &key };
        let mut cfg = Cfg { dir: dir.display().to_string(), ..Default::default() };

        // 第一轮：还没写过，马上写；附件写进 files/
        assert!(round(&a.db, &a.data, &f, "公司电脑", &mut cfg, When::IfChanged).unwrap());
        assert!(cfg.last.is_some() && cfg.mark.is_some());
        assert_eq!(std::fs::read_dir(dir.join("files")).unwrap().count(), 1);
        let first = folder::last_snapshot(&f).unwrap();

        // 没变化：到时间了也不重写
        assert!(!snapshot(&a.db, &f, "公司电脑", &mut cfg, When::IfChanged).unwrap());
        assert_eq!(folder::last_snapshot(&f), Some(first));

        // 有变化：没满 30 分钟不写；满了写一份新的，旧快照删掉
        add_reminder(&a, "交周报");
        assert!(!round(&a.db, &a.data, &f, "公司电脑", &mut cfg, When::IfChanged).unwrap());
        assert!(snapshot(&a.db, &f, "公司电脑", &mut cfg, When::IfChanged).unwrap());
        assert_eq!(folder::last_snapshot(&f), Some(first + 1));
        assert_eq!(snaps(&dir, &a), 1);
        // 备份不碰同步在用的「没传」标记
        assert!(records::has_dirty(&a.db.lock().unwrap()));

        // 另一台电脑：网盘文件夹的加入流程，选这个文件夹、输密码，全部数据都在
        assert_eq!(folder::probe(&dir).unwrap(), folder::Probe::Space { devices: 1 });
        let s = folder::read_space(&dir).unwrap();
        let k2 = crypto::unlock("password1", &s.salt, &s.box1, None).unwrap();
        let z = pc(&b, 2);
        assert!(folder::pull(&z.db, &z.data, &Folder { dir: &dir, me: &z.id, key: &k2 }).unwrap());
        assert_eq!(titles(&z), ["交周报", "周四评审"]);
        assert_eq!(db::list_notes(&z.db.lock().unwrap()).unwrap()[0].title, "装修");
        assert_eq!(std::fs::read(db::attachment_dir(&z.data).join(&image)).unwrap(), b"\x89PNG fake");
        let seen = folder::devices(&Folder { dir: &dir, me: &z.id, key: &k2 });
        assert!(seen.iter().any(|d| d.name == "公司电脑" && d.last_seen.is_some()));

        // 同一台电脑（设备编号没变）清空以后拿自己的备份当同步：也能读回来
        db::wipe_synced(&a.db.lock().unwrap()).unwrap();
        db::kv_del(&a.db.lock().unwrap(), "folder_cursor").unwrap();
        assert!(folder::pull(&a.db, &a.data, &Folder { dir: &dir, me: &a.id, key: &k2 }).unwrap());
        assert_eq!(titles(&a), ["交周报", "周四评审"]);

        // 已经有数据的文件夹：密码不对打不开；对的沿用里面的主密钥
        assert!(open_or_create(&dir, "password2").unwrap_err().contains("打不开"));
        assert_eq!(open_or_create(&dir, "password1").unwrap(), key);
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn missing_folder_and_purged_files() {
        let b = base("files");
        let dir = b.join(folder::DIR_NAME);
        let a = pc(&b, 1);
        let image = add_image_note(&a);
        let key = open_or_create(&dir, "password1").unwrap();
        let bid = backup_id(&a.id);
        let f = Folder { dir: &dir, me: &bid, key: &key };
        let mut cfg = Cfg::default();
        round(&a.db, &a.data, &f, "家里电脑", &mut cfg, When::Now).unwrap();
        assert_eq!(std::fs::read_dir(dir.join("files")).unwrap().count(), 1);
        // 回收站里彻底删除的图片：备份里的文件也删掉
        a.db.lock().unwrap().execute("UPDATE attachments SET purged=1 WHERE file=?1", [&image]).unwrap();
        round(&a.db, &a.data, &f, "家里电脑", &mut cfg, When::IfChanged).unwrap();
        assert_eq!(std::fs::read_dir(dir.join("files")).unwrap().count(), 0);
        // 换了密码的 space.json 等文件夹回来再写；文件夹不在时报错、不会自己建一个
        let (salt, box1) = crypto::lock("password2", &key, None).unwrap();
        cfg.space = Some(SpaceFile { v: 1, salt, box1 });
        let moved = b.join("挪走了");
        std::fs::rename(&dir, &moved).unwrap();
        assert_eq!(round(&a.db, &a.data, &f, "家里电脑", &mut cfg, When::Now).unwrap_err(), MISSING);
        assert!(!dir.exists() && cfg.space.is_some());
        std::fs::rename(&moved, &dir).unwrap();
        round(&a.db, &a.data, &f, "家里电脑", &mut cfg, When::Now).unwrap();
        assert!(cfg.space.is_none());
        assert_eq!(open_or_create(&dir, "password2").unwrap(), key);
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn same_folder_check() {
        let b = base("same");
        let dir = b.join(folder::DIR_NAME);
        std::fs::create_dir_all(&dir).unwrap();
        let upper = PathBuf::from(dir.display().to_string().to_uppercase() + "\\");
        assert!(same_dir(&dir, &upper));
        assert!(!same_dir(&dir, &b.join("别的").join(folder::DIR_NAME)));
        let _ = std::fs::remove_dir_all(&b);
    }
}
