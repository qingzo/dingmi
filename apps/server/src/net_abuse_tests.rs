//! 防滥用的测试（安全审查发现的问题）：能写成攻击的都写成攻击，以前的程序过不了、改了以后过得了。
//! 放在 net.rs 下面（`net::abuse_tests`），能看到 Server 里面的东西。

use super::*;
use crate::ext::{Ext, FileLimits};
use crate::testkit::*;
use dingmi_sync::generate_keypair;
use tokio::io::AsyncWriteExt;

/// 握手限时、上传名额这些调小，测试不用等那么久
fn quick() -> Caps {
    Caps { handshake: Duration::from_millis(500), drain: Duration::from_millis(500), upload_wait: Duration::from_millis(300), ..Caps::default() }
}

async fn start_quick(name: &str, caps: Caps) -> (Arc<Server>, std::net::SocketAddr, PathBuf) {
    start_caps(name, Arc::new(SelfHosted), caps).await
}

fn fails(server: &Server, ip: &str) -> u32 {
    server.strikes.lock().unwrap().map.get(&strike_key(ip.parse().unwrap())).map_or(0, |s| s.fails)
}

/// 配对一台电脑、加一部手机，回 (电脑的连接, 手机令牌)
async fn with_phone(server: &Arc<Server>, addr: std::net::SocketAddr) -> (dingmi_sync::NoiseConn<TcpStream>, String) {
    let spub = server_public(server);
    let (dev, _) = generate_keypair().unwrap();
    let (mut conn, _) = dial(addr, &dev, &spub, Some(invite(server, "2999-01-01 00:00:00"))).await.unwrap();
    let Msg::PhoneAdded { token, .. } = ask(&mut conn, Msg::PhoneAdd { name: "iPhone".into() }).await else { panic!("没加上手机") };
    (conn, token)
}

fn post_inbox(token: &str, len: usize, extra: &str) -> String {
    format!("POST /m/api/inbox HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\n{extra}Content-Length: {len}\r\n\r\n")
}

/// 读到对方关掉，最多等 `secs` 秒；没等到回 None
async fn read_all<S: AsyncRead + Unpin>(s: &mut S, secs: u64) -> Option<String> {
    let mut out = Vec::new();
    match timeout(Duration::from_secs(secs), s.read_to_end(&mut out)).await {
        Ok(_) => Some(String::from_utf8_lossy(&out).into_owned()),
        Err(_) => None,
    }
}

// ---------- 第 2 条：/sync 升级以后读 DNGM 没有限时 ----------

/// 攻击：走 /sync 升级成 WebSocket 以后什么都不发。以前读 `DNGM` 没限时，连接一直占着；
/// 现在和握手一起限时，到了就断开、算一次违规
#[tokio::test]
async fn websocket_silence_is_cut_off() {
    let (server, addr, _) = start_quick("ws-idle", quick()).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    dingmi_sync::ws::client_handshake(&mut s, "sync.example.com").await.unwrap();
    assert!(read_all(&mut s, 5).await.is_some(), "升级以后一直不发东西，该断开了");
    assert_eq!(fails(&server, "127.0.0.1"), 1, "算一次违规");

    // 发的开头不是 DNGM：也断开、算违规
    let mut s = TcpStream::connect(addr).await.unwrap();
    dingmi_sync::ws::client_handshake(&mut s, "sync.example.com").await.unwrap();
    let mut ws = dingmi_sync::ws::WsStream::client(s);
    ws.write_all(b"XXXXXXXX").await.unwrap();
    ws.flush().await.unwrap();
    assert!(read_all(&mut ws, 5).await.is_some());
    assert_eq!(fails(&server, "127.0.0.1"), 2);

    // 直连的：发了 DNGM 就不往下握手，一样限时
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(b"DNGM").await.unwrap();
    assert!(read_all(&mut s, 5).await.is_some());
    assert_eq!(fails(&server, "127.0.0.1"), 3);
}

// ---------- 第 3 条：令牌不对也先读完 52 MB 的请求体；上传名额 ----------

/// 攻击：令牌不对的 POST /m/api/inbox，说要发 10 MB、只发一点就不发了。以前先把请求体整个收进内存再看令牌，
/// 这条连接一直等着（最长 5 分钟）、收到的都攒在内存里；现在先看令牌，请求体读掉就扔，最多读 Caps.drain 这么久就回 401
#[tokio::test]
async fn wrong_token_inbox_is_not_buffered() {
    let (server, addr, dir) = start_quick("inbox-token", quick()).await;
    let mut s = tls(addr, &dir).await;
    s.write_all(post_inbox("wrong", 10 << 20, "").as_bytes()).await.unwrap();
    s.write_all(&[1u8; 1024]).await.unwrap();
    s.flush().await.unwrap();
    let out = read_all(&mut s, 5).await.expect("令牌不对，该很快回 401");
    assert!(out.starts_with("HTTP/1.1 401"), "{out}");
    assert_eq!(fails(&server, "127.0.0.1"), 1);

    // 写着比上限还大的：不用读，直接 401（以前是读到一半回 413，看不出令牌不对）
    let out = https(addr, &dir, &post_inbox("wrong", MAX_INBOX_ITEM + 1, ""), b"").await;
    assert!(out.starts_with("HTTP/1.1 401"), "{out}");
}

/// 令牌对的上传有名额（Caps.uploads）：满了等一会儿，等不到回 503；前面的传完了照常
#[tokio::test]
async fn uploads_share_a_few_slots() {
    let (server, addr, dir) = start_quick("inbox-slots", Caps { uploads: 1, ..quick() }).await;
    let (_conn, token) = with_phone(&server, addr).await;
    // 第一个慢慢传：占着唯一的名额
    let mut slow = tls(addr, &dir).await;
    slow.write_all(post_inbox(&token, 1000, "").as_bytes()).await.unwrap();
    slow.write_all(&[7u8; 10]).await.unwrap();
    slow.flush().await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let out = https(addr, &dir, &post_inbox(&token, 100, ""), &[7u8; 100]).await;
    assert!(out.starts_with("HTTP/1.1 503"), "{out}");
    let out = https(addr, &dir, &format!("POST /m/api/file/ab01 HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\nContent-Length: 100\r\n\r\n"), &[7u8; 100]).await;
    assert!(out.starts_with("HTTP/1.1 503"), "附件也算：{out}");
    // 第一个传完：200，名额还回来，后面的照常
    slow.write_all(&[7u8; 990]).await.unwrap();
    let out = read_all(&mut slow, 5).await.unwrap();
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    let out = https(addr, &dir, &post_inbox(&token, 100, ""), &[7u8; 100]).await;
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
}

// ---------- 第 4 条：经过反向代理时屏蔽没生效 ----------

/// 攻击：手机网页放在 Nginx 后面（连接都从 127.0.0.1 来，带 X-Real-IP），同一个真实 IP 猜令牌。
/// 以前错 5 次只是记下了，没有哪里按真实 IP 查屏蔽，可以一直猜；现在 /m/ 下的请求一开头就按真实 IP 查，回 429
#[tokio::test]
async fn ban_works_behind_reverse_proxy() {
    let (server, addr, dir) = start_quick("proxy-ban", quick()).await;
    let (_conn, token) = with_phone(&server, addr).await;
    let get = |tok: &str, from: &str| {
        format!("GET /m/api/space HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {tok}\r\nX-Real-IP: {from}\r\nX-Forwarded-For: {from}\r\n\r\n")
    };
    for _ in 0..MAX_FAILS {
        assert!(https(addr, &dir, &get("guess", "203.0.113.7"), b"").await.starts_with("HTTP/1.1 401"));
    }
    assert!(server.banned("203.0.113.7".parse().unwrap()));
    // 猜中了也不行；别的接口、收件箱、网页也一样
    let out = https(addr, &dir, &get(&token, "203.0.113.7"), b"").await;
    assert!(out.starts_with("HTTP/1.1 429"), "{out}");
    let out = https(addr, &dir, &post_inbox(&token, 100, "X-Real-IP: 203.0.113.7\r\n"), &[1u8; 100]).await;
    assert!(out.starts_with("HTTP/1.1 429"), "{out}");
    let out = https(addr, &dir, "GET /m/ HTTP/1.1\r\nHost: x\r\nX-Real-IP: 203.0.113.7\r\n\r\n", b"").await;
    assert!(out.starts_with("HTTP/1.1 429"), "{out}");
    // 别的手机（别的真实 IP）、反向代理本身不受影响
    assert!(https(addr, &dir, &get(&token, "203.0.113.8"), b"").await.starts_with("HTTP/1.1 200"));
    let own = format!("GET /m/api/space HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\n\r\n");
    assert!(https(addr, &dir, &own, b"").await.starts_with("HTTP/1.1 200"));
    // 只写 X-Forwarded-For、把别人的 IP 写在前面想躲开屏蔽：认的是最后一个（反向代理加的）
    let dodge = format!("GET /m/api/space HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\nX-Forwarded-For: 198.51.100.1, 203.0.113.7\r\n\r\n");
    assert!(https(addr, &dir, &dodge, b"").await.starts_with("HTTP/1.1 429"));
}

// ---------- 第 5 条：附件分片续传不核对 offset ----------

/// 攻击：一上来就从 59 MB 处写 1 个字节。以前照收：服务器上多出一个 59 MB 的文件（中间全是零），
/// 空间的占用也记成 59 MB；现在接着传必须正好接在已有的后面、还没传完，不然回「附件要从头传」
#[tokio::test]
async fn file_put_must_continue_where_it_left_off() {
    let (server, addr, dir) = start_quick("file-offset", quick()).await;
    let spub = server_public(&server);
    let (dev, _) = generate_keypair().unwrap();
    let (mut c, _) = dial(addr, &dev, &spub, Some(invite(&server, "2999-01-01 00:00:00"))).await.unwrap();
    let space = server.store.latest_space().unwrap();
    let put = |uid: &str, offset: u64, n: usize, done: bool| Msg::FilePut { uid: uid.into(), offset, data: STANDARD.encode(vec![1u8; n]), done };
    let restart = |m: Msg| matches!(m, Msg::Error { msg } if msg == FILE_RESTART);

    assert!(restart(ask(&mut c, put("aa01", 59 << 20, 1, true)).await));
    assert_eq!(server.store.usage(&space).1, 0);
    assert!(!space_files(&dir, &space).join("aa01").exists());

    assert_eq!(ask(&mut c, put("aa02", 0, 10, false)).await, Msg::Ok);
    assert!(restart(ask(&mut c, put("aa02", 20, 10, false)).await), "中间空了一段");
    assert!(restart(ask(&mut c, put("aa02", 5, 10, false)).await), "盖掉一段");
    assert_eq!(ask(&mut c, put("aa02", 10, 10, true)).await, Msg::Ok);
    assert!(restart(ask(&mut c, put("aa02", 20, 10, true)).await), "已经传完了");
    assert_eq!(server.store.usage(&space).1, 20);
    // 从头传总是可以的（电脑每次都从头传）
    assert_eq!(ask(&mut c, put("aa02", 0, 5, true)).await, Msg::Ok);
    assert_eq!(server.store.usage(&space).1, 5);
}

// ---------- 第 7 条：全局连接数、违规记录的上限 ----------

/// 攻击：开一堆连接什么都不发。以前没有上限；现在同时最多 Caps.conns 个，再多的直接关掉，断开的名额还回来。
/// 升级成 WebSocket 的电脑连接一直带着名额
#[tokio::test]
async fn connection_cap() {
    let (server, addr, _) = start_quick("conns", Caps { conns: 2, handshake: Duration::from_secs(10), ..quick() }).await;
    let a = TcpStream::connect(addr).await.unwrap();
    let b = TcpStream::connect(addr).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(raw(addr, b"GET /ca HTTP/1.1\r\nHost: x\r\n\r\n").await, "", "满了，直接关掉");
    drop((a, b));
    let mut ok = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if raw(addr, b"GET /ca HTTP/1.1\r\nHost: x\r\n\r\n").await.starts_with("HTTP/1.1 200") {
            ok = true;
            break;
        }
    }
    assert!(ok, "断开以后名额还回来了");

    // 走 WebSocket 的电脑：升级以后连接还占着名额
    let spub = server_public(&server);
    let (dev, _) = generate_keypair().unwrap();
    let mut s = TcpStream::connect(addr).await.unwrap();
    dingmi_sync::ws::client_handshake(&mut s, "x").await.unwrap();
    let hello = Hello { version: VERSION, name: "ws".into(), invite: Some(invite(&server, "2999-01-01 00:00:00")), ..Default::default() };
    let (mut ws, _) = dingmi_sync::connect(dingmi_sync::ws::WsStream::client(s), &dev, &spub, &hello).await.unwrap();
    let c = TcpStream::connect(addr).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(raw(addr, b"GET /ca HTTP/1.1\r\nHost: x\r\n\r\n").await, "", "WebSocket 一个、空连接一个，满了");
    assert_eq!(ask(&mut ws, Msg::Ping).await, Msg::Pong);
    drop(c);
}

/// 攻击：从很多 IP（IPv6 一个人就有一大片）各错一次，把违规记录撑大。现在最多记 MAX_STRIKES 个，满了先清最早的；
/// IPv6 按 /64 记，同一个 /64 里换着地址试也算同一个
#[test]
fn strike_table_is_bounded() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let (server, _, _) = rt.block_on(start_quick("strikes", quick()));
    let ip = |i: u32| IpAddr::from(std::net::Ipv4Addr::from(0x0A00_0000 + i));
    for i in 0..(MAX_STRIKES as u32 + 10) {
        server.strike(ip(i));
    }
    {
        let m = server.strikes.lock().unwrap();
        assert_eq!((m.map.len(), m.order.len()), (MAX_STRIKES, MAX_STRIKES));
        assert!(!m.map.contains_key(&ip(0)), "最早的清掉了");
        assert!(m.map.contains_key(&ip(MAX_STRIKES as u32 + 9)));
    }
    for i in 0..MAX_FAILS {
        server.strike(format!("2001:db8:1:2::{i:x}").parse().unwrap());
    }
    assert!(server.banned("2001:db8:1:2:ffff::1".parse().unwrap()), "同一个 /64");
    assert!(!server.banned("2001:db8:1:3::1".parse().unwrap()));
}

// ---------- 追加 A：手机收件箱不受限额 ----------

/// 单个附件上限是 1000 字节的服务器（像叮咪同步按套餐限的）
struct Capped;

impl Ext for Capped {
    fn file_limits(&self, _s: &Server, _space: &str) -> FileLimits {
        FileLimits { max_image: Some(1000), max_file: Some(1000), cap: Some(1000), too_big: "单个不能超过 1000 字节".into() }
    }
}

/// 攻击：附件有单个上限、空间有容量上限，手机收件箱以前都不管（只有 1000 条 / 1 GB），可以拿它绕过去塞东西。
/// 现在一条按一个附件算单个上限（413），收件箱里没取走的算进容量（507）
#[tokio::test]
async fn inbox_respects_limits_and_quota() {
    let (server, addr, dir) = start_caps("inbox-limits", Arc::new(Capped), quick()).await;
    let (mut conn, token) = with_phone(&server, addr).await;
    let space = server.store.latest_space().unwrap();
    let out = https(addr, &dir, &post_inbox(&token, 2000, ""), &[1u8; 2000]).await;
    assert!(out.starts_with("HTTP/1.1 413") && out.contains("1000 字节"), "{out}");
    let out = https(addr, &dir, &post_inbox(&token, 600, ""), &[1u8; 600]).await;
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");

    // 容量 1000：已有 600 在收件箱里，再来 600 放不下；电脑取走（删掉）以后又能发
    admin::set_quota(&server.store, &space, Some(1000)).unwrap();
    let out = https(addr, &dir, &post_inbox(&token, 600, ""), &[1u8; 600]).await;
    assert!(out.starts_with("HTTP/1.1 507") && out.contains("空间满了"), "{out}");
    let Msg::InboxItems { items } = ask(&mut conn, Msg::InboxList).await else { panic!() };
    assert_eq!(items.len(), 1);
    assert_eq!(ask(&mut conn, Msg::InboxDel { id: items[0].id.clone() }).await, Msg::Ok);
    let out = https(addr, &dir, &post_inbox(&token, 600, ""), &[1u8; 600]).await;
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
}

/// 超过 INBOX_DAYS 天没被电脑取走的，每天收尾时删掉（记录和文件）
#[tokio::test]
async fn old_inbox_items_are_purged() {
    let (server, addr, dir) = start_quick("inbox-purge", quick()).await;
    let (mut conn, token) = with_phone(&server, addr).await;
    for _ in 0..2 {
        assert!(https(addr, &dir, &post_inbox(&token, 100, ""), &[1u8; 100]).await.starts_with("HTTP/1.1 200"));
    }
    let Msg::InboxItems { items } = ask(&mut conn, Msg::InboxList).await else { panic!() };
    let (old, new) = (&items[0].id, &items[1].id);
    server.store.c().execute("UPDATE inbox_items SET created_at='2000-01-01 00:00:00' WHERE id=?1", [old]).unwrap();
    let before = (chrono::Local::now() - chrono::TimeDelta::days(INBOX_DAYS)).format("%Y-%m-%d %H:%M:%S").to_string();
    assert_eq!(server.purge_inbox(&before), 1);
    assert!(!inbox_path(&dir, old).exists());
    assert!(inbox_path(&dir, new).exists());
    let Msg::InboxItems { items } = ask(&mut conn, Msg::InboxList).await else { panic!() };
    assert_eq!(items.iter().map(|i| &i.id).collect::<Vec<_>>(), [new]);
}

// ---------- 追加 B：设备上限先查后加 ----------

/// 最多 2 台设备（电脑加手机），查的时候慢一点（像查数据库、算套餐），好让同时登记的挤进来
struct TwoDevices;

impl Ext for TwoDevices {
    fn room(&self, s: &Server, space: &str) -> Result<(), dingmi_sync::Refusal> {
        let n = s.store.device_count(space);
        std::thread::sleep(Duration::from_millis(30));
        if n >= 2 {
            Err(dingmi_sync::Refusal { code: dingmi_sync::refuse::DEVICE_LIMIT.into(), msg: "满了".into() })
        } else {
            Ok(())
        }
    }
}

/// 攻击：几台电脑同时登记（或者同时添加手机）。以前「还有空位吗」和「加进去」之间别的登记能插进来，大家都看到有空位，
/// 一起超过上限；现在这两步之间串行
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn device_limit_holds_under_concurrency() {
    let (server, addr, _) = start_caps("admit", Arc::new(TwoDevices), quick()).await;
    let spub = server_public(&server);
    let toks: Vec<String> = (0..6).map(|_| invite(&server, "2999-01-01 00:00:00")).collect();
    let tries: Vec<_> = toks
        .into_iter()
        .map(|tok| {
            let spub = spub.clone();
            tokio::spawn(async move {
                let (dev, _) = generate_keypair().unwrap();
                dial(addr, &dev, &spub, Some(tok)).await.ok().filter(|(_, w)| w.refused.is_none()).map(|(c, _)| (c, dev))
            })
        })
        .collect();
    let mut pcs = vec![];
    for t in tries {
        pcs.extend(t.await.unwrap());
    }
    let space = server.store.latest_space().unwrap();
    assert_eq!(server.store.device_count(&space), 2, "同时登记的不超过上限");
    assert_eq!(pcs.len(), 2);

    // 作废一台腾出一个位置，剩下那台开几个连接同时添加手机：只加上一部
    let (mut c, dev) = pcs.pop().unwrap();
    let Msg::Pcs { pcs: list } = ask(&mut c, Msg::PcList).await else { panic!() };
    let other = list.into_iter().find(|p| !p.me).unwrap().id;
    assert_eq!(ask(&mut c, Msg::PcRevoke { id: other }).await, Msg::Ok);
    let mut conns = vec![c];
    for _ in 0..3 {
        conns.push(dial(addr, &dev, &spub, None).await.unwrap().0);
    }
    let adds: Vec<_> = conns
        .into_iter()
        .map(|mut c| tokio::spawn(async move { matches!(ask(&mut c, Msg::PhoneAdd { name: "手机".into() }).await, Msg::PhoneAdded { .. }) }))
        .collect();
    let mut added = 0;
    for a in adds {
        added += a.await.unwrap() as u32;
    }
    assert_eq!(added, 1);
    assert_eq!(server.store.device_count(&space), 2);
}
