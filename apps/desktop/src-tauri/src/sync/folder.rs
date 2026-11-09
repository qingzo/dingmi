//! 网盘文件夹同步：叮咪只读写一个本地文件夹（「叮咪同步」），网盘客户端负责在电脑之间传。
//! 设计见 docs/方案/同步方案.md 第 15 节。只依赖数据库连接和文件，不依赖 Tauri，可以用临时文件夹测试。
//!
//! 文件夹里：
//! - `space.json`：salt 和盒子 1（用同步密码锁着的主密钥），新电脑输同步密码打开它；
//! - `请勿改动.txt`；
//! - `devices/<设备编号>/`：每台电脑只写自己的这一个文件夹，从不改别人的。
//!   `log/<序号>.dlog` 是一段段改动日志；攒多了合成一份 `snap/<序号>.dsnap` 快照（这台电脑当时的全部数据），
//!   之前的日志删掉；`info.dinfo` 记着设备名和最后出现的时间；
//! - `files/<uid>.dfile`：附件，所有电脑共用，写一次不再改。
//!
//! 全部用主密钥加密，附加数据里带着设备编号、种类和序号，文件被挪到别处就解不开。
//! 写的时候先写临时文件再改名；名字不认识的（网盘的冲突副本、临时文件）一律不读也不删。
//! OneDrive 还没下载的占位文件先不读，把它和整个文件夹设成「始终保留在此设备上」，OneDrive 下载好了下一轮再读。
//! 读别的电脑的日志时按序号一段一段往下读，缺了中间的就停下，等网盘传完下次再读，不会跳过。

use super::crypto::{self, DataKey};
use super::records::{self, Payload};
use crate::{clock, db};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 在用户选的网盘文件夹里建的子文件夹，叮咪只用这一个。
/// 2026-11-07 从「叮咪同步」改名（和收费的叮咪同步撞名了），已经有旧名字的文件夹照旧用，见 sync::sync_dir
pub const DIR_NAME: &str = "叮咪数据";
pub const OLD_DIR_NAME: &str = "叮咪同步";
pub(super) const SPACE_FILE: &str = "space.json";
const README: &str = "请勿改动.txt";
const README_TEXT: &str = "这个文件夹是叮咪的同步文件，里面的内容都是加密的。\r\n\
    请不要手动改动、移动或删除里面的文件，网盘客户端会负责在几台电脑之间传。\r\n\
    想停止同步，在叮咪的「设置 → 同步」里点「停止同步」。\r\n";
/// 自己的日志攒到这么多段，就合成一份快照
const COMPACT_AFTER: usize = 50;
/// 一段日志最多放这么多条记录
const BATCH: usize = 500;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// `space.json`：没有秘密，只有 salt 和锁着主密钥的盒子 1
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SpaceFile {
    pub v: u32,
    pub salt: String,
    pub box1: String,
}

/// 选中的文件夹里有什么
#[derive(Debug, PartialEq)]
pub enum Probe {
    /// 没有叮咪的数据：这台是第一台电脑
    Empty,
    /// 已经有人在用：有几台电脑的数据
    Space { devices: usize },
}

/// 一台电脑的身份：文件夹位置、本机设备编号、主密钥
pub struct Folder<'a> {
    pub dir: &'a Path,
    pub me: &'a str,
    pub key: &'a DataKey,
}

/// 另一台（或这台）电脑最后出现的时间
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Seen {
    pub id: String,
    pub name: String,
    pub last_seen: Option<String>,
    pub me: bool,
}

#[derive(Serialize, Deserialize)]
struct Item {
    ver: String,
    p: Payload,
}

/// 一段日志或一份快照里的内容
#[derive(Serialize, Deserialize)]
struct Bundle {
    v: u32,
    items: Vec<Item>,
}

#[derive(Serialize, Deserialize)]
struct Info {
    name: String,
    last_seen: String,
}

fn aad(device: &str, kind: &str, seq: u64) -> Vec<u8> {
    format!("dingmi-folder|{device}|{kind}|{seq}").into_bytes()
}

fn file_aad(uid: &str) -> Vec<u8> {
    format!("file|{uid}").into_bytes()
}

/// 设备编号会拼进路径，只认 字母数字 -（网盘的冲突副本「xxx (1)」之类自然就不认）
fn valid_id(s: &str) -> bool {
    (8..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// OneDrive「按需下载」还没下到本机的占位文件：读它会卡住等下载，先跳过
#[cfg(windows)]
fn placeholder(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const OFFLINE: u32 = 0x1000;
    const RECALL_ON_OPEN: u32 = 0x4_0000;
    const RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;
    meta.file_attributes() & (OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS) != 0
}

#[cfg(not(windows))]
fn placeholder(_: &std::fs::Metadata) -> bool {
    false
}

/// 叫 OneDrive 把它一直留在本机（等于右键「始终保留在此设备上」、attrib +P）。
/// 别的电脑新写的文件在这台电脑上默认只是占位，不去要就一直不下载；设了以后 OneDrive 在后台下载，下一轮就能读。
/// 文件夹设了，以后新来的文件也跟着留在本机。不是 OneDrive 的文件夹设不上也没关系
#[cfg(windows)]
fn keep_local(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{GetFileAttributesW, SetFileAttributesW, INVALID_FILE_ATTRIBUTES};
    const PINNED: u32 = 0x8_0000;
    const UNPINNED: u32 = 0x10_0000;
    // SetFileAttributesW 能设的只有这几位（只读、隐藏、系统、存档、临时、脱机、不索引、两个保留标记）
    const SETTABLE: u32 = 0x1 | 0x2 | 0x4 | 0x20 | 0x100 | 0x1000 | 0x2000 | PINNED | UNPINNED;
    let w: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    unsafe {
        let a = GetFileAttributesW(w.as_ptr());
        if a == INVALID_FILE_ATTRIBUTES || (a & PINNED != 0 && a & UNPINNED == 0) {
            return;
        }
        SetFileAttributesW(w.as_ptr(), (a & SETTABLE | PINNED) & !UNPINNED);
    }
}

#[cfg(not(windows))]
fn keep_local(_: &Path) {}

/// 文件夹里名字是 `<10 位序号>.<ext>` 的文件的序号，从小到大。
/// local_only：只要已经下载到本机的（读别的电脑的日志时）；还是占位的叫 OneDrive 去下载，这次先不算。
/// 自己的文件夹算序号、删旧文件时不管下没下载都算，免得被网盘腾空间收回去的文件被当成没有、序号重复
fn seqs(dir: &Path, ext: &str, local_only: bool) -> Vec<u64> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<u64> = rd
        .flatten()
        .filter(|e| {
            e.metadata().is_ok_and(|m| {
                if !m.is_file() {
                    return false;
                }
                if local_only && placeholder(&m) {
                    keep_local(&e.path());
                    return false;
                }
                true
            })
        })
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let num = name.strip_suffix(ext)?.strip_suffix('.')?;
            (num.len() == 10 && num.bytes().all(|b| b.is_ascii_digit())).then(|| num.parse().ok())?
        })
        .collect();
    out.sort_unstable();
    out
}

fn seq_name(seq: u64, ext: &str) -> String {
    format!("{seq:010}.{ext}")
}

/// 先写到同一个文件夹里的临时文件，写完再改名：别的电脑和网盘永远看不到写了一半的文件
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().ok_or("路径不对")?;
    std::fs::create_dir_all(dir).map_err(err)?;
    let name = path.file_name().and_then(|n| n.to_str()).ok_or("路径不对")?;
    let tmp = dir.join(format!(".{name}.tmp"));
    std::fs::write(&tmp, bytes).map_err(err)?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        err(e)
    })
}

fn devices_dir(dir: &Path) -> PathBuf {
    dir.join("devices")
}

fn files_dir(dir: &Path) -> PathBuf {
    dir.join("files")
}

/// 文件夹里各台电脑的编号（名字不认识的不算）
fn device_ids(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(devices_dir(dir)) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| valid_id(n))
        .collect();
    ids.sort();
    ids
}

// ---------- 开启 ----------

/// 看看选中的文件夹（`…\叮咪同步`）里有没有叮咪的数据
pub fn probe(dir: &Path) -> Result<Probe, String> {
    let space = dir.join(SPACE_FILE);
    let devices = device_ids(dir).len();
    // 选了 OneDrive 里的文件夹：先叫它把叮咪的同步文件都下载到这台电脑
    keep_local(dir);
    match std::fs::metadata(&space) {
        Ok(m) if placeholder(&m) => {
            keep_local(&space);
            Err("网盘还没把叮咪的同步文件下载到这台电脑，等它下载完再试".into())
        }
        Ok(_) => Ok(Probe::Space { devices }),
        Err(_) if devices > 0 => Err("文件夹里的叮咪数据还没传完整，等网盘传完再试".into()),
        Err(_) => Ok(Probe::Empty),
    }
}

pub fn read_space(dir: &Path) -> Result<SpaceFile, String> {
    let text = std::fs::read_to_string(dir.join(SPACE_FILE)).map_err(|e| format!("读不了同步文件夹里的 {SPACE_FILE}：{e}"))?;
    let s: SpaceFile = serde_json::from_str(&text).map_err(|_| format!("同步文件夹里的 {SPACE_FILE} 坏了"))?;
    if s.v != 1 {
        return Err("这个同步文件夹是更新版本的叮咪建的，请先更新叮咪".into());
    }
    Ok(s)
}

/// 第一台电脑：建好文件夹、写 space.json 和说明
pub fn create(dir: &Path, space: &SpaceFile) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("建不了同步文件夹：{e}"))?;
    if dir.join(SPACE_FILE).exists() {
        return Err("这个文件夹里已经有叮咪的数据了，请选「加入」".into());
    }
    write_space(dir, space)?;
    let _ = std::fs::write(dir.join(README), README_TEXT);
    Ok(())
}

/// 写 space.json（建空间、换同步密码时）
pub fn write_space(dir: &Path, space: &SpaceFile) -> Result<(), String> {
    write_atomic(&dir.join(SPACE_FILE), &serde_json::to_vec_pretty(space).map_err(err)?)
}

// ---------- 写 ----------

fn own_dir(f: &Folder) -> PathBuf {
    devices_dir(f.dir).join(f.me)
}

/// 下一段日志的序号：看文件夹里自己已经写到哪了（换过电脑数据、重新加入同一个文件夹也不会撞号）
fn next_seq(f: &Folder) -> u64 {
    let own = own_dir(f);
    let last = seqs(&own.join("log"), "dlog", false).last().copied().unwrap_or(0);
    let snap = seqs(&own.join("snap"), "dsnap", false).last().copied().unwrap_or(0);
    last.max(snap) + 1
}

fn seal_bundle(f: &Folder, kind: &str, seq: u64, items: Vec<Item>) -> Result<Vec<u8>, String> {
    let json = serde_json::to_vec(&Bundle { v: 1, items }).map_err(err)?;
    Ok(crypto::seal_with(f.key, &aad(f.me, kind, seq), &json))
}

/// 把本机改过的写成新的日志段（附件先写进 files/）。返回写了几条
pub fn push(db: &Mutex<Connection>, data_dir: &Path, f: &Folder) -> Result<usize, String> {
    if !f.dir.join(SPACE_FILE).exists() {
        return Err("找不到同步文件夹".into());
    }
    let mut total = 0;
    loop {
        let batch = records::dirty(&db.lock().unwrap(), BATCH).map_err(err)?;
        if batch.is_empty() {
            break;
        }
        for d in &batch {
            if let Payload::Attachment(a) = &d.payload {
                if !db::valid_att_uid(&a.uid) {
                    continue;
                }
                let path = files_dir(f.dir).join(format!("{}.dfile", a.uid));
                if a.purged {
                    let _ = std::fs::remove_file(&path);
                } else if !path.exists() {
                    if let Ok(bytes) = std::fs::read(db::attachment_dir(data_dir).join(&a.file)) {
                        write_atomic(&path, &crypto::seal_with(f.key, &file_aad(&a.uid), &bytes))?;
                    }
                }
            }
        }
        let seq = next_seq(f);
        let items = batch.iter().map(|d| Item { ver: d.ver.clone(), p: d.payload.clone() }).collect();
        let bytes = seal_bundle(f, "log", seq, items)?;
        write_atomic(&own_dir(f).join("log").join(seq_name(seq, "dlog")), &bytes)?;
        let c = db.lock().unwrap();
        for d in &batch {
            records::clear_dirty(&c, d.table, &d.uid, &d.ver).map_err(err)?;
        }
        total += batch.len();
    }
    if total > 0 {
        compact(db, f)?;
    }
    Ok(total)
}

/// 自己的日志攒多了：写一份快照（本机现在的全部数据），再删掉快照之前的日志和旧快照
fn compact(db: &Mutex<Connection>, f: &Folder) -> Result<(), String> {
    let own = own_dir(f);
    let logs = seqs(&own.join("log"), "dlog", false);
    if logs.len() < COMPACT_AFTER {
        return Ok(());
    }
    let seq = *logs.last().unwrap();
    let items = records::all(&db.lock().unwrap())
        .map_err(err)?
        .into_iter()
        .map(|d| Item { ver: d.ver, p: d.payload })
        .collect();
    let bytes = seal_bundle(f, "snap", seq, items)?;
    write_atomic(&own.join("snap").join(seq_name(seq, "dsnap")), &bytes)?;
    for s in logs.into_iter().filter(|s| *s <= seq) {
        let _ = std::fs::remove_file(own.join("log").join(seq_name(s, "dlog")));
    }
    for s in seqs(&own.join("snap"), "dsnap", false).into_iter().filter(|s| *s < seq) {
        let _ = std::fs::remove_file(own.join("snap").join(seq_name(s, "dsnap")));
    }
    Ok(())
}

/// 记一下这台电脑的名字和最后出现的时间（别的电脑上显示「公司电脑 · 3 分钟前」）
pub fn heartbeat(f: &Folder, name: &str) -> Result<(), String> {
    let info = Info { name: name.into(), last_seen: db::now_sec() };
    let bytes = crypto::seal_with(f.key, &aad(f.me, "info", 0), &serde_json::to_vec(&info).map_err(err)?);
    write_atomic(&own_dir(f).join("info.dinfo"), &bytes)
}

// ---------- 备份 ----------

/// 另存一份备份：把本机的全部记录写成一份新快照（序号接着往下编），再删掉旧快照和之前的日志。
/// 备份文件夹只写不读，也不能动同步在用的「没传」标记，所以每次都写全部。返回写了几条
pub fn backup(db: &Mutex<Connection>, f: &Folder) -> Result<usize, String> {
    if !f.dir.join(SPACE_FILE).exists() {
        return Err("找不到备份文件夹".into());
    }
    let items: Vec<Item> = records::all(&db.lock().unwrap())
        .map_err(err)?
        .into_iter()
        .map(|d| Item { ver: d.ver, p: d.payload })
        .collect();
    let n = items.len();
    let own = own_dir(f);
    let seq = next_seq(f);
    let bytes = seal_bundle(f, "snap", seq, items)?;
    write_atomic(&own.join("snap").join(seq_name(seq, "dsnap")), &bytes)?;
    for s in seqs(&own.join("snap"), "dsnap", false).into_iter().filter(|s| *s < seq) {
        let _ = std::fs::remove_file(own.join("snap").join(seq_name(s, "dsnap")));
    }
    // 这个文件夹以前当过同步文件夹的话，自己还有日志：都已经在新快照里了
    for s in seqs(&own.join("log"), "dlog", false).into_iter().filter(|s| *s < seq) {
        let _ = std::fs::remove_file(own.join("log").join(seq_name(s, "dlog")));
    }
    Ok(n)
}

/// 自己最新的一份快照的序号；还没写过是 None
pub fn last_snapshot(f: &Folder) -> Option<u64> {
    seqs(&own_dir(f).join("snap"), "dsnap", false).last().copied()
}

/// 备份的附件：本机有、files/ 里还没有的写进去，彻底删除的删掉。返回写了几个
pub fn backup_files(db: &Mutex<Connection>, data_dir: &Path, f: &Folder) -> Result<usize, String> {
    if !f.dir.join(SPACE_FILE).exists() {
        return Err("找不到备份文件夹".into());
    }
    let list = records::attachment_files(&db.lock().unwrap()).map_err(err)?;
    let mut n = 0;
    for (uid, file, purged) in list {
        if !db::valid_att_uid(&uid) {
            continue;
        }
        let path = files_dir(f.dir).join(format!("{uid}.dfile"));
        if purged {
            let _ = std::fs::remove_file(&path);
        } else if !path.exists() {
            // 原图还没下载到这台电脑的（别的电脑传的文件附件）先跳过，下载了下一轮再写
            if let Ok(bytes) = std::fs::read(db::attachment_dir(data_dir).join(&file)) {
                write_atomic(&path, &crypto::seal_with(f.key, &file_aad(&uid), &bytes))?;
                n += 1;
            }
        }
    }
    Ok(n)
}

// ---------- 读 ----------

/// 网盘文件夹里一段日志、一份快照最大多大：读之前先看大小，别人往文件夹里放个几十 GB 的文件也不会把内存撑爆
pub(super) const MAX_BUNDLE: u64 = 512 * 1024 * 1024;

/// 读网盘文件夹里的一个文件：比 `max` 大的不读
fn read_capped(path: &Path, max: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    if f.metadata().ok()?.len() > max {
        return None;
    }
    let mut out = Vec::new();
    // 读的时候文件又变大了也只读到上限多一个字节，多了就不要
    f.take(max + 1).read_to_end(&mut out).ok()?;
    (out.len() as u64 <= max).then_some(out)
}

fn open_bundle(f: &Folder, device: &str, kind: &str, seq: u64, path: &Path) -> Option<Bundle> {
    let bytes = read_capped(path, MAX_BUNDLE)?;
    let json = crypto::open_with(f.key, &aad(device, kind, seq), &bytes)?;
    serde_json::from_slice::<Bundle>(&json).ok().filter(|b| b.v == 1)
}

fn apply_bundle(c: &Connection, data_dir: &Path, b: &Bundle) -> rusqlite::Result<bool> {
    let mut changed = false;
    for it in &b.items {
        clock::observe(&it.ver);
        changed |= records::apply(c, data_dir, &it.ver, &it.p)?;
    }
    Ok(changed)
}

/// 每台电脑读到了第几段，记在 kv 的 folder_cursor
fn load_cursor(c: &Connection) -> BTreeMap<String, u64> {
    db::kv_get(c, "folder_cursor").and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// 读别的电脑新写的日志（或者快照），合并进本机，再把缺的原图从 files/ 取过来。返回本机有没有变化
pub fn pull(db: &Mutex<Connection>, data_dir: &Path, f: &Folder) -> Result<bool, String> {
    if !f.dir.join(SPACE_FILE).exists() {
        return Err("找不到同步文件夹".into());
    }
    keep_local(f.dir);
    let mut cursor = load_cursor(&db.lock().unwrap());
    let mut changed = false;
    for id in device_ids(f.dir).into_iter().filter(|id| id != f.me) {
        let dev = devices_dir(f.dir).join(&id);
        let logs = seqs(&dev.join("log"), "dlog", true);
        let snaps = seqs(&dev.join("snap"), "dsnap", true);
        let mut cur = cursor.get(&id).copied().unwrap_or(0);
        loop {
            // 接着读下一段；下一段已经被合进快照（日志删了）就读快照
            let (kind, seq, path) = if logs.binary_search(&(cur + 1)).is_ok() {
                ("log", cur + 1, dev.join("log").join(seq_name(cur + 1, "dlog")))
            } else if let Some(&s) = snaps.iter().rev().find(|s| **s > cur) {
                ("snap", s, dev.join("snap").join(seq_name(s, "dsnap")))
            } else {
                break;
            };
            // 解不开、读不了（还没传完、密钥不对）：停在这里，下次再读
            let Some(b) = open_bundle(f, &id, kind, seq, &path) else {
                break;
            };
            let c = db.lock().unwrap();
            let tx = c.unchecked_transaction().map_err(err)?;
            changed |= apply_bundle(&tx, data_dir, &b).map_err(err)?;
            records::fix_links(&tx).map_err(err)?;
            cur = seq;
            cursor.insert(id.clone(), cur);
            db::kv_set(&tx, "folder_cursor", &serde_json::to_string(&cursor).map_err(err)?).map_err(err)?;
            tx.commit().map_err(err)?;
        }
    }
    let missing = records::missing_files(&db.lock().unwrap()).map_err(err)?;
    for (uid, file) in missing {
        // 别的电脑的网盘可能还没把附件传过来，下次再试
        if fetch_file(db, data_dir, f, &uid, &file).is_ok() {
            changed = true;
        }
    }
    Ok(changed)
}

/// 从 files/ 取一个附件，解密后放进本机的 attachments/
pub fn fetch_file(db: &Mutex<Connection>, data_dir: &Path, f: &Folder, uid: &str, file: &str) -> Result<(), String> {
    if !db::valid_att_uid(uid) {
        return Err("附件的编号不对".into());
    }
    let path = files_dir(f.dir).join(format!("{uid}.dfile"));
    let meta = std::fs::metadata(&path).map_err(|_| "这个文件还没传到这台电脑的网盘文件夹里，等网盘传完再试".to_string())?;
    if placeholder(&meta) {
        keep_local(&path);
        return Err("网盘还没把这个文件下载到这台电脑，等它下载完再试".into());
    }
    let blob = read_capped(&path, super::engine::MAX_SEALED_FILE as u64).ok_or("这个文件太大了，或者读不了")?;
    let bytes = crypto::open_with(f.key, &file_aad(uid), &blob).ok_or("这个文件解不开")?;
    super::engine::write_attachment(data_dir, file, &bytes)?;
    records::file_ok(&db.lock().unwrap(), uid).map_err(err)
}

/// 文件夹里有哪几台电脑、各自最后出现的时间，最近的在前
pub fn devices(f: &Folder) -> Vec<Seen> {
    let mut out: Vec<Seen> = device_ids(f.dir)
        .into_iter()
        .map(|id| {
            let info = read_capped(&devices_dir(f.dir).join(&id).join("info.dinfo"), 64 * 1024)
                .and_then(|b| crypto::open_with(f.key, &aad(&id, "info", 0), &b))
                .and_then(|j| serde_json::from_slice::<Info>(&j).ok());
            let me = id == f.me;
            match info {
                Some(i) => Seen { id, name: i.name, last_seen: Some(i.last_seen), me },
                None => Seen { id, name: "另一台电脑".into(), last_seen: None, me },
            }
        })
        .collect();
    out.sort_by(|a, b| b.me.cmp(&a.me).then(b.last_seen.cmp(&a.last_seen)));
    out
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
        let d = std::env::temp_dir().join(format!("dingmi-folder-{name}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn folder<'a>(dir: &'a Path, p: &'a Pc, key: &'a DataKey) -> Folder<'a> {
        Folder { dir, me: &p.id, key }
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

    fn setup(name: &str) -> (PathBuf, PathBuf, DataKey) {
        let b = base(name);
        let dir = b.join(DIR_NAME);
        assert_eq!(probe(&dir).unwrap(), Probe::Empty);
        let ns = crypto::new_space("password1", false).unwrap();
        create(&dir, &SpaceFile { v: 1, salt: ns.salt, box1: ns.box1 }).unwrap();
        (b, dir, ns.master)
    }

    /// 和真人配合测试：冒充「另一台电脑」读写同一个网盘文件夹（数据放在临时目录的 dingmi-peer，下次接着用）。平时跳过：
    /// DINGMI_TEST_FOLDER=<叮咪同步 文件夹> DINGMI_TEST_PASSWORD=<同步密码> [DINGMI_TEST_ADD=<要加的提醒>]
    /// cargo test --lib -- --ignored folder::tests::peer --nocapture
    #[test]
    #[ignore]
    fn peer() {
        let dir = PathBuf::from(std::env::var("DINGMI_TEST_FOLDER").expect("要设 DINGMI_TEST_FOLDER"));
        let s = read_space(&dir).unwrap();
        let key = crypto::unlock(&std::env::var("DINGMI_TEST_PASSWORD").unwrap(), &s.salt, &s.box1, None).unwrap();
        // DINGMI_TEST_PEER 换一个名字就是又一台新电脑
        let name = std::env::var("DINGMI_TEST_PEER").unwrap_or_else(|_| "dingmi-peer".into());
        let data = std::env::temp_dir().join(&name);
        let p = Pc { db: Mutex::new(db::open(&data).unwrap()), data, id: format!("{name}-0000-test-pc") };
        let f = folder(&dir, &p, &key);
        println!("PULLED={}", pull(&p.db, &p.data, &f).unwrap());
        if let Ok(t) = std::env::var("DINGMI_TEST_ADD") {
            add_reminder(&p, &t);
        }
        println!("PUSHED={}", push(&p.db, &p.data, &f).unwrap());
        heartbeat(&f, "测试用的另一台电脑").unwrap();
        for r in db::list_reminders(&p.db.lock().unwrap()).unwrap() {
            println!("R|{}|{}|done={}|deleted={:?}", r.title, r.due_at, r.done, r.deleted_at);
        }
        for n in db::list_notes(&p.db.lock().unwrap()).unwrap() {
            println!("N|{}|{}", n.title, n.body.replace('\n', "⏎"));
        }
    }

    /// 攻击：有人往网盘文件夹里放一个巨大的日志段、附件（修之前直接 fs::read 整个读进内存）。
    /// 现在先看大小，超了不读；附件编号带 ..\ 的不去拼路径
    #[test]
    fn huge_files_in_folder_are_not_read() {
        let (b, dir, key) = setup("huge");
        let a = pc(&b, 1);
        let f = folder(&dir, &a, &key);
        let small = b.join("small.bin");
        std::fs::write(&small, [1u8; 11]).unwrap();
        assert_eq!(read_capped(&small, 11).as_deref(), Some(&[1u8; 11][..]));
        assert!(read_capped(&small, 10).is_none());
        // 别的电脑的日志段说自己有 600 MB（只设长度，不真的写）：不读、不合并，下次再看
        let other = devices_dir(&dir).join("device-9-0000-aaaa").join("log");
        std::fs::create_dir_all(&other).unwrap();
        let big = std::fs::File::create(other.join(seq_name(1, "dlog"))).unwrap();
        big.set_len(MAX_BUNDLE + 1).unwrap();
        drop(big);
        assert!(open_bundle(&f, "device-9-0000-aaaa", "log", 1, &other.join(seq_name(1, "dlog"))).is_none());
        assert!(!pull(&a.db, &a.data, &f).unwrap());
        // files/ 里的附件比附件上限还大：不读
        let blob = files_dir(&dir).join("u1.dfile");
        std::fs::create_dir_all(files_dir(&dir)).unwrap();
        std::fs::File::create(&blob).unwrap().set_len(super::super::engine::MAX_SEALED_FILE as u64 + 1).unwrap();
        assert!(fetch_file(&a.db, &a.data, &f, "u1", "u1.png").is_err());
        assert!(fetch_file(&a.db, &a.data, &f, r"..\..\x", "x.png").is_err());
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn two_computers_through_a_folder() {
        let (b, dir, key) = setup("two");
        let (a, z) = (pc(&b, 1), pc(&b, 2));
        add_reminder(&a, "周四评审");
        assert_eq!(push(&a.db, &a.data, &folder(&dir, &a, &key)).unwrap(), 1);
        heartbeat(&folder(&dir, &a, &key), "公司电脑").unwrap();

        // 第二台：选同一个文件夹、输同一个密码就能拿到主密钥
        assert_eq!(probe(&dir).unwrap(), Probe::Space { devices: 1 });
        let s = read_space(&dir).unwrap();
        assert_eq!(crypto::unlock("password1", &s.salt, &s.box1, None).unwrap(), key);
        assert!(pull(&z.db, &z.data, &folder(&dir, &z, &key)).unwrap());
        assert_eq!(titles(&z), ["周四评审"]);
        // 再读一遍没有新的
        assert!(!pull(&z.db, &z.data, &folder(&dir, &z, &key)).unwrap());

        // 反过来也行；自己写的不会读回来
        add_reminder(&z, "交周报");
        push(&z.db, &z.data, &folder(&dir, &z, &key)).unwrap();
        assert!(pull(&a.db, &a.data, &folder(&dir, &a, &key)).unwrap());
        assert_eq!(titles(&a), ["交周报", "周四评审"]);
        assert!(!pull(&z.db, &z.data, &folder(&dir, &z, &key)).unwrap());

        let seen = devices(&folder(&dir, &z, &key));
        assert_eq!(seen.len(), 2);
        assert!(seen[0].me && seen[0].name == "另一台电脑");
        assert_eq!((seen[1].name.as_str(), seen[1].last_seen.is_some()), ("公司电脑", true));
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn skips_odd_files_and_waits_for_gaps() {
        let (b, dir, key) = setup("odd");
        let (a, z) = (pc(&b, 1), pc(&b, 2));
        let fa = folder(&dir, &a, &key);
        for t in ["一", "二", "三"] {
            add_reminder(&a, t);
            push(&a.db, &a.data, &fa).unwrap();
        }
        let log = devices_dir(&dir).join(&a.id).join("log");
        // 网盘还没传来第 2 段：只读到第 1 段，不会跳过去读第 3 段
        let second = log.join(seq_name(2, "dlog"));
        let held = std::fs::read(&second).unwrap();
        std::fs::remove_file(&second).unwrap();
        pull(&z.db, &z.data, &folder(&dir, &z, &key)).unwrap();
        assert_eq!(titles(&z), ["一"]);
        // 写了一半的（解不开）也停在那里
        std::fs::write(&second, &held[..held.len() / 2]).unwrap();
        pull(&z.db, &z.data, &folder(&dir, &z, &key)).unwrap();
        assert_eq!(titles(&z), ["一"]);
        // 网盘的冲突副本、临时文件、不认识的文件夹：不读也不删
        std::fs::write(log.join("0000000002 (1).dlog"), &held).unwrap();
        std::fs::write(log.join(".0000000002.dlog.tmp"), &held).unwrap();
        std::fs::create_dir_all(devices_dir(&dir).join(format!("{} (冲突)", a.id))).unwrap();
        pull(&z.db, &z.data, &folder(&dir, &z, &key)).unwrap();
        assert_eq!(titles(&z), ["一"]);
        assert!(log.join("0000000002 (1).dlog").exists());
        // 传完了：接着读完
        std::fs::write(&second, &held).unwrap();
        pull(&z.db, &z.data, &folder(&dir, &z, &key)).unwrap();
        assert_eq!(titles(&z), ["一", "三", "二"]);
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn files_moved_between_devices_do_not_open() {
        let (b, dir, key) = setup("moved");
        let (a, z) = (pc(&b, 1), pc(&b, 2));
        add_reminder(&a, "一");
        push(&a.db, &a.data, &folder(&dir, &a, &key)).unwrap();
        // 把 A 的日志冒充成 C 的：附加数据对不上，解不开
        let fake = devices_dir(&dir).join("device-3-0000-cccc").join("log");
        std::fs::create_dir_all(&fake).unwrap();
        std::fs::copy(devices_dir(&dir).join(&a.id).join("log").join(seq_name(1, "dlog")), fake.join(seq_name(1, "dlog"))).unwrap();
        std::fs::remove_dir_all(devices_dir(&dir).join(&a.id)).unwrap();
        pull(&z.db, &z.data, &folder(&dir, &z, &key)).unwrap();
        assert!(titles(&z).is_empty());
        // 密钥不对也解不开
        std::fs::remove_dir_all(devices_dir(&dir).join("device-3-0000-cccc")).unwrap();
        push(&a.db, &a.data, &folder(&dir, &a, &key)).unwrap();
        db::mark_all_dirty(&a.db.lock().unwrap()).unwrap();
        push(&a.db, &a.data, &folder(&dir, &a, &key)).unwrap();
        pull(&z.db, &z.data, &folder(&dir, &z, &[7; 32])).unwrap();
        assert!(titles(&z).is_empty());
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn snapshot_replaces_old_logs() {
        let (b, dir, key) = setup("snap");
        let (a, z, y) = (pc(&b, 1), pc(&b, 2), pc(&b, 3));
        let fa = folder(&dir, &a, &key);
        add_reminder(&a, "第一条");
        push(&a.db, &a.data, &fa).unwrap();
        // Z 早就读过第 1 段
        pull(&z.db, &z.data, &folder(&dir, &z, &key)).unwrap();
        for i in 0..COMPACT_AFTER {
            add_reminder(&a, &format!("r{i:02}"));
            push(&a.db, &a.data, &fa).unwrap();
        }
        let own = devices_dir(&dir).join(&a.id);
        // 攒满 50 段时合成了快照，之前的日志都删了，只剩快照之后写的那段
        let n = COMPACT_AFTER as u64;
        assert_eq!(seqs(&own.join("snap"), "dsnap", false), [n]);
        assert_eq!(seqs(&own.join("log"), "dlog", false), [n + 1]);
        add_reminder(&a, "快照之后");
        push(&a.db, &a.data, &fa).unwrap();
        assert_eq!(seqs(&own.join("log"), "dlog", false), [n + 1, n + 2]);
        // 新来的 Y 和读到一半的 Z：都读快照再读后面的日志，全齐
        for p in [&z, &y] {
            pull(&p.db, &p.data, &folder(&dir, p, &key)).unwrap();
            assert_eq!(titles(p).len(), COMPACT_AFTER + 2);
        }
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn notes_and_images_travel_through_files() {
        let (b, dir, key) = setup("files");
        let (a, z) = (pc(&b, 1), pc(&b, 2));
        let file = {
            let c = a.db.lock().unwrap();
            let nid = db::upsert_note(&c, &Note { title: "装修".into(), ..Default::default() }).unwrap();
            let mut n = db::get_note(&c, nid).unwrap().unwrap();
            let name = db::save_file(&c, &a.data, "image/png", "", b"\x89PNG fake", Some(&n.uid), None).unwrap();
            n.body = format!("![](att:{name})");
            db::upsert_note(&c, &n).unwrap();
            name
        };
        push(&a.db, &a.data, &folder(&dir, &a, &key)).unwrap();
        assert_eq!(std::fs::read_dir(files_dir(&dir)).unwrap().count(), 1);
        pull(&z.db, &z.data, &folder(&dir, &z, &key)).unwrap();
        assert_eq!(db::list_notes(&z.db.lock().unwrap()).unwrap()[0].title, "装修");
        assert_eq!(std::fs::read(db::attachment_dir(&z.data).join(&file)).unwrap(), b"\x89PNG fake");
        let _ = std::fs::remove_dir_all(&b);
    }

    /// OneDrive 的占位文件造不出来，只能核对「始终保留在此设备上」的标记设得上、原来的属性不丢
    #[cfg(windows)]
    #[test]
    fn keep_local_sets_pinned() {
        use std::os::windows::fs::MetadataExt;
        let b = base("pin");
        let file = b.join("0000000001.dlog");
        std::fs::write(&file, b"x").unwrap();
        let mut perm = std::fs::metadata(&file).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(&file, perm).unwrap();
        keep_local(&file);
        keep_local(&b);
        let a = std::fs::metadata(&file).unwrap().file_attributes();
        assert_ne!(a & 0x8_0000, 0, "文件没设上");
        assert_ne!(a & 0x1, 0, "只读丢了");
        assert_ne!(std::fs::metadata(&b).unwrap().file_attributes() & 0x8_0000, 0, "文件夹没设上");
        assert_eq!(seqs(&b, "dlog", true), [1]);
        let mut perm = std::fs::metadata(&file).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        std::fs::set_permissions(&file, perm).unwrap();
        let _ = std::fs::remove_dir_all(&b);
    }
}
