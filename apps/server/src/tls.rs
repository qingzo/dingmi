//! 自签证书：一张私有根证书（10 年），再用它给服务器 IP 签发 HTTPS 证书（1 年，到期前 30 天自动换新）。
//! iPhone 上安装并信任一次根证书，手机网页就能正常走 HTTPS；不需要域名，也就不涉及备案。
//!
//! 根证书可以带「名称约束」（NameConstraints，`ca --new` 生成）：只能给这台服务器的 IP 签证书，
//! 根证书私钥万一泄露，别人也拿它冒充不了别的网站。init 默认生成的还是不带约束的（装好的手机照旧能用），见部署文档第 6 节。

use crate::store::Store;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use rcgen::{
    BasicConstraints, CertificateParams, CidrSubnet, DistinguishedName, DnType, ExtendedKeyUsagePurpose, GeneralSubtree,
    IsCa, KeyPair, KeyUsagePurpose, NameConstraints, SanType,
};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::ServerConfig;
use sha2::{Digest, Sha256};
use std::fs;
use std::net::IpAddr;
use std::path::Path;
use std::sync::{Arc, RwLock};
use time::{Duration, OffsetDateTime};

const CA_CERT: &str = "ca.crt.pem";
const CA_KEY: &str = "ca.key.pem";
const SERVER_CERT: &str = "server.crt.pem";
const SERVER_KEY: &str = "server.key.pem";
const CA_DAYS: i64 = 3650;
/// 苹果要求 TLS 证书有效期不超过 825 天
const SERVER_DAYS: i64 = 365;
const RENEW_BEFORE_DAYS: i64 = 30;

type Res<T> = Result<T, String>;

fn e<E: std::fmt::Display>(err: E) -> String {
    err.to_string()
}

/// 写私钥文件，只有自己能读
pub fn write_private(path: &Path, data: &str) -> Res<()> {
    fs::write(path, data).map_err(e)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(e)?;
    }
    Ok(())
}

fn validity(p: &mut CertificateParams, days: i64) {
    let now = OffsetDateTime::now_utc();
    // 往前留一天，避免手机时间稍慢时证书"还没生效"
    p.not_before = now - Duration::days(1);
    p.not_after = now + Duration::days(days);
}

/// 带约束的根证书里放的「域名」：不存在的（.invalid 是保留的），等于不许签任何域名
const NO_DNS: &str = "dingmi.invalid";

/// 名称约束：只许签 `ip` 这一个地址（RFC 5280「Name Constraints」那一节，rcgen 写成 critical，RFC 要求的）。
/// 同一类名字只要有一条约束，这一类的名字就都得在里面：所以再放一个不存在的域名，域名就一个也签不了
fn only(ip: IpAddr) -> NameConstraints {
    let full = if ip.is_ipv4() { 32 } else { 128 };
    NameConstraints {
        permitted_subtrees: vec![GeneralSubtree::IpAddress(CidrSubnet::from_addr_prefix(ip, full)), GeneralSubtree::DnsName(NO_DNS.into())],
        excluded_subtrees: vec![],
    }
}

/// 没有根证书就生成一张（不带约束）；已经有了什么都不做（装好的手机照旧能用）
pub fn init_ca(dir: &Path) -> Res<()> {
    if dir.join(CA_CERT).exists() && dir.join(CA_KEY).exists() {
        return Ok(());
    }
    new_ca(dir, None)
}

/// 生成一张根证书，`limit` 是只许签哪个 IP（None 不限）
fn new_ca(dir: &Path, limit: Option<IpAddr>) -> Res<()> {
    let mut p = CertificateParams::new(Vec::<String>::new()).map_err(e)?;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "dingmi 私有根证书");
    dn.push(DnType::OrganizationName, "dingmi");
    p.distinguished_name = dn;
    p.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign, KeyUsagePurpose::DigitalSignature];
    p.name_constraints = limit.map(only);
    validity(&mut p, CA_DAYS);
    let key = KeyPair::generate().map_err(e)?;
    let cert = p.self_signed(&key).map_err(e)?;
    write_private(&dir.join(CA_KEY), &key.serialize_pem())?;
    fs::write(dir.join(CA_CERT), cert.pem()).map_err(e)?;
    Ok(())
}

/// 换一张只认 `ip` 的根证书（`ca --new`）：旧的两个文件改名留着（返回加的后缀），服务器证书下次启动时用新的重签。
/// 换了以后装过旧根证书的手机要重装（绑了域名的手机不用装证书，不受影响）
pub fn renew_ca(dir: &Path, ip: IpAddr, store: &Store) -> Res<String> {
    let tag = format!(".旧-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"));
    for f in [CA_CERT, CA_KEY] {
        if dir.join(f).exists() {
            fs::rename(dir.join(f), dir.join(format!("{f}{tag}"))).map_err(e)?;
        }
    }
    new_ca(dir, Some(ip))?;
    store.kv_set("server_cert_until", "0").map_err(e)?;
    Ok(tag)
}

/// 根证书带约束的话，它只许签哪些 IP；不带约束是 None
pub fn ca_limit(dir: &Path) -> Option<Vec<IpAddr>> {
    let p = CertificateParams::from_ca_cert_pem(&fs::read_to_string(dir.join(CA_CERT)).ok()?).ok()?;
    let nc = p.name_constraints?;
    let ip = |c: &CidrSubnet| match c {
        CidrSubnet::V4(a, _) => IpAddr::from(*a),
        CidrSubnet::V6(a, _) => IpAddr::from(*a),
    };
    Some(nc.permitted_subtrees.iter().filter_map(|t| if let GeneralSubtree::IpAddress(c) = t { Some(ip(c)) } else { None }).collect())
}

pub fn ca_der(dir: &Path) -> Res<Vec<u8>> {
    CertificateDer::from_pem_file(dir.join(CA_CERT))
        .map(|c| c.to_vec())
        .map_err(|err| format!("读不了根证书：{err}"))
}

/// 用根证书给 `ip` 签一张服务器证书，返回到期时间
fn issue_server(dir: &Path, ip: IpAddr) -> Res<OffsetDateTime> {
    let ca_key = KeyPair::from_pem(&fs::read_to_string(dir.join(CA_KEY)).map_err(e)?).map_err(e)?;
    let ca_params = CertificateParams::from_ca_cert_pem(&fs::read_to_string(dir.join(CA_CERT)).map_err(e)?).map_err(e)?;
    let limited = ca_params.name_constraints.is_some();
    let ca = ca_params.self_signed(&ca_key).map_err(e)?;
    let mut p = CertificateParams::new(Vec::<String>::new()).map_err(e)?;
    let mut dn = DistinguishedName::new();
    // 带约束的根证书签的：CN 不写成 IP 的样子（带空格），有的老实现没有域名的 SAN 时会把像域名的 CN 当域名去对约束。
    // 真正认地址的是 SAN 里的 IP，苹果和浏览器早就不看 CN 了
    dn.push(DnType::CommonName, if limited { format!("dingmi {ip}") } else { ip.to_string() });
    p.distinguished_name = dn;
    p.subject_alt_names = vec![SanType::IpAddress(ip)];
    p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    validity(&mut p, SERVER_DAYS);
    let not_after = p.not_after;
    let key = KeyPair::generate().map_err(e)?;
    let cert = p.signed_by(&key, &ca, &ca_key).map_err(e)?;
    write_private(&dir.join(SERVER_KEY), &key.serialize_pem())?;
    fs::write(dir.join(SERVER_CERT), cert.pem()).map_err(e)?;
    Ok(not_after)
}

/// 服务器证书不存在、IP 变了或者快到期了，就签一张新的；签了返回 true
fn ensure_server(dir: &Path, ip: IpAddr, store: &Store) -> Res<bool> {
    let ip_s = ip.to_string();
    let renew_at = store
        .kv_get("server_cert_until")
        .and_then(|s| s.parse::<i64>().ok())
        .map(|t| t - RENEW_BEFORE_DAYS * 86400);
    let ok = dir.join(SERVER_CERT).exists()
        && dir.join(SERVER_KEY).exists()
        && store.kv_get("server_cert_ip").as_deref() == Some(ip_s.as_str())
        && renew_at.is_some_and(|t| t > OffsetDateTime::now_utc().unix_timestamp());
    if ok {
        return Ok(false);
    }
    let until = issue_server(dir, ip)?;
    store.kv_set("server_cert_ip", &ip_s).map_err(e)?;
    store.kv_set("server_cert_until", &until.unix_timestamp().to_string()).map_err(e)?;
    Ok(true)
}

fn build_config(dir: &Path) -> Res<Arc<ServerConfig>> {
    let chain = vec![
        CertificateDer::from_pem_file(dir.join(SERVER_CERT)).map_err(e)?,
        CertificateDer::from_pem_file(dir.join(CA_CERT)).map_err(e)?,
    ];
    let key = PrivateKeyDer::from_pem_file(dir.join(SERVER_KEY)).map_err(e)?;
    let cfg = ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(e)?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(e)?;
    Ok(Arc::new(cfg))
}

/// 当前用的 HTTPS 配置；换证书时整个替换掉，已经建立的连接不受影响
pub struct Tls {
    config: RwLock<Arc<ServerConfig>>,
}

impl Tls {
    pub fn load(dir: &Path, ip: IpAddr, store: &Store) -> Res<Tls> {
        ensure_server(dir, ip, store)?;
        Ok(Tls { config: RwLock::new(build_config(dir)?) })
    }

    pub fn config(&self) -> Arc<ServerConfig> {
        self.config.read().unwrap().clone()
    }

    pub fn renew(&self, dir: &Path, ip: IpAddr, store: &Store) -> Res<bool> {
        if !ensure_server(dir, ip, store)? {
            return Ok(false);
        }
        *self.config.write().unwrap() = build_config(dir)?;
        Ok(true)
    }
}

/// 根证书的 SHA-256 指纹，安装时在 iPhone 上核对
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

/// 由内容算出一个固定的 UUID，同一张根证书每次生成的描述文件都一样
fn uuid_from(der: &[u8], salt: &str) -> String {
    let h: String = Sha256::new()
        .chain_update(der)
        .chain_update(salt.as_bytes())
        .finalize()
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// iPhone 的描述文件（.mobileconfig），里面装着根证书
pub fn mobileconfig(ca_der: &[u8]) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>PayloadContent</key>
  <array>
    <dict>
      <key>PayloadCertificateFileName</key><string>dingmi-ca.cer</string>
      <key>PayloadContent</key><data>{cert}</data>
      <key>PayloadDescription</key><string>让这部手机信任你自己的叮咪服务器</string>
      <key>PayloadDisplayName</key><string>叮咪私有根证书</string>
      <key>PayloadIdentifier</key><string>app.dingmi.ca.{cert_uuid}</string>
      <key>PayloadType</key><string>com.apple.security.root</string>
      <key>PayloadUUID</key><string>{cert_uuid}</string>
      <key>PayloadVersion</key><integer>1</integer>
    </dict>
  </array>
  <key>PayloadDescription</key><string>安装后，到 设置 → 通用 → 关于本机 → 证书信任设置 里打开对叮咪私有根证书的完全信任。</string>
  <key>PayloadDisplayName</key><string>叮咪服务器证书</string>
  <key>PayloadIdentifier</key><string>app.dingmi.profile</string>
  <key>PayloadType</key><string>Configuration</string>
  <key>PayloadUUID</key><string>{profile_uuid}</string>
  <key>PayloadVersion</key><integer>1</integer>
</dict>
</plist>
"#,
        cert = B64.encode(ca_der),
        cert_uuid = uuid_from(ca_der, "cert"),
        profile_uuid = uuid_from(ca_der, "profile"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::client::danger::ServerCertVerifier;
    use rustls::client::WebPkiServerVerifier;
    use rustls::pki_types::{ServerName, UnixTime};

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("dingmi-tls-{name}-{}", rand::random::<u64>()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// 拿根证书私钥签一张给 `san` 的证书（拿到私钥的人能做的事）
    fn forge(dir: &Path, san: SanType) -> CertificateDer<'static> {
        let ca_key = KeyPair::from_pem(&fs::read_to_string(dir.join(CA_KEY)).unwrap()).unwrap();
        let ca = CertificateParams::from_ca_cert_pem(&fs::read_to_string(dir.join(CA_CERT)).unwrap()).unwrap().self_signed(&ca_key).unwrap();
        let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
        p.subject_alt_names = vec![san];
        p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        validity(&mut p, 30);
        p.signed_by(&KeyPair::generate().unwrap(), &ca, &ca_key).unwrap().der().clone()
    }

    /// 信任了这张根证书的手机认不认 `cert`（当成 `name` 的证书）
    fn trusted(dir: &Path, cert: &CertificateDer<'_>, name: &str) -> bool {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(CertificateDer::from(ca_der(dir).unwrap())).unwrap();
        let v = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), Arc::new(rustls::crypto::ring::default_provider()))
            .build()
            .unwrap();
        let name = ServerName::try_from(name.to_string()).unwrap();
        v.verify_server_cert(cert, &[], &name, &[], UnixTime::now()).is_ok()
    }

    fn server_cert(dir: &Path) -> CertificateDer<'static> {
        CertificateDer::from_pem_file(dir.join(SERVER_CERT)).unwrap()
    }

    /// 攻击：根证书私钥泄露，拿它给别的网站签证书。不带约束的根证书（以前的、init 默认的）手机照单全收；
    /// 带约束的（ca --new）只认这台服务器的 IP，别的 IP、任何域名都不认
    #[test]
    fn name_constraints_stop_forged_certs() {
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        let evil_dns = || SanType::DnsName("bank.example.com".try_into().unwrap());
        let evil_ip = || SanType::IpAddress("10.0.0.1".parse().unwrap());

        let old = tmp("plain");
        init_ca(&old).unwrap();
        assert_eq!(ca_limit(&old), None);
        issue_server(&old, ip).unwrap();
        assert!(trusted(&old, &server_cert(&old), "127.0.0.1"));
        assert!(trusted(&old, &forge(&old, evil_dns()), "bank.example.com"), "不带约束：什么都能签");
        assert!(trusted(&old, &forge(&old, evil_ip()), "10.0.0.1"));

        let new = tmp("limited");
        new_ca(&new, Some(ip)).unwrap();
        assert_eq!(ca_limit(&new), Some(vec![ip]));
        issue_server(&new, ip).unwrap();
        assert!(trusted(&new, &server_cert(&new), "127.0.0.1"), "这台服务器的证书照样认");
        assert!(!trusted(&new, &forge(&new, evil_dns()), "bank.example.com"), "域名一个也签不了");
        assert!(!trusted(&new, &forge(&new, evil_ip()), "10.0.0.1"), "别的 IP 也不行");
        let v6 = tmp("limited6");
        new_ca(&v6, Some("::1".parse().unwrap())).unwrap();
        assert!(!trusted(&v6, &forge(&v6, evil_ip()), "10.0.0.1"));
        assert!(trusted(&v6, &forge(&v6, SanType::IpAddress("::1".parse().unwrap())), "::1"));
    }

    /// 已经有根证书的不重新生成（装好的手机不能失效）；ca --new 换一张带约束的，旧的改名留着，服务器证书重签
    #[test]
    fn existing_ca_is_kept_and_renew_keeps_old_files() {
        let dir = tmp("renew");
        crate::init(&dir, "127.0.0.1".parse().unwrap(), 7788).unwrap();
        let before = ca_der(&dir).unwrap();
        crate::init(&dir, "127.0.0.2".parse().unwrap(), 7788).unwrap();
        assert_eq!(ca_der(&dir).unwrap(), before, "再 init 一次（换了 IP）根证书不变");
        let store = Store::open(&dir).unwrap();
        Tls::load(&dir, "127.0.0.1".parse().unwrap(), &store).unwrap();
        let tag = renew_ca(&dir, "127.0.0.1".parse().unwrap(), &store).unwrap();
        assert_ne!(ca_der(&dir).unwrap(), before);
        assert_eq!(fs::read(dir.join(format!("{CA_CERT}{tag}"))).map(|b| !b.is_empty()).ok(), Some(true));
        assert!(dir.join(format!("{CA_KEY}{tag}")).exists());
        // 下次启动重签服务器证书，新的根证书认它
        assert!(ensure_server(&dir, "127.0.0.1".parse().unwrap(), &store).unwrap());
        assert!(trusted(&dir, &server_cert(&dir), "127.0.0.1"));
    }
}
