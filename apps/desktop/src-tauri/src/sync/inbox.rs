//! 从服务器取走手机发来的东西：下载、解密、拆开，文字、图片和文件存进本机的收件箱。

use super::crypto::{self, DataKey};
use crate::{db, notify_data, AppState};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use dingmi_sync::{InboxItem, Msg};
use serde::Deserialize;
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tokio::time::timeout;

const REPLY_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
struct ImageHead {
    mime: String,
    size: usize,
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct Head {
    #[serde(default)]
    text: String,
    sent_at: Option<String>,
    #[serde(default)]
    images: Vec<ImageHead>,
}

pub struct Parcel {
    pub text: String,
    pub sent_at: Option<String>,
    /// 图片和文件，按发来的顺序
    pub images: Vec<Piece>,
}

/// 手机发来的一张图片或一个文件
#[derive(Debug, PartialEq)]
pub struct Piece {
    pub mime: String,
    /// 原文件名
    pub name: String,
    pub bytes: Vec<u8>,
}

/// 拆开手机网页打的包：4 字节（大端）说明长度 + 说明 JSON + 每张图片（或每个文件）依次接在后面
pub fn unpack(plain: &[u8]) -> Option<Parcel> {
    let len = u32::from_be_bytes(plain.get(..4)?.try_into().ok()?) as usize;
    let head: Head = serde_json::from_slice(plain.get(4..4 + len)?).ok()?;
    let mut rest = &plain[4 + len..];
    let mut images = Vec::with_capacity(head.images.len());
    for img in head.images {
        if img.size > rest.len() {
            return None;
        }
        let (bytes, tail) = rest.split_at(img.size);
        let mime = if img.mime.is_empty() { "application/octet-stream".into() } else { img.mime };
        images.push(Piece { mime, name: img.name, bytes: bytes.to_vec() });
        rest = tail;
    }
    Some(Parcel { text: head.text.trim().to_string(), sent_at: head.sent_at, images })
}

/// 发一条、等一条回答；服务器回 Error、Refused（协议 5 起说得清的拒绝）时变成 Err
pub(super) async fn ask(conn: &mut super::transport::Conn, msg: &Msg) -> Result<Msg, String> {
    match ask_any(conn, msg).await? {
        Msg::Error { msg } | Msg::Refused { msg, .. } => Err(msg),
        m => Ok(m),
    }
}

/// 发一条、等一条回答，Error、Refused 也原样给回来（要按拒绝原因区分时用）
pub(super) async fn ask_any(conn: &mut super::transport::Conn, msg: &Msg) -> Result<Msg, String> {
    conn.send(msg).await.map_err(|e| e.to_string())?;
    match timeout(REPLY_TIMEOUT, conn.recv::<Msg>()).await {
        Ok(Ok(m)) => Ok(m),
        _ => Err("和服务器的连接断了".into()),
    }
}

/// 收件箱里一条最大多大（加密后的）：和服务器收的上限一样（apps/server 的 MAX_INBOX_ITEM：一次最多 50 MB 的图片和文件，
/// 加上文字、说明和加密多出来的）。服务器说的、实际下载的超过这个就不要了
pub const MAX_INBOX_ITEM: u64 = 52 * 1024 * 1024;

/// 服务器上一条的编号：服务器生成的 32 位小写十六进制。会拼进附件的文件名，别的一律不认（服务器可能不怀好意）
pub fn valid_inbox_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// 下载出的问题：连接断了（下次再试）/ 这一条不对（太大、服务器乱回，记进 bad 不再试）
#[derive(Debug, PartialEq)]
enum Fail {
    Net(String),
    Bad,
}

async fn download(conn: &mut super::transport::Conn, item: &InboxItem, max: u64) -> Result<Vec<u8>, Fail> {
    if item.size > max {
        return Err(Fail::Bad);
    }
    // 不按服务器说的大小先占内存：边下边长，超过上限就停
    let mut data = Vec::new();
    loop {
        let get = Msg::InboxGet { id: item.id.clone(), offset: data.len() as u64 };
        let Msg::InboxChunk { data: chunk, done, .. } = ask(conn, &get).await.map_err(Fail::Net)? else {
            return Err(Fail::Bad);
        };
        let chunk = STANDARD.decode(chunk).map_err(|_| Fail::Bad)?;
        // 一块都没有又说没完：服务器在原地打转
        if chunk.is_empty() && !done {
            return Err(Fail::Bad);
        }
        if data.len() as u64 + chunk.len() as u64 > max {
            return Err(Fail::Bad);
        }
        data.extend(chunk);
        if done || data.len() as u64 >= item.size {
            return Ok(data);
        }
    }
}

/// 拆开的一条存进本机：图片、文件存进附件文件夹（标上「来自网络」），文字和附件列表存进收件箱
pub(crate) fn store_parcel(c: &Connection, data_dir: &Path, id: &str, phone: &str, p: Parcel) -> Result<(), String> {
    if !valid_inbox_id(id) {
        return Err("服务器给的编号不对".into());
    }
    let mut files = Vec::new();
    for (i, img) in p.images.iter().enumerate() {
        // 编号由服务器上这一条的编号（32 位十六进制）和序号拼成：几台电脑同时取到同一条，存下的也是同一张
        let uid = format!("{id}-{i:02x}");
        // 图片不用存文件名（手机相册里的名字没什么意义）
        let name = if img.mime.starts_with("image/") { "" } else { img.name.as_str() };
        let file = db::save_file(c, data_dir, &img.mime, name, &img.bytes, None, Some(&uid))?;
        if let Some(path) = db::att_path(&db::attachment_dir(data_dir), &file) {
            db::mark_from_web(&path);
        }
        files.push(file);
    }
    db::add_inbox(c, id, phone, p.sent_at.as_deref(), &p.text, &files).map_err(|e| e.to_string())
}

/// 把服务器上的都取下来。解不开的（比如改过同步密码之前发的）记在 `bad` 里，先留在服务器上不再重试。
pub async fn fetch(
    app: &AppHandle,
    conn: &mut super::transport::Conn,
    key: &DataKey,
    bad: &mut HashSet<String>,
) -> Result<(), String> {
    let st = app.state::<AppState>();
    let got = fetch_into(&st.db, &st.data_dir, conn, key, bad).await?;
    if got > 0 {
        notify_data(app);
    }
    Ok(())
}

/// 取下来存进这个数据库（不依赖 Tauri，测试里用）。回存了几条
pub(crate) async fn fetch_into(
    db: &Mutex<Connection>,
    data_dir: &Path,
    conn: &mut super::transport::Conn,
    key: &DataKey,
    bad: &mut HashSet<String>,
) -> Result<usize, String> {
    let Msg::InboxItems { items } = ask(conn, &Msg::InboxList).await? else {
        return Err("服务器回的内容不对".into());
    };
    let mut got = 0;
    for item in &items {
        if bad.contains(&item.id) {
            continue;
        }
        // 编号不对的（服务器不怀好意，想让文件名拼到附件文件夹外面去）：不下载、不存，也不叫服务器删
        if !valid_inbox_id(&item.id) {
            crate::diag::log("同步", "服务器给的收件箱编号不对，跳过了");
            bad.insert(item.id.clone());
            continue;
        }
        let known = db::has_inbox(&db.lock().unwrap(), &item.id);
        // 上次存好了但没来得及让服务器删掉的，直接删
        if !known {
            let data = match download(conn, item, MAX_INBOX_ITEM).await {
                Ok(d) => d,
                Err(Fail::Net(e)) => return Err(e),
                Err(Fail::Bad) => {
                    crate::diag::log("同步", "收件箱里有一条太大或者服务器回的不对，跳过了");
                    bad.insert(item.id.clone());
                    continue;
                }
            };
            let Some(parcel) = crypto::open_inbox(key, &data).and_then(|p| unpack(&p)) else {
                bad.insert(item.id.clone());
                continue;
            };
            store_parcel(&db.lock().unwrap(), data_dir, &item.id, &item.phone, parcel)?;
            got += 1;
        }
        ask(conn, &Msg::InboxDel { id: item.id.clone() }).await?;
    }
    Ok(got)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn pack(head: &str, images: &[&[u8]]) -> Vec<u8> {
        let mut out = (head.len() as u32).to_be_bytes().to_vec();
        out.extend(head.as_bytes());
        for i in images {
            out.extend(*i);
        }
        out
    }

    #[test]
    fn unpacks_text_and_images() {
        let head = r#"{"v":1,"text":" 看看这个 ","sent_at":"2026-10-31 10:00:00","images":[{"name":"a.png","mime":"image/png","size":3},{"name":"b.jpg","mime":"image/jpeg","size":2}]}"#;
        let p = unpack(&pack(head, &[b"abc", b"de"])).unwrap();
        assert_eq!(p.text, "看看这个");
        assert_eq!(p.sent_at.as_deref(), Some("2026-10-31 10:00:00"));
        let piece = |mime: &str, name: &str, bytes: &[u8]| Piece { mime: mime.into(), name: name.into(), bytes: bytes.to_vec() };
        assert_eq!(p.images, vec![piece("image/png", "a.png", b"abc"), piece("image/jpeg", "b.jpg", b"de")]);
        // 文件也收，带着原文件名；没写类型的当成二进制
        let head = r#"{"v":1,"text":"","images":[{"name":"合同.pdf","mime":"application/pdf","size":2},{"name":"x","mime":"","size":1}]}"#;
        let p = unpack(&pack(head, &[b"%P", b"z"])).unwrap();
        assert_eq!(p.images, vec![piece("application/pdf", "合同.pdf", b"%P"), piece("application/octet-stream", "x", b"z")]);
    }

    /// 用 node + tweetnacl 按手机网页的方式打包加密，电脑这边解开、拆开。平时跳过：
    /// DINGMI_TEST_JS=<脚本> DINGMI_TEST_NACL=<nacl-fast.js> cargo test --lib -- --ignored parcel_from_phone_page
    #[test]
    #[ignore]
    fn parcel_from_phone_page() {
        let key = [5u8; 32];
        let public: String = crypto::inbox_public(&key).iter().map(|b| format!("{b:02x}")).collect();
        let out = std::process::Command::new("node")
            .args([std::env::var("DINGMI_TEST_JS").unwrap(), std::env::var("DINGMI_TEST_NACL").unwrap(), public])
            .output()
            .unwrap();
        let hex = String::from_utf8(out.stdout).unwrap();
        let data: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        let p = unpack(&crypto::open_inbox(&key, &data).expect("解不开")).expect("拆不开");
        assert_eq!(p.text, "周四下午三点评审 https://mp.weixin.qq.com/s/x");
        assert_eq!(p.sent_at.as_deref(), Some("2026-10-31 14:20:05"));
        assert_eq!(p.images.len(), 2);
        assert_eq!((p.images[0].mime.as_str(), p.images[0].bytes.clone()), ("image/png", vec![137, 80, 78, 71, 1, 2, 3]));
        assert_eq!((p.images[1].mime.as_str(), p.images[1].bytes.len()), ("image/jpeg", 70000));
        // 另一把密钥解不开
        assert!(crypto::open_inbox(&[6u8; 32], &data).is_none());
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("dingmi-inbox-{name}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// 不怀好意的服务器：握手完以后，收到一条就照 `reply` 回一条。回 (电脑这头的连接, 服务器收到过的消息)
    pub(crate) async fn evil_server(
        reply: impl Fn(&Msg) -> Msg + Send + 'static,
    ) -> (super::super::transport::Conn, std::sync::Arc<Mutex<Vec<Msg>>>) {
        use tokio::io::AsyncReadExt;
        let (client, mut server) = tokio::io::duplex(1 << 20);
        let (server_private, server_public) = dingmi_sync::generate_keypair().unwrap();
        let (device_private, _) = dingmi_sync::generate_keypair().unwrap();
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            let mut magic = [0u8; 4];
            server.read_exact(&mut magic).await.unwrap();
            let pending = dingmi_sync::accept(server, &server_private).await.unwrap();
            let mut conn = pending.welcome(&dingmi_sync::Welcome::default()).await.unwrap();
            while let Ok(m) = conn.recv::<Msg>().await {
                let r = reply(&m);
                log.lock().unwrap().push(m);
                if conn.send(&r).await.is_err() {
                    break;
                }
            }
        });
        let stream: super::super::transport::Stream = Box::new(client);
        let hello = dingmi_sync::Hello::default();
        let (conn, _) = dingmi_sync::connect(stream, &device_private, &server_public, &hello).await.unwrap();
        (conn, seen)
    }

    fn files_under(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if e.path().is_dir() {
                out.extend(files_under(&e.path()));
            } else {
                out.push(e.path());
            }
        }
        out
    }

    /// 攻击：服务器给的编号带 ..\ 或者是绝对路径，想让手机发来的文件写到附件文件夹外面（比如「启动」文件夹）。
    /// 修之前 store 直接拿 item.id 拼文件名，文件会写到数据文件夹外面；现在编号不对的不下载、不存、记进 bad
    #[tokio::test]
    async fn evil_inbox_ids_cannot_write_outside() {
        let base = temp("evil-id");
        let data_dir = base.join("data");
        let db = Mutex::new(db::open(&data_dir).unwrap());
        let key = [9u8; 32];
        let parcel = {
            let head = r#"{"v":1,"text":"hi","images":[{"name":"x.exe","mime":"application/octet-stream","size":2}]}"#;
            crypto::seal_inbox(&crypto::inbox_public(&key), &pack(head, &[b"MZ"]))
        };
        let evil = [
            r"..\..\escaped".to_string(),
            "../../../../../../../../../escaped".to_string(),
            base.join("abs").display().to_string(),
            "0123456789abcdef0123456789abcdef/../../x".to_string(),
            "0123456789ABCDEF0123456789ABCDEF".to_string(),
        ];
        let items: Vec<InboxItem> = evil
            .iter()
            .map(|id| InboxItem { id: id.clone(), phone: "p".into(), size: parcel.len() as u64, created_at: String::new() })
            .collect();
        let blob = STANDARD.encode(&parcel);
        let (mut conn, seen) = evil_server(move |m| match m {
            Msg::InboxList => Msg::InboxItems { items: items.clone() },
            Msg::InboxGet { id, offset } => Msg::InboxChunk { id: id.clone(), offset: *offset, data: blob.clone(), done: true },
            _ => Msg::Ok,
        })
        .await;
        let mut bad = HashSet::new();
        let got = fetch_into(&db, &data_dir, &mut conn, &key, &mut bad).await.unwrap();
        assert_eq!(got, 0);
        assert_eq!(bad.len(), evil.len());
        // 一个都没下载、没叫服务器删
        assert!(seen.lock().unwrap().iter().all(|m| matches!(m, Msg::InboxList)));
        // 数据文件夹外面什么都没多出来，附件文件夹里也没有
        let outside: Vec<_> = files_under(&base).into_iter().filter(|p| !p.starts_with(&data_dir)).collect();
        assert!(outside.is_empty(), "{outside:?}");
        assert!(files_under(&db::attachment_dir(&data_dir)).is_empty());
        // 直接调存的那一步也一样不收
        let p = unpack(&crypto::open_inbox(&key, &parcel).unwrap()).unwrap();
        assert!(store_parcel(&db.lock().unwrap(), &data_dir, r"..\..\escaped", "p", p).is_err());
        assert!(files_under(&base).iter().all(|p| p.starts_with(&data_dir)));
    }

    /// 正常的编号照常收；存下的附件标上「来自网络」（Zone.Identifier，ZoneId=3）
    #[tokio::test]
    async fn good_inbox_items_are_stored_and_marked() {
        let base = temp("good");
        let data_dir = base.join("data");
        let db = Mutex::new(db::open(&data_dir).unwrap());
        let key = [3u8; 32];
        let head = r#"{"v":1,"text":"合同","images":[{"name":"合同.pdf","mime":"application/pdf","size":4}]}"#;
        let parcel = crypto::seal_inbox(&crypto::inbox_public(&key), &pack(head, &[b"%PDF"]));
        let id = "0123456789abcdef0123456789abcdef".to_string();
        let items = vec![InboxItem { id: id.clone(), phone: "p".into(), size: parcel.len() as u64, created_at: String::new() }];
        let blob = STANDARD.encode(&parcel);
        let (mut conn, seen) = evil_server(move |m| match m {
            Msg::InboxList => Msg::InboxItems { items: items.clone() },
            Msg::InboxGet { id, offset } => Msg::InboxChunk { id: id.clone(), offset: *offset, data: blob.clone(), done: true },
            _ => Msg::Ok,
        })
        .await;
        let mut bad = HashSet::new();
        assert_eq!(fetch_into(&db, &data_dir, &mut conn, &key, &mut bad).await.unwrap(), 1);
        assert!(bad.is_empty());
        assert!(seen.lock().unwrap().iter().any(|m| matches!(m, Msg::InboxDel { id: d } if *d == id)));
        let path = db::attachment_dir(&data_dir).join(format!("{id}-00.pdf"));
        assert_eq!(std::fs::read(&path).unwrap(), b"%PDF");
        if cfg!(windows) {
            let mut ads = path.as_os_str().to_owned();
            ads.push(":Zone.Identifier");
            assert_eq!(std::fs::read_to_string(std::path::PathBuf::from(ads)).unwrap(), "[ZoneTransfer]\r\nZoneId=3\r\n");
            assert!(db::from_web(&path));
        }
    }

    /// 攻击：服务器说这一条有 1 TB（修之前 Vec::with_capacity(1 TB) 直接让程序崩掉）、
    /// 或者说得很小却一直往下传、或者一块都不给又说没完。现在都不要了，记进 bad
    #[tokio::test]
    async fn evil_inbox_sizes() {
        let item = |size: u64| InboxItem { id: "aa".repeat(16), phone: "p".into(), size, created_at: String::new() };
        // 说有 1 TB：不下载
        let (mut conn, seen) = evil_server(|_| Msg::Ok).await;
        assert_eq!(download(&mut conn, &item(1 << 40), MAX_INBOX_ITEM).await, Err(Fail::Bad));
        assert!(seen.lock().unwrap().is_empty());
        // 说只有 10 字节，实际一直传：超过上限就停
        let (mut conn, _) = evil_server(|m| match m {
            Msg::InboxGet { id, offset } => Msg::InboxChunk { id: id.clone(), offset: *offset, data: STANDARD.encode([1u8; 4096]), done: false },
            _ => Msg::Ok,
        })
        .await;
        let mut it = item(10);
        it.size = u64::MAX / 2;
        assert_eq!(download(&mut conn, &it, 64 * 1024).await, Err(Fail::Bad));
        // 一块都不给又说没完
        let (mut conn, _) = evil_server(|m| match m {
            Msg::InboxGet { id, offset } => Msg::InboxChunk { id: id.clone(), offset: *offset, data: String::new(), done: false },
            _ => Msg::Ok,
        })
        .await;
        assert_eq!(download(&mut conn, &item(100), MAX_INBOX_ITEM).await, Err(Fail::Bad));
        // fetch_into：1 TB 的那条跳过、记进 bad，不中断后面的
        let base = temp("huge");
        let db = Mutex::new(db::open(&base).unwrap());
        let items = vec![item(1 << 40)];
        let (mut conn, _) = evil_server(move |m| match m {
            Msg::InboxList => Msg::InboxItems { items: items.clone() },
            _ => Msg::Ok,
        })
        .await;
        let mut bad = HashSet::new();
        assert_eq!(fetch_into(&db, &base, &mut conn, &[1u8; 32], &mut bad).await.unwrap(), 0);
        assert!(bad.contains(&"aa".repeat(16)));
    }

    #[test]
    fn inbox_ids() {
        assert!(valid_inbox_id("0123456789abcdef0123456789abcdef"));
        for bad in ["", "0123456789abcdef0123456789abcde", "0123456789ABCDEF0123456789ABCDEF", r"..\..\0123456789abcdef0123456789", "0123456789abcdef0123456789abcdef0"] {
            assert!(!valid_inbox_id(bad), "{bad}");
        }
    }

    #[test]
    fn rejects_broken_parcels() {
        let head = r#"{"v":1,"text":"x","images":[{"name":"a","mime":"image/png","size":10}]}"#;
        assert!(unpack(&pack(head, &[b"short"])).is_none());
        assert!(unpack(b"\x00\x00").is_none());
        assert!(unpack(&pack("not json", &[])).is_none());
    }
}
