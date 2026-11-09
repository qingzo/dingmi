//! 把电脑和服务器之间的字节流包进 WebSocket（RFC 6455），好让它走 `wss://域名/sync`：
//! 宝塔、Nginx、Cloudflare 都能转 WebSocket，只放行 443 的网络也连得上（需求-1.0 第 31.2 节 A）。
//!
//! 只是外面多一层包装：里面跑的还是原来的 Noise 加密连接（`MAGIC` + 握手 + 加密消息），内容一个字节不变。
//! 所以这里只做最少的：二进制帧、分片、Ping 回 Pong、Close；不做文本帧、压缩扩展。

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use sha1::{Digest, Sha1};
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};

/// 电脑端连的路径
pub const PATH: &str = "/sync";
/// 一帧最大多少（Noise 一条消息最多 64 KB，留足余量），超过当成坏数据
const MAX_FRAME: u64 = 1 << 20;
/// 写的时候每帧最多放多少
const WRITE_CHUNK: usize = 64 * 1024;
/// 控制帧（Close、Ping、Pong）最多带多少字节（RFC 6455 第 5.5 节），超过当成坏数据
const MAX_CONTROL: u64 = 125;
/// `out` 里最多攒多少还没写出去的：一帧数据（WRITE_CHUNK 加帧头）加一个 Pong、一个 Close 就够了。
/// 对方光发 Ping 不收的话，`out` 也不会越攒越多（只留最后一个 Pong），这是兜底：超了就断开
const MAX_OUT: usize = WRITE_CHUNK + 1024;
const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// 服务器回给浏览器 / 电脑的 `Sec-WebSocket-Accept`
pub fn accept_key(key: &str) -> String {
    let mut h = Sha1::new();
    h.update(key.trim().as_bytes());
    h.update(GUID.as_bytes());
    B64.encode(h.finalize())
}

/// 电脑端：在已经连好的通道（一般是 TLS）上发 WebSocket 握手，服务器回 101 才算成
pub async fn client_handshake<S: AsyncRead + AsyncWrite + Unpin>(io: &mut S, host: &str) -> io::Result<()> {
    let mut seed = seed();
    let key = B64.encode((0..16).map(|_| next(&mut seed) as u8).collect::<Vec<_>>());
    let req = format!(
        "GET {PATH} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nUser-Agent: Dingmi\r\n\r\n"
    );
    io.write_all(req.as_bytes()).await?;
    io.flush().await?;
    // 一个字节一个字节读到空行为止，不多读：后面紧跟着的就是 WebSocket 帧
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 8192 {
            return Err(bad("服务器回的头太长了"));
        }
        let mut b = [0u8; 1];
        io.read_exact(&mut b).await?;
        head.push(b[0]);
    }
    let text = String::from_utf8_lossy(&head);
    let mut lines = text.split("\r\n");
    let status = lines.next().unwrap_or_default();
    if status.split_whitespace().nth(1) != Some("101") {
        return Err(bad(&format!("服务器没有接受 WebSocket（{status}）")));
    }
    let want = accept_key(&key);
    let ok = lines.filter_map(|l| l.split_once(':')).any(|(k, v)| k.trim().eq_ignore_ascii_case("sec-websocket-accept") && v.trim() == want);
    if !ok {
        return Err(bad("服务器回的 Sec-WebSocket-Accept 不对"));
    }
    Ok(())
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

/// 掩码用的随机数：WebSocket 要求电脑端发的帧带掩码（防的是代理缓存投毒，不是加密，里面本来就是 Noise 密文），
/// 不需要密码学强度，用时间和地址当种子的 xorshift 就够
fn seed() -> u64 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    let here = &t as *const u64 as u64;
    (t ^ here.rotate_left(32)) | 1
}

fn next(s: &mut u64) -> u32 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    (*s >> 16) as u32
}

/// 握手之后的 WebSocket 连接，读写的是帧里面的字节
pub struct WsStream<S> {
    io: S,
    /// 电脑端（发的帧要带掩码）还是服务器
    client: bool,
    seed: u64,
    /// 收到、还没拆成帧的原始字节
    raw: Vec<u8>,
    /// 拆出来、还没被读走的内容
    data: Vec<u8>,
    data_pos: usize,
    /// 要发出去、还没写完的帧
    out: Vec<u8>,
    out_pos: usize,
    /// 要回的 Pong（Ping 的内容）：等 `out` 写空了再放进去；这之前又来了 Ping 就只留最后一个（RFC 6455 第 5.5.3 节允许）
    pong: Option<Vec<u8>>,
    eof: bool,
}

impl<S> WsStream<S> {
    pub fn client(io: S) -> Self {
        Self::new(io, true)
    }

    pub fn server(io: S) -> Self {
        Self::new(io, false)
    }

    fn new(io: S, client: bool) -> Self {
        Self { io, client, seed: seed(), raw: Vec::new(), data: Vec::new(), data_pos: 0, out: Vec::new(), out_pos: 0, pong: None, eof: false }
    }

    /// 拼一帧放进 `out`；`out` 里攒得太多（见 MAX_OUT）就出错断开
    fn queue(&mut self, opcode: u8, payload: &[u8]) -> io::Result<()> {
        if self.out.len() - self.out_pos + payload.len() + 14 > MAX_OUT {
            return Err(bad("WebSocket 要发的东西攒得太多了"));
        }
        self.out.push(0x80 | opcode);
        let mask = if self.client { 0x80 } else { 0 };
        let n = payload.len();
        if n < 126 {
            self.out.push(mask | n as u8);
        } else if n <= 0xFFFF {
            self.out.push(mask | 126);
            self.out.extend_from_slice(&(n as u16).to_be_bytes());
        } else {
            self.out.push(mask | 127);
            self.out.extend_from_slice(&(n as u64).to_be_bytes());
        }
        if self.client {
            let key = next(&mut self.seed).to_be_bytes();
            self.out.extend_from_slice(&key);
            self.out.extend(payload.iter().enumerate().map(|(i, b)| b ^ key[i % 4]));
        } else {
            self.out.extend_from_slice(payload);
        }
        Ok(())
    }

    /// 从 `raw` 里拆出一帧；不够一帧返回 Ok(false)
    fn take_frame(&mut self) -> io::Result<bool> {
        let r = &self.raw;
        if r.len() < 2 {
            return Ok(false);
        }
        let opcode = r[0] & 0x0F;
        let masked = r[1] & 0x80 != 0;
        let mut len = (r[1] & 0x7F) as u64;
        let mut at = 2;
        if len == 126 {
            if r.len() < 4 {
                return Ok(false);
            }
            len = u16::from_be_bytes([r[2], r[3]]) as u64;
            at = 4;
        } else if len == 127 {
            if r.len() < 10 {
                return Ok(false);
            }
            len = u64::from_be_bytes(r[2..10].try_into().unwrap());
            at = 10;
        }
        if len > MAX_FRAME {
            return Err(bad("WebSocket 帧太大"));
        }
        // 控制帧不能超过 125 字节、不能分片（RFC 6455 第 5.5 节）
        if opcode & 0x8 != 0 && (len > MAX_CONTROL || r[0] & 0x80 == 0) {
            return Err(bad("WebSocket 控制帧不对"));
        }
        let key_at = at;
        if masked {
            at += 4;
        }
        let end = at + len as usize;
        if r.len() < end {
            return Ok(false);
        }
        let mut payload = r[at..end].to_vec();
        if masked {
            let key: [u8; 4] = r[key_at..key_at + 4].try_into().unwrap();
            payload.iter_mut().enumerate().for_each(|(i, b)| *b ^= key[i % 4]);
        }
        self.raw.drain(..end);
        match opcode {
            0x0..=0x2 => {
                if self.data_pos == self.data.len() {
                    self.data.clear();
                    self.data_pos = 0;
                }
                self.data.extend_from_slice(&payload);
            }
            0x8 => self.eof = true,
            // 先记下，等 `out` 写空了再回（poll_out）；还没回又来了就只留最后一个
            0x9 => self.pong = Some(payload),
            _ => {}
        }
        Ok(true)
    }
}

impl<S: AsyncWrite + Unpin> WsStream<S> {
    /// 把 `out` 里攒着的写出去，写空了再把要回的 Pong 放进去接着写；写不动返回 Pending
    fn poll_out(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        loop {
            while self.out_pos < self.out.len() {
                match Pin::new(&mut self.io).poll_write(cx, &self.out[self.out_pos..]) {
                    Poll::Ready(Ok(0)) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                    Poll::Ready(Ok(n)) => self.out_pos += n,
                    Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    Poll::Pending => return Poll::Pending,
                }
            }
            self.out.clear();
            self.out_pos = 0;
            match self.pong.take() {
                Some(p) => self.queue(0xA, &p)?,
                None => return Poll::Ready(Ok(())),
            }
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for WsStream<S> {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let me = self.get_mut();
        loop {
            if me.data_pos < me.data.len() {
                let n = buf.remaining().min(me.data.len() - me.data_pos);
                buf.put_slice(&me.data[me.data_pos..me.data_pos + n]);
                me.data_pos += n;
                return Poll::Ready(Ok(()));
            }
            // 顺手把要回的 Pong 发出去；写不动也不耽误读
            if let Poll::Ready(Err(e)) = me.poll_out(cx) {
                return Poll::Ready(Err(e));
            }
            if me.take_frame()? {
                continue;
            }
            if me.eof {
                return Poll::Ready(Ok(()));
            }
            let mut tmp = [0u8; 16 * 1024];
            let mut rb = ReadBuf::new(&mut tmp);
            match Pin::new(&mut me.io).poll_read(cx, &mut rb) {
                Poll::Ready(Ok(())) if rb.filled().is_empty() => me.eof = true,
                Poll::Ready(Ok(())) => me.raw.extend_from_slice(rb.filled()),
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for WsStream<S> {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, data: &[u8]) -> Poll<io::Result<usize>> {
        let me = self.get_mut();
        // 上一帧没写完先写完，免得 out 越攒越多
        match me.poll_out(cx) {
            Poll::Ready(Ok(())) => {}
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            Poll::Pending => return Poll::Pending,
        }
        let n = data.len().min(WRITE_CHUNK);
        me.queue(0x2, &data[..n])?;
        if let Poll::Ready(Err(e)) = me.poll_out(cx) {
            return Poll::Ready(Err(e));
        }
        Poll::Ready(Ok(n))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let me = self.get_mut();
        match me.poll_out(cx) {
            Poll::Ready(Ok(())) => Pin::new(&mut me.io).poll_flush(cx),
            other => other,
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let me = self.get_mut();
        if !me.eof {
            me.eof = true;
            me.queue(0x8, &[0x03, 0xE8])?; // 1000：正常关闭
        }
        match me.poll_out(cx) {
            Poll::Ready(Ok(())) => Pin::new(&mut me.io).poll_shutdown(cx),
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_key_matches_rfc() {
        // RFC 6455 第 1.3 节的例子
        assert_eq!(accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    /// 电脑端写的东西，服务器那头原样读出来，反过来也一样；大块会被拆成好几帧
    #[tokio::test]
    async fn bytes_round_trip() {
        let (a, b) = tokio::io::duplex(4096);
        let mut c = WsStream::client(a);
        let mut s = WsStream::server(b);
        let big: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let send = big.clone();
        let w = tokio::spawn(async move {
            c.write_all(b"DNGM").await.unwrap();
            c.write_all(&send).await.unwrap();
            c.flush().await.unwrap();
            let mut back = [0u8; 5];
            c.read_exact(&mut back).await.unwrap();
            assert_eq!(&back, b"hello");
            c.shutdown().await.unwrap();
        });
        let mut got = vec![0u8; 4 + big.len()];
        s.read_exact(&mut got).await.unwrap();
        assert_eq!(&got[..4], b"DNGM");
        assert_eq!(&got[4..], &big[..]);
        s.write_all(b"hello").await.unwrap();
        s.flush().await.unwrap();
        // 对方 Close 以后读到结尾
        let mut rest = Vec::new();
        s.read_to_end(&mut rest).await.unwrap();
        assert!(rest.is_empty());
        w.await.unwrap();
    }

    /// 服务器收到 Ping 回 Pong，内容不混进数据里
    #[tokio::test]
    async fn ping_gets_pong() {
        let (mut raw, b) = tokio::io::duplex(1024);
        let mut s = WsStream::server(b);
        // 电脑端的帧要带掩码：Ping "hi"，再一个二进制帧 "ok"
        let key = [1u8, 2, 3, 4];
        let mut frames = vec![0x89, 0x82];
        frames.extend_from_slice(&key);
        frames.extend(b"hi".iter().enumerate().map(|(i, x)| x ^ key[i % 4]));
        frames.extend_from_slice(&[0x82, 0x82]);
        frames.extend_from_slice(&key);
        frames.extend(b"ok".iter().enumerate().map(|(i, x)| x ^ key[i % 4]));
        raw.write_all(&frames).await.unwrap();
        let mut got = [0u8; 2];
        s.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"ok");
        let mut pong = [0u8; 4];
        raw.read_exact(&mut pong).await.unwrap();
        assert_eq!(pong, [0x8A, 0x02, b'h', b'i']);
    }

    /// 电脑端发的一帧（带掩码）
    fn masked(first: u8, payload: &[u8]) -> Vec<u8> {
        let key = [9u8, 8, 7, 6];
        let mut f = vec![first];
        match payload.len() {
            n if n < 126 => f.push(0x80 | n as u8),
            n => {
                f.push(0x80 | 126);
                f.extend_from_slice(&(n as u16).to_be_bytes());
            }
        }
        f.extend_from_slice(&key);
        f.extend(payload.iter().enumerate().map(|(i, x)| x ^ key[i % 4]));
        f
    }

    /// 攻击：一直发 Ping、从来不收。以前每个 Ping 都往 out 里排一个 Pong，写不出去就一直攒，内存越来越大；
    /// 现在 out 写不动时只留最后一个 Pong，攒着的不超过两帧
    #[tokio::test]
    async fn ping_flood_does_not_pile_up() {
        let (mut raw, b) = tokio::io::duplex(1024);
        let mut s = WsStream::server(b);
        let pings = 5000;
        let w = tokio::spawn(async move {
            for i in 0..pings {
                let mut p = vec![b'x'; 125];
                p[..4].copy_from_slice(&(i as u32).to_be_bytes());
                raw.write_all(&masked(0x89, &p)).await.unwrap();
            }
            raw.write_all(&masked(0x82, b"ok")).await.unwrap();
            raw
        });
        let mut got = [0u8; 2];
        s.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"ok");
        let mut raw = w.await.unwrap();
        let waiting = s.out.len() - s.out_pos + s.pong.as_ref().map_or(0, |p| p.len() + 2);
        assert!(waiting <= 2 * 127, "攒着没写出去的 {waiting} 字节");
        // 对方开始收了：写完就关掉，最后回的是最后一个 Ping 的内容
        let srv = tokio::spawn(async move {
            s.flush().await.unwrap();
        });
        let mut back = Vec::new();
        raw.read_to_end(&mut back).await.unwrap();
        srv.await.unwrap();
        let n = back.len();
        let last = &back[n - 127..n];
        assert_eq!(&last[..2], &[0x8A, 125]);
        assert_eq!(&last[2..6], &((pings - 1) as u32).to_be_bytes());
    }

    /// 控制帧超过 125 字节、分片的控制帧：断开（以前 Ping 能带 1 MB）
    #[tokio::test]
    async fn oversized_control_frame_is_rejected() {
        for frame in [masked(0x89, &[0u8; 126]), masked(0x09, b"hi"), masked(0x88, &[0u8; 300])] {
            let (mut raw, b) = tokio::io::duplex(4096);
            let mut s = WsStream::server(b);
            raw.write_all(&frame).await.unwrap();
            let mut got = [0u8; 1];
            let e = s.read(&mut got).await.unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::InvalidData);
        }
    }

    /// 兜底：out 不会超过 MAX_OUT
    #[test]
    fn out_has_a_cap() {
        let mut s = WsStream::server(tokio::io::empty());
        s.queue(0x2, &[0u8; WRITE_CHUNK]).unwrap();
        assert!(s.queue(0x2, &[0u8; WRITE_CHUNK]).is_err());
    }
}
