//! 配对码：`dingmi://<IP>:<端口>/<服务器公钥>/<邀请码>[/<空间密钥>]`。
//! 服务器公钥、空间密钥用 base64url（不带填充）；邀请码一次有效，服务器只存它的哈希。
//! 电脑上生成的配对码带空间密钥；服务器上 `dingmi-server invite` 生成的不带（服务器没有空间密钥）。

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use std::fmt;
use std::str::FromStr;

const PREFIX: &str = "dingmi://";

#[derive(Debug, Clone, PartialEq)]
pub struct PairCode {
    pub host: String,
    pub port: u16,
    pub server_public: Vec<u8>,
    pub invite: String,
    /// 空间密钥（32 字节），只有电脑上生成的配对码才有
    pub space_key: Option<Vec<u8>>,
}

impl fmt::Display for PairCode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{PREFIX}{}:{}/{}/{}", self.host, self.port, B64.encode(&self.server_public), self.invite)?;
        match &self.space_key {
            Some(k) => write!(f, "/{}", B64.encode(k)),
            None => Ok(()),
        }
    }
}

impl FromStr for PairCode {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        const BAD: &str = "配对码不对，请完整复制服务器上输出的那一行";
        let s = s.trim();
        let rest = s.strip_prefix(PREFIX).ok_or(BAD)?;
        let mut parts = rest.split('/');
        let (addr, key, invite) = (parts.next().ok_or(BAD)?, parts.next().ok_or(BAD)?, parts.next().ok_or(BAD)?);
        let space_key = match parts.next() {
            Some(k) => Some(B64.decode(k).ok().filter(|k| k.len() == 32).ok_or(BAD)?),
            None => None,
        };
        if parts.next().is_some() || invite.is_empty() {
            return Err(BAD);
        }
        let (host, port) = addr.rsplit_once(':').ok_or(BAD)?;
        let server_public = B64.decode(key).map_err(|_| BAD)?;
        if host.is_empty() || server_public.len() != 32 {
            return Err(BAD);
        }
        Ok(PairCode {
            host: host.into(),
            port: port.parse().map_err(|_| BAD)?,
            server_public,
            invite: invite.into(),
            space_key,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut p =
            PairCode { host: "1.2.3.4".into(), port: 7788, server_public: vec![7; 32], invite: "abc_DEF-1".into(), space_key: None };
        let s = p.to_string();
        assert!(s.starts_with("dingmi://1.2.3.4:7788/"));
        assert_eq!(s.parse::<PairCode>().unwrap(), p);
        assert_eq!(format!("  {s}\n").parse::<PairCode>().unwrap(), p);
        // 电脑上生成的带空间密钥
        p.space_key = Some(vec![9; 32]);
        let s = p.to_string();
        assert_eq!(s.matches('/').count(), 5);
        assert_eq!(s.parse::<PairCode>().unwrap(), p);
    }

    #[test]
    fn rejects_garbage() {
        for s in ["", "http://x", "other://1.2.3.4:7788/AAAA/inv", "dingmi://1.2.3.4/abc/def", "dingmi://1.2.3.4:77/short/inv", "dingmi://1.2.3.4:x/AAAA/inv", "dingmi://1.2.3.4:7788/BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc/inv/short"] {
            assert!(s.parse::<PairCode>().is_err(), "{s}");
        }
    }
}
