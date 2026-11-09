//! 管理操作（需求-0.4 第 15.4 节）：命令行（cli.rs）用这一份，规则都写在这里、不直接写 SQL。
//!
//! - 空间：备注名、容量上限、停用和恢复；停用满几天（默认 30）由每天的收尾连数据一起删掉
//! - 电脑、手机：列出、作废（正在连着的电脑下一条消息就断开，见 net.rs）
//! - 配对码：给已有空间或新空间生成
//! - 备份：列出每天的备份
//! - 几个天数能改（停用后几天删、备份留几天）、新空间的默认容量上限：存在 kv 里，命令行 `config` 改
//! - 拿旧备份恢复（`restore-backup`）：换数据库、换「代」（epoch），电脑和手机连上时自己从头同步
//!
//! 服务器上只有密文，这里能看到的只有设备名、手机名、大小和时间。

use crate::net::{hash_token, inbox_path, space_files};
use crate::store::{now, Device, Store, INVITE_HOURS};

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use dingmi_sync::PairCode;
use std::path::{Path, PathBuf};

/// 能改的天数：存在 kv 里，没改过用默认的
pub struct Days {
    pub key: &'static str,
    pub default: i64,
    pub min: i64,
    pub max: i64,
    pub name: &'static str,
}

/// 停用以后几天真删。改了对已经停用的空间也算（删除日期 = 停用时间 + 现在的天数）
pub const DELETE_DAYS: Days = Days { key: "delete_after_days", default: 30, min: 1, max: 365, name: "停用的空间几天后删掉" };
/// 每天的数据库备份留几份
pub const BACKUP_DAYS: Days = Days { key: "backup_keep_days", default: 14, min: 1, max: 90, name: "备份留几天" };

pub fn days(store: &Store, d: &Days) -> i64 {
    store.kv_get(d.key).and_then(|v| v.parse().ok()).filter(|n| (d.min..=d.max).contains(n)).unwrap_or(d.default)
}

pub fn set_days(store: &Store, d: &Days, n: i64) -> Result<(), String> {
    if !(d.min..=d.max).contains(&n) {
        return Err(format!("{}只能在 {} 到 {} 天之间", d.name, d.min, d.max));
    }
    store.kv_set(d.key, &n.to_string()).map_err(|e| e.to_string())
}

/// 备注名最多几个字
const NOTE_MAX: usize = 20;
/// 用到百分之几算「快满了」
pub const NEAR_FULL_PERCENT: u64 = 90;

pub const SPACE_OFF: &str = "这个空间停用了，问问服务器管理员";

pub struct SpaceInfo {
    pub id: String,
    pub note: Option<String>,
    pub created_at: String,
    /// 没作废的电脑、手机
    pub computers: usize,
    pub phones: usize,
    /// 记录、附件各占多少字节（密文）
    pub records: u64,
    pub files: u64,
    pub quota: Option<u64>,
    /// 电脑、手机最后一次连上的时间里最晚的那个
    pub last_active: Option<String>,
    pub disabled_at: Option<String>,
    /// 停用了的话哪天真删（日期）
    pub delete_on: Option<String>,
    /// 设过同步密码没有（服务器生成的配对码要不要输恢复码）
    pub set_up: bool,
}

impl SpaceInfo {
    pub fn used(&self) -> u64 {
        self.records + self.files
    }

    /// 给人看的名字：有备注名用备注名，没有就是编号前 8 位
    pub fn label(&self) -> String {
        self.note.clone().unwrap_or_else(|| short(&self.id))
    }

    pub fn full(&self) -> bool {
        self.quota.is_some_and(|q| self.used() >= q)
    }
}

fn short(id: &str) -> String {
    id.chars().take(8).collect()
}

fn delete_on(disabled_at: &str, days: i64) -> Option<String> {
    let t = chrono::NaiveDateTime::parse_from_str(disabled_at, "%Y-%m-%d %H:%M:%S").ok()?;
    Some((t + chrono::TimeDelta::days(days)).format("%Y-%m-%d").to_string())
}

pub fn spaces(store: &Store) -> Vec<SpaceInfo> {
    let devices = store.devices();
    let keep = days(store, &DELETE_DAYS);
    store
        .space_rows()
        .into_iter()
        .map(|(id, note, created_at, quota, disabled_at)| {
            let mine: Vec<&Device> = devices.iter().filter(|d| d.space_id == id && !d.revoked).collect();
            let phones = store.phones(&id);
            let last_active = mine
                .iter()
                .filter_map(|d| d.last_seen.clone())
                .chain(phones.iter().filter_map(|p| p.last_seen.clone()))
                .max();
            let (records, files) = store.usage(&id);
            SpaceInfo {
                set_up: store.space(&id).salt.is_some(),
                computers: mine.len(),
                phones: phones.len(),
                id,
                note,
                created_at,
                records,
                files,
                quota,
                last_active,
                delete_on: disabled_at.as_deref().and_then(|t| delete_on(t, keep)),
                disabled_at,
            }
        })
        .collect()
}

/// 按备注名（一字不差）或编号开头（至少 4 位）找空间；找到好几个时报错，让人写得更具体
pub fn find_space(store: &Store, who: &str) -> Result<SpaceInfo, String> {
    let who = who.trim();
    let mut hits: Vec<SpaceInfo> = spaces(store)
        .into_iter()
        .filter(|s| s.note.as_deref() == Some(who) || (who.len() >= 4 && s.id.starts_with(who)))
        .collect();
    match hits.len() {
        0 => Err(format!("没找到空间「{who}」：写备注名或编号前几位（至少 4 位），用 spaces 看看")),
        1 => Ok(hits.remove(0)),
        _ => Err(format!(
            "「{who}」对上了好几个空间：{}，写长一点的编号",
            hits.iter().map(|s| format!("{}（{}）", s.label(), short(&s.id))).collect::<Vec<_>>().join("、")
        )),
    }
}

/// 改备注名；空的就是去掉备注名。不能和别的空间重名（命令行要靠备注名找空间）
pub fn set_note(store: &Store, space: &str, note: &str) -> Result<(), String> {
    let note: String = note.trim().chars().filter(|c| !c.is_control()).collect();
    if note.chars().count() > NOTE_MAX {
        return Err(format!("备注名最多 {NOTE_MAX} 个字"));
    }
    if !note.is_empty() && spaces(store).iter().any(|s| s.id != space && s.note.as_deref() == Some(note.as_str())) {
        return Err(format!("已经有一个空间叫「{note}」了"));
    }
    if store.set_note(space, (!note.is_empty()).then_some(note.as_str())) {
        Ok(())
    } else {
        Err("没有这个空间".into())
    }
}

/// 设容量上限，None 是不限。设得比现在用的还小也行：已经存着的不动，只是推不上新的
pub fn set_quota(store: &Store, space: &str, quota: Option<u64>) -> Result<(), String> {
    if store.set_quota(space, quota) {
        Ok(())
    } else {
        Err("没有这个空间".into())
    }
}

/// 再存 `add` 字节还放不放得下（没设上限就总放得下）。`add` 是这次多出来的：新版本顶掉旧版本的，先减掉旧的
pub fn fits(store: &Store, space: &str, add: u64) -> bool {
    match store.quota(space) {
        None => true,
        Some(q) => {
            let (r, f) = store.usage(space);
            r + f + add <= q
        }
    }
}

/// 记录（提醒、笔记这些文字）对容量放宽（需求-1.0 第 17.3 节：满了只拦附件）：满了照收，
/// 最多超出上限 5%（2026-11-07 作者定：满了要升级或者删内容，不能再多）
pub fn fits_records(store: &Store, space: &str, add: u64) -> bool {
    match store.quota(space) {
        None => true,
        Some(q) => {
            let (r, f) = store.usage(space);
            r + f + add <= q.saturating_add(q / 20)
        }
    }
}

/// 用了 `used`、上限 `quota`：满了（full）、快满了（near_full）、还行（ok），给电脑看的（Account.storage）
pub fn storage(used: u64, quota: Option<u64>) -> &'static str {
    match quota {
        Some(q) if used >= q => "full",
        Some(q) if used.saturating_mul(100) >= q.saturating_mul(NEAR_FULL_PERCENT) => "near_full",
        _ => "ok",
    }
}

/// 空间满了时给人看的话。`fix` 是最后怎么办（自建服务器是「请服务器管理员调高上限」，扩展点可以换）
pub fn full_msg(store: &Store, space: &str, fix: &str) -> String {
    let q = store.quota(space).map(fmt_size).unwrap_or_default();
    format!("空间满了（上限 {q}）：新的图片和文件暂时只在这台电脑上，文字照常同步。删掉一些附件、清空回收站，{fix}")
}

/// 停用：电脑和手机都连不上了，过几天（DELETE_DAYS）连数据一起删掉，之前可以恢复。返回哪天删
pub fn disable(store: &Store, space: &str) -> Result<String, String> {
    if store.disabled_at(space).is_some() {
        return Err("这个空间已经停用了".into());
    }
    let at = now();
    if !store.set_disabled(space, Some(&at)) {
        return Err("没有这个空间".into());
    }
    Ok(delete_on(&at, days(store, &DELETE_DAYS)).unwrap_or_default())
}

pub fn restore(store: &Store, space: &str) -> Result<(), String> {
    if store.disabled_at(space).is_none() {
        return Err("这个空间没有停用".into());
    }
    store.set_disabled(space, None);
    Ok(())
}

/// 新空间的默认容量上限（字节），None 是不限
pub fn default_quota(store: &Store) -> Option<u64> {
    store.kv_get("default_quota").and_then(|v| v.trim().parse::<u64>().ok()).filter(|q| *q > 0)
}

/// 改新空间的默认容量上限：只管以后新建的空间，已有的不变
pub fn set_default_quota(store: &Store, quota: Option<u64>) -> Result<(), String> {
    let v = quota.filter(|q| *q > 0).map(|q| q.to_string()).unwrap_or_default();
    store.kv_set("default_quota", &v).map_err(|e| e.to_string())
}

/// 手机网页的网址：只要 https:// 加域名（可以带端口），不要后面的路径（反向代理要放在网站根目录）
pub fn check_web_url(s: &str) -> Result<String, String> {
    let s = s.trim().trim_end_matches('/');
    let host = s.strip_prefix("https://").ok_or("要写 https:// 开头的网址，比如：https://sync.example.com")?;
    if host.is_empty() || host.contains(['/', '?', '#', ' ', '"', '\'', '<', '>', '\\']) {
        return Err("只写 https:// 加域名，后面不要带路径，比如：https://sync.example.com".into());
    }
    Ok(s.to_string())
}

/// 记下手机网页的网址，空的是不绑域名。电脑每次连上时带过去，改了不用重启
pub fn set_web_url(store: &Store, url: &str) -> Result<String, String> {
    let url = if url.trim().is_empty() { String::new() } else { check_web_url(url)? };
    store.kv_set("web_url", &url).map_err(|e| e.to_string())?;
    Ok(url)
}

/// 拿某一天的备份恢复数据库（服务要先停掉）。现在的数据库改名留着（`dingmi-server.db.恢复前-<时间>`），
/// 换上备份，再换一个新的「代」：电脑和手机连上时看到代变了，从头拉、把本机的全部重新传一遍，
/// 备份之后新加的、附件（备份里没有）就都回来了。返回留着的旧数据库叫什么
pub fn restore_backup(dir: &Path, day: &str) -> Result<String, String> {
    let src = backup_file(dir, day).ok_or(format!("没有 {day} 的备份，用 backups 看看有哪些"))?;
    let db = dir.join(crate::store::DB_FILE);
    let keep = format!("{}.恢复前-{}", crate::store::DB_FILE, chrono::Local::now().format("%Y%m%d-%H%M%S"));
    for ext in ["", "-wal", "-shm"] {
        let from = dir.join(format!("{}{ext}", crate::store::DB_FILE));
        if from.exists() {
            std::fs::rename(&from, dir.join(format!("{keep}{ext}"))).map_err(|e| format!("没法把现在的数据库改名：{e}"))?;
        }
    }
    std::fs::copy(&src, &db).map_err(|e| format!("没法拷贝备份：{e}"))?;
    let store = Store::open(dir)?;
    let epoch: String = rand::random::<[u8; 16]>().iter().map(|b| format!("{b:02x}")).collect();
    store.kv_set("epoch", &epoch).map_err(|e| e.to_string())?;
    // 今天的收尾（备份）当作做过了：不然一启动就把今天的备份换成恢复后的，恢复前那份就只剩改名留着的了
    store.kv_set("last_daily", &chrono::Local::now().format("%Y-%m-%d").to_string()).map_err(|e| e.to_string())?;
    // 备份里还在、后来已经被电脑取走（文件删了）的收件箱条目：删掉，不然电脑取的时候会出错
    for (space, id) in store.inbox_ids() {
        if !inbox_path(dir, &id).exists() {
            store.del_inbox(&space, &id);
        }
    }
    Ok(keep)
}

/// 删掉一个空间：数据库里的记录、附件、收件箱、设备，和它的文件
pub fn delete_space(store: &Store, dir: &Path, space: &str) -> Result<(), String> {
    let inbox = store.delete_space(space).map_err(|e| e.to_string())?;
    for id in inbox {
        let _ = std::fs::remove_file(inbox_path(dir, &id));
    }
    let _ = std::fs::remove_dir_all(space_files(dir, space));
    Ok(())
}

/// 每天收尾时调：停用满 DELETE_DAYS 天的空间连记录、附件、收件箱一起删掉。返回删了哪些：(编号, 给人看的名字)
pub fn purge_due(store: &Store, dir: &Path) -> Vec<(String, String)> {
    purge_due_with(store, dir, |_| true)
}

/// 同上，删每一个之前先问 `may`（扩展点的 `Ext::may_purge`，比如官方版冻结了的空间不删）：false 的留着，明天再问
pub fn purge_due_with(store: &Store, dir: &Path, may: impl Fn(&str) -> bool) -> Vec<(String, String)> {
    let before =
        (chrono::Local::now() - chrono::TimeDelta::days(days(store, &DELETE_DAYS))).format("%Y-%m-%d %H:%M:%S").to_string();
    let mut done = vec![];
    let names = spaces(store);
    for space in store.disabled_before(&before).into_iter().filter(|s| may(s)) {
        let label = names.iter().find(|s| s.id == space).map(|s| s.label()).unwrap_or_else(|| short(&space));
        if delete_space(store, dir, &space).is_ok() {
            done.push((space, label));
        }
    }
    done
}

pub struct PhoneInfo {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub last_seen: Option<String>,
    pub revoked: bool,
}

/// 某个空间的电脑，作废了的也列出来
pub fn computers(store: &Store, space: &str) -> Vec<Device> {
    store.devices().into_iter().filter(|d| d.space_id == space).collect()
}

pub fn phones(store: &Store, space: &str) -> Vec<PhoneInfo> {
    store
        .all_phones(space)
        .into_iter()
        .map(|(id, name, created_at, last_seen, revoked)| PhoneInfo { id, name, created_at, last_seen, revoked })
        .collect()
}

/// 作废一台电脑（按公钥）。它正在连着的话，下一条消息就被断开
pub fn revoke_computer(store: &Store, pubkey: &str) -> Result<String, String> {
    let d = store.device(pubkey).filter(|d| !d.revoked).ok_or("没有这台电脑，或者已经作废了")?;
    store.revoke_device(pubkey);
    Ok(d.name)
}

pub fn revoke_phone(store: &Store, space: &str, id: &str) -> Result<String, String> {
    let p =
        phones(store, space).into_iter().find(|p| p.id == id && !p.revoked).ok_or("没有这部手机，或者已经作废了")?;
    store.revoke_phone(space, id);
    Ok(p.name)
}

/// 命令行的 revoke：按电脑名、手机名，或者编号开头（至少 6 位）。对上好几台时不动手，列出来让人写编号
pub fn revoke(store: &Store, who: &str) -> Result<String, String> {
    let who = who.trim();
    let hit = |name: &str, id: &str| name == who || (who.len() >= 6 && id.starts_with(who));
    let pcs: Vec<Device> = store.devices().into_iter().filter(|d| !d.revoked && hit(&d.name, &d.pubkey)).collect();
    let mut mobiles = vec![];
    for s in store.space_rows() {
        for p in phones(store, &s.0) {
            if !p.revoked && hit(&p.name, &p.id) {
                mobiles.push((s.0.clone(), p));
            }
        }
    }
    match (pcs.len(), mobiles.len()) {
        (0, 0) => Err(format!("没找到「{who}」，用 devices 看看名字和编号")),
        (1, 0) => revoke_computer(store, &pcs[0].pubkey).map(|n| format!("电脑 {n}")),
        (0, 1) => revoke_phone(store, &mobiles[0].0, &mobiles[0].1.id).map(|n| format!("手机 {n}")),
        _ => Err(format!(
            "「{who}」对上了好几台：{}，写编号前几位",
            pcs.iter()
                .map(|d| format!("电脑 {}（{}）", d.name, short(&d.pubkey)))
                .chain(mobiles.iter().map(|(_, p)| format!("手机 {}（{}）", p.name, short(&p.id))))
                .collect::<Vec<_>>()
                .join("、")
        )),
    }
}

pub struct Invite {
    /// `dingmi://…` 一整行
    pub code: String,
    pub expires_at: String,
    pub space: String,
    /// 空间已经设过同步密码：服务器生成的配对码不带空间密钥，新电脑粘贴后要输恢复码
    pub set_up: bool,
    /// 邀请码的哈希（数据库里存的就是它），拿它查用没用、作废
    pub token_hash: String,
    /// 给人认的：配对码最后一段的前 4 位和后 4 位
    pub hint: String,
}

/// 生成过的配对码现在怎样：还在 → 没用（或过期）；不在了 → 用了（或被作废）
pub fn invite_pending(store: &Store, token_hash: &str) -> bool {
    store.invite_exists(token_hash)
}

/// 作废一个还没用的配对码
pub fn revoke_invite(store: &Store, token_hash: &str) -> bool {
    store.del_invite(token_hash)
}

/// 新建一个空间（容量上限用默认的），返回编号
pub fn new_space(store: &Store) -> Result<String, String> {
    let id: String = rand::random::<[u8; 8]>().iter().map(|b| format!("{b:02x}")).collect();
    store.create_space(&id).map_err(|e| e.to_string())?;
    store.set_quota(&id, default_quota(store));
    Ok(id)
}

/// 生成配对码，24 小时内有效、只能用一次。`space` 为 None 时新建一个空间
pub fn invite(store: &Store, space: Option<&str>) -> Result<Invite, String> {
    let space = match space {
        Some(s) => s.to_string(),
        None => {
            check_inited(store)?;
            new_space(store)?
        }
    };
    invite_for(store, &space)
}

fn check_inited(store: &Store) -> Result<(), String> {
    pair_addr(store)?;
    store.kv_get("noise_public").ok_or("找不到服务器密钥，先执行 init")?;
    Ok(())
}

/// 官方版在宝塔后面时记的同步域名（kv，官方版的 `hosts --sync` 改）。开源版没有这一项
pub const HOST_SYNC_KEY: &str = "host_sync";

/// 配对码里给电脑的地址：开源版是 init 记下的 IP 和端口（电脑直连）；
/// 官方版在宝塔后面（记了 [`HOST_SYNC_KEY`]）是同步域名和 443，电脑认得出是域名，只走 `wss://域名/sync`
pub fn pair_addr(store: &Store) -> Result<(String, u16), String> {
    if let Some(host) = store.kv_get(HOST_SYNC_KEY).filter(|h| !h.is_empty()) {
        return Ok((host, 443));
    }
    let host = store.kv_get("ip").ok_or("还没有初始化，先执行 init")?;
    let port: u16 = store.kv_get("port").and_then(|p| p.parse().ok()).ok_or("还没有初始化，先执行 init")?;
    Ok((host, port))
}

/// 给已有的空间生成配对码（停用的不行）
pub fn invite_for(store: &Store, space: &str) -> Result<Invite, String> {
    let (host, port) = pair_addr(store)?;
    let server_public =
        store.kv_get("noise_public").and_then(|k| B64.decode(k).ok()).ok_or("找不到服务器密钥，先执行 init")?;
    if store.disabled_at(space).is_some() {
        return Err("这个空间停用了，先恢复再生成配对码".into());
    }
    let (token, expires_at) = store.new_invite(space, INVITE_HOURS * 60).map_err(|e| e.to_string())?;
    let set_up = store.space(space).salt.is_some();
    let hint = format!("{}…{}", &token[..4], &token[token.len() - 4..]);
    let token_hash = hash_token(&token);
    let code = PairCode { host, port, server_public, invite: token, space_key: None }.to_string();
    Ok(Invite { code, expires_at, space: space.to_string(), set_up, token_hash, hint })
}

/// 某一天的备份文件；日期格式不对（防止拼出别的路径）或者没有这一天的返回 None
pub fn backup_file(dir: &Path, day: &str) -> Option<PathBuf> {
    let ok = day.len() == 10 && day.bytes().enumerate().all(|(i, b)| if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() });
    let p = dir.join("backups").join(format!("dingmi-server-{day}.db"));
    (ok && p.is_file()).then_some(p)
}

/// 每天的备份：(日期, 大小)，新的在前
pub fn backups(dir: &Path) -> Vec<(String, u64)> {
    let Ok(rd) = std::fs::read_dir(dir.join("backups")) else {
        return vec![];
    };
    let mut list: Vec<(String, u64)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let day = name.strip_prefix("dingmi-server-")?.strip_suffix(".db")?.to_string();
            Some((day, e.metadata().ok()?.len()))
        })
        .collect();
    list.sort_by(|a, b| b.0.cmp(&a.0));
    list
}

/// 「2G」「500M」「1.5g」「off」→ 字节；off、不限、0 是不限（None）
pub fn parse_size(s: &str) -> Result<Option<u64>, String> {
    let s = s.trim().to_ascii_uppercase();
    if matches!(s.as_str(), "OFF" | "NONE" | "0" | "不限") {
        return Ok(None);
    }
    let s = s.trim_end_matches('B');
    let (num, unit) = match s.char_indices().last() {
        Some((i, c @ ('K' | 'M' | 'G' | 'T'))) => (&s[..i], c),
        _ => (s, 'B'),
    };
    let n: f64 = num.trim().parse().map_err(|_| "大小写成 2G、500M 这样，不限写 off".to_string())?;
    let mul = match unit {
        'K' => 1u64 << 10,
        'M' => 1 << 20,
        'G' => 1 << 30,
        'T' => 1 << 40,
        _ => 1,
    };
    if !n.is_finite() || n <= 0.0 {
        return Err("大小要大于 0，不限写 off".into());
    }
    Ok(Some((n * mul as f64) as u64))
}

pub fn fmt_size(n: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else if v >= 100.0 || v.fract() < 0.05 {
        format!("{v:.0} {}", U[i])
    } else {
        format!("{v:.1} {}", U[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> (Store, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("dingmi-admin-{name}-{}", rand::random::<u32>()));
        crate::init(&dir, "127.0.0.1".parse().unwrap(), 7788).unwrap();
        (Store::open(&dir).unwrap(), dir)
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("2G").unwrap(), Some(2 << 30));
        assert_eq!(parse_size(" 500m ").unwrap(), Some(500 << 20));
        assert_eq!(parse_size("1.5GB").unwrap(), Some(3 << 29));
        assert_eq!(parse_size("off").unwrap(), None);
        assert_eq!(parse_size("1024").unwrap(), Some(1024));
        for bad in ["", "G", "-1G", "abc", "NaNG"] {
            assert!(parse_size(bad).is_err(), "{bad}");
        }
        assert_eq!(fmt_size(512), "512 B");
        assert_eq!(fmt_size(2 << 30), "2 GB");
        assert_eq!(fmt_size(1536 << 10), "1.5 MB");
        assert_eq!((storage(100, None), storage(89, Some(100)), storage(90, Some(100)), storage(100, Some(100))), ("ok", "ok", "near_full", "full"));
    }

    #[test]
    fn spaces_and_rules() {
        let (s, dir) = store("rules");
        let a = invite(&s, None).unwrap();
        let b = invite(&s, None).unwrap();
        assert!(a.code.starts_with("dingmi://") && !a.set_up);
        assert_eq!(spaces(&s).len(), 2);

        // 备注名：能用来找空间，不能重名，空的就是去掉
        set_note(&s, &a.space, " 爸妈 ").unwrap();
        assert_eq!(find_space(&s, "爸妈").unwrap().id, a.space);
        assert!(set_note(&s, &b.space, "爸妈").is_err());
        assert_eq!(find_space(&s, &b.space[..6]).unwrap().label(), b.space[..8]);
        assert!(find_space(&s, "没有").is_err());
        assert!(find_space(&s, "abc").is_err(), "编号太短不认");

        // 容量上限：存着的大小加上新的超了就放不下
        s.push_item(&a.space, "r1", "1", &[0u8; 1000]).unwrap();
        s.file_put(&a.space, "f1", 3000, true).unwrap();
        s.file_put(&a.space, "f2", 500, false).unwrap();
        assert_eq!(s.usage(&a.space), (1000, 3500));
        // 没传完的放了很久：清掉，不再算占用；传完的不动
        assert!(s.stale_uploads("2000-01-01 00:00:00").is_empty());
        assert_eq!(s.stale_uploads("2999-01-01 00:00:00"), vec![(a.space.clone(), "f2".to_string())]);
        assert_eq!(s.usage(&a.space), (1000, 3000));
        assert!(fits(&s, &a.space, 1 << 40), "没设上限");
        set_quota(&s, &a.space, Some(4500)).unwrap();
        assert!(fits(&s, &a.space, 500));
        assert!(!fits(&s, &a.space, 501));
        let msg = full_msg(&s, &a.space, "或者请服务器管理员调高上限");
        assert!(msg.contains("4.4 KB") && msg.ends_with("或者请服务器管理员调高上限"));
        set_quota(&s, &a.space, None).unwrap();
        assert!(fits(&s, &a.space, 1 << 40));

        // 停用：不能生成配对码，能恢复；停用满 30 天（默认）才真删
        let day = disable(&s, &a.space).unwrap();
        assert_eq!(day.len(), 10);
        assert!(disable(&s, &a.space).is_err());
        assert!(invite(&s, Some(&a.space)).is_err());
        assert_eq!(find_space(&s, "爸妈").unwrap().delete_on.as_deref(), Some(day.as_str()));
        assert!(purge_due(&s, &dir).is_empty());
        // 天数改了，已经停用的也按新的算
        set_days(&s, &DELETE_DAYS, 1).unwrap();
        assert!(find_space(&s, "爸妈").unwrap().delete_on.unwrap() < day);
        assert!(purge_due(&s, &dir).is_empty(), "刚停用，还没满 1 天");
        assert!(set_days(&s, &DELETE_DAYS, 0).is_err());
        assert!(set_days(&s, &BACKUP_DAYS, 91).is_err());
        assert_eq!(days(&s, &BACKUP_DAYS), 14);
        set_days(&s, &DELETE_DAYS, 30).unwrap();
        restore(&s, &a.space).unwrap();
        assert!(restore(&s, &a.space).is_err());
        assert!(invite(&s, Some(&a.space)).unwrap().code.starts_with("dingmi://"));

        std::fs::create_dir_all(space_files(&dir, &a.space)).unwrap();
        std::fs::write(space_files(&dir, &a.space).join("f1"), b"x").unwrap();
        s.set_disabled(&a.space, Some("2000-01-01 00:00:00"));
        assert_eq!(purge_due(&s, &dir), vec![(a.space.clone(), "爸妈".to_string())]);
        assert!(!space_files(&dir, &a.space).exists());
        assert_eq!(s.usage(&a.space), (0, 0));
        assert_eq!(spaces(&s).len(), 1);
    }

    #[test]
    fn defaults_web_and_restore() {
        let (s, dir) = store("defaults");
        // 默认容量上限只套在以后新建的空间上
        let old = invite(&s, None).unwrap().space;
        set_default_quota(&s, Some(1 << 30)).unwrap();
        let new = invite(&s, None).unwrap().space;
        assert_eq!((s.quota(&old), s.quota(&new)), (None, Some(1 << 30)));
        assert_eq!(default_quota(&s), Some(1 << 30));
        set_default_quota(&s, None).unwrap();
        assert_eq!(default_quota(&s), None);
        // 手机网页网址
        assert_eq!(set_web_url(&s, "https://sync.example.com/").unwrap(), "https://sync.example.com");
        assert!(set_web_url(&s, "http://x.com").is_err());
        assert_eq!(set_web_url(&s, " ").unwrap(), "");
        for bad in ["http://sync.example.com", "sync.example.com", "https://", "https://a.com/m", "https://a.com?x", "https://a\"b"] {
            assert!(check_web_url(bad).is_err(), "{bad}");
        }
        // 拿备份恢复：数据回到备份那时，代换了，原来的数据库留着；文件已经没了的收件箱条目清掉
        s.kv_set("epoch", "old").unwrap();
        let sp = spaces(&s)[0].id.clone();
        s.add_inbox("gone1", &sp, "p1", 100).unwrap();
        std::fs::create_dir_all(dir.join("inbox")).unwrap();
        std::fs::write(inbox_path(&dir, "kept1"), b"x").unwrap();
        s.add_inbox("kept1", &sp, "p1", 1).unwrap();
        s.backup(&dir, "2000-01-01", 14).unwrap();
        invite(&s, None).unwrap();
        assert_eq!(spaces(&s).len(), 3);
        drop(s);
        assert!(restore_backup(&dir, "1999-01-01").is_err());
        let kept = restore_backup(&dir, "2000-01-01").unwrap();
        assert!(dir.join(&kept).exists());
        let s = Store::open(&dir).unwrap();
        assert_eq!(spaces(&s).len(), 2);
        assert_ne!(s.kv_get("epoch").as_deref(), Some("old"));
        assert_eq!(s.inbox_ids(), vec![(sp.clone(), "kept1".to_string())]);
        // 今天不再自动备份（不然今天的备份会被换成恢复后的）
        assert_eq!(s.kv_get("last_daily"), Some(chrono::Local::now().format("%Y-%m-%d").to_string()));
        // 改名留着的旧数据库：超过留的天数就在备份时删掉（留 0 天 = 马上删）
        s.backup(&dir, "2000-01-02", 14).unwrap();
        assert!(dir.join(&kept).exists());
        std::thread::sleep(std::time::Duration::from_millis(20));
        s.backup(&dir, "2000-01-03", 0).unwrap();
        assert!(!dir.join(&kept).exists());
    }

    #[test]
    fn revoke_by_name() {
        let (s, _) = store("revoke");
        let sp = invite(&s, None).unwrap().space;
        s.add_device("pubkeyAAAAAAAA", &sp, "DESKTOP-1").unwrap();
        s.add_device("pubkeyBBBBBBBB", &sp, "DESKTOP-1").unwrap();
        s.add_phone("phone0001", &sp, "iPhone", "h1").unwrap();
        // 同名的两台：不动手，让写编号
        assert!(revoke(&s, "DESKTOP-1").unwrap_err().contains("好几台"));
        assert_eq!(revoke(&s, "pubkeyB").unwrap(), "电脑 DESKTOP-1");
        assert_eq!(revoke(&s, "DESKTOP-1").unwrap(), "电脑 DESKTOP-1");
        assert_eq!(revoke(&s, "iPhone").unwrap(), "手机 iPhone");
        assert!(revoke(&s, "iPhone").is_err());
        assert_eq!(computers(&s, &sp).iter().filter(|d| d.revoked).count(), 2);
        assert!(phones(&s, &sp)[0].revoked);
        assert_eq!(spaces(&s)[0].computers, 0);
    }
}
