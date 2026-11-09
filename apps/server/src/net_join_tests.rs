//! 新电脑方便加入（需求-1.x 第 34 节）的开源那一半：盒子 3 和加入核对值的存取、用手机加入。
//! 兑换码加入要兑换码，只有叮咪同步有，测试在官方版里。
//! 放在 net.rs 下面（`net::join_tests`），能看到 Server 里面的东西。

use super::*;
use crate::testkit::*;
use dingmi_sync::{generate_keypair, JoinLock};

fn lock(box3: &str, proof: &str) -> JoinLock {
    JoinLock { box3: box3.into(), proof: proof.into() }
}

/// 像官方版那样收盒子 3 的扩展点（开源版默认不收），测存取用
struct Joins;
impl crate::ext::Ext for Joins {
    fn joins(&self) -> bool {
        true
    }
}

fn join_set(join: JoinLock) -> Msg {
    Msg::JoinSet { join, salt: None }
}

/// 收盒子 3 的服务器（官方版）：设同步密码时带上盒子 3，账上 join 是开着；换同步密码不带就删掉；JoinSet / JoinOff 开关；
/// 平时连上的 Welcome 里不带盒子 3，已经登记过的设备 Hello 里带了 join 也不带
#[tokio::test]
async fn join_lock_follows_password() {
    let (server, addr, _) = start_with("join-lock", Arc::new(Joins)).await;
    let spub = server_public(&server);
    let (dev, _) = generate_keypair().unwrap();
    let (mut conn, w) = dial(addr, &dev, &spub, Some(invite(&server, "2999-01-01 00:00:00"))).await.unwrap();
    assert!(!w.account.as_ref().unwrap().join);
    let setup = Msg::SetupSpace {
        salt: "salt".into(),
        box1: "b1".into(),
        box2: "b2".into(),
        recovery: "rec".into(),
        join: Some(lock("b3", "proof")),
    };
    assert_eq!(ask(&mut conn, setup).await, Msg::Ok);
    let Msg::AccountInfo { account } = ask(&mut conn, Msg::AccountGet).await else { panic!() };
    assert!(account.join);
    let space = server.store.devices()[0].space_id.clone();
    assert_eq!(server.store.join_lock(&space), Some(("salt".into(), hash_token("proof"))));
    assert_eq!(server.store.space_with_box3(&space).box3.as_deref(), Some("b3"));
    drop(conn);
    let (mut conn, w) = dial(addr, &dev, &spub, None).await.unwrap();
    assert_eq!(w.space.box3, None, "平时不带盒子 3");
    // 登记过的设备自己说是「兑换码加入」也不给：只有扩展点核对过加入核对值、这次才登记的那一下给
    let hello = Hello { version: VERSION, join: Some(dingmi_sync::Join { code: "x".into(), proof: "wrong".into() }), ..Default::default() };
    let (_, w) = dial_hello(addr, &dev, &spub, hello).await.unwrap();
    assert!(w.refused.is_none());
    assert_eq!(w.space.box3, None, "登记过的设备要不到盒子 3");

    // 换同步密码：带了就换成新的，没带（新密码不够长、旧电脑）就删掉
    let set = |join| Msg::SetLock { salt: "salt2".into(), box1: "b1n".into(), join };
    assert_eq!(ask(&mut conn, set(Some(lock("b3n", "p2")))).await, Msg::Ok);
    assert_eq!(server.store.join_lock(&space), Some(("salt2".into(), hash_token("p2"))));
    assert_eq!(ask(&mut conn, set(None)).await, Msg::Ok);
    assert_eq!(server.store.join_lock(&space), None);
    assert_eq!(server.store.space_with_box3(&space).box3, None);

    // 补开、关掉
    // 打开时带的 salt 和服务器上的对不上（别的电脑刚换了同步密码）：不收，说清楚
    let stale = Msg::JoinSet { join: lock("b3x", "p3"), salt: Some("salt".into()) };
    assert!(matches!(ask(&mut conn, stale).await, Msg::Refused { code, .. } if code == refuse::JOIN_BAD));
    assert!(server.store.join_lock(&space).is_none());
    let fresh = Msg::JoinSet { join: lock("b3x", "p3"), salt: Some("salt2".into()) };
    assert_eq!(ask(&mut conn, fresh).await, Msg::Ok);
    assert!(server.store.join_lock(&space).is_some());
    assert_eq!(ask(&mut conn, Msg::JoinOff).await, Msg::Ok);
    assert!(server.store.join_lock(&space).is_none());
    let Msg::AccountInfo { account } = ask(&mut conn, Msg::AccountGet).await else { panic!() };
    assert!(!account.join && account.join_off);
    // 关掉以后，别的电脑换同步密码时带来的不收；明说打开（JoinSet）才开
    assert_eq!(ask(&mut conn, set(Some(lock("b3y", "p4")))).await, Msg::Ok);
    assert!(server.store.join_lock(&space).is_none() && server.store.join_turned_off(&space));
    assert_eq!(ask(&mut conn, join_set(lock("b3z", "p5"))).await, Msg::Ok);
    assert!(server.store.join_lock(&space).is_some() && !server.store.join_turned_off(&space));

}

/// 自建服务器（开源版）：不存盒子 3（设同步密码、换同步密码带来的都不存，JoinSet 说不支持），
/// 启动时清掉以前存下的；兑换码加入、删掉服务器上的数据都说「这台服务器不支持」（不算违规）
#[tokio::test]
async fn self_hosted_keeps_no_box3() {
    let (server, addr, dir) = start("join-self").await;
    let spub = server_public(&server);
    let (dev, _) = generate_keypair().unwrap();
    let (mut conn, _) = dial(addr, &dev, &spub, Some(invite(&server, "2999-01-01 00:00:00"))).await.unwrap();
    let setup = Msg::SetupSpace { salt: "salt".into(), box1: "b1".into(), box2: "b2".into(), recovery: "rec".into(), join: Some(lock("b3", "proof")) };
    assert_eq!(ask(&mut conn, setup).await, Msg::Ok);
    let space = server.store.devices()[0].space_id.clone();
    assert_eq!(server.store.space(&space).salt.as_deref(), Some("salt"), "同步密码照样设好");
    assert_eq!(server.store.space_with_box3(&space).box3, None);
    let set = Msg::SetLock { salt: "salt2".into(), box1: "b1n".into(), join: Some(lock("b3n", "p2")) };
    assert_eq!(ask(&mut conn, set).await, Msg::Ok);
    assert_eq!(server.store.space_with_box3(&space).box3, None);
    assert!(matches!(ask(&mut conn, join_set(lock("b3x", "p3"))).await, Msg::Refused { code, .. } if code == refuse::REDEEM_OFF));
    let Msg::AccountInfo { account } = ask(&mut conn, Msg::AccountGet).await else { panic!() };
    assert!(!account.join);
    assert!(matches!(ask(&mut conn, Msg::SpaceDelete { proof: None }).await, Msg::Refused { .. }));

    let (dev2, _) = generate_keypair().unwrap();
    let hello = Hello { version: VERSION, join_ask: Some("DM-2222-3333-4444-5555".into()), ..Default::default() };
    let (_, w) = dial_hello(addr, &dev2, &spub, hello).await.unwrap();
    assert_eq!(refused_code(&w), refuse::REDEEM_OFF);
    assert!(!server.banned("127.0.0.1".parse().unwrap()));

    // 1.0.6、1.0.7 的电脑传上来过的：重新启动时清掉
    assert!(server.store.set_join(&space, Some(("old-b3", "old-hash")), None));
    assert!(server.store.join_lock(&space).is_some());
    drop(conn);
    let again = Server::load(&dir).unwrap();
    assert_eq!(again.store.space_with_box3(&space).box3, None);
    assert!(again.store.join_lock(&space).is_none());
}

/// 用手机加入：电脑要数字 → 手机输数字看到电脑的名字和临时公钥 → 允许（带加密好的空间密钥）→
/// 电脑等到邀请码和加密的空间密钥 → 凭邀请码登记，连接日志记「用手机加入」。拒绝、过期、输错太多次也测
#[tokio::test]
async fn phone_helps_join() {
    let (server, addr, dir) = start("phone-join").await;
    let spub = server_public(&server);
    let (dev, _) = generate_keypair().unwrap();
    let (mut conn, _) = dial(addr, &dev, &spub, Some(invite(&server, "2999-01-01 00:00:00"))).await.unwrap();
    assert_eq!(ask(&mut conn, setup("salt")).await, Msg::Ok);
    let Msg::PhoneAdded { token, .. } = ask(&mut conn, Msg::PhoneAdd { name: "iPhone".into() }).await else { panic!() };
    let req = |method: &str, path: &str, len: usize| {
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Length: {len}\r\n\r\n")
    };

    // 新电脑：临时钥匙要数字
    let key = STANDARD.encode([7u8; 32]);
    let (tmp, _) = generate_keypair().unwrap();
    let ask_hello = |phone_ask, phone_poll| Hello { version: VERSION, name: "工作室电脑".into(), phone_ask, phone_poll, ..Default::default() };
    let pa = PhoneAsk { key: key.clone(), name: "工作室电脑".into(), code: String::new() };
    let (_, w) = dial_hello(addr, &tmp, &spub, ask_hello(Some(pa.clone()), None)).await.unwrap();
    let pj = w.phone.unwrap();
    assert_eq!(pj.code.len(), 8);
    assert!(pj.invite.is_none() && pj.sealed.is_none());
    // 临时钥匙不对的不收
    let bad = PhoneAsk { key: "abc".into(), ..pa.clone() };
    let (_, w) = dial_hello(addr, &tmp, &spub, ask_hello(Some(bad), None)).await.unwrap();
    assert_eq!(refused_code(&w), refuse::PHONE_GONE);

    // 手机网页：数字对了回名字和临时公钥；手机网页知道这是不是叮咪同步
    let out = https(addr, &dir, &req("GET", &format!("/m/api/join/{}", pj.code), 0), b"").await;
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    let v: serde_json::Value = serde_json::from_str(out.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!((v["name"].as_str(), v["key"].as_str()), (Some("工作室电脑"), Some(key.as_str())));
    let out = https(addr, &dir, &req("GET", "/m/api/space", 0), b"").await;
    assert!(out.contains("\"official\":false"), "{out}");

    // 手机允许之前电脑问：还在等（一次等 20 秒，这里让手机在等的时候允许）
    let poll = PhoneAsk { code: pj.code.clone(), ..pa.clone() };
    let s2 = server.clone();
    let code = pj.code.clone();
    let answer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(600)).await;
        let space = s2.store.devices().into_iter().next().unwrap().space_id;
        let phone = s2.store.phones(&space)[0].id.clone();
        s2.phone_answer("127.0.0.1".parse().unwrap(), &space, &phone, &code, Some("sealed-k".into()))
    });
    let (_, w) = dial_hello(addr, &tmp, &spub, ask_hello(None, Some(poll.clone()))).await.unwrap();
    answer.await.unwrap().unwrap();
    let got = w.phone.unwrap();
    assert_eq!(got.sealed.as_deref(), Some("sealed-k"));
    let invite_token = got.invite.unwrap();
    // 取走了先留着（交给电脑那一下断了，再问还能拿到），到过期为止
    let (_, w) = dial_hello(addr, &tmp, &spub, ask_hello(None, Some(poll))).await.unwrap();
    assert_eq!(w.phone.unwrap().invite.as_deref(), Some(invite_token.as_str()));

    // 凭邀请码登记（真正的设备钥匙）
    let (dev2, _) = generate_keypair().unwrap();
    let (_, w) = dial(addr, &dev2, &spub, Some(invite_token)).await.unwrap();
    assert!(w.refused.is_none());
    assert_eq!(w.account.unwrap().devices, 3, "两台电脑加一部手机");

    // 拒绝：电脑收到「不是我」
    let (_, w) = dial_hello(addr, &tmp, &spub, ask_hello(Some(pa.clone()), None)).await.unwrap();
    let pj = w.phone.unwrap();
    // 没先看过（GET）这个数字的手机不能允许、拒绝：记一次输错
    let out = https(addr, &dir, &req("POST", &format!("/m/api/join/{}", pj.code), 2), b"{}").await;
    assert!(out.starts_with("HTTP/1.1 409"), "{out}");
    let out = https(addr, &dir, &req("GET", &format!("/m/api/join/{}", pj.code), 0), b"").await;
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    let out = https(addr, &dir, &req("POST", &format!("/m/api/join/{}", pj.code), 2), b"{}").await;
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    let poll = PhoneAsk { code: pj.code.clone(), ..pa.clone() };
    let (_, w) = dial_hello(addr, &tmp, &spub, ask_hello(None, Some(poll))).await.unwrap();
    assert!(w.refused.unwrap().msg.contains("不是我"));
    // 拒绝过的数字手机再允许不了
    let body = br#"{"sealed":"x"}"#;
    let out = https(addr, &dir, &req("POST", &format!("/m/api/join/{}", pj.code), body.len()), body).await;
    assert!(out.starts_with("HTTP/1.1 409"), "{out}");

    // 手机输错太多次：10 分钟内不让再试，对的数字也不行；直接 POST 扫数字也算输错（上面已经错了 2 次）
    let (_, w) = dial_hello(addr, &tmp, &spub, ask_hello(Some(pa), None)).await.unwrap();
    let good = w.phone.unwrap().code;
    let body = br#"{"sealed":"x"}"#;
    let out = https(addr, &dir, &req("POST", "/m/api/join/00000000", body.len()), body).await;
    assert!(out.starts_with("HTTP/1.1 409"), "{out}");
    for _ in 3..PHONE_MAX_MISSES {
        let out = https(addr, &dir, &req("GET", "/m/api/join/00000000", 0), b"").await;
        assert!(out.starts_with("HTTP/1.1 404"), "{out}");
    }
    let out = https(addr, &dir, &req("GET", &format!("/m/api/join/{good}"), 0), b"").await;
    assert!(out.starts_with("HTTP/1.1 429"), "{out}");
    let out = https(addr, &dir, &req("POST", &format!("/m/api/join/{good}"), body.len()), body).await;
    assert!(out.starts_with("HTTP/1.1 429"), "{out}");
}

/// 同一个 IPv6 /64 换着地址要数字，和同一个 IP 一样只给 PHONE_REQS_PER_IP 个
#[tokio::test]
async fn phone_ask_limits_ipv6_net() {
    let (server, _, _) = start("phone-v6").await;
    let ask = || PhoneAsk { key: STANDARD.encode([7u8; 32]), name: "电脑".into(), code: String::new() };
    for i in 0..PHONE_REQS_PER_IP {
        let ip: IpAddr = format!("2001:db8:1:2::{}", i + 1).parse().unwrap();
        assert!(server.phone_ask(ask(), ip).is_ok());
    }
    assert!(server.phone_ask(ask(), "2001:db8:1:2::ff".parse().unwrap()).is_err(), "同一个 /64");
    assert!(server.phone_ask(ask(), "2001:db8:1:3::1".parse().unwrap()).is_ok(), "别的 /64 不受影响");
}

/// 自建服务器的手机给新电脑生成配对码（不带空间密钥，手机自己接在后面）
#[tokio::test]
async fn phone_makes_pair_code() {
    let (server, addr, dir) = start("phone-invite").await;
    let spub = server_public(&server);
    let (dev, _) = generate_keypair().unwrap();
    let (mut conn, _) = dial(addr, &dev, &spub, Some(invite(&server, "2999-01-01 00:00:00"))).await.unwrap();
    assert_eq!(ask(&mut conn, setup("salt")).await, Msg::Ok);
    let Msg::PhoneAdded { token, .. } = ask(&mut conn, Msg::PhoneAdd { name: "iPhone".into() }).await else { panic!() };
    let req = format!("GET /m/api/invite HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\n\r\n");
    let out = https(addr, &dir, &req, b"").await;
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    let v: serde_json::Value = serde_json::from_str(out.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    let code: dingmi_sync::PairCode = v["code"].as_str().unwrap().parse().unwrap();
    assert!(code.space_key.is_none());
    let (dev2, _) = generate_keypair().unwrap();
    let (_, w) = dial(addr, &dev2, &spub, Some(code.invite)).await.unwrap();
    assert!(w.refused.is_none());
}
