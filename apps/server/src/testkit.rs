//! 测试用的工具：在本机起一个服务器、像电脑和手机一样连上去。
//! net.rs 自己的测试用它；官方版（带着自己的扩展点）的测试打开 `testkit` 这个 feature 也用它。
//! 只在测试里编译，正式的程序里没有。

use crate::ext::Ext;
use crate::net::{hash_token, Caps, Server};
use crate::tls;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD as B64};
use base64::Engine;
use dingmi_sync::{connect, Hello, Msg, NoiseConn, Record, Welcome, VERSION};
use rustls::pki_types::{CertificateDer, ServerName};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

/// 在临时目录里 init、起一个开源版的服务器（扩展点是 SelfHosted），听本机随便一个端口
pub async fn start(name: &str) -> (Arc<Server>, SocketAddr, PathBuf) {
    start_with(name, Arc::new(crate::ext::SelfHosted)).await
}

/// 同上，带着扩展点（官方版的测试用）
pub async fn start_with(name: &str, ext: Arc<dyn Ext>) -> (Arc<Server>, SocketAddr, PathBuf) {
    start_caps(name, ext, Caps::default()).await
}

/// 同上，自己定几个上限（握手限时、连接数、上传名额……）
pub async fn start_caps(name: &str, ext: Arc<dyn Ext>, caps: Caps) -> (Arc<Server>, SocketAddr, PathBuf) {
    let dir = std::env::temp_dir().join(format!("dingmi-server-test-{name}-{}", rand::random::<u64>()));
    crate::init(&dir, "127.0.0.1".parse().unwrap(), 7788).unwrap();
    let server = Server::load_caps(&dir, ext, caps).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(server.clone().serve(listener));
    (server, addr, dir)
}

/// 像官方版那样在宝塔后面：只 init_keys（不要 IP、证书）、`Server::load_proxied`，本机端口上收普通 HTTP，
/// 每个连接拿一个名额、交给 http 那一套（当成已经是 HTTPS）。真的官方版前面还要按域名分，这里不分
pub async fn start_proxied(name: &str, ext: Arc<dyn Ext>) -> (Arc<Server>, SocketAddr, PathBuf) {
    let dir = std::env::temp_dir().join(format!("dingmi-server-test-{name}-{}", rand::random::<u64>()));
    crate::init_keys(&dir).unwrap();
    let server = Server::load_proxied(&dir, ext).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let s = server.clone();
    tokio::spawn(async move {
        loop {
            let Ok((sock, peer)) = listener.accept().await else { continue };
            let Some(permit) = s.conn_slot() else { continue };
            tokio::spawn(crate::http::serve(sock, true, peer.ip(), s.clone(), permit));
        }
    });
    (server, addr, dir)
}

/// 给最近建的空间（没有就建一个 s1）生成一个邀请码
pub fn invite(server: &Server, expires_at: &str) -> String {
    let space = server.store.latest_space().unwrap_or_else(|| {
        server.store.create_space("s1").unwrap();
        "s1".into()
    });
    let token = format!("tok{}", rand::random::<u64>());
    server.store.add_invite(&hash_token(&token), &space, expires_at).unwrap();
    token
}

pub fn server_public(server: &Server) -> Vec<u8> {
    B64.decode(server.store.kv_get("noise_public").unwrap()).unwrap()
}

pub async fn dial(
    addr: SocketAddr,
    dev: &[u8],
    server_pub: &[u8],
    invite: Option<String>,
) -> dingmi_sync::Result<(NoiseConn<TcpStream>, Welcome)> {
    dial_v(addr, dev, server_pub, invite, VERSION).await
}

/// 指定协议版本（像旧电脑一样连）
pub async fn dial_v(
    addr: SocketAddr,
    dev: &[u8],
    server_pub: &[u8],
    invite: Option<String>,
    version: u32,
) -> dingmi_sync::Result<(NoiseConn<TcpStream>, Welcome)> {
    let s = TcpStream::connect(addr).await?;
    connect(s, dev, server_pub, &Hello { version, name: "测试电脑".into(), invite, ..Default::default() }).await
}

/// 没有配对码，凭恢复凭证登记
pub async fn dial_recovery(
    addr: SocketAddr,
    dev: &[u8],
    server_pub: &[u8],
    recovery: &str,
) -> dingmi_sync::Result<(NoiseConn<TcpStream>, Welcome)> {
    let s = TcpStream::connect(addr).await?;
    let hello = Hello { version: VERSION, name: "找回的电脑".into(), recovery: Some(recovery.into()), ..Default::default() };
    connect(s, dev, server_pub, &hello).await
}

/// 带着自己写的 Hello 连
pub async fn dial_hello(
    addr: SocketAddr,
    dev: &[u8],
    server_pub: &[u8],
    hello: Hello,
) -> dingmi_sync::Result<(NoiseConn<TcpStream>, Welcome)> {
    connect(TcpStream::connect(addr).await?, dev, server_pub, &hello).await
}

pub fn setup(salt: &str) -> Msg {
    Msg::SetupSpace { salt: salt.into(), box1: "b1".into(), box2: "b2".into(), recovery: "rec".into(), join: None }
}

/// 等一条消息（被叫醒），最多 5 秒
pub async fn woke(conn: &mut NoiseConn<TcpStream>) -> Msg {
    timeout(Duration::from_secs(5), conn.recv::<Msg>()).await.unwrap().unwrap()
}

pub fn record(uid: &str, ver: &str, blob: &[u8]) -> Record {
    Record { uid: uid.into(), ver: ver.into(), seq: 0, blob: STANDARD.encode(blob) }
}

/// 发一个原始的请求（普通 TCP），读到对方关掉
pub async fn raw(addr: SocketAddr, req: &[u8]) -> String {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let _ = s.write_all(req).await;
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out).await;
    String::from_utf8_lossy(&out).into_owned()
}

/// 像信任了根证书的 iPhone 一样走 HTTPS，发一个原始请求。30 秒没回完就算失败
/// （以前在机器很忙时卡过一次，一直等到服务器 300 秒的连接上限才失败，看不出卡在哪）
pub async fn https(addr: SocketAddr, dir: &Path, head: &str, body: &[u8]) -> String {
    timeout(Duration::from_secs(30), https_inner(addr, dir, head, body)).await.expect("HTTPS 请求 30 秒没回完")
}

async fn https_inner(addr: SocketAddr, dir: &Path, head: &str, body: &[u8]) -> String {
    let mut s = tls(addr, dir).await;
    s.write_all(head.as_bytes()).await.unwrap();
    s.write_all(body).await.unwrap();
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out).await;
    String::from_utf8_lossy(&out).into_owned()
}

/// 像信任了根证书的 iPhone 一样连上 HTTPS，后面自己发
pub async fn tls(addr: SocketAddr, dir: &Path) -> tokio_rustls::client::TlsStream<TcpStream> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(tls::ca_der(dir).unwrap())).unwrap();
    let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let tcp = TcpStream::connect(addr).await.unwrap();
    tokio_rustls::TlsConnector::from(Arc::new(cfg)).connect(ServerName::try_from("127.0.0.1").unwrap(), tcp).await.unwrap()
}

/// 发一条、等一条
pub async fn ask<S: AsyncRead + AsyncWrite + Unpin>(conn: &mut NoiseConn<S>, msg: Msg) -> Msg {
    conn.send(&msg).await.unwrap();
    conn.recv().await.unwrap()
}

/// Welcome 里拒绝的原因（没拒绝是空的）
pub fn refused_code(w: &Welcome) -> &str {
    w.refused.as_ref().map(|r| r.code.as_str()).unwrap_or("")
}

/// 今天往后（负数往前）第 n 天，`YYYY-MM-DD`
pub fn day_from_today(n: i64) -> String {
    (chrono::Local::now().date_naive() + chrono::TimeDelta::days(n)).format("%Y-%m-%d").to_string()
}
