//! dingmi-server 的代码：命令行（main.rs → cli.rs）用它。管理操作都在 admin.rs。
//!
//! 开源的这一份只管「自己搭、自己同步」（需求-1.0 第 32.1 节第 12 条）。别人要在上面加东西
//! （比如叮咪同步的兑换码、到期），用 ext.rs 的扩展点，编译时换上自己的实现。

pub mod admin;
pub mod cli;
pub mod ext;
pub mod http;
pub mod net;
pub mod store;
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;
pub mod tls;

/// 服务器的版本号（跟着电脑端一起发版）。官方版的日志也用这个，不用它自己那个包的版本
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use std::error::Error;
use std::net::IpAddr;
use std::path::Path;
use store::Store;

/// 生成（或保留已有的）服务器密钥和根证书，记下 IP 和端口
pub fn init(dir: &Path, ip: IpAddr, port: u16) -> Result<(), Box<dyn Error>> {
    let store = init_keys(dir)?;
    store.kv_set("ip", &ip.to_string())?;
    store.kv_set("port", &port.to_string())?;
    tls::init_ca(dir)?;
    Ok(())
}

/// 建数据目录（只有自己能读）、数据库，生成（或保留已有的）服务器密钥。
/// 开源版的 init 在这之后还要记 IP、端口、生成根证书；官方版在宝塔后面，只要这一步
pub fn init_keys(dir: &Path) -> Result<Store, Box<dyn Error>> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let store = Store::open(dir)?;
    if store.kv_get("noise_private").is_none() {
        let (private, public) = dingmi_sync::generate_keypair()?;
        store.kv_set("noise_private", &B64.encode(private))?;
        store.kv_set("noise_public", &B64.encode(public))?;
    }
    Ok(store)
}
