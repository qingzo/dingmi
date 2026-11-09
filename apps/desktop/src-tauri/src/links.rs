//! 链接：从文字里找出网址、抓网页标题（包括微信公众号文章），以及用默认浏览器 / 看图程序打开。

use crate::db::InboxEntry;
use std::collections::HashMap;
use std::time::Duration;

/// 网址到这些字符就算结束（中文标点也算）
const URL_END: &[char] = &['"', '\'', '<', '>', ')', ']', '}', '，', '。', '！', '？', '、', '；', '：', '」', '』', '）', '】', '“', '”'];

pub fn find_urls(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("http") {
        let s = &rest[i..];
        if s.starts_with("http://") || s.starts_with("https://") {
            let end = s.find(|c: char| c.is_whitespace() || URL_END.contains(&c)).unwrap_or(s.len());
            let url = s[..end].trim_end_matches(['.', ',', ';', ':', '!', '?']);
            if url.len() > 8 && !out.iter().any(|u| u == url) {
                out.push(url.to_string());
            }
            rest = &s[end.max(1)..];
        } else {
            rest = &s[4..];
        }
    }
    out
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
}

fn tidy(s: &str) -> Option<String> {
    let t: String = decode_entities(s).split_whitespace().collect::<Vec<_>>().join(" ");
    let t: String = t.chars().take(80).collect();
    (!t.is_empty()).then_some(t)
}

/// 从网页里找标题：先找 og:title（微信公众号文章的标题在这里），再找 <title>
pub fn title_from_html(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    if let Some(i) = lower.find("og:title") {
        // 往前找到这个 <meta 标签的开头，再在标签里找 content="..."
        let start = lower[..i].rfind('<').unwrap_or(i);
        let end = lower[i..].find('>').map(|e| i + e).unwrap_or(lower.len());
        let tag = &html[start..end];
        let tag_lower = &lower[start..end];
        if let Some(c) = tag_lower.find("content=") {
            let v = &tag[c + 8..];
            if let Some(q) = v.chars().next().filter(|q| *q == '"' || *q == '\'') {
                if let Some(e) = v[1..].find(q) {
                    if let Some(t) = tidy(&v[1..1 + e]) {
                        return Some(t);
                    }
                }
            }
        }
    }
    let i = lower.find("<title")?;
    let open_end = lower[i..].find('>')? + i + 1;
    let close = lower[open_end..].find("</title")? + open_end;
    tidy(&html[open_end..close])
}

/// 抓网页标题最多跟几次跳转、最多读多少
const MAX_REDIRECTS: usize = 3;
const MAX_PAGE: usize = 1024 * 1024;

/// 抓标题只连公网上的地址：本机、局域网、链路本地、组播、保留的地址都不连（手机发来的网址不能拿来探内网，SSRF）
pub fn public_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    fn v4(a: std::net::Ipv4Addr) -> bool {
        let [x, y, z, _] = a.octets();
        !(a.is_loopback()
            || a.is_private()
            || a.is_link_local()
            || a.is_multicast()
            || a.is_broadcast()
            || a.is_unspecified()
            || a.is_documentation()
            || x == 0
            || (x == 100 && (64..128).contains(&y)) // 运营商级 NAT（100.64/10）
            || (x == 192 && y == 0 && z == 0) // IETF 协议用的 192.0.0/24
            || (x == 198 && (y == 18 || y == 19)) // 测试网 198.18/15
            || x >= 240)
    }
    match ip {
        IpAddr::V4(a) => v4(a),
        IpAddr::V6(a) => {
            if let Some(m) = a.to_ipv4_mapped() {
                return v4(m);
            }
            let s = a.segments();
            // NAT64（64:ff9b::/96）里包着的 IPv4 也要看
            if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                return v4(std::net::Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8));
            }
            !(a.is_loopback()
                || a.is_unspecified()
                || a.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // 唯一本地 fc00::/7
                || (s[0] & 0xffc0) == 0xfe80 // 链路本地 fe80::/10
                || (s[0] & 0xffc0) == 0xfec0 // 站点本地 fec0::/10
                || (s[0] == 0x2001 && s[1] == 0xdb8) // 文档用
                || s[..6] == [0, 0, 0, 0, 0, 0]) // ::a.b.c.d（旧的 IPv4 兼容写法）
        }
    }
}

pub async fn fetch_title(url: &str) -> Option<String> {
    fetch_title_with(url, public_ip).await
}

/// 自己跟跳转（最多 3 次）：每一跳都先解析域名、每个地址都要能连，再把连接钉在解析出来的这几个地址上（防 DNS 换绑）。
/// 不走代理（走代理的话连的是哪里就管不住了）
async fn fetch_title_with(url: &str, allow: impl Fn(std::net::IpAddr) -> bool) -> Option<String> {
    let mut url = reqwest::Url::parse(url).ok()?;
    for _ in 0..=MAX_REDIRECTS {
        if !matches!(url.scheme(), "http" | "https") {
            return None;
        }
        let host = url.host_str()?.trim_start_matches('[').trim_end_matches(']').to_string();
        let port = url.port_or_known_default()?;
        let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.as_str(), port)).await.ok()?.collect();
        if addrs.is_empty() || !addrs.iter().all(|a| allow(a.ip())) {
            return None;
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(6))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .resolve_to_addrs(&host, &addrs)
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36")
            .build()
            .ok()?;
        let mut resp = client.get(url.clone()).send().await.ok()?;
        if resp.status().is_redirection() {
            let to = resp.headers().get(reqwest::header::LOCATION)?.to_str().ok()?;
            url = url.join(to).ok()?;
            continue;
        }
        if !resp.status().is_success() || resp.content_length().is_some_and(|n| n > 5 * 1024 * 1024) {
            return None;
        }
        let ctype = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        if !ctype.is_empty() && !ctype.contains("html") {
            return None;
        }
        // 一点一点读，最多 1 MB（标题都在开头，读满了就用读到的）
        let mut body = Vec::new();
        while let Ok(Some(chunk)) = resp.chunk().await {
            body.extend_from_slice(&chunk[..chunk.len().min(MAX_PAGE - body.len())]);
            if body.len() >= MAX_PAGE {
                break;
            }
        }
        return title_from_html(&String::from_utf8_lossy(&body));
    }
    None
}

/// 手机发来的一条整理成笔记：(标题, 正文)。网址有标题的写成 [标题](网址)，图片和文件接在最后。
/// `files`：其中的文件附件（不是图片的）→ 原文件名，写成 [原文件名](att:文件名)
pub fn compose_note(e: &InboxEntry, titles: &HashMap<String, String>, files: &HashMap<String, String>) -> (String, String) {
    let safe = |t: &str| t.replace('[', "【").replace(']', "】");
    let mut body = e.text.clone();
    for (url, title) in titles {
        body = body.replacen(url.as_str(), &format!("[{}]({url})", safe(title)), 1);
    }
    for f in &e.images {
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        match files.get(f) {
            Some(name) => body.push_str(&format!("[{}](att:{f})", if name.is_empty() { "文件" } else { name.as_str() })),
            None => body.push_str(&format!("![](att:{f})")),
        }
    }
    let first = e.text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let urls = find_urls(first);
    let only_url = urls.len() == 1 && first == urls[0];
    let title = if only_url {
        titles.get(&urls[0]).cloned().unwrap_or_else(|| "手机发来的链接".into())
    } else if !first.is_empty() {
        first.to_string()
    } else if let [only] = e.images.as_slice() {
        match files.get(only).filter(|n| !n.is_empty()) {
            Some(name) => name.clone(),
            None => {
                let when = e.sent_at.as_deref().unwrap_or(&e.received_at);
                format!("手机发来的图片 {}", when.get(5..16).unwrap_or(when))
            }
        }
    } else if !e.images.is_empty() {
        let when = e.sent_at.as_deref().unwrap_or(&e.received_at);
        let what = if e.images.iter().any(|f| files.contains_key(f)) { "文件" } else { "图片" };
        format!("手机发来的{what} {}", when.get(5..16).unwrap_or(when))
    } else {
        "手机发来的".into()
    };
    let mut t: String = title.chars().take(30).collect();
    if title.chars().count() > 30 {
        t.push('…');
    }
    (t, body)
}

/// 附件里能直接用默认程序打开的类型（白名单）：图片、PDF、纯文本、不带宏的 Office 文档、常见的音频视频。
/// 别的（程序、脚本、快捷方式、网页、压缩包、带宏的 docm/xlsm、老格式的 doc/xls……）只在资源管理器里选中，让人自己决定
pub fn safe_to_open(name: &str) -> bool {
    const OK: [&str; 27] = [
        "png", "jpg", "jpeg", "gif", "webp", "bmp", "heic", "heif", "pdf", "txt", "md", "docx", "xlsx", "pptx", "mp3", "m4a",
        "wav", "flac", "aac", "ogg", "opus", "mp4", "m4v", "mov", "webm", "mkv", "avi",
    ];
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    // 备用数据流、末尾的点和空格（Windows 会去掉，a.exe. 就是 a.exe）这类花样不认
    if base.contains(':') || base.ends_with(['.', ' ']) {
        return false;
    }
    let ext = base.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    OK.contains(&ext.as_str())
}

/// 附件用原文件名另存一份打开时的文件名：已经去掉了路径和不能用的字符（db::clean_file_name），
/// 这里再挡掉空的、只有点的、Windows 的设备名（CON、NUL、COM1……），不合适就回 None（用编号的文件名打开）
pub fn copy_name(name: &str) -> Option<String> {
    let n = crate::db::clean_file_name(name);
    let stem = n.split('.').next().unwrap_or("").trim().to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit());
    (!n.trim_matches('.').trim().is_empty() && !device && !n.contains(['/', '\\', ':'])).then_some(n)
}

/// 在资源管理器里打开文件所在的文件夹，并选中它
pub fn shell_reveal(path: &str) -> Result<(), String> {
    std::process::Command::new("explorer")
        .arg(format!("/select,{path}"))
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("打不开：{e}"))
}

/// 解开 URL 编码（%E5%90%88 → 合），坏掉的 % 原样留着
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        match (b[i], b.get(i + 1).and_then(|&c| hex(c)), b.get(i + 2).and_then(|&c| hex(c))) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            (c, ..) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 用 Windows 默认的程序打开（网址用浏览器，图片用看图程序）
pub fn shell_open(target: &str) -> Result<(), String> {
    std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", target])
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("打不开：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_urls_in_chinese_text() {
        let t = "看看这篇https://mp.weixin.qq.com/s/AbC_1-2，还有（http://example.com/a?b=1）。重复 https://mp.weixin.qq.com/s/AbC_1-2";
        assert_eq!(find_urls(t), vec!["https://mp.weixin.qq.com/s/AbC_1-2", "http://example.com/a?b=1"]);
        assert!(find_urls("没有链接 httpx http:/").is_empty());
    }

    #[test]
    fn titles() {
        let wx = r#"<html><head><title></title><meta property="og:title" content="预算&amp;方案 | 周报" /></head>"#;
        assert_eq!(title_from_html(wx).as_deref(), Some("预算&方案 | 周报"));
        let rev = r#"<meta content='反着写的' property="og:title">"#;
        assert_eq!(title_from_html(rev).as_deref(), Some("反着写的"));
        assert_eq!(title_from_html("<TITLE>\n  普通 网页 \n</TITLE>").as_deref(), Some("普通 网页"));
        assert_eq!(title_from_html("<p>没有标题</p>"), None);
    }

    fn entry(text: &str, images: &[&str]) -> InboxEntry {
        InboxEntry {
            id: 1,
            uid: "u".into(),
            phone: "iPhone".into(),
            received_at: "2026-10-31 14:20:05".into(),
            sent_at: None,
            text: text.into(),
            images: images.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn composes_notes() {
        let url = "https://mp.weixin.qq.com/s/x";
        let titles = HashMap::from([(url.to_string(), "好文[章]".to_string())]);
        let none = HashMap::new();
        let (t, b) = compose_note(&entry(url, &[]), &titles, &none);
        assert_eq!(t, "好文[章]");
        assert_eq!(b, format!("[好文【章】]({url})"));

        let (t, b) = compose_note(&entry("记一下预算\n二十万", &["a.png"]), &HashMap::new(), &none);
        assert_eq!(t, "记一下预算");
        assert_eq!(b, "记一下预算\n二十万\n\n![](att:a.png)");

        let (t, b) = compose_note(&entry("", &["a.png", "b.jpg"]), &HashMap::new(), &none);
        assert_eq!(t, "手机发来的图片 10-31 14:20");
        assert_eq!(b, "![](att:a.png)\n\n![](att:b.jpg)");

        // 文件写成 [原文件名](att:…)；只发了一个文件时，原文件名当标题
        let files = HashMap::from([("c.pdf".to_string(), "合同 v2.pdf".to_string())]);
        let (t, b) = compose_note(&entry("", &["c.pdf"]), &HashMap::new(), &files);
        assert_eq!(t, "合同 v2.pdf");
        assert_eq!(b, "[合同 v2.pdf](att:c.pdf)");
        let (t, b) = compose_note(&entry("", &["a.png", "c.pdf"]), &HashMap::new(), &files);
        assert_eq!(t, "手机发来的文件 10-31 14:20");
        assert_eq!(b, "![](att:a.png)\n\n[合同 v2.pdf](att:c.pdf)");
    }

    /// 附件只直接打开白名单里的类型；程序、脚本、快捷方式、压缩包、带宏的文档、网页这些只在文件夹里选中
    #[test]
    fn open_whitelist() {
        for ok in ["a.png", "照片.JPG", r"C:\t\合同 v2.pdf", "a.txt", "readme.md", "a.docx", "b.xlsx", "c.pptx", "v.mp4", "s.mp3"] {
            assert!(safe_to_open(ok), "{ok}");
        }
        for bad in [
            "a.exe", "a.EXE", "a.com", "a.bat", "a.cmd", "a.ps1", "a.vbs", "a.js", "a.hta", "a.lnk", "a.url", "a.scr", "a.msi",
            "a.jar", "a.zip", "a.rar", "a.7z", "a.iso", "a.img", "a.vhd", "a.docm", "a.xlsm", "a.pptm", "a.doc", "a.xls", "a.rtf",
            "a.html", "a.svg", "a.chm", "a.reg", "a.appref-ms", "a.library-ms", "a.searchConnector-ms", "a.cpl", "a.settingcontent-ms",
            "noext", "a.pdf.exe", "a.exe.", "a.pdf ", "a.txt:evil.exe", "",
        ] {
            assert!(!safe_to_open(bad), "{bad}");
        }
    }

    #[test]
    fn copy_names() {
        assert_eq!(copy_name("合同 v2.pdf").as_deref(), Some("合同 v2.pdf"));
        assert_eq!(copy_name(r"..\..\Startup\a.pdf").as_deref(), Some("a.pdf"));
        for bad in ["", "..", ".", "CON", "con.pdf", "NUL.txt", "COM1.docx", "lpt9.png"] {
            assert_eq!(copy_name(bad), None, "{bad}");
        }
    }

    #[test]
    fn public_addresses_only() {
        let ip = |s: &str| s.parse::<std::net::IpAddr>().unwrap();
        for ok in ["1.1.1.1", "8.8.4.4", "2606:4700:4700::1111", "::ffff:8.8.8.8"] {
            assert!(public_ip(ip(ok)), "{ok}");
        }
        for bad in [
            "127.0.0.1", "10.1.2.3", "172.16.0.1", "172.31.255.255", "192.168.1.1", "169.254.169.254", "100.64.0.1", "0.0.0.0",
            "224.0.0.1", "255.255.255.255", "198.18.0.1", "192.0.2.1", "240.0.0.1", "::1", "::", "fe80::1", "fc00::1", "fd12::1",
            "ff02::1", "::ffff:127.0.0.1", "::ffff:192.168.0.1", "64:ff9b::a9fe:a9fe", "::7f00:1",
        ] {
            assert!(!public_ip(ip(bad)), "{bad}");
        }
    }

    /// 在 127.0.0.1 上起一个小网站：/t 有标题，/r<n> 跳到 /r<n+1>（/r4 跳到 /t），/big 是一个 3 MB 的网页。回 (地址开头, 访问记录)
    fn site() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = log.clone();
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { continue };
                let mut req = Vec::new();
                let mut buf = [0u8; 1024];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    match s.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => req.extend_from_slice(&buf[..n]),
                    }
                }
                let path = String::from_utf8_lossy(&req).split(' ').nth(1).unwrap_or("").to_string();
                seen.lock().unwrap().push(path.clone());
                let resp = match path.as_str() {
                    "/t" => "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 30\r\nConnection: close\r\n\r\n<title>内网的页面</title>".to_string(),
                    "/big" => {
                        let body = format!("<html>{}<title>太靠后</title>", " ".repeat(3 << 20));
                        format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n{body}")
                    }
                    p if p.starts_with("/r") => {
                        let n: u32 = p[2..].parse().unwrap_or(0);
                        let to = if n >= 4 { "/t".to_string() } else { format!("/r{}", n + 1) };
                        format!("HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    }
                    _ => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
                };
                let _ = s.write_all(resp.as_bytes());
            }
        });
        (base, log)
    }

    /// 攻击：手机发来一个指向内网（本机、路由器、云服务器的元数据地址）的网址，或者先跳到公网再跳回内网。
    /// 修之前 fetch_title 照抓不误（还跟着跳转）；现在本机地址不连，每一跳都看，跳转最多 3 次，最多读 1 MB
    #[tokio::test]
    async fn title_fetch_refuses_private_addresses() {
        let (base, log) = site();
        // 本机地址：不连（服务器一次请求都没收到）
        assert_eq!(fetch_title(&format!("{base}/t")).await, None);
        assert_eq!(fetch_title(&format!("http://localhost:{}/t", base.rsplit(':').next().unwrap())).await, None);
        assert!(log.lock().unwrap().is_empty());
        // 测试里放开第一跳（当它是公网），之后的不放：跳转的目标也要检查，不会去要 /r2
        let first = std::sync::atomic::AtomicBool::new(true);
        let once = |_: std::net::IpAddr| first.swap(false, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(fetch_title_with(&format!("{base}/r1"), once).await, None);
        assert_eq!(*log.lock().unwrap(), vec!["/r1".to_string()]);
        // 都放开时：跳 3 次以内能拿到，超过 3 次不跟了
        let all = |_: std::net::IpAddr| true;
        assert_eq!(fetch_title_with(&format!("{base}/r2"), all).await.as_deref(), Some("内网的页面"));
        assert_eq!(fetch_title_with(&format!("{base}/r1"), all).await, None);
        // 只读前 1 MB：标题在 3 MB 以后的拿不到（也不会整个读进来）
        assert_eq!(fetch_title_with(&format!("{base}/big"), all).await, None);
        // 不是 http(s) 的不抓
        assert_eq!(fetch_title_with("file:///C:/Windows/win.ini", all).await, None);
    }

    #[test]
    fn decodes_percent() {
        assert_eq!(percent_decode("%E5%90%88%E5%90%8C%20v2.pdf"), "合同 v2.pdf");
        assert_eq!(percent_decode("100%_ok%2"), "100%_ok%2");
    }
}
