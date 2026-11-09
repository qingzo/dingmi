//! 电脑和服务器之间的底层通道。Noise 加密连接（dingmi_sync::NoiseConn）不挑底下是什么：
//! 直连 TCP（`IP:端口`），或者 `wss://域名/sync`（需求-1.0 第 31.2 节 A），都包成同一个 [`Stream`]。
//!
//! 服务器绑了域名（Welcome.web_url，宝塔 / Nginx 反向代理过去的那个）时先走 wss：只放行 443 的网络也连得上，
//! 服务器搬家换了 IP 只要改 DNS，电脑不用重新配对。连不上再退回配对码里的 `IP:端口`。
//! 叮咪同步的地址本身就是域名（`sync.dingmi.app:443`）：只走 wss，不退回（官方服务器不对外开端口，见 mod.rs 的 domain_web）。

use dingmi_sync::{ws, NoiseConn};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// 能读能写的通道
pub trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

pub type Stream = Box<dyn Io>;
/// 握手之后的加密连接
pub type Conn = NoiseConn<Stream>;

/// 直连配对码里的地址
pub async fn direct(host: &str, port: u16, wait: Duration) -> Result<Stream, String> {
    let tcp = timeout(wait, TcpStream::connect((host, port)))
        .await
        .map_err(|_| "连不上服务器：超时了，检查一下服务器地址、端口和防火墙".to_string())?
        .map_err(|e| format!("连不上服务器：{e}"))?;
    let _ = tcp.set_nodelay(true);
    Ok(Box::new(tcp))
}

/// `https://sync.example.com`（或者带端口）→ (域名, 端口)；不是 https 的不走 wss
pub fn web_host(web: &str) -> Option<(String, u16)> {
    let rest = web.trim().trim_end_matches('/').strip_prefix("https://")?;
    let host = rest.split('/').next()?;
    let (name, port) = match host.rsplit_once(':') {
        Some((n, p)) => (n, p.parse().ok()?),
        None => (host, 443),
    };
    (!name.is_empty()).then(|| (name.to_ascii_lowercase(), port))
}

fn tls() -> Arc<rustls::ClientConfig> {
    static CFG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CFG.get_or_init(|| {
        let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
        let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("TLS 默认版本")
            .with_root_certificates(roots)
            .with_no_client_auth();
        Arc::new(cfg)
    })
    .clone()
}

/// `wss://域名/sync`：TCP → TLS（正规证书，按系统常用的根证书核对）→ WebSocket 握手
pub async fn wss(web: &str, wait: Duration) -> Result<Stream, String> {
    let (host, port) = web_host(web).ok_or_else(|| format!("服务器网址不对：{web}"))?;
    let go = async {
        let tcp = TcpStream::connect((host.as_str(), port)).await.map_err(|e| format!("连不上 {host}：{e}"))?;
        let _ = tcp.set_nodelay(true);
        let name = rustls::pki_types::ServerName::try_from(host.clone()).map_err(|_| format!("域名不对：{host}"))?;
        let mut tls = tokio_rustls::TlsConnector::from(tls()).connect(name, tcp).await.map_err(|e| format!("{host} 的证书有问题：{e}"))?;
        ws::client_handshake(&mut tls, &host).await.map_err(|e| format!("{host} 没有转发同步连接：{e}"))?;
        Ok::<Stream, String>(Box::new(ws::WsStream::client(tls)))
    };
    timeout(wait, go).await.map_err(|_| format!("连 {host} 超时了"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_host_parsing() {
        assert_eq!(web_host("https://sync.example.com"), Some(("sync.example.com".into(), 443)));
        assert_eq!(web_host("https://Sync.Example.com:8443/"), Some(("sync.example.com".into(), 8443)));
        assert_eq!(web_host("https://sync.example.com/m/"), Some(("sync.example.com".into(), 443)));
        assert_eq!(web_host("http://sync.example.com"), None);
        assert_eq!(web_host(""), None);
        assert_eq!(web_host("https://:443"), None);
    }

    /// 域名连不上要很快失败（好退回直连），直连照常
    #[tokio::test]
    async fn wss_fails_fast_direct_works() {
        let t = std::time::Instant::now();
        let e = wss("https://nothing.invalid", Duration::from_secs(6)).await.err().unwrap();
        assert!(e.contains("nothing.invalid"), "{e}");
        assert!(t.elapsed() < Duration::from_secs(6));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(direct("127.0.0.1", port, Duration::from_secs(3)).await.is_ok());
    }

    /// 连一个真的、绑了域名的 dingmi-server，只看 wss 通道能不能建起来（平时跳过）：
    /// DINGMI_TEST_WEB=https://sync.example.com cargo test --lib -- --ignored wss_real_server
    #[tokio::test]
    #[ignore]
    async fn wss_real_server() {
        let web = std::env::var("DINGMI_TEST_WEB").expect("要设 DINGMI_TEST_WEB");
        wss(&web, Duration::from_secs(10)).await.unwrap();
    }
}
