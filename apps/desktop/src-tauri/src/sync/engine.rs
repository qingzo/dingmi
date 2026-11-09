//! 电脑之间同步的网络部分：拉取别的电脑传来的、上传本机改过的、图片原图的上传下载。
//! 只依赖数据库连接和一条加密连接，不依赖 Tauri，可以用临时数据库和本地服务器测试。

use super::crypto::{self, DataKey};
use super::inbox::{ask, ask_any};
use super::records::{self, Limits, Payload};
use super::shrink;
use crate::{clock, db};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use dingmi_sync::{refuse, Msg, Record, INBOX_CHUNK};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

use super::transport::Conn;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn record_aad(uid: &str, ver: &str) -> Vec<u8> {
    format!("{uid}|{ver}").into_bytes()
}

fn file_aad(uid: &str) -> Vec<u8> {
    format!("file|{uid}").into_bytes()
}

/// 把服务器上比本机新的记录都拉下来合并，再下载缺的原图。返回本机有没有变化
pub async fn pull(db: &Mutex<Connection>, data_dir: &Path, conn: &mut Conn, key: &DataKey) -> Result<bool, String> {
    let mut since: u64 = {
        let c = db.lock().unwrap();
        db::kv_get(&c, "sync_seq").and_then(|s| s.parse().ok()).unwrap_or(0)
    };
    let mut changed = false;
    // 拉到的「附件已彻底删除」：不管是哪台设备删的（手机删不了服务器上的文件），都让服务器把文件删掉，
    // 不然它一直占着这个空间的容量。删过的再删一次也没关系
    let mut purged = vec![];
    loop {
        let Msg::Records { items, more } = ask(conn, &Msg::Pull { since }).await? else {
            return Err("服务器回的内容不对".into());
        };
        {
            let c = db.lock().unwrap();
            let tx = c.unchecked_transaction().map_err(err)?;
            for it in &items {
                since = since.max(it.seq);
                // 解不开的（不是这个同步密码加密的）跳过，不卡住后面的
                let payload = STANDARD
                    .decode(&it.blob)
                    .ok()
                    .and_then(|b| crypto::open_with(key, &record_aad(&it.uid, &it.ver), &b))
                    .and_then(|j| serde_json::from_slice::<Payload>(&j).ok());
                let Some(p) = payload.filter(|p| p.uid() == it.uid) else {
                    continue;
                };
                clock::observe(&it.ver);
                if let Payload::Attachment(a) = &p {
                    if a.purged {
                        purged.push(a.uid.clone());
                    }
                }
                changed |= records::apply(&tx, data_dir, &it.ver, &p).map_err(err)?;
            }
            records::fix_links(&tx).map_err(err)?;
            db::kv_set(&tx, "sync_seq", &since.to_string()).map_err(err)?;
            tx.commit().map_err(err)?;
        }
        if !more {
            break;
        }
    }
    for uid in purged {
        ask(conn, &Msg::FileDel { uid }).await?;
    }
    let missing = {
        let c = db.lock().unwrap();
        records::missing_files(&c).map_err(err)?
    };
    for (uid, file) in missing {
        // 别的电脑可能还没传完原图，下次再试
        let Ok(bytes) = download_file(conn, key, &uid).await else {
            continue;
        };
        write_attachment(data_dir, &file, &bytes)?;
        records::file_ok(&db.lock().unwrap(), &uid).map_err(err)?;
        changed = true;
    }
    Ok(changed)
}

/// 上传本机改过的记录（图片先传原图）。返回传了几条
#[cfg(test)]
pub async fn push(db: &Mutex<Connection>, data_dir: &Path, conn: &mut Conn, key: &DataKey) -> Result<usize, String> {
    push_with(db, data_dir, conn, key, true, &Limits::default()).await.map(|p| p.count)
}

/// 上传了多少
#[derive(Debug, Default, PartialEq)]
pub struct Pushed {
    /// 传了几条记录
    pub count: usize,
    /// 空间满了（叮咪同步，需求-1.0 第 17.3 节）：没传上去、先只在这台电脑上的附件有几个；没满是 0
    pub held: u32,
    /// 超过套餐单个附件的大小（或者图片压不下来）、叮咪同步不传、只在这台电脑上的附件有几个（32.1 第 5 条）
    pub only_here: u32,
}

/// 传一个附件的原文件之前看看套餐的限额：传什么（图片超了压一份小的），还是不传（太大、只留在这台电脑上）
enum Send {
    Bytes(Vec<u8>),
    TooBig,
}

fn prepare(mime: &str, bytes: Vec<u8>, limits: &Limits) -> Send {
    if limits.fits(mime, bytes.len() as u64) {
        return Send::Bytes(bytes);
    }
    // 图片：只压传上去的那一份，本机笔记里的还是原图（理由见 shrink.rs）
    match limits.max_image.filter(|_| mime.starts_with("image/")) {
        Some(max) => shrink::fit(&bytes, mime, max).map_or(Send::TooBig, Send::Bytes),
        None => Send::TooBig,
    }
}

/// 上传本机改过的记录（图片先传原图）。`files` 是 false（已经知道空间满了）时不传附件；
/// 传的时候服务器说满了也一样：附件先留在这台电脑上（还是「没传」），文字照常推，等空间腾出来再传。
/// `limits` 是叮咪同步套餐的单个图片 / 附件大小（32.1 第 5 条）：图片超了压一份小的传；文件超了（图片压不下来）不传原文件，
/// 记下「只在这台电脑上」、记录照常推（别的设备看得到它在哪）；后来放得下了再传
pub async fn push_with(
    db: &Mutex<Connection>,
    data_dir: &Path,
    conn: &mut Conn,
    key: &DataKey,
    mut files: bool,
    limits: &Limits,
) -> Result<Pushed, String> {
    let mut total = 0;
    let mut held_any = !files;
    // 以前太大没传的，现在放得下了（换了大套餐）：清掉记号，这一趟跟着传
    records::unmark_fitting(&db.lock().unwrap(), limits).map_err(err)?;
    loop {
        let batch = {
            let c = db.lock().unwrap();
            if files { records::dirty(&c, 100) } else { records::dirty_without_files(&c, 100) }.map_err(err)?
        };
        if batch.is_empty() {
            break;
        }
        // 这一批里没传上原文件的附件：它的记录也先不推（不然别的电脑收到记录却下载不到）
        let mut held = std::collections::HashSet::new();
        for d in &batch {
            if let Payload::Attachment(a) = &d.payload {
                if a.purged {
                    ask(conn, &Msg::FileDel { uid: a.uid.clone() }).await?;
                } else if a.only_on.is_some() {
                    // 原文件不传（太大，只在那台设备上）：只推记录
                } else if !files {
                    held.insert(a.uid.clone());
                } else if let Ok(bytes) = std::fs::read(db::attachment_dir(data_dir).join(&a.file)) {
                    let up = match prepare(&a.mime, bytes, limits) {
                        Send::Bytes(b) => upload_file(conn, key, &a.uid, &b).await?,
                        Send::TooBig => Upload::TooBig,
                    };
                    match up {
                        Upload::Done => {}
                        Upload::Full => {
                            files = false;
                            held_any = true;
                            held.insert(a.uid.clone());
                        }
                        // 太大：记下只在这台电脑上（改了版本），这一批先不推它，下一轮带着记号推记录
                        Upload::TooBig => {
                            records::mark_only_here(&db.lock().unwrap(), &a.uid, &limits.me).map_err(err)?;
                            held.insert(a.uid.clone());
                        }
                    }
                }
            }
        }
        let batch: Vec<_> = batch.into_iter().filter(|d| !held.contains(&d.uid)).collect();
        if batch.is_empty() {
            // 全是传不上去的附件：换成不带附件的接着取
            continue;
        }
        let items = batch
            .iter()
            .map(|d| {
                let json = serde_json::to_vec(&d.payload).map_err(err)?;
                let blob = crypto::seal_with(key, &record_aad(&d.uid, &d.ver), &json);
                Ok(Record { uid: d.uid.clone(), ver: d.ver.clone(), seq: 0, blob: STANDARD.encode(blob) })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let Msg::Acks { acks } = ask(conn, &Msg::Push { items }).await? else {
            return Err("服务器回的内容不对".into());
        };
        let mut ok = 0;
        {
            let c = db.lock().unwrap();
            for (d, a) in batch.iter().zip(&acks) {
                if a.ok && a.uid == d.uid {
                    records::clear_dirty(&c, d.table, &d.uid, &d.ver).map_err(err)?;
                    ok += 1;
                }
            }
        }
        total += ok;
        // 有被拒的（服务器上已经有更新的版本）就先停，等拉取到那个更新的版本时自然解决，免得原地打转
        if ok < batch.len() {
            break;
        }
    }
    let c = db.lock().unwrap();
    let held = if held_any { records::held_files(&c).map_err(err)? } else { 0 };
    let only_here = records::only_here(&c, &limits.me).map_err(err)?;
    Ok(Pushed { count: total, held, only_here })
}

/// 传附件的结果
enum Upload {
    Done,
    /// 空间满了，服务器不收（叮咪同步回 Refused full；旧服务器回「空间满了…」）
    Full,
    /// 超过套餐单个附件的大小，服务器不收（Refused too_big；电脑本来就先拦了，这是兜底）
    TooBig,
}

/// 传一个附件的原文件
async fn upload_file(conn: &mut Conn, key: &DataKey, uid: &str, bytes: &[u8]) -> Result<Upload, String> {
    let blob = crypto::seal_with(key, &file_aad(uid), bytes);
    let count = blob.len().div_ceil(INBOX_CHUNK);
    for (i, chunk) in blob.chunks(INBOX_CHUNK).enumerate() {
        let put = Msg::FilePut {
            uid: uid.into(),
            offset: (i * INBOX_CHUNK) as u64,
            data: STANDARD.encode(chunk),
            done: i + 1 == count,
        };
        match ask_any(conn, &put).await? {
            Msg::Refused { code, .. } if code == refuse::FULL => return Ok(Upload::Full),
            Msg::Refused { code, .. } if code == refuse::TOO_BIG => return Ok(Upload::TooBig),
            Msg::Error { msg } if msg.starts_with("空间满了") => return Ok(Upload::Full),
            Msg::Refused { msg, .. } | Msg::Error { msg } => return Err(msg),
            _ => {}
        }
    }
    Ok(Upload::Done)
}

/// 点开一个还没下载的文件附件：从服务器下载、解密，存到 attachments/ 里
pub async fn fetch_file(db: &Mutex<Connection>, data_dir: &Path, conn: &mut Conn, key: &DataKey, uid: &str, file: &str) -> Result<(), String> {
    let bytes = download_file(conn, key, uid).await.map_err(|e| {
        if e.contains("还没传完") {
            "这个文件还没传到服务器，等传上去的那台设备同步完再试".to_string()
        } else {
            e
        }
    })?;
    write_attachment(data_dir, file, &bytes)?;
    records::file_ok(&db.lock().unwrap(), uid).map_err(err)
}

/// 别的设备传来的附件写进 attachments/：文件名要合规矩、正好在附件文件夹里，写好标上「来自网络」
pub(super) fn write_attachment(data_dir: &Path, file: &str, bytes: &[u8]) -> Result<(), String> {
    let dir = db::attachment_dir(data_dir);
    let path = db::att_path(&dir, file).ok_or("附件的文件名不对")?;
    std::fs::create_dir_all(&dir).map_err(err)?;
    std::fs::write(&path, bytes).map_err(err)?;
    db::mark_from_web(&path);
    Ok(())
}

/// 一个附件加密后最大多大：附件本身最大 50 MB，加上加密多出来的（留宽一点）。服务器传来的超过这个就不要了
pub(super) const MAX_SEALED_FILE: usize = db::MAX_ATTACHMENT + 64 * 1024;

async fn download_file(conn: &mut Conn, key: &DataKey, uid: &str) -> Result<Vec<u8>, String> {
    download_capped(conn, key, uid, MAX_SEALED_FILE).await
}

async fn download_capped(conn: &mut Conn, key: &DataKey, uid: &str, max: usize) -> Result<Vec<u8>, String> {
    if !db::valid_att_uid(uid) {
        return Err("附件的编号不对".into());
    }
    let mut blob = Vec::new();
    loop {
        let get = Msg::FileGet { uid: uid.into(), offset: blob.len() as u64 };
        let Msg::FileChunk { data, done, .. } = ask(conn, &get).await? else {
            return Err("服务器回的内容不对".into());
        };
        let chunk = STANDARD.decode(data).map_err(err)?;
        // 一块都没有又说没完（服务器原地打转），或者越传越多超过了上限：不要了
        if chunk.is_empty() && !done {
            return Err("服务器回的内容不对".into());
        }
        if blob.len() + chunk.len() > max {
            return Err("服务器传来的附件太大了，没有下载".into());
        }
        blob.extend(chunk);
        if done {
            break;
        }
    }
    crypto::open_with(key, &file_aad(uid), &blob).ok_or_else(|| "附件解不开".to_string())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::db::{Note, Reminder, Repeat};
    use dingmi_sync::{Hello, PairCode, VERSION};
    use tokio::net::TcpStream;

    pub(crate) async fn dial(code: &PairCode, dev: &[u8], invite: Option<String>) -> Conn {
        let tcp: super::super::transport::Stream = Box::new(TcpStream::connect((code.host.as_str(), code.port)).await.unwrap());
        let hello = Hello { version: VERSION, name: "测试".into(), invite, ..Default::default() };
        dingmi_sync::connect(tcp, dev, &code.server_public, &hello).await.unwrap().0
    }

    /// 在本机起一个 dingmi-server（和服务器自己的测试一样），回 (服务器, 端口, 服务器公钥, 数据目录)
    pub(crate) async fn local_server(name: &str) -> (std::sync::Arc<dingmi_server::net::Server>, u16, Vec<u8>, std::path::PathBuf) {
        local_server_with(name, std::sync::Arc::new(dingmi_server::ext::SelfHosted)).await
    }

    /// 测试用：像叮咪同步那样回话的服务器。开源的 dingmi-server 不带叮咪同步（需求-1.0 第 32.1 节第 12 条），
    /// 这里用它的扩展点装一个最简单的：几个兑换码、套餐「标准」、单个附件的限额、客服核对码。
    /// 只为看电脑端怎么处理这些回话，规则（换档、到期、公测……）不在这里，那些在官方版自己的测试里
    #[derive(Default)]
    pub(crate) struct FakeOfficial {
        /// 单个图片、单个附件最大多大
        pub limits: Mutex<(Option<u64>, Option<u64>)>,
        /// 兑换码 → 用过了没有
        pub codes: Mutex<std::collections::HashMap<String, bool>>,
        /// 空间 → 到期日
        until: Mutex<std::collections::HashMap<String, chrono::NaiveDate>>,
        /// 客服核对码 → 申请的那台电脑的名字
        pub support: Mutex<std::collections::HashMap<String, String>>,
        /// 兑换码 → 用它开通的空间（兑换码加入用）
        pub spaces: Mutex<std::collections::HashMap<String, String>>,
    }

    impl FakeOfficial {
        /// 用掉一个兑换码
        fn take(&self, code: &str) -> Result<(), dingmi_sync::Refusal> {
            let say = |c: &str, m: &str| dingmi_sync::Refusal { code: c.into(), msg: m.into() };
            match self.codes.lock().unwrap().get_mut(code) {
                Some(used) if *used => Err(say(dingmi_sync::refuse::REDEEM_USED, "这个兑换码已经用过了")),
                Some(used) => {
                    *used = true;
                    Ok(())
                }
                None => Err(say(dingmi_sync::refuse::REDEEM_BAD, "这个兑换码不存在，看看是不是输错了")),
            }
        }
    }

    impl dingmi_server::ext::Ext for FakeOfficial {
        /// 和叮咪同步一样收盒子 3
        fn joins(&self) -> bool {
            true
        }

        fn open_space(&self, s: &dingmi_server::net::Server, hello: &Hello, _device: &str, _ip: std::net::IpAddr) -> dingmi_server::ext::Opening {
            use dingmi_server::ext::Opening;
            let say = |c: &str, m: &str| Opening::Refused(dingmi_sync::Refusal { code: c.into(), msg: m.into() });
            // 兑换码加入：码 → 空间；核对值的哈希对上了才回空间（和叮咪同步一样，只是不锁）
            let lock_of = |code: &str| self.spaces.lock().unwrap().get(code).cloned().map(|sp| (s.store.join_lock(&sp), sp));
            if let Some(code) = &hello.join_ask {
                return match lock_of(code) {
                    Some((Some((salt, _)), _)) => Opening::JoinSalt(salt),
                    Some((None, _)) => say(dingmi_sync::refuse::JOIN_OFF, "这个空间没开「兑换码加入」"),
                    None => say(dingmi_sync::refuse::REDEEM_BAD, "这个兑换码不存在，看看是不是输错了"),
                };
            }
            if let Some(j) = &hello.join {
                return match lock_of(&j.code) {
                    Some((Some((_, hash)), sp)) if hash == dingmi_server::net::hash_token(&j.proof) => Opening::Space(sp),
                    Some((Some(_), _)) => say(dingmi_sync::refuse::JOIN_BAD, "同步密码不对，还能再试 4 次"),
                    _ => say(dingmi_sync::refuse::JOIN_OFF, "这个空间没开「兑换码加入」"),
                };
            }
            let Some(code) = &hello.redeem else {
                return Opening::Unknown;
            };
            if let Err(r) = self.take(code) {
                if r.code == dingmi_sync::refuse::REDEEM_USED && lock_of(code).is_some_and(|(l, _)| l.is_some()) {
                    return say(dingmi_sync::refuse::REDEEM_JOINABLE, "这个兑换码已经开通过叮咪同步了");
                }
                return Opening::Refused(r);
            }
            let space = dingmi_server::admin::new_space(&s.store).unwrap();
            self.until.lock().unwrap().insert(space.clone(), chrono::Local::now().date_naive() + chrono::TimeDelta::days(365));
            self.spaces.lock().unwrap().insert(code.clone(), space.clone());
            Opening::Space(space)
        }

        fn account(&self, _s: &dingmi_server::net::Server, space: &str, base: dingmi_sync::Account) -> dingmi_sync::Account {
            let (max_image, max_file) = *self.limits.lock().unwrap();
            let until = self.until.lock().unwrap().get(space).map(|d| d.format("%Y-%m-%d").to_string());
            let short: String = space.chars().take(5).collect();
            dingmi_sync::Account {
                space: format!("S-{}", short.to_ascii_uppercase()),
                official: true,
                plan: "标准".into(),
                expires_at: until.clone(),
                until,
                max_image,
                max_file,
                ..base
            }
        }

        fn handle(&self, s: &dingmi_server::net::Server, who: &dingmi_server::ext::Who, msg: Msg) -> Result<Msg, dingmi_sync::Refusal> {
            match msg {
                Msg::Renew { code, .. } => {
                    self.take(&code)?;
                    if let Some(d) = self.until.lock().unwrap().get_mut(who.space) {
                        *d += chrono::TimeDelta::days(30);
                    }
                    Ok(Msg::AccountInfo { account: s.account(who.space) })
                }
                Msg::SupportNew => {
                    let code = format!("{:06}", uuid::Uuid::new_v4().as_u128() % 1_000_000);
                    let name = s.store.device(who.device).map(|d| d.name).unwrap_or_default();
                    self.support.lock().unwrap().insert(code.clone(), name);
                    Ok(Msg::Support { code, expires_at: "2999-01-01 00:00:00".into() })
                }
                _ => Err(dingmi_sync::Refusal { code: dingmi_sync::refuse::REDEEM_OFF.into(), msg: "这台服务器不支持".into() }),
            }
        }

        fn file_limits(&self, _s: &dingmi_server::net::Server, _space: &str) -> dingmi_server::ext::FileLimits {
            let (max_image, max_file) = *self.limits.lock().unwrap();
            // 和叮咪同步一样：服务器分不清图片和附件，按两个里大的那个多放 64 KB；有一个不限就不拦
            let cap = match (max_image, max_file) {
                (Some(i), Some(f)) => Some(i.max(f) + (64 << 10)),
                _ => None,
            };
            dingmi_server::ext::FileLimits { max_image, max_file, cap, too_big: "这个附件太大了".into() }
        }
    }

    /// 同上，带着扩展点（比如 FakeOfficial）
    pub(crate) async fn local_server_with(
        name: &str,
        ext: std::sync::Arc<dyn dingmi_server::ext::Ext>,
    ) -> (std::sync::Arc<dingmi_server::net::Server>, u16, Vec<u8>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("dingmi-desk-srv-{name}-{}", uuid::Uuid::new_v4().simple()));
        dingmi_server::init(&dir, "127.0.0.1".parse().unwrap(), 7788).unwrap();
        let server = dingmi_server::net::Server::load_with(&dir, ext).unwrap();
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(server.clone().serve(l));
        let spub = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(server.store.kv_get("noise_public").unwrap()).unwrap();
        (server, port, spub, dir)
    }

    /// 攻击：服务器一直往下传附件（修之前 download_file 不限总长度，能把内存撑爆）、一块都不给又说没完、
    /// 给个带 ..\ 的编号。现在超过上限就停，编号不对的不去要
    #[tokio::test]
    async fn evil_server_file_downloads() {
        use super::super::inbox::tests::evil_server;
        let key = [4u8; 32];
        let big = STANDARD.encode(vec![0u8; INBOX_CHUNK]);
        let (mut conn, seen) = evil_server(move |m| match m {
            Msg::FileGet { uid, offset } => Msg::FileChunk { uid: uid.clone(), offset: *offset, data: big.clone(), done: false },
            _ => Msg::Ok,
        })
        .await;
        let e = download_capped(&mut conn, &key, "019a0000-0000-7000-8000-000000000001", 4 * INBOX_CHUNK).await.unwrap_err();
        assert!(e.contains("太大"), "{e}");
        // 到上限就停了，没有一直要下去
        let asked = seen.lock().unwrap().len();
        assert_eq!(asked, 5, "到上限就停了，没有一直要下去");
        let (mut conn, _) = evil_server(|m| match m {
            Msg::FileGet { uid, offset } => Msg::FileChunk { uid: uid.clone(), offset: *offset, data: String::new(), done: false },
            _ => Msg::Ok,
        })
        .await;
        assert!(download_file(&mut conn, &key, "u1").await.is_err());
        let (mut conn, seen) = evil_server(|_| Msg::Ok).await;
        assert!(download_file(&mut conn, &key, r"..\..\x").await.is_err());
        assert!(seen.lock().unwrap().is_empty());
        // 文件名不对的附件不往盘上写
        let dir = std::env::temp_dir().join(format!("dingmi-wa-{}", uuid::Uuid::new_v4().simple()));
        assert!(write_attachment(&dir, r"..\escaped.exe", b"MZ").is_err());
        assert!(!dir.join("escaped.exe").exists());
        write_attachment(&dir, "u1.png", b"png").unwrap();
        assert!(db::from_web(&db::attachment_dir(&dir).join("u1.png")) || !cfg!(windows));
    }

    /// 需求-1.0 第 17.3 节：空间满了，附件传不上去就先留在这台电脑上，文字照常推；腾出地方后附件接着传
    #[tokio::test]
    async fn full_space_holds_files_pushes_text() {
        let (server, port, spub, _) = local_server("full").await;
        let inv = dingmi_server::admin::invite(&server.store, None).unwrap();
        let code = PairCode { host: "127.0.0.1".into(), port, server_public: spub, invite: String::new(), space_key: None };
        let tok: PairCode = inv.code.parse().unwrap();
        let dir = std::env::temp_dir().join(format!("dingmi-full-{}", uuid::Uuid::new_v4().simple()));
        let a = Mutex::new(db::open(&dir).unwrap());
        let (dev, _) = dingmi_sync::generate_keypair().unwrap();
        let mut conn = dial(&code, &dev, Some(tok.invite)).await;
        let key = [3u8; 32];
        {
            let c = a.lock().unwrap();
            db::upsert_reminder(&c, &Reminder { title: "交报销".into(), due_at: "2099-01-01 09:00".into(), ..Default::default() }).unwrap();
            let nid = db::upsert_note(&c, &Note { title: "发票".into(), ..Default::default() }).unwrap();
            let uid = db::get_note(&c, nid).unwrap().unwrap().uid;
            let f = db::save_file(&c, &dir, "image/png", "", &[7u8; 300_000], Some(&uid), None).unwrap();
            let mut n = db::get_note(&c, nid).unwrap().unwrap();
            n.body = format!("![](att:{f})");
            db::upsert_note(&c, &n).unwrap();
        }
        // 容量 1000 字节：图片放不下，提醒和笔记照样推上去，图片留在这台电脑上
        dingmi_server::admin::set_quota(&server.store, &inv.space, Some(1000)).unwrap();
        let p = push_with(&a, &dir, &mut conn, &key, true, &Limits::default()).await.unwrap();
        assert_eq!(p, Pushed { count: 2, held: 1, only_here: 0 });
        assert_eq!(server.store.usage(&inv.space).1, 0, "服务器上没有附件");
        assert!(records::has_dirty(&a.lock().unwrap()), "图片还是「没传」");
        // 已经知道满了：不再试附件，也不出错
        assert_eq!(push_with(&a, &dir, &mut conn, &key, false, &Limits::default()).await.unwrap(), Pushed { count: 0, held: 1, only_here: 0 });
        // 腾出地方（调高上限）：图片接着传上去
        dingmi_server::admin::set_quota(&server.store, &inv.space, None).unwrap();
        assert_eq!(push_with(&a, &dir, &mut conn, &key, true, &Limits::default()).await.unwrap(), Pushed { count: 1, held: 0, only_here: 0 });
        assert!(!records::has_dirty(&a.lock().unwrap()));
        assert!(server.store.usage(&inv.space).1 > 300_000);
    }

    /// 需求-1.0 第 32.1 节第 5 条：套餐限单个图片 / 附件多大。超了的图片压一份小的传（本机还是原图），
    /// 压不下来的图片、超了的文件不传、只留在这台电脑上，记录照常推（别的电脑知道在哪、不去下载）；放得下了再传
    #[tokio::test]
    async fn big_files_stay_here_images_shrink() {
        use image::{DynamicImage, Rgb, RgbImage};
        let fake = std::sync::Arc::new(FakeOfficial::default());
        let (server, port, spub, _) = local_server_with("limits", fake.clone()).await;
        let s = &server.store;
        let inv = dingmi_server::admin::invite(s, None).unwrap();
        // 服务器那边也按套餐拦（单个图片 64 KB、附件 100 KB）
        *fake.limits.lock().unwrap() = (Some(64 << 10), Some(100 << 10));
        let code = PairCode { host: "127.0.0.1".into(), port, server_public: spub, invite: String::new(), space_key: None };
        let tok: PairCode = inv.code.parse().unwrap();
        let tok_b: PairCode = dingmi_server::admin::invite(s, Some(&inv.space)).unwrap().code.parse().unwrap();
        let (da, db_) = (
            std::env::temp_dir().join(format!("dingmi-lim-a-{}", uuid::Uuid::new_v4().simple())),
            std::env::temp_dir().join(format!("dingmi-lim-b-{}", uuid::Uuid::new_v4().simple())),
        );
        let (a, b) = (Mutex::new(db::open(&da).unwrap()), Mutex::new(db::open(&db_).unwrap()));
        let (ka, _) = dingmi_sync::generate_keypair().unwrap();
        let (kb, _) = dingmi_sync::generate_keypair().unwrap();
        let mut ca = dial(&code, &ka, Some(tok.invite)).await;
        let mut cb = dial(&code, &kb, Some(tok_b.invite)).await;
        let key = [5u8; 32];

        // 一张 1200x900 的照片（JPEG，最高质量，超过 64 KB）、一个 200 KB 的文件、一张 100 KB 的动图（不压）
        let mut seed = 7u32;
        let img = RgbImage::from_fn(1200, 900, |x, y| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            Rgb([(x / 5) as u8, (y / 4) as u8, ((seed >> 24) as u8) / 8])
        });
        let mut jpeg = Vec::new();
        DynamicImage::ImageRgb8(img)
            .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 100))
            .unwrap();
        assert!(jpeg.len() > 64 << 10, "{}", jpeg.len());
        let (photo, pdf, gif) = {
            let c = a.lock().unwrap();
            (
                db::save_file(&c, &da, "image/jpeg", "", &jpeg, None, None).unwrap(),
                db::save_file(&c, &da, "application/pdf", "合同.pdf", &[3u8; 200 << 10], None, None).unwrap(),
                db::save_file(&c, &da, "image/gif", "", &[4u8; 100 << 10], None, None).unwrap(),
            )
        };
        let me = db::device_id(&a.lock().unwrap()).unwrap();
        let limits = Limits { max_image: Some(64 << 10), max_file: Some(100 << 10), me: me.clone() };
        let p = push_with(&a, &da, &mut ca, &key, true, &limits).await.unwrap();
        assert_eq!(p, Pushed { count: 3, held: 0, only_here: 2 });
        // 服务器上只有压过的那张照片；本机还是原图
        let on_server = server.store.usage(&inv.space).1;
        assert!(on_server > 0 && on_server <= (64 << 10) + 100, "{on_server}");
        assert_eq!(std::fs::read(db::attachment_dir(&da).join(&photo)).unwrap(), jpeg);
        assert!(!records::has_dirty(&a.lock().unwrap()));
        // 再推一次：没有要传的了，还是 2 个只在这台电脑上
        assert_eq!(push_with(&a, &da, &mut ca, &key, true, &limits).await.unwrap(), Pushed { count: 0, held: 0, only_here: 2 });

        // 另一台：三条记录都到了，照片是压过的那份（还看得出是 JPEG）；文件、动图知道只在 A 上，不去下载
        pull(&b, &db_, &mut cb, &key).await.unwrap();
        {
            let c = b.lock().unwrap();
            let got = std::fs::read(db::attachment_dir(&db_).join(&photo)).unwrap();
            assert!(got.len() < jpeg.len() && got.starts_with(&[0xff, 0xd8]));
            let info = |f: &str| db::get_attachment(&c, f).unwrap().unwrap();
            assert_eq!((info(&pdf).local, info(&pdf).only_on.as_deref()), (false, Some(me.as_str())));
            assert_eq!(info(&gif).only_on.as_deref(), Some(me.as_str()));
            assert!(records::missing_files(&c).unwrap().is_empty(), "只在别的电脑上的不去下载");
        }

        // 放得下了（换了大套餐 / 不限）：A 清掉记号、把文件传上去；B 再拉，记号没了、能下载
        *fake.limits.lock().unwrap() = (None, None);
        let open = Limits { me: me.clone(), ..Default::default() };
        assert_eq!(push_with(&a, &da, &mut ca, &key, true, &open).await.unwrap(), Pushed { count: 2, held: 0, only_here: 0 });
        pull(&b, &db_, &mut cb, &key).await.unwrap();
        let info = db::get_attachment(&b.lock().unwrap(), &pdf).unwrap().unwrap();
        assert_eq!(info.only_on, None);
        fetch_file(&b, &db_, &mut cb, &key, &info.uid, &pdf).await.unwrap();
        assert_eq!(std::fs::read(db::attachment_dir(&db_).join(&pdf)).unwrap(), vec![3u8; 200 << 10]);
        assert!(std::fs::read(db::attachment_dir(&db_).join(&gif)).is_ok(), "动图也下载了");
    }

    /// 两台"电脑"（两个临时数据库）通过一个真的 dingmi-server 互相同步，平时跳过。
    /// 第二台用第一台生成的配对码加入：DINGMI_TEST_PAIR=<配对码> cargo test --lib -- --ignored two_devices_through_server
    #[tokio::test]
    #[ignore]
    async fn two_devices_through_server() {
        let code1: PairCode = std::env::var("DINGMI_TEST_PAIR").unwrap().parse().unwrap();
        let key = [9u8; 32];
        let dirs: Vec<_> = (0..2)
            .map(|i| std::env::temp_dir().join(format!("dingmi-e2e-{i}-{}", uuid::Uuid::new_v4().simple())))
            .collect();
        let a = Mutex::new(db::open(&dirs[0]).unwrap());
        let b = Mutex::new(db::open(&dirs[1]).unwrap());
        let (ka, _) = dingmi_sync::generate_keypair().unwrap();
        let (kb, _) = dingmi_sync::generate_keypair().unwrap();
        let mut ca = dial(&code1, &ka, Some(code1.invite.clone())).await;
        ca.send(&Msg::InviteNew).await.unwrap();
        let Msg::Invited { token, .. } = ca.recv().await.unwrap() else { panic!("没拿到配对码") };
        let mut cb = dial(&code1, &kb, Some(token)).await;

        // A 记一条提醒、一条带图片的笔记
        {
            let c = a.lock().unwrap();
            db::upsert_reminder(&c, &Reminder { title: "交报销".into(), due_at: "2099-01-01 09:00".into(), ..Default::default() }).unwrap();
            let nid = db::upsert_note(&c, &Note { title: "发票".into(), ..Default::default() }).unwrap();
            let uid = db::get_note(&c, nid).unwrap().unwrap().uid;
            let f = db::save_file(&c, &dirs[0], "image/png", "", &[7u8; 300_000], Some(&uid), None).unwrap();
            let mut n = db::get_note(&c, nid).unwrap().unwrap();
            n.body = format!("![](att:{f})");
            db::upsert_note(&c, &n).unwrap();
        }
        assert_eq!(push(&a, &dirs[0], &mut ca, &key).await.unwrap(), 3);
        assert_eq!(push(&a, &dirs[0], &mut ca, &key).await.unwrap(), 0);

        // B 拉下来：提醒、笔记、原图都到了
        assert!(pull(&b, &dirs[1], &mut cb, &key).await.unwrap());
        {
            let c = b.lock().unwrap();
            assert_eq!(db::list_reminders(&c).unwrap()[0].title, "交报销");
            let n = &db::list_notes(&c).unwrap()[0];
            let f = n.body.trim_start_matches("![](att:").trim_end_matches(')');
            assert_eq!(std::fs::read(db::attachment_dir(&dirs[1]).join(f)).unwrap(), vec![7u8; 300_000]);
            assert!(!records::has_dirty(&c));
        }
        // 再拉一次没有新的
        assert!(!pull(&b, &dirs[1], &mut cb, &key).await.unwrap());

        // B 完成提醒，A 拉到
        {
            let c = b.lock().unwrap();
            let mut r = db::list_reminders(&c).unwrap()[0].clone();
            r.done = true;
            db::upsert_reminder(&c, &r).unwrap();
        }
        assert_eq!(push(&b, &dirs[1], &mut cb, &key).await.unwrap(), 1);
        assert!(pull(&a, &dirs[0], &mut ca, &key).await.unwrap());
        assert!(db::list_reminders(&a.lock().unwrap()).unwrap()[0].done);

        // 换一把钥匙就什么都解不开
        let c = Mutex::new(db::open(&std::env::temp_dir().join(format!("dingmi-e2e-x-{}", uuid::Uuid::new_v4().simple()))).unwrap());
        assert!(!pull(&c, &dirs[1], &mut cb, &[1u8; 32]).await.unwrap());
        assert!(db::list_reminders(&c.lock().unwrap()).unwrap().is_empty());
    }

    /// 手机版的端到端测试，第一步：在一个真的 dingmi-server 上建空间（同步密码 password1）、加一部手机、
    /// 用电脑端的代码传一批数据上去，打印手机网页的网址。之后用浏览器打开网址测手机版，再跑 phone_e2e_check。
    /// DINGMI_TEST_PAIR=<dingmi-server invite --new-space 的配对码> cargo test --lib -- --ignored phone_e2e_seed --nocapture
    #[tokio::test]
    #[ignore]
    async fn phone_e2e_seed() {
        use chrono::{Duration, Local};
        let code: PairCode = std::env::var("DINGMI_TEST_PAIR").unwrap().parse().unwrap();
        let dir = std::env::temp_dir().join(format!("dingmi-phone-e2e-{}", uuid::Uuid::new_v4().simple()));
        let a = Mutex::new(db::open(&dir).unwrap());
        let (dev, _) = dingmi_sync::generate_keypair().unwrap();
        let mut conn = dial(&code, &dev, Some(code.invite.clone())).await;
        let ns = crypto::new_space("password1", true).unwrap();
        let (key, space_key) = (ns.master, ns.space_key.unwrap());
        let recovery = crypto::recovery_text(&ns.recovery);
        let setup = Msg::SetupSpace { salt: ns.salt, box1: ns.box1, box2: ns.box2, recovery: crypto::recovery_token(&ns.recovery), join: None };
        assert_eq!(ask(&mut conn, &setup).await.unwrap(), Msg::Ok);
        let Msg::PhoneAdded { token, .. } = ask(&mut conn, &Msg::PhoneAdd { name: "测试 iPhone".into() }).await.unwrap() else {
            panic!("没加上手机")
        };
        let fmt = |d: chrono::DateTime<Local>| d.format(db::FMT).to_string();
        let now = Local::now();
        {
            let c = a.lock().unwrap();
            let rem = |title: &str, detail: &str, tag: Option<&str>, due: String, repeat: Repeat| Reminder {
                title: title.into(),
                detail: detail.into(),
                tag: tag.map(Into::into),
                due_at: due,
                repeat,
                ..Default::default()
            };
            let none = Repeat::default();
            let tomorrow9 = (now + Duration::days(1)).format("%Y-%m-%d 09:00").to_string();
            db::upsert_reminder(&c, &rem("取干洗的大衣", "店 18:00 关门", Some("life"), fmt(now - Duration::minutes(40)), none.clone())).unwrap();
            db::upsert_reminder(&c, &rem("给妈妈打个电话", "问问体检结果", Some("life"), fmt(now + Duration::hours(2)), none.clone())).unwrap();
            let weekly = Repeat { kind: "weekly".into(), days: vec![2, 4, 6], ..Default::default() };
            db::upsert_reminder(&c, &rem("跑步 5 公里", "", Some("like"), fmt(now + Duration::hours(3)), weekly)).unwrap();
            db::upsert_reminder(&c, &rem("交周报", "", Some("work"), tomorrow9, none.clone())).unwrap();
            let nid = db::upsert_note(
                &c,
                &Note {
                    title: "年底前跑完 10 公里".into(),
                    body: "> 写给 12 月的自己：每周多跑 500 米就够了。\n## 计划\n- [x] 买新跑鞋\n- [ ] 报名元旦线上跑\n- 每周二、四、六晚上跑".into(),
                    tag: Some("like".into()),
                    ..Default::default()
                },
            )
            .unwrap();
            let run_note = db::get_note(&c, nid).unwrap().unwrap();
            db::upsert_reminder(
                &c,
                &Reminder {
                    note_uid: Some(run_note.uid.clone()),
                    todo: Some("报名元旦线上跑".into()),
                    ..rem("报名元旦线上跑", "", Some("like"), fmt(now + Duration::hours(5)), none.clone())
                },
            )
            .unwrap();
            let nid = db::upsert_note(&c, &Note { title: "滇小馆的汽锅鸡很好吃".into(), tag: Some("like".into()), ..Default::default() }).unwrap();
            let uid = db::get_note(&c, nid).unwrap().unwrap().uid;
            // 一张 2x2 的橙色 PNG
            let png = base64::engine::general_purpose::STANDARD
                .decode("iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAFklEQVR4nGP4n8XwP4uBgSGL4X8WAwAoNgRfp2VNXwAAAABJRU5ErkJggg==")
                .unwrap();
            let f = db::save_file(&c, &dir, "image/png", "", &png, Some(&uid), None).unwrap();
            let mut n = db::get_note(&c, nid).unwrap().unwrap();
            n.body = format!("楼下右转第二家的云南菜。\n- 两个人点小份就够\n- 周末晚上要排队\n![](att:{f})\n店里推荐的 https://example.com/menu");
            db::upsert_note(&c, &n).unwrap();
            db::upsert_note(&c, &Note { title: "周报改成三段式".into(), body: "做了什么 / 卡在哪 / 下周打算".into(), tag: Some("work".into()), ..Default::default() }).unwrap();
            // 带一个文件附件的笔记：手机上点开时才下载
            let nid = db::upsert_note(&c, &Note { title: "装修报价".into(), tag: Some("life".into()), ..Default::default() }).unwrap();
            let uid = db::get_note(&c, nid).unwrap().unwrap().uid;
            let f = db::save_file(&c, &dir, "application/pdf", "报价单 v2.pdf", b"%PDF-1.4 dingmi test", Some(&uid), None).unwrap();
            let mut n = db::get_note(&c, nid).unwrap().unwrap();
            n.body = format!("师傅给的报价，周末再对一下。\n[报价单 v2.pdf](att:{f})");
            db::upsert_note(&c, &n).unwrap();
        }
        let n = push(&a, &dir, &mut conn, &key).await.unwrap();
        println!("PUSHED={n}");
        let pk = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(crypto::inbox_public(&key));
        let t = token.replace('+', "-").replace('/', "_").trim_end_matches('=').to_string();
        let k = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(space_key);
        println!("PHONE_URL=https://{}:{}/m/#t={}.{}&k={}", code.host, code.port, t, pk, k);
        println!("PHONE_TOKEN={token}");
        println!("RECOVERY={recovery}");
    }

    /// 手机版的端到端测试，第二步：在手机网页上改过之后，用电脑端的代码从头拉一遍，打印出来核对。
    /// 服务器上生成的配对码不带空间密钥，用第一步打印的恢复码拿主密钥：
    /// DINGMI_TEST_PAIR=<另一个配对码> DINGMI_TEST_RECOVERY=<恢复码> cargo test --lib -- --ignored phone_e2e_check --nocapture
    #[tokio::test]
    #[ignore]
    async fn phone_e2e_check() {
        let code: PairCode = std::env::var("DINGMI_TEST_PAIR").unwrap().parse().unwrap();
        let dir = std::env::temp_dir().join(format!("dingmi-phone-check-{}", uuid::Uuid::new_v4().simple()));
        let a = Mutex::new(db::open(&dir).unwrap());
        let (dev, _) = dingmi_sync::generate_keypair().unwrap();
        let tcp: super::super::transport::Stream = Box::new(TcpStream::connect((code.host.as_str(), code.port)).await.unwrap());
        let hello = Hello { version: VERSION, name: "核对".into(), invite: Some(code.invite.clone()), ..Default::default() };
        let (mut conn, w) = dingmi_sync::connect(tcp, &dev, &code.server_public, &hello).await.unwrap();
        let recovery = crypto::parse_recovery(&std::env::var("DINGMI_TEST_RECOVERY").unwrap()).unwrap();
        let (key, _) = crypto::open_box2(&recovery, w.space.box2.as_deref().unwrap()).unwrap();
        pull(&a, &dir, &mut conn, &key).await.unwrap();
        let files: Vec<_> = {
            let c = a.lock().unwrap();
            for r in db::list_reminders(&c).unwrap() {
                println!("R|{}|{}|done={}|repeat={}|tag={:?}|urgent={}|deleted={:?}", r.title, r.due_at, r.done, r.repeat.kind, r.tag, r.urgent, r.deleted_at);
            }
            for n in db::list_notes(&c).unwrap() {
                println!("N|{}|tag={:?}|{}", n.title, n.tag, n.body.replace('\n', "⏎"));
            }
            // 文件附件不自动下载；像点开时那样下载下来，核对内容
            db::list_attachments(&c).unwrap().into_iter().filter(|a| !a.mime.starts_with("image/")).collect()
        };
        for f in files {
            let before = f.local;
            fetch_file(&a, &dir, &mut conn, &key, &f.uid, &f.file).await.unwrap();
            let bytes = std::fs::read(db::attachment_dir(&dir).join(&f.file)).unwrap();
            println!("F|{}|{}|{} bytes|local_before={before}|{:?}", f.name, f.mime, bytes.len(), String::from_utf8_lossy(&bytes[..bytes.len().min(24)]));
        }
    }
}
