//! 服务器的 SQLite：服务器自己的设置（kv）、空间、设备、邀请码。
//! 空间的备注名、容量上限、停用时间是给管理用的（需求-0.4 第 15.4 节），见 admin.rs。

use crate::net::hash_token;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use dingmi_sync::{InboxItem, Phone, Space};
use rusqlite::{params, Connection, OptionalExtension, Row};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

/// 服务器上 `dingmi-server invite` 生成的配对码的有效期
pub const INVITE_HOURS: i64 = 24;
/// 电脑上生成的配对码带着空间密钥，有效期短一些
pub const DEVICE_INVITE_MINUTES: i64 = 24 * 60;
/// 一个空间最多同时有几个没用掉的配对码
pub const MAX_PENDING_INVITES: i64 = 10;

/// 数据库文件名
pub const DB_FILE: &str = "dingmi-server.db";
pub fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// spaces.join_hash 写成这个：用户（或后台）关掉了兑换码加入（需求-1.x 第 34.2 节）
pub const JOIN_OFF: &str = "off";

pub struct Device {
    /// 设备的 Noise 公钥（base64url）
    pub pubkey: String,
    pub space_id: String,
    pub name: String,
    pub created_at: String,
    pub last_seen: Option<String>,
    pub revoked: bool,
}

fn row_device(r: &Row) -> rusqlite::Result<Device> {
    Ok(Device {
        pubkey: r.get(0)?,
        space_id: r.get(1)?,
        name: r.get(2)?,
        created_at: r.get(3)?,
        last_seen: r.get(4)?,
        revoked: r.get::<_, i64>(5)? != 0,
    })
}

const D_COLS: &str = "pubkey, space_id, name, created_at, last_seen, revoked";

/// 数据库被别的连接占着时（命令行在改、备份在读）最多等多久再报错
const BUSY_WAIT: Duration = Duration::from_secs(10);

pub struct Store {
    /// 数据库文件，备份时另开一个连接用
    path: PathBuf,
    c: Mutex<Connection>,
}

impl Store {
    /// 打开失败时的错误信息直接告诉用户多半是什么原因
    pub fn open(dir: &Path) -> Result<Store, String> {
        let hint = "多半是权限不对：dingmi-server 要用执行 init 时的同一个用户运行（宝塔里运行用户选 root）";
        Self::open_db(dir).map_err(|e| format!("打不开 {}：{e}。{hint}", dir.join(DB_FILE).display()))
    }

    fn open_db(dir: &Path) -> rusqlite::Result<Store> {
        let path = dir.join(DB_FILE);
        let c = Connection::open(&path)?;
        // 服务在跑的时候在终端里敲 quota、disable 这些：两边同时写会撞上，等一下就好，不直接报「数据库被锁」
        c.busy_timeout(BUSY_WAIT)?;
        c.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS kv(key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS spaces(
                id TEXT PRIMARY KEY,
                salt TEXT,
                box1 TEXT,
                box2 TEXT,
                recovery_hash TEXT UNIQUE,
                created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS devices(
                pubkey TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                name TEXT NOT NULL,
                created_at TEXT NOT NULL,
                last_seen TEXT,
                revoked INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS invites(
                token_hash TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                expires_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS phones(
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                name TEXT NOT NULL,
                token_hash TEXT NOT NULL UNIQUE,
                created_at TEXT NOT NULL,
                last_seen TEXT,
                revoked INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS inbox_items(
                id TEXT PRIMARY KEY,
                space_id TEXT NOT NULL,
                phone_id TEXT NOT NULL,
                size INTEGER NOT NULL,
                created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS items(
                space_id TEXT NOT NULL,
                uid TEXT NOT NULL,
                ver TEXT NOT NULL,
                seq INTEGER NOT NULL,
                blob BLOB NOT NULL,
                PRIMARY KEY(space_id, uid)
             );
             CREATE INDEX IF NOT EXISTS items_seq ON items(space_id, seq);
             CREATE TABLE IF NOT EXISTS files(
                space_id TEXT NOT NULL,
                uid TEXT NOT NULL,
                size INTEGER NOT NULL,
                complete INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                PRIMARY KEY(space_id, uid)
             );",
        )?;
        // 0.4 加的三列：备注名、容量上限（字节，空着是不限）、停用时间（空着是在用）
        let cols: Vec<String> = c
            .prepare("SELECT name FROM pragma_table_info('spaces')")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        // 1.x 加的两列（需求-1.x 第 34.2 节）：盒子 3（用同步密码锁着的空间密钥）、加入核对值的哈希，没开兑换码加入是空的
        for (col, ty) in [("note", "TEXT"), ("quota", "INTEGER"), ("disabled_at", "TEXT"), ("box3", "TEXT"), ("join_hash", "TEXT")] {
            if !cols.iter().any(|c| c == col) {
                c.execute_batch(&format!("ALTER TABLE spaces ADD COLUMN {col} {ty}"))?;
            }
        }
        // 叮咪同步（官方版）在同一个库里另外加的列和表（spaces 上的套餐、到期、邮箱……，兑换码、连接日志……）
        // 由它自己建（Ext::open）。开源版不建、不读；以前的库里有也没关系
        Ok(Store { path, c: Mutex::new(c) })
    }

    /// 数据库连接（官方版用它建自己的表、读写自己的列）
    pub fn c(&self) -> MutexGuard<'_, Connection> {
        self.c.lock().unwrap()
    }

    pub fn kv_get(&self, key: &str) -> Option<String> {
        self.c()
            .query_row("SELECT value FROM kv WHERE key=?1", [key], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn kv_set(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.c().execute(
            "INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// 新空间。容量上限由调用的地方设
    pub fn create_space(&self, id: &str) -> rusqlite::Result<()> {
        self.c().execute("INSERT INTO spaces(id, created_at) VALUES(?1, ?2)", params![id, now()])?;
        Ok(())
    }

    /// 最近建的空间（个人版通常只有一个）
    pub fn latest_space(&self) -> Option<String> {
        self.c()
            .query_row("SELECT id FROM spaces ORDER BY created_at DESC, rowid DESC LIMIT 1", [], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn space(&self, id: &str) -> Space {
        self.c()
            .query_row("SELECT salt, box1, box2 FROM spaces WHERE id=?1", [id], |r| {
                Ok(Space { salt: r.get(0)?, box1: r.get(1)?, box2: r.get(2)?, box3: None })
            })
            .unwrap_or_default()
    }

    /// 兑换码加入登记的那次连接：Welcome.space 里多带盒子 3（关掉的没有盒子 3）
    pub fn space_with_box3(&self, id: &str) -> Space {
        let mut sp = self.space(id);
        sp.box3 = self.c().query_row("SELECT box3 FROM spaces WHERE id=?1", [id], |r| r.get(0)).ok().flatten();
        sp
    }

    /// 只有还没设过的空间才能设；设过了返回 false。`join`：(盒子 3, 加入核对值的哈希)，没开兑换码加入是 None
    pub fn setup_space(&self, id: &str, salt: &str, box1: &str, box2: &str, recovery_hash: &str, join: Option<(&str, &str)>) -> bool {
        let (box3, hash) = join.unzip();
        self.c()
            .execute(
                "UPDATE spaces SET salt=?2, box1=?3, box2=?4, recovery_hash=?5, box3=?6, join_hash=?7 WHERE id=?1 AND salt IS NULL",
                params![id, salt, box1, box2, recovery_hash, box3, hash],
            )
            .map(|n| n == 1)
            .unwrap_or(false)
    }

    /// 换同步密码：只有已经设过的空间才能换。盒子 3 和核对值是按 salt 算的，一起换；没带就删掉原来的。
    /// 用户关掉了兑换码加入的（join_hash 是 JOIN_OFF）照旧关着，别的电脑换密码时带来的不收
    pub fn set_lock(&self, id: &str, salt: &str, box1: &str, join: Option<(&str, &str)>) -> bool {
        let (box3, hash) = join.unzip();
        self.c()
            .execute(
                "UPDATE spaces SET salt=?2, box1=?3,
                    box3=CASE WHEN join_hash=?6 THEN NULL ELSE ?4 END,
                    join_hash=CASE WHEN join_hash=?6 THEN ?6 ELSE ?5 END
                 WHERE id=?1 AND salt IS NOT NULL",
                params![id, salt, box1, box3, hash, JOIN_OFF],
            )
            .map(|n| n == 1)
            .unwrap_or(false)
    }

    /// 打开（Some）/ 关掉（None）兑换码加入：只有已经设过同步密码的空间才行。
    /// 关掉的记成 JOIN_OFF（不是空着）：以后哪台电脑换同步密码都不再自动打开，要打开得明说（JoinSet）
    /// `salt`：打开时电脑算盒子 3 用的 salt，和服务器上的对不上（别的电脑刚换了同步密码）就不收，返回 false
    pub fn set_join(&self, id: &str, join: Option<(&str, &str)>, salt: Option<&str>) -> bool {
        let (box3, hash) = join.unzip();
        self.c()
            .execute(
                "UPDATE spaces SET box3=?2, join_hash=?3 WHERE id=?1 AND salt IS NOT NULL AND (?4 IS NULL OR salt=?4)",
                params![id, box3, hash.unwrap_or(JOIN_OFF), salt],
            )
            .map(|n| n == 1)
            .unwrap_or(false)
    }

    /// 自建服务器（Ext::joins 是 false）启动时：清掉 1.0.6、1.0.7 的电脑传上来的盒子 3 和加入核对值。回清了几个空间
    pub fn clear_joins(&self) -> usize {
        self.c()
            .execute("UPDATE spaces SET box3=NULL, join_hash=NULL WHERE box3 IS NOT NULL OR join_hash IS NOT NULL", [])
            .unwrap_or(0)
    }

    /// 开着兑换码加入：回 (salt, 加入核对值的哈希)
    pub fn join_lock(&self, id: &str) -> Option<(String, String)> {
        self.c()
            .query_row(
                "SELECT salt, join_hash FROM spaces WHERE id=?1 AND salt IS NOT NULL AND box3 IS NOT NULL AND join_hash IS NOT NULL AND join_hash<>?2",
                params![id, JOIN_OFF],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()
    }

    /// 用户（或后台）关掉了兑换码加入
    pub fn join_turned_off(&self, id: &str) -> bool {
        self.c().query_row("SELECT 1 FROM spaces WHERE id=?1 AND join_hash=?2", params![id, JOIN_OFF], |_| Ok(())).is_ok()
    }

    /// 换恢复码：旧的恢复凭证跟着作废
    pub fn set_recovery(&self, id: &str, box2: &str, recovery_hash: &str) -> bool {
        self.c()
            .execute(
                "UPDATE spaces SET box2=?2, recovery_hash=?3 WHERE id=?1 AND salt IS NOT NULL",
                params![id, box2, recovery_hash],
            )
            .map(|n| n == 1)
            .unwrap_or(false)
    }

    /// 凭恢复凭证找到空间（没有电脑可以生成配对码时，用恢复码登记新电脑）
    pub fn space_by_recovery(&self, recovery_hash: &str) -> Option<String> {
        self.c()
            .query_row("SELECT id FROM spaces WHERE recovery_hash=?1", [recovery_hash], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    }

    // ---------- 管理（admin.rs 用） ----------

    /// 所有空间：(编号, 备注名, 建立时间, 容量上限, 停用时间)，先建的在前
    #[allow(clippy::type_complexity)]
    pub fn space_rows(&self) -> Vec<(String, Option<String>, String, Option<u64>, Option<String>)> {
        let c = self.c();
        let Ok(mut st) =
            c.prepare("SELECT id, note, created_at, quota, disabled_at FROM spaces ORDER BY created_at, rowid")
        else {
            return vec![];
        };
        let rows = st.query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, Option<i64>>(3)?.map(|q| q as u64), r.get(4)?))
        });
        rows.map(|rows| rows.filter_map(Result::ok).collect()).unwrap_or_default()
    }

    /// (记录占的字节, 附件占的字节)
    pub fn usage(&self, space_id: &str) -> (u64, u64) {
        let c = self.c();
        let q = |sql: &str| c.query_row(sql, [space_id], |r| r.get::<_, i64>(0)).unwrap_or(0) as u64;
        (
            q("SELECT COALESCE(SUM(length(blob)), 0) FROM items WHERE space_id=?1"),
            q("SELECT COALESCE(SUM(size), 0) FROM files WHERE space_id=?1"),
        )
    }

    pub fn quota(&self, space_id: &str) -> Option<u64> {
        self.c()
            .query_row("SELECT quota FROM spaces WHERE id=?1", [space_id], |r| r.get::<_, Option<i64>>(0))
            .ok()
            .flatten()
            .map(|q| q as u64)
    }

    /// 这几条记录现在在服务器上占多少（推上来的新版本会顶掉它们）
    pub fn items_size(&self, space_id: &str, uids: &[&str]) -> u64 {
        let c = self.c();
        uids.iter()
            .map(|u| {
                c.query_row("SELECT length(blob) FROM items WHERE space_id=?1 AND uid=?2", params![space_id, u], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap_or(0) as u64
            })
            .sum()
    }

    pub fn file_size(&self, space_id: &str, uid: &str) -> u64 {
        self.c()
            .query_row("SELECT size FROM files WHERE space_id=?1 AND uid=?2", params![space_id, uid], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(0) as u64
    }

    /// 停用时间；空间不存在或在用都是 None
    pub fn disabled_at(&self, space_id: &str) -> Option<String> {
        self.c().query_row("SELECT disabled_at FROM spaces WHERE id=?1", [space_id], |r| r.get(0)).ok().flatten()
    }

    pub fn set_note(&self, space_id: &str, note: Option<&str>) -> bool {
        self.c().execute("UPDATE spaces SET note=?2 WHERE id=?1", params![space_id, note]).is_ok_and(|n| n == 1)
    }

    pub fn set_quota(&self, space_id: &str, quota: Option<u64>) -> bool {
        self.c()
            .execute("UPDATE spaces SET quota=?2 WHERE id=?1", params![space_id, quota.map(|q| q as i64)])
            .is_ok_and(|n| n == 1)
    }

    /// 停用（at 是时间）或恢复（None）
    pub fn set_disabled(&self, space_id: &str, at: Option<&str>) -> bool {
        self.c().execute("UPDATE spaces SET disabled_at=?2 WHERE id=?1", params![space_id, at]).is_ok_and(|n| n == 1)
    }

    /// 停用时间早于 before 的空间
    pub fn disabled_before(&self, before: &str) -> Vec<String> {
        let c = self.c();
        let Ok(mut st) = c.prepare("SELECT id FROM spaces WHERE disabled_at IS NOT NULL AND disabled_at < ?1") else {
            return vec![];
        };
        let rows = st.query_map([before], |r| r.get(0));
        rows.map(|rows| rows.filter_map(Result::ok).collect()).unwrap_or_default()
    }

    /// 真删一个空间在数据库里的所有东西，返回它在收件箱里还有哪些（文件要另外删）
    pub fn delete_space(&self, space_id: &str) -> rusqlite::Result<Vec<String>> {
        let c = self.c();
        let tx = c.unchecked_transaction()?;
        let inbox: Vec<String> = tx
            .prepare("SELECT id FROM inbox_items WHERE space_id=?1")?
            .query_map([space_id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for table in ["items", "files", "inbox_items", "phones", "devices", "invites"] {
            tx.execute(&format!("DELETE FROM {table} WHERE space_id=?1"), [space_id])?;
        }
        tx.execute("DELETE FROM spaces WHERE id=?1", [space_id])?;
        tx.commit()?;
        Ok(inbox)
    }

    /// 某个空间的所有手机，作废了的也在（后台、命令行看）：(id, 名字, 添加时间, 最后连上, 作废了没有)
    #[allow(clippy::type_complexity)]
    pub fn all_phones(&self, space_id: &str) -> Vec<(String, String, String, Option<String>, bool)> {
        let c = self.c();
        let Ok(mut st) = c.prepare(
            "SELECT id, name, created_at, last_seen, revoked FROM phones WHERE space_id=?1 ORDER BY created_at",
        ) else {
            return vec![];
        };
        let rows =
            st.query_map([space_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, i64>(4)? != 0)));
        rows.map(|rows| rows.filter_map(Result::ok).collect()).unwrap_or_default()
    }

    pub fn revoke_device(&self, pubkey: &str) -> bool {
        self.c().execute("UPDATE devices SET revoked=1 WHERE pubkey=?1 AND revoked=0", [pubkey]).is_ok_and(|n| n == 1)
    }

    /// 生成一个新的邀请码，`minutes` 分钟后过期，返回 (邀请码, 过期时间)。数据库里只存它的哈希
    pub fn new_invite(&self, space_id: &str, minutes: i64) -> rusqlite::Result<(String, String)> {
        let token = B64.encode(rand::random::<[u8; 16]>());
        let expires = (chrono::Local::now() + chrono::TimeDelta::minutes(minutes))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        self.add_invite(&hash_token(&token), space_id, &expires)?;
        Ok((token, expires))
    }

    pub fn add_invite(&self, token_hash: &str, space_id: &str, expires_at: &str) -> rusqlite::Result<()> {
        self.c().execute(
            "INSERT INTO invites(token_hash, space_id, expires_at) VALUES(?1, ?2, ?3)",
            params![token_hash, space_id, expires_at],
        )?;
        Ok(())
    }

    /// 邀请码还在不在（没用、没删；过没过期另外看 expires_at）
    pub fn invite_exists(&self, token_hash: &str) -> bool {
        self.c().query_row("SELECT 1 FROM invites WHERE token_hash=?1", [token_hash], |_| Ok(())).is_ok()
    }

    pub fn del_invite(&self, token_hash: &str) -> bool {
        self.c().execute("DELETE FROM invites WHERE token_hash=?1", [token_hash]).is_ok_and(|n| n == 1)
    }

    /// 邀请码属于哪个空间（没过期的），不用掉它：先核对能不能登记（设备满了没有），再 take_invite
    pub fn invite_space(&self, token_hash: &str) -> Option<String> {
        self.c()
            .query_row(
                "SELECT space_id FROM invites WHERE token_hash=?1 AND expires_at > ?2",
                params![token_hash, now()],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    /// 用掉一个邀请码：不管过没过期都删掉，没过期才返回它属于哪个空间
    pub fn take_invite(&self, token_hash: &str) -> Option<String> {
        let c = self.c();
        let row: Option<(String, String)> = c
            .query_row(
                "SELECT space_id, expires_at FROM invites WHERE token_hash=?1",
                [token_hash],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .ok()
            .flatten();
        let _ = c.execute("DELETE FROM invites WHERE token_hash=?1", [token_hash]);
        row.filter(|(_, exp)| exp.as_str() > now().as_str()).map(|(space, _)| space)
    }

    /// 一个空间最多留 `keep` 个没用掉的邀请码，多的删最早过期的（配对码 24 小时有效，免得一直要、一直堆）
    pub fn trim_invites(&self, space_id: &str, keep: i64) {
        let _ = self.c().execute(
            "DELETE FROM invites WHERE space_id=?1 AND token_hash NOT IN
               (SELECT token_hash FROM invites WHERE space_id=?1 ORDER BY expires_at DESC, rowid DESC LIMIT ?2)",
            params![space_id, keep],
        );
    }

    pub fn purge_invites(&self) {
        let _ = self.c().execute("DELETE FROM invites WHERE expires_at <= ?1", [now()]);
    }

    pub fn device(&self, pubkey: &str) -> Option<Device> {
        self.c()
            .query_row(&format!("SELECT {D_COLS} FROM devices WHERE pubkey=?1"), [pubkey], row_device)
            .optional()
            .ok()
            .flatten()
    }

    /// 登记设备；作废过的设备用新邀请码重新配对时恢复
    pub fn add_device(&self, pubkey: &str, space_id: &str, name: &str) -> rusqlite::Result<()> {
        self.c().execute(
            "INSERT INTO devices(pubkey, space_id, name, created_at) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(pubkey) DO UPDATE SET space_id=excluded.space_id, name=excluded.name, revoked=0",
            params![pubkey, space_id, name, now()],
        )?;
        Ok(())
    }

    pub fn touch_device(&self, pubkey: &str) {
        let _ = self
            .c()
            .execute("UPDATE devices SET last_seen=?2 WHERE pubkey=?1", params![pubkey, now()]);
    }

    pub fn devices(&self) -> Vec<Device> {
        let c = self.c();
        let Ok(mut st) = c.prepare(&format!("SELECT {D_COLS} FROM devices ORDER BY created_at")) else {
            return vec![];
        };
        let rows = st.query_map([], row_device);
        rows.map(|rows| rows.filter_map(Result::ok).collect()).unwrap_or_default()
    }

    /// 按设备名或公钥开头（至少 6 位）作废设备，返回作废了哪些
    pub fn revoke(&self, who: &str) -> Vec<String> {
        let hits: Vec<Device> = self
            .devices()
            .into_iter()
            .filter(|d| !d.revoked && (d.name == who || (who.len() >= 6 && d.pubkey.starts_with(who))))
            .collect();
        for d in &hits {
            let _ = self.c().execute("UPDATE devices SET revoked=1 WHERE pubkey=?1", [&d.pubkey]);
        }
        hits.into_iter().map(|d| d.name).collect()
    }

    // ---------- 手机 ----------

    pub fn add_phone(&self, id: &str, space_id: &str, name: &str, token_hash: &str) -> rusqlite::Result<()> {
        self.c().execute(
            "INSERT INTO phones(id, space_id, name, token_hash, created_at) VALUES(?1, ?2, ?3, ?4, ?5)",
            params![id, space_id, name, token_hash, now()],
        )?;
        Ok(())
    }

    pub fn phones(&self, space_id: &str) -> Vec<Phone> {
        let c = self.c();
        let Ok(mut st) = c.prepare(
            "SELECT id, name, created_at, last_seen FROM phones WHERE space_id=?1 AND revoked=0 ORDER BY created_at",
        ) else {
            return vec![];
        };
        let rows = st.query_map([space_id], |r| {
            Ok(Phone { id: r.get(0)?, name: r.get(1)?, created_at: r.get(2)?, last_seen: r.get(3)? })
        });
        rows.map(|rows| rows.filter_map(Result::ok).collect()).unwrap_or_default()
    }

    pub fn revoke_phone(&self, space_id: &str, id: &str) -> bool {
        self.c()
            .execute("UPDATE phones SET revoked=1 WHERE id=?1 AND space_id=?2", params![id, space_id])
            .map(|n| n == 1)
            .unwrap_or(false)
    }

    /// 令牌对应的手机：(手机 id, 空间 id)；作废了的不算
    pub fn phone_by_token(&self, token_hash: &str) -> Option<(String, String)> {
        self.c()
            .query_row(
                "SELECT id, space_id FROM phones WHERE token_hash=?1 AND revoked=0",
                [token_hash],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .ok()
            .flatten()
    }

    pub fn touch_phone(&self, id: &str) {
        let _ = self.c().execute("UPDATE phones SET last_seen=?2 WHERE id=?1", params![id, now()]);
    }

    // ---------- 手机收件箱 ----------

    pub fn add_inbox(&self, id: &str, space_id: &str, phone_id: &str, size: u64) -> rusqlite::Result<()> {
        self.c().execute(
            "INSERT INTO inbox_items(id, space_id, phone_id, size, created_at) VALUES(?1, ?2, ?3, ?4, ?5)",
            params![id, space_id, phone_id, size as i64, now()],
        )?;
        Ok(())
    }

    pub fn inbox(&self, space_id: &str) -> Vec<InboxItem> {
        let c = self.c();
        let Ok(mut st) = c.prepare(
            "SELECT i.id, COALESCE(p.name, ''), i.size, i.created_at FROM inbox_items i
             LEFT JOIN phones p ON p.id = i.phone_id
             WHERE i.space_id=?1 ORDER BY i.created_at",
        ) else {
            return vec![];
        };
        let rows = st.query_map([space_id], |r| {
            Ok(InboxItem { id: r.get(0)?, phone: r.get(1)?, size: r.get::<_, i64>(2)? as u64, created_at: r.get(3)? })
        });
        rows.map(|rows| rows.filter_map(Result::ok).collect()).unwrap_or_default()
    }

    /// (条数, 总大小)：防止手机令牌泄露后被人塞满服务器
    pub fn inbox_usage(&self, space_id: &str) -> (u64, u64) {
        self.c()
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM inbox_items WHERE space_id=?1",
                [space_id],
                |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)),
            )
            .unwrap_or((0, 0))
    }

    /// `before` 之前发来、一直没被电脑取走的：删掉记录，返回编号好删文件
    pub fn stale_inbox(&self, before: &str) -> Vec<String> {
        let c = self.c();
        let Ok(mut st) = c.prepare("SELECT id FROM inbox_items WHERE created_at<?1") else {
            return vec![];
        };
        let ids: Vec<String> =
            st.query_map([before], |r| r.get(0)).map(|rows| rows.filter_map(Result::ok).collect()).unwrap_or_default();
        let _ = c.execute("DELETE FROM inbox_items WHERE created_at<?1", [before]);
        ids
    }

    pub fn has_inbox(&self, space_id: &str, id: &str) -> bool {
        self.c()
            .query_row("SELECT 1 FROM inbox_items WHERE id=?1 AND space_id=?2", params![id, space_id], |_| Ok(()))
            .is_ok()
    }

    pub fn del_inbox(&self, space_id: &str, id: &str) -> bool {
        self.c()
            .execute("DELETE FROM inbox_items WHERE id=?1 AND space_id=?2", params![id, space_id])
            .map(|n| n == 1)
            .unwrap_or(false)
    }

    // ---------- 记录同步 ----------

    /// 存一条记录，见 push_items
    pub fn push_item(&self, space_id: &str, uid: &str, ver: &str, blob: &[u8]) -> rusqlite::Result<bool> {
        Ok(self.push_items(space_id, &[(uid, ver, Some(blob))])?[0])
    }

    /// 存一批记录（编号, 版本, 密文；密文是 None 的是不收的，直接算没存上），**一个事务、只写一次盘**：
    /// 一条一条提交的话每条都要等硬盘写好，新用户第一次同步几千条会把数据库占住好几秒（压力测试量过）。
    /// 每条：服务器上没有、或者带来的版本更新才存，并给一个新的序号。
    /// 返回每条在服务器上现在是不是这个版本（一模一样的版本已经有了也算，比如重新配对后又传了一遍）
    #[allow(clippy::type_complexity)]
    pub fn push_items(&self, space_id: &str, items: &[(&str, &str, Option<&[u8]>)]) -> rusqlite::Result<Vec<bool>> {
        let c = self.c();
        let tx = c.unchecked_transaction()?;
        let mut seq: i64 =
            tx.query_row("SELECT COALESCE(MAX(seq), 0) FROM items WHERE space_id=?1", [space_id], |r| r.get(0))?;
        let mut out = Vec::with_capacity(items.len());
        {
            let mut have = tx.prepare_cached("SELECT ver FROM items WHERE space_id=?1 AND uid=?2")?;
            let mut put = tx.prepare_cached(
                "INSERT INTO items(space_id, uid, ver, seq, blob) VALUES(?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(space_id, uid) DO UPDATE SET ver=excluded.ver, seq=excluded.seq, blob=excluded.blob
                 WHERE excluded.ver > items.ver",
            )?;
            for (uid, ver, blob) in items {
                let Some(blob) = blob else {
                    out.push(false);
                    continue;
                };
                let now: Option<String> = have.query_row(params![space_id, uid], |r| r.get(0)).optional()?;
                if now.as_deref() == Some(*ver) {
                    out.push(true);
                    continue;
                }
                let n = put.execute(params![space_id, uid, ver, seq + 1, blob])?;
                if n == 1 {
                    seq += 1;
                }
                out.push(n == 1);
            }
        }
        tx.commit()?;
        Ok(out)
    }

    /// 序号大于 since 的记录，按序号排；凑够 max_bytes 或 max_count 就先停。返回 (记录, 后面还有没有)
    #[allow(clippy::type_complexity)]
    pub fn pull_items(
        &self,
        space_id: &str,
        since: u64,
        max_bytes: usize,
        max_count: usize,
    ) -> (Vec<(String, String, u64, Vec<u8>)>, bool) {
        let c = self.c();
        let Ok(mut st) = c.prepare("SELECT uid, ver, seq, blob FROM items WHERE space_id=?1 AND seq>?2 ORDER BY seq") else {
            return (vec![], false);
        };
        let Ok(rows) = st.query_map(params![space_id, since as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)? as u64, r.get::<_, Vec<u8>>(3)?))
        }) else {
            return (vec![], false);
        };
        let mut out = Vec::new();
        let mut size = 0;
        for row in rows.filter_map(Result::ok) {
            if !out.is_empty() && (size + row.3.len() > max_bytes || out.len() >= max_count) {
                return (out, true);
            }
            size += row.3.len();
            out.push(row);
        }
        (out, false)
    }

    /// 开始或继续上传一个图片文件，记下大小；传完时标记 complete
    pub fn file_put(&self, space_id: &str, uid: &str, size: u64, done: bool) -> rusqlite::Result<()> {
        self.c().execute(
            "INSERT INTO files(space_id, uid, size, complete, created_at) VALUES(?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(space_id, uid) DO UPDATE SET size=excluded.size, complete=excluded.complete",
            params![space_id, uid, size as i64, done as i64, now()],
        )?;
        Ok(())
    }

    pub fn file_complete(&self, space_id: &str, uid: &str) -> bool {
        self.c()
            .query_row(
                "SELECT complete FROM files WHERE space_id=?1 AND uid=?2",
                params![space_id, uid],
                |r| r.get::<_, i64>(0),
            )
            .is_ok_and(|c| c == 1)
    }

    /// 传了一半就没再传的附件（created_at 早于 before、还没传完）：删掉记录，返回 (空间, 编号) 好删文件。
    /// 不删的话它一直算在空间的占用里
    pub fn stale_uploads(&self, before: &str) -> Vec<(String, String)> {
        let c = self.c();
        let Ok(mut st) = c.prepare("SELECT space_id, uid FROM files WHERE complete=0 AND created_at<?1") else {
            return vec![];
        };
        let rows: Vec<(String, String)> = st
            .query_map([before], |r| Ok((r.get(0)?, r.get(1)?)))
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default();
        let _ = c.execute("DELETE FROM files WHERE complete=0 AND created_at<?1", [before]);
        rows
    }

    pub fn file_del(&self, space_id: &str, uid: &str) -> bool {
        self.c()
            .execute("DELETE FROM files WHERE space_id=?1 AND uid=?2", params![space_id, uid])
            .is_ok_and(|n| n == 1)
    }

    /// 把整个数据库备份成 backups/dingmi-server-<day>.db，只留最近 `keep` 份。
    /// 另开一个连接读（WAL 模式下读和写互不挡），备份期间同步照常进行，不会卡住整个服务
    pub fn backup(&self, dir: &Path, day: &str, keep: usize) -> Result<(), String> {
        let bdir = dir.join("backups");
        std::fs::create_dir_all(&bdir).map_err(|e| e.to_string())?;
        let file = bdir.join(format!("dingmi-server-{day}.db"));
        let _ = std::fs::remove_file(&file);
        let path = file.to_string_lossy().to_string();
        let c = Connection::open(&self.path).map_err(|e| e.to_string())?;
        c.busy_timeout(BUSY_WAIT).map_err(|e| e.to_string())?;
        c.execute("VACUUM INTO ?1", [&path]).map_err(|e| e.to_string())?;
        drop(c);
        let mut old: Vec<_> = std::fs::read_dir(&bdir)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("dingmi-server-") && n.ends_with(".db"))
                    .unwrap_or(false)
            })
            .collect();
        // 文件名里带日期，按名字排就是按时间排
        old.sort();
        let extra = old.len().saturating_sub(keep);
        for p in old.into_iter().take(extra) {
            let _ = std::fs::remove_file(p);
        }
        // restore-backup 改名留着的旧数据库（dingmi-server.db.恢复前-…）：也只留 keep 天，不让它一直占地方
        let cutoff = std::time::SystemTime::now() - Duration::from_secs(keep as u64 * 86400);
        for e in std::fs::read_dir(dir).map_err(|e| e.to_string())?.filter_map(|e| e.ok()) {
            let name = e.file_name().to_string_lossy().to_string();
            let old = e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t < cutoff);
            if name.starts_with(&format!("{DB_FILE}.恢复前-")) && old {
                let _ = std::fs::remove_file(e.path());
            }
        }
        Ok(())
    }

    /// 收件箱里所有的编号（各个空间的），restore-backup 用来清掉文件已经没了的
    pub fn inbox_ids(&self) -> Vec<(String, String)> {
        let c = self.c();
        let Ok(mut st) = c.prepare("SELECT space_id, id FROM inbox_items") else {
            return vec![];
        };
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)));
        rows.map(|rows| rows.filter_map(Result::ok).collect()).unwrap_or_default()
    }


    /// 没作废的电脑加手机
    pub fn device_count(&self, space: &str) -> u32 {
        self.c()
            .query_row(
                "SELECT (SELECT COUNT(*) FROM devices WHERE space_id=?1 AND revoked=0)
                      + (SELECT COUNT(*) FROM phones WHERE space_id=?1 AND revoked=0)",
                [space],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0) as u32
    }
}
