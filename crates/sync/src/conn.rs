//! Noise 加密连接：握手、分帧、长消息分块。
//!
//! 线上的格式：每条 Noise 消息前面加 2 字节长度（大端）。Noise 单条消息最长 65535 字节，
//! 所以业务消息按块发送，每块明文的第一个字节表示后面还有没有（1 有 / 0 这是最后一块）。

use crate::{Error, Hello, Result, Welcome, MAGIC, NOISE_PARAMS};
use serde::{de::DeserializeOwned, Serialize};
use snow::{Builder, HandshakeState, TransportState};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const NOISE_MAX: usize = 65535;
const TAG_LEN: usize = 16;
/// 每块业务数据的大小，加上 1 字节标记和 16 字节认证码后不超过 Noise 上限
const CHUNK: usize = 60 * 1024;
/// 单条业务消息的上限，防止对方发来超大消息占满内存
const MAX_MSG: usize = 8 * 1024 * 1024;

fn builder() -> Builder<'static> {
    Builder::new(NOISE_PARAMS.parse().expect("NOISE_PARAMS 写错了"))
}

/// 生成一对 Noise 密钥：(私钥, 公钥)
pub fn generate_keypair() -> Result<(Vec<u8>, Vec<u8>)> {
    let kp = builder().generate_keypair()?;
    Ok((kp.private, kp.public))
}

async fn read_frame<S: AsyncRead + Unpin>(io: &mut S) -> Result<Vec<u8>> {
    let len = io.read_u16().await? as usize;
    let mut buf = vec![0u8; len];
    io.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn write_frame<S: AsyncWrite + Unpin>(io: &mut S, data: &[u8]) -> Result<()> {
    io.write_u16(data.len() as u16).await?;
    io.write_all(data).await?;
    Ok(())
}

/// 电脑端：连上服务器、完成握手。服务器不认这台设备时会直接断开，这里返回 Io 错误。
pub async fn connect<S: AsyncRead + AsyncWrite + Unpin>(
    mut io: S,
    local_private: &[u8],
    server_public: &[u8],
    hello: &Hello,
) -> Result<(NoiseConn<S>, Welcome)> {
    let mut hs = builder()
        .local_private_key(local_private)
        .remote_public_key(server_public)
        .build_initiator()?;
    let mut buf = vec![0u8; NOISE_MAX];
    let n = hs.write_message(&serde_json::to_vec(hello)?, &mut buf)?;
    io.write_all(MAGIC).await?;
    write_frame(&mut io, &buf[..n]).await?;
    io.flush().await?;
    let frame = read_frame(&mut io).await?;
    let n = hs.read_message(&frame, &mut buf)?;
    let welcome = serde_json::from_slice(&buf[..n])?;
    Ok((NoiseConn::new(io, hs.into_transport_mode()?), welcome))
}

/// 服务端：读完握手第一条消息、还没回复的连接。看过 `remote`（设备公钥）和 `hello` 决定收不收；
/// 不收就直接丢掉，对方什么也收不到。
pub struct Pending<S> {
    io: S,
    hs: HandshakeState,
    pub remote: Vec<u8>,
    pub hello: Hello,
}

/// 服务端：`MAGIC` 已经被读走之后调用
pub async fn accept<S: AsyncRead + AsyncWrite + Unpin>(mut io: S, local_private: &[u8]) -> Result<Pending<S>> {
    let mut hs = builder().local_private_key(local_private).build_responder()?;
    let frame = read_frame(&mut io).await?;
    let mut buf = vec![0u8; NOISE_MAX];
    let n = hs.read_message(&frame, &mut buf)?;
    let hello = serde_json::from_slice(&buf[..n])?;
    let remote = hs.get_remote_static().ok_or(Error::Protocol("握手里没有设备公钥"))?.to_vec();
    Ok(Pending { io, hs, remote, hello })
}

impl<S: AsyncRead + AsyncWrite + Unpin> Pending<S> {
    pub async fn welcome(mut self, w: &Welcome) -> Result<NoiseConn<S>> {
        let mut buf = vec![0u8; NOISE_MAX];
        let n = self.hs.write_message(&serde_json::to_vec(w)?, &mut buf)?;
        write_frame(&mut self.io, &buf[..n]).await?;
        self.io.flush().await?;
        Ok(NoiseConn::new(self.io, self.hs.into_transport_mode()?))
    }
}

/// 握手完成后的加密连接
pub struct NoiseConn<S> {
    io: S,
    noise: TransportState,
    out: Vec<u8>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> NoiseConn<S> {
    fn new(io: S, noise: TransportState) -> Self {
        NoiseConn { io, noise, out: vec![0u8; NOISE_MAX] }
    }

    /// 对方的公钥
    pub fn remote(&self) -> Option<&[u8]> {
        self.noise.get_remote_static()
    }

    pub async fn send<T: Serialize>(&mut self, msg: &T) -> Result<()> {
        let data = serde_json::to_vec(msg)?;
        let count = data.len().div_ceil(CHUNK).max(1);
        for i in 0..count {
            let part = &data[(i * CHUNK).min(data.len())..((i + 1) * CHUNK).min(data.len())];
            let mut plain = Vec::with_capacity(part.len() + 1);
            plain.push((i + 1 < count) as u8);
            plain.extend_from_slice(part);
            let n = self.noise.write_message(&plain, &mut self.out)?;
            write_frame(&mut self.io, &self.out[..n]).await?;
        }
        self.io.flush().await?;
        Ok(())
    }

    pub async fn recv<T: DeserializeOwned>(&mut self) -> Result<T> {
        let mut data = Vec::new();
        let mut plain = vec![0u8; NOISE_MAX];
        loop {
            let frame = read_frame(&mut self.io).await?;
            if frame.len() < TAG_LEN + 1 {
                return Err(Error::Protocol("收到的数据块太短"));
            }
            let n = self.noise.read_message(&frame, &mut plain)?;
            data.extend_from_slice(&plain[1..n]);
            if data.len() > MAX_MSG {
                return Err(Error::Protocol("消息太大"));
            }
            if plain[0] == 0 {
                break;
            }
        }
        Ok(serde_json::from_slice(&data)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Msg, Space};

    #[tokio::test]
    async fn handshake_and_messages() {
        let (server_priv, server_pub) = generate_keypair().unwrap();
        let (dev_priv, dev_pub) = generate_keypair().unwrap();
        let (a, mut b) = tokio::io::duplex(1 << 20);

        let server = tokio::spawn(async move {
            let mut magic = [0u8; 4];
            b.read_exact(&mut magic).await.unwrap();
            assert_eq!(&magic, MAGIC);
            let p = accept(b, &server_priv).await.unwrap();
            assert_eq!(p.remote, dev_pub);
            assert_eq!(p.hello.name, "公司电脑");
            let w = Welcome { version: 1, space: Space { salt: Some("s".into()), ..Default::default() }, ..Default::default() };
            let mut conn = p.welcome(&w).await.unwrap();
            let m: Msg = conn.recv().await.unwrap();
            assert_eq!(m, Msg::Ping);
            conn.send(&Msg::Pong).await.unwrap();
            // 超过一块的长消息
            let big: String = conn.recv().await.unwrap();
            conn.send(&big).await.unwrap();
        });

        let hello = Hello { version: 1, name: "公司电脑".into(), ..Default::default() };
        let (mut conn, welcome) = connect(a, &dev_priv, &server_pub, &hello).await.unwrap();
        assert_eq!(welcome.space.salt.as_deref(), Some("s"));
        conn.send(&Msg::Ping).await.unwrap();
        assert_eq!(conn.recv::<Msg>().await.unwrap(), Msg::Pong);
        let big = "嘿".repeat(100_000);
        conn.send(&big).await.unwrap();
        assert_eq!(conn.recv::<String>().await.unwrap(), big);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn wrong_server_key_fails() {
        let (server_priv, _) = generate_keypair().unwrap();
        let (_, other_pub) = generate_keypair().unwrap();
        let (dev_priv, _) = generate_keypair().unwrap();
        let (a, mut b) = tokio::io::duplex(1 << 16);
        let server = tokio::spawn(async move {
            let mut magic = [0u8; 4];
            b.read_exact(&mut magic).await.unwrap();
            // 设备以为的服务器公钥不对，服务器解不开第一条消息
            assert!(accept(b, &server_priv).await.is_err());
        });
        assert!(connect(a, &dev_priv, &other_pub, &Hello::default()).await.is_err());
        server.await.unwrap();
    }
}
