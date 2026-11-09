//! 叮咪同步（收费的官方同步）的服务器：写死在电脑里（需求-1.0 第 31.3 节）。
//! 「用兑换码开通」「用恢复码找回」时用这里的地址和公钥连上去；开通以后和别的服务器一样存在 kv 的 "sync" 里。
//!
//! 服务器公钥是 Noise 静态公钥，和配对码里的那一串一样，在官方服务器的数据目录里（`dingmi-server.db` 的 kv
//! `noise_public`），搬家不换。作者部署好官方服务器后，在服务器上执行 `dingmi-server pubkey`，把打出来的那一行
//! 填进下面的 [`SERVER_PUBLIC`]，再发版。没填的时候（空的）叮咪同步的几个按钮会说「还没接上」，不会乱连。

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;

/// 叮咪同步的域名（官方服务器 `hosts` 里的同步域名，默认就是它；那边换了这里要跟着改、发版）
pub const HOST: &str = "sync.dingmi.app";
/// 存进配置、配对码里的端口：443。官方服务器只听本机，外面只有宝塔的 443，电脑只走下面的 WEB，
/// 不直连（需求-1.x 第 36.1 节）。以前存的是 7788（那时还直连），连的时候一样当 443（见 mod.rs 的 domain_web）
pub const PORT: u16 = 443;
/// 走 `wss://sync.dingmi.app/sync`（宝塔在 443 上转给官方服务器，需求-1.0 第 31.2 节 A）
pub const WEB: &str = "https://sync.dingmi.app";
/// 官方服务器的 Noise 公钥（base64url，43 个字符），就是官方服务器上 `dingmi-server pubkey` 打出来的那一行
pub const SERVER_PUBLIC: &str = "EYGe3u7jt_w1ZtswpVJDnkyHlfSufgWw53nmH-XZFjo";

/// 写死的公钥；还没填（或者填错了）时说清楚
pub fn server_public() -> Result<Vec<u8>, String> {
    let k = B64.decode(SERVER_PUBLIC.trim()).unwrap_or_default();
    if k.len() == 32 {
        Ok(k)
    } else {
        Err("这一版叮咪还没接上叮咪同步的服务器，等下一版再试".into())
    }
}

/// 这台电脑连的是不是叮咪同步（看地址）
pub fn is_official(host: &str) -> bool {
    host.eq_ignore_ascii_case(HOST)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_is_explained() {
        // 公钥还是占位（空的）时不连，给一句人话；填了 32 字节的就能用
        if SERVER_PUBLIC.is_empty() {
            assert!(server_public().unwrap_err().contains("叮咪同步"));
        } else {
            assert_eq!(server_public().unwrap().len(), 32);
        }
        assert!(is_official("SYNC.dingmi.app") && !is_official("1.2.3.4"));
    }
}
