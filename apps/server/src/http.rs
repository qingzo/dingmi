//! 给浏览器的那一部分：普通 HTTP 只用来下载根证书，其余都走 HTTPS。
//!
//! - `GET /ca`：iPhone 的描述文件（根证书）；`GET /ca.crt`：根证书本身，给安卓用
//! - `GET /m/#t=<手机令牌>.<收件箱公钥>&k=…`：手机网页（收件箱和手机版）。配对信息在 # 后面，浏览器不发给服务器、不进访问日志；
//!   以前的 `GET /m/<手机令牌>.<收件箱公钥>` 照样给网页（已经加到主屏幕的还用它），网页自己把它挪到 # 后面。
//!   网页带着严格的 CSP（只认网页里那两段脚本，按内容算的哈希），见 page_csp
//! - 安卓分享菜单（需求-0.3 第 11 节）：`GET /m/<…>/app.webmanifest` 网页清单（按网址给，带着令牌）、
//!   `GET /m/sw.js` 接住分享内容的 Service Worker；`POST /m/<…>/share` 平时被它接住，没接住时回到网页
//! - 手机网页可以放在宝塔、Nginx 后面绑域名（反向代理到 `https://127.0.0.1:7788`），证书在那边；
//!   从本机来的请求看 X-Real-IP 认真实 IP。电脑端的连接不是 HTTP，照旧直连 7788
//! - `POST /m/api/inbox`：手机网页发来加密好的一条，`Authorization: Bearer <手机令牌>`
//! - 手机版（输过同步密码的手机）：`GET /m/api/space`、`GET /m/api/pull?since=N`、`POST /m/api/push`、
//!   `GET /m/api/file/<编号>`、`POST /m/api/file/<编号>`（上传附件，整个加密好的文件），
//!   同样用手机令牌，规则和电脑的 Pull / Push / FileGet / FilePut 一样，见同步方案第 14 节
//! - 用手机帮新电脑加入（同步方案第 17.2 节）：`GET /m/api/join/<8 位数字>` 看是哪台电脑、拿它的临时公钥，
//!   `POST /m/api/join/<8 位数字>` 带 `{"sealed": …}` 允许（不带是「不是我」）；自建服务器的手机用 `GET /m/api/invite` 生成配对码
//! - 在微信、QQ 这些 App 里打开证书和手机网页（用它们扫的码）：先给一页「请在浏览器里打开」，
//!   App 里的网页存不住数据、也装不了证书
//! - `GET /sync` 升级成 WebSocket：电脑端走 `wss://域名/sync`（经过宝塔 / Nginx / Cloudflare 转过来），
//!   里面跑的和直连 7788 一模一样（需求-1.0 第 31.2 节 A，`dingmi_sync::ws`）
//! - 同步节点不给搜索引擎收录：`/robots.txt` 全部不让爬，每个回复都带 `X-Robots-Tag: noindex`
//! - 官方版在宝塔后面（`Server::load_proxied`）：没有根证书（`/ca`、`/ca.crt` 是 404），程序自己收连接、按域名分好，
//!   同步域名的请求交给 [`respond`]，当成已经是 HTTPS（外面一定是宝塔的 443）

use crate::admin::SPACE_OFF;
use crate::net::{Reject, Server, MAX_FILE, MAX_INBOX_ITEM};
use dingmi_sync::{ws, Record};
use serde::Deserialize;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{header, Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use std::convert::Infallible;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::OwnedSemaphorePermit;

const TEXT: &str = "text/plain; charset=utf-8";
const REVOKED: &str = "这部手机已经不能用了，请在电脑上重新添加";
const ICON: &[u8] = include_bytes!("../web/icon.png");
/// 安卓分享菜单用的 Service Worker（需求-0.3 第 11 节），见 web/sw.js
const SW: &str = include_str!("../web/sw.js");
/// 在微信这些 App 里打开时先给的一页，见 web/open-in-browser.html
const OPEN_IN_BROWSER: &str = include_str!("../web/open-in-browser.html");
const ROBOTS: &str = "User-agent: *
Disallow: /
";
/// 手机版一次推上来的记录最多多大（JSON）
const MAX_PUSH: usize = 4 * 1024 * 1024;

/// 手机网页：源码在 apps/mobile/，npm run build:mobile 打包成这一个文件；tweetnacl 在这里内嵌进去，不依赖任何外部资源。
/// 换行统一成 \n：Windows 上 git 检出来是 \r\n，编进去以后浏览器解析时会把脚本里的 \r\n 换成 \n 再算哈希，
/// 和 page_csp 按原文算的对不上，整段脚本被拦、页面一片空白
fn page() -> &'static str {
    static PAGE: OnceLock<String> = OnceLock::new();
    PAGE.get_or_init(|| {
        include_str!("../web/app.html")
            .replace("/*NACL*/", include_str!("../web/nacl-fast.min.js"))
            .replace("\r\n", "\n")
            .replace('\r', "\n")
    })
}

/// 浏览器真正的 IP。手机网页放在宝塔、Nginx 后面时连接都是从本机来的，看反向代理带过来的
/// X-Real-IP / X-Forwarded-For；不是本机来的连接不看（谁都能伪造这两个头）
pub fn real_ip(headers: &hyper::HeaderMap, ip: IpAddr) -> IpAddr {
    if !ip.is_loopback() {
        return ip;
    }
    // 先看 X-Real-IP（反向代理自己填的，部署文档里都配了）；没有再看 X-Forwarded-For 的**最后一个**：
    // 那是反向代理自己加上的「谁连的我」，前面的是浏览器自己带来的，随便写
    let real = || headers.get("x-real-ip")?.to_str().ok()?.trim().parse::<IpAddr>().ok();
    let last = || headers.get("x-forwarded-for")?.to_str().ok()?.rsplit(',').next()?.trim().parse::<IpAddr>().ok();
    real().or_else(last).unwrap_or(ip)
}

/// 是不是在微信、QQ 这些 App 自带的浏览器里打开的：这些里面下载不了证书，网页存的数据也不一定留得住
fn in_app(headers: &hyper::HeaderMap) -> bool {
    let ua = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("");
    // 企业微信也带 MicroMessenger；手机 QQ 是「 QQ/版本号」（QQ 浏览器是 MQQBrowser，是正经浏览器，不算）
    ["MicroMessenger", " QQ/", "DingTalk", "Weibo", "AlipayClient", "Lark/", "Feishu"].iter().any(|k| ua.contains(k))
}

/// 给不给网页清单：只给安卓上的 Chrome、Edge、三星浏览器（分享菜单要靠清单，装到桌面以后和浏览器共用存储）。
/// 有清单时「添加到主屏幕」用清单的 start_url（/m/），# 后面的配对信息丢了，只能靠上次存在浏览器里的；
/// iPhone 主屏幕上的网页和 Safari 的存储是分开的，国产浏览器的「添加到桌面」各家不一样（无痕、退出清数据、单独的容器），
/// 读不到就只剩「要从电脑上的叮咪打开」。不给清单就存当前网址（/m/#t=…&k=…），不靠存储
fn wants_manifest(headers: &hyper::HeaderMap) -> bool {
    let ua = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("");
    if !ua.contains("Android") || ua.contains("; wv)") || ua.contains("Version/") {
        return false;
    }
    // 正经的 Chrome 是「(KHTML, like Gecko) Chrome/版本 Mobile Safari/537.36」，三星在 Chrome 前面多一个 SamsungBrowser/版本，
    // Edge 在最后多一个 EdgA/版本；国产浏览器都会在中间或后面插自己的名字（Quark/、HuaweiBrowser/、MiuiBrowser/……）
    let Some((head, tail)) = ua.split_once("(KHTML, like Gecko) ").map(|(_, r)| r).and_then(|r| r.split_once("Chrome/")) else {
        return false;
    };
    let word = |s: &str, name: &str| s.strip_prefix(name).is_some_and(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit() || b == b'.'));
    let head_ok = head.is_empty() || head.strip_suffix(' ').is_some_and(|h| word(h, "SamsungBrowser/"));
    let mut rest = tail.split(' ');
    let ver = rest.next().is_some_and(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit() || b == b'.'));
    let rest: Vec<&str> = rest.collect();
    let tail_ok = match rest.as_slice() {
        ["Mobile", s] | [s] => word(s, "Safari/"),
        ["Mobile", s, e] | [s, e] => word(s, "Safari/") && word(e, "EdgA/"),
        _ => false,
    };
    head_ok && ver && tail_ok
}

/// 手机网页的内容安全策略：脚本只认网页里内嵌的那两段（tweetnacl 和打包好的网页，按内容算 sha256），
/// 不许别的内联脚本、内联事件和外面的脚本；只连自己这台服务器。hash-wasm 算 Argon2 要 WebAssembly（'wasm-unsafe-eval'）；
/// iOS 15 及更早的 Safari 不认这个写法，只能给它 'unsafe-eval'，不然输不了同步密码
fn page_csp(headers: &hyper::HeaderMap) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    static HASHES: OnceLock<String> = OnceLock::new();
    let hashes = HASHES.get_or_init(|| {
        let html = page();
        let mut out = Vec::new();
        let mut rest = html;
        while let Some(i) = rest.find("<script>") {
            let body = &rest[i + 8..];
            let Some(end) = body.find("</script>") else { break };
            out.push(format!("'sha256-{}'", base64::engine::general_purpose::STANDARD.encode(Sha256::digest(&body.as_bytes()[..end]))));
            rest = &body[end..];
        }
        out.join(" ")
    });
    let ua = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or("");
    let old_ios = ["OS 10_", "OS 11_", "OS 12_", "OS 13_", "OS 14_", "OS 15_"].iter().any(|v| ua.contains(v));
    let wasm = if old_ios { "'unsafe-eval'" } else { "'wasm-unsafe-eval'" };
    format!(
        "default-src 'none'; script-src {hashes} {wasm}; style-src 'unsafe-inline'; img-src 'self' data: blob:; \
         connect-src 'self'; worker-src 'self'; manifest-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'"
    )
}

/// 网址里 /m/ 后面那一段（令牌.收件箱公钥）：只认 base64url 和点，拼进清单和跳转地址前先核对
fn valid_seg(s: &str) -> bool {
    !s.is_empty() && s.len() <= 200 && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// /m/<令牌>/app.webmanifest、/m/<令牌>/share 里的那一段
fn seg_of<'a>(path: &'a str, tail: &str) -> Option<&'a str> {
    path.strip_prefix("/m/")?.strip_suffix(tail).filter(|s| valid_seg(s))
}

/// 网页清单：安卓上「安装」以后，别的 App 的分享菜单里才有叮咪。以前的网址带着令牌，清单也按网址给；
/// 新的网址令牌在 # 后面，清单是同一份（`seg` 为空）：打开 /m/，网页从本机存的配对信息里读令牌
fn manifest(seg: &str) -> String {
    let start = if seg.is_empty() { "/m/".to_string() } else { format!("/m/{seg}") };
    let share = if seg.is_empty() { "/m/share".to_string() } else { format!("/m/{seg}/share") };
    serde_json::json!({
        "name": "叮咪",
        "short_name": "叮咪",
        "start_url": start,
        "scope": "/m/",
        "display": "standalone",
        "background_color": "#f6f1ec",
        "theme_color": "#f6f1ec",
        "icons": [{ "src": "/m-icon.png", "sizes": "256x256", "type": "image/png", "purpose": "any" }],
        "share_target": {
            "action": share,
            "method": "POST",
            "enctype": "multipart/form-data",
            "params": { "title": "title", "text": "text", "url": "url", "files": [{ "name": "files", "accept": ["*/*"] }] }
        }
    })
    .to_string()
}

#[derive(Deserialize)]
struct PushBody {
    items: Vec<Record>,
}

/// 手机上点了「一样，允许」（带着加密好的空间密钥）或「不是我」
#[derive(Deserialize)]
struct JoinAnswer {
    #[serde(default)]
    sealed: Option<String>,
}

fn json(status: StatusCode, v: serde_json::Value) -> Response<Full<Bytes>> {
    reply(status, "application/json; charset=utf-8", v.to_string())
}

fn bearer(req: &Request<Incoming>) -> Option<String> {
    req.headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string)
}

/// 令牌不对（手机作废了）回 401，手机网页认这个；空间停用了、扩展点说停了（叮咪同步到期）回 403（手机网页把那句话显示出来）
fn refused(why: Reject) -> Response<Full<Bytes>> {
    match why {
        Reject::Off => reply(StatusCode::FORBIDDEN, TEXT, SPACE_OFF),
        Reject::Stopped(msg) => reply(StatusCode::FORBIDDEN, TEXT, msg),
        _ => reply(StatusCode::UNAUTHORIZED, TEXT, REVOKED),
    }
}

/// 把请求体读掉、不留（最多 `max` 字节，最多读 `within` 这么久）。不读完就回话、断开的话，
/// 手机浏览器多半只看到「网络错误」，以为没网、存起来一直重发
async fn drain(body: Incoming, max: u64, within: Duration) {
    let _ = tokio::time::timeout(within, async move {
        let mut body = body;
        let mut n = 0u64;
        while let Some(Ok(frame)) = body.frame().await {
            n += frame.data_ref().map_or(0, |d| d.len() as u64);
            if n > max {
                break;
            }
        }
    })
    .await;
}

/// 请求头里写的内容有多大（没写是 None）
fn body_len(req: &Request<Incoming>) -> Option<u64> {
    hyper::body::Body::size_hint(req.body()).exact()
}

/// 拿一个上传名额（Caps.uploads）：手机传上来的整个先收进内存，同时传的太多会撑爆内存。等不到回 None（503）
async fn upload_slot(server: &Server) -> Option<OwnedSemaphorePermit> {
    tokio::time::timeout(server.caps.upload_wait, server.uploads.clone().acquire_owned()).await.ok()?.ok()
}

const BUSY: &str = "服务器正忙，过一会儿再试";

/// 手机版的接口：同步记录、看图片。令牌不对时 server.phone 会记一次违规
async fn sync_api(req: Request<Incoming>, path: &str, ip: IpAddr, server: &Arc<Server>) -> Response<Full<Bytes>> {
    let (phone, space) = match server.phone(ip, bearer(&req).as_deref()) {
        Ok(who) => who,
        Err(why) => return refused(why),
    };
    let get = req.method() == Method::GET;
    if get && path == "/m/api/space" {
        let s = server.store.space(&space);
        // official：叮咪同步（手机上「加一台电脑」输数字）；自建的手机上改成「给新电脑生成配对码」
        let official = server.account(&space).official;
        return json(StatusCode::OK, serde_json::json!({ "salt": s.salt, "box1": s.box1, "official": official }));
    }
    // 用手机加入（需求-1.x 第 34.3 节）：手机输电脑上显示的 8 位数字，看是哪台电脑、拿它的临时公钥；
    // 允许时把空间密钥加密给它，服务器生成邀请码交给那台电脑
    if let Some(code) = path.strip_prefix("/m/api/join/") {
        let code = code.to_string();
        if get {
            return match server.phone_req(&phone, &code) {
                Ok((name, key)) => json(StatusCode::OK, serde_json::json!({ "name": name, "key": key })),
                Err(true) => reply(StatusCode::TOO_MANY_REQUESTS, TEXT, "数字输错太多次了，过 10 分钟再试"),
                Err(false) => reply(StatusCode::NOT_FOUND, TEXT, "这个数字不对，或者已经过期了：看看电脑上显示的"),
            };
        }
        if req.method() == Method::POST {
            let body = match Limited::new(req.into_body(), 4096).collect().await {
                Ok(b) => b.to_bytes(),
                Err(_) => return reply(StatusCode::PAYLOAD_TOO_LARGE, TEXT, "内容不对"),
            };
            let Ok(a) = serde_json::from_slice::<JoinAnswer>(&body) else {
                return reply(StatusCode::BAD_REQUEST, TEXT, "内容不对");
            };
            return match server.phone_answer(ip, &space, &phone, &code, a.sealed) {
                Ok(()) => reply(StatusCode::OK, TEXT, "ok"),
                Err((true, msg)) => reply(StatusCode::TOO_MANY_REQUESTS, TEXT, msg),
                Err((false, msg)) => reply(StatusCode::CONFLICT, TEXT, msg),
            };
        }
    }
    // 自建服务器的手机：给新电脑生成配对码（不带空间密钥，手机自己接在后面），24 小时内有效
    if get && path == "/m/api/invite" {
        if let Err(why) = server.room(&space) {
            return reply(StatusCode::CONFLICT, TEXT, why);
        }
        return match crate::admin::invite_for(&server.store, &space) {
            Ok(inv) => {
                // 和电脑生成的一样，一个空间最多留 MAX_PENDING_INVITES 个没用的：手机令牌不能无限生成
                server.store.trim_invites(&space, crate::store::MAX_PENDING_INVITES);
                server.event(ip, &space, &phone, "手机生成了一个配对码");
                json(StatusCode::OK, serde_json::json!({ "code": inv.code, "expires_at": inv.expires_at }))
            }
            Err(why) => reply(StatusCode::CONFLICT, TEXT, why),
        };
    }
    // 单个图片、单个附件的限额（扩展点定，叮咪同步按套餐，32.1 第 5 条；开源版不限）：手机发图片前在浏览器里压缩，文件超了直接说
    if get && path == "/m/api/limits" {
        let l = server.file_limits(&space);
        return json(StatusCode::OK, serde_json::json!({ "max_image": l.max_image, "max_file": l.max_file }));
    }
    if get && path == "/m/api/pull" {
        let since = req
            .uri()
            .query()
            .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("since=")))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        let (items, more) = server.pull_records(&space, since);
        // epoch 见 Welcome.epoch：手机记着上次的，变了就从头拉
        let epoch = server.store.kv_get("epoch").unwrap_or_default();
        return json(StatusCode::OK, serde_json::json!({ "items": items, "more": more, "epoch": epoch }));
    }
    if req.method() == Method::POST && path == "/m/api/push" {
        let body = match Limited::new(req.into_body(), MAX_PUSH).collect().await {
            Ok(b) => b.to_bytes(),
            Err(_) => return reply(StatusCode::PAYLOAD_TOO_LARGE, TEXT, "一次推的太多了"),
        };
        let Ok(p) = serde_json::from_slice::<PushBody>(&body) else {
            return reply(StatusCode::BAD_REQUEST, TEXT, "内容不对");
        };
        let Ok(acks) = server.push_records(&space, &format!("phone:{phone}"), p.items) else {
            return reply(StatusCode::INSUFFICIENT_STORAGE, TEXT, server.full_msg(&space));
        };
        server.store.touch_phone(&phone);
        return json(StatusCode::OK, serde_json::json!({ "acks": acks }));
    }
    if let Some(uid) = path.strip_prefix("/m/api/file/") {
        let uid = uid.to_string();
        // 读写整个附件（最大 60 MB）放到别的线程上做，不卡住别的连接
        let s = server.clone();
        if get {
            let read = tokio::task::spawn_blocking(move || s.file_bytes(&space, &uid)).await.ok().flatten();
            return match read {
                Some(b) => reply(StatusCode::OK, "application/octet-stream", b),
                None => reply(StatusCode::NOT_FOUND, TEXT, "这个附件还没传到服务器"),
            };
        }
        if req.method() == Method::POST {
            if body_len(&req).is_some_and(|n| n > MAX_FILE) {
                return reply(StatusCode::PAYLOAD_TOO_LARGE, TEXT, "文件不能超过 50 MB");
            }
            let Some(_slot) = upload_slot(server).await else {
                return reply(StatusCode::SERVICE_UNAVAILABLE, TEXT, BUSY);
            };
            let body = match Limited::new(req.into_body(), MAX_FILE as usize).collect().await {
                Ok(b) => b.to_bytes(),
                Err(_) => return reply(StatusCode::PAYLOAD_TOO_LARGE, TEXT, "文件不能超过 50 MB"),
            };
            let sp = space.clone();
            let stored = tokio::task::spawn_blocking(move || s.file_store(&sp, &uid, &body)).await.unwrap_or(Err(Reject::Server));
            return match stored {
                Ok(()) => reply(StatusCode::OK, TEXT, "ok"),
                Err(Reject::Server) => reply(StatusCode::INTERNAL_SERVER_ERROR, TEXT, "服务器存不下来，稍后再试"),
                Err(Reject::Quota) => reply(StatusCode::INSUFFICIENT_STORAGE, TEXT, server.full_msg(&space)),
                Err(Reject::TooBig(msg)) => reply(StatusCode::PAYLOAD_TOO_LARGE, TEXT, msg),
                Err(_) => reply(StatusCode::BAD_REQUEST, TEXT, "内容不对"),
            };
        }
    }
    reply(StatusCode::NOT_FOUND, TEXT, "没有这个接口")
}

/// 处理一个浏览器连接；`secure` 表示是不是 HTTPS。`permit` 是这个连接占的名额（Caps.conns），升级成 WebSocket 时交给那条连接
pub async fn serve<I>(io: I, secure: bool, ip: IpAddr, server: Arc<Server>, permit: OwnedSemaphorePermit)
where
    I: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let permit = Arc::new(Mutex::new(Some(permit)));
    let svc = service_fn(move |req: Request<Incoming>| {
        let server = server.clone();
        let permit = permit.clone();
        async move { Ok::<_, Infallible>(respond(req, secure, ip, server, &permit).await) }
    });
    let mut b = http1::Builder::new();
    b.timer(TokioTimer::new())
        .header_read_timeout(Duration::from_secs(10))
        .keep_alive(false);
    // 一个连接最多占 5 分钟（手机网络慢时传一个 50 MB 的文件要一会儿），防止慢慢发数据拖住服务器
    // 升级成 WebSocket 的连接交出去以后这里就结束了，那条连接不受这 5 分钟限制
    let _ = tokio::time::timeout(Duration::from_secs(300), b.serve_connection(TokioIo::new(io), svc).with_upgrades()).await;
}

/// 一个请求：`GET /sync` 升级成 WebSocket（电脑），别的是手机网页那些。`secure` 表示是不是 HTTPS；
/// `permit` 是这个连接的名额，升级成 WebSocket 时拿走（交给那条连接）。
/// 官方版自己收连接、按域名分好以后，同步域名的请求也交到这里（`secure` 是 true，`ip` 是宝塔带来的真实 IP）
pub async fn respond(
    req: Request<Incoming>,
    secure: bool,
    ip: IpAddr,
    server: Arc<Server>,
    permit: &Mutex<Option<OwnedSemaphorePermit>>,
) -> Response<Full<Bytes>> {
    if is_sync_upgrade(&req) {
        let permit = permit.lock().unwrap().take();
        return sync_upgrade(req, ip, server, permit);
    }
    route(req, secure, ip, &server).await
}

fn is_sync_upgrade(req: &Request<Incoming>) -> bool {
    let has = |name: header::HeaderName, want: &str| {
        req.headers().get(name).and_then(|v| v.to_str().ok()).is_some_and(|v| v.to_ascii_lowercase().contains(want))
    };
    req.method() == Method::GET && req.uri().path() == ws::PATH && has(header::UPGRADE, "websocket") && has(header::CONNECTION, "upgrade")
}

/// 电脑端的 WebSocket：回 101，连接交给 `Server::device`，和直连的电脑一样处理；连接的名额跟着它走
fn sync_upgrade(mut req: Request<Incoming>, ip: IpAddr, server: Arc<Server>, permit: Option<OwnedSemaphorePermit>) -> Response<Full<Bytes>> {
    let ip = real_ip(req.headers(), ip);
    let key = req.headers().get("sec-websocket-key").and_then(|v| v.to_str().ok()).map(str::to_string);
    let (Some(key), false, Some(permit)) = (key, server.banned(ip), permit) else {
        return reply(StatusCode::BAD_REQUEST, "text/plain; charset=utf-8", "");
    };
    let on = hyper::upgrade::on(&mut req);
    tokio::spawn(async move {
        let _permit = permit;
        if let Ok(up) = on.await {
            server.device(ws::WsStream::server(TokioIo::new(up)), ip).await;
        }
    });
    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(header::UPGRADE, "websocket")
        .header(header::CONNECTION, "Upgrade")
        .header(header::SEC_WEBSOCKET_ACCEPT, ws::accept_key(&key))
        .body(Full::new(Bytes::new()))
        .unwrap()
}

fn reply(status: StatusCode, ctype: &str, body: impl Into<Bytes>) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, ctype)
        .header(header::CACHE_CONTROL, "no-store")
        .header("x-robots-tag", "noindex, nofollow")
        .body(Full::new(body.into()))
        .unwrap()
}

async fn route(req: Request<Incoming>, secure: bool, ip: IpAddr, server: &Arc<Server>) -> Response<Full<Bytes>> {
    let path = req.uri().path().to_string();
    let get = req.method() == Method::GET || req.method() == Method::HEAD;
    let ip = real_ip(req.headers(), ip);
    // 经过反向代理来的连接都是 127.0.0.1，连上时（net.rs 的 handle）查不出来：这里按真实 IP 再查一次
    if server.banned(ip) {
        return reply(StatusCode::TOO_MANY_REQUESTS, TEXT, "失败次数太多，过 10 分钟再试");
    }
    if get && path == "/robots.txt" {
        return reply(StatusCode::OK, TEXT, ROBOTS);
    }
    // 证书和手机网页在 App 里打开时先请他换到浏览器；接口、清单这些不管。官方版没有根证书，/ca 是 404
    let ca_path = server.ca_der().is_some() && (path == "/ca" || path == "/ca.crt");
    let page_path = ca_path
        || path == "/m"
        || (path.starts_with("/m/")
            && !path.starts_with("/m/api/")
            && path != "/m/sw.js"
            && !path.ends_with("/app.webmanifest"));
    if get && page_path && in_app(req.headers()) {
        return reply(StatusCode::OK, "text/html; charset=utf-8", OPEN_IN_BROWSER);
    }
    if let Some(m) = server.mobileconfig().filter(|_| get && path == "/ca") {
        let mut r = reply(StatusCode::OK, "application/x-apple-aspen-config", m.to_string());
        r.headers_mut().insert(
            header::CONTENT_DISPOSITION,
            header::HeaderValue::from_static("attachment; filename=\"dingmi-ca.mobileconfig\""),
        );
        return r;
    }
    if let Some(ca) = server.ca_der().filter(|_| get && path == "/ca.crt") {
        let mut r = reply(StatusCode::OK, "application/x-x509-ca-cert", ca.to_vec());
        r.headers_mut().insert(
            header::CONTENT_DISPOSITION,
            header::HeaderValue::from_static("attachment; filename=\"dingmi-ca.crt\""),
        );
        return r;
    }
    if !secure {
        return reply(StatusCode::BAD_REQUEST, TEXT, "请用 https:// 打开");
    }
    if get && path == "/m-icon.png" {
        return reply(StatusCode::OK, "image/png", Bytes::from_static(ICON));
    }
    if get && path == "/m/sw.js" {
        return reply(StatusCode::OK, "text/javascript; charset=utf-8", SW);
    }
    if let Some(seg) = seg_of(&path, "/app.webmanifest").filter(|_| get) {
        return reply(StatusCode::OK, "application/manifest+json", manifest(seg));
    }
    if get && path == "/m/app.webmanifest" {
        return reply(StatusCode::OK, "application/manifest+json", manifest(""));
    }
    if req.method() == Method::POST && path == "/m/share" {
        return Response::builder()
            .status(StatusCode::SEE_OTHER)
            .header(header::LOCATION, "/m/")
            .header("x-robots-tag", "noindex, nofollow")
            .body(Full::new(Bytes::new()))
            .unwrap();
    }
    // 分享过来的内容平时由 sw.js 接住，不会到这里；没装好 Service Worker 时内容带不过来，回到网页
    if let Some(seg) = seg_of(&path, "/share").filter(|_| req.method() == Method::POST) {
        return Response::builder()
            .status(StatusCode::SEE_OTHER)
            .header(header::LOCATION, format!("/m/{seg}"))
            .header("x-robots-tag", "noindex, nofollow")
            .body(Full::new(Bytes::new()))
            .unwrap();
    }
    if get && (path == "/m" || (path.starts_with("/m/") && !path.starts_with("/m/api/"))) {
        let seg = path.strip_prefix("/m/").filter(|s| valid_seg(s));
        let link = match seg {
            _ if !wants_manifest(req.headers()) => String::new(),
            Some(s) => format!(r#"<link rel="manifest" href="/m/{s}/app.webmanifest">"#),
            None => r#"<link rel="manifest" href="/m/app.webmanifest">"#.to_string(),
        };
        let mut r = reply(StatusCode::OK, "text/html; charset=utf-8", page().replace("<!--MANIFEST-->", &link));
        if let Ok(v) = header::HeaderValue::from_str(&page_csp(req.headers())) {
            r.headers_mut().insert(header::CONTENT_SECURITY_POLICY, v);
        }
        return r;
    }
    if path.starts_with("/m/api/") && path != "/m/api/inbox" {
        return sync_api(req, &path, ip, server).await;
    }
    if req.method() == Method::POST && path == "/m/api/inbox" {
        return inbox(req, ip, server).await;
    }
    reply(StatusCode::NOT_FOUND, TEXT, "没有这个页面")
}

/// `POST /m/api/inbox`：先核对令牌，再看收不收得下（单个上限、容量、收件箱满没满），最后才读内容
async fn inbox(req: Request<Incoming>, ip: IpAddr, server: &Arc<Server>) -> Response<Full<Bytes>> {
    let len = body_len(&req);
    let (phone, space) = match server.phone(ip, bearer(&req).as_deref()) {
        Ok(who) => who,
        Err(why) => {
            // 令牌不对：内容不收，读掉就扔（不攒在内存里），读完再回 401，手机能看到「不能用了」。
            // 写着比上限还大的不用读了；假令牌试 5 次就会被屏蔽（route 一开头就挡掉）
            if len.is_none_or(|n| n <= MAX_INBOX_ITEM as u64) {
                drain(req.into_body(), MAX_INBOX_ITEM as u64, server.caps.drain).await;
            }
            return refused(why);
        }
    };
    let say = |why: Reject| match why {
        Reject::Token => reply(StatusCode::UNAUTHORIZED, TEXT, REVOKED),
        Reject::Full => reply(StatusCode::INSUFFICIENT_STORAGE, TEXT, "收件箱满了，打开电脑上的叮咪取走一些再发"),
        Reject::Quota => reply(StatusCode::INSUFFICIENT_STORAGE, TEXT, server.inbox_full_msg(&space)),
        Reject::TooBig(msg) => reply(StatusCode::PAYLOAD_TOO_LARGE, TEXT, msg),
        Reject::Bad => reply(StatusCode::BAD_REQUEST, TEXT, "内容不对"),
        why @ (Reject::Off | Reject::Stopped(_)) => refused(why),
        Reject::Server => reply(StatusCode::INTERNAL_SERVER_ERROR, TEXT, "服务器存不下来，稍后再试"),
    };
    const TOO_BIG: &str = "一次发的太大了，少选几张图片试试";
    if len.is_some_and(|n| n > MAX_INBOX_ITEM as u64) {
        return reply(StatusCode::PAYLOAD_TOO_LARGE, TEXT, TOO_BIG);
    }
    if let Some(n) = len {
        if let Err(why) = server.inbox_room(&space, n) {
            drain(req.into_body(), n, server.caps.drain).await;
            return say(why);
        }
    }
    let Some(_slot) = upload_slot(server).await else {
        return reply(StatusCode::SERVICE_UNAVAILABLE, TEXT, BUSY);
    };
    let body = match Limited::new(req.into_body(), MAX_INBOX_ITEM).collect().await {
        Ok(b) => b.to_bytes(),
        Err(_) => return reply(StatusCode::PAYLOAD_TOO_LARGE, TEXT, TOO_BIG),
    };
    let (s, sp) = (server.clone(), space.clone());
    match tokio::task::spawn_blocking(move || s.receive(&phone, &sp, &body)).await.unwrap_or(Err(Reject::Server)) {
        Ok(()) => reply(StatusCode::OK, TEXT, "ok"),
        Err(why) => say(why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_paths() {
        assert_eq!(seg_of("/m/Ab-_1.xY2/app.webmanifest", "/app.webmanifest"), Some("Ab-_1.xY2"));
        assert_eq!(seg_of("/m/Ab.c/share", "/share"), Some("Ab.c"));
        // 拼进清单和跳转地址的那一段只认 base64url 和点
        for bad in ["/m//share", "/m/a/b/share", "/m/a%22/share", "/m/a\"x/share", "/x/a/share"] {
            assert_eq!(seg_of(bad, "/share"), None, "{bad}");
        }
        let m: serde_json::Value = serde_json::from_str(&manifest("tok.pk")).unwrap();
        assert_eq!(m["start_url"], "/m/tok.pk");
        assert_eq!(m["share_target"]["action"], "/m/tok.pk/share");
        assert_eq!(m["share_target"]["params"]["files"][0]["name"], "files");
        // 网页里放了清单的位置，服务器才能按网址填进去
        assert!(page().contains("<!--MANIFEST-->"));
        // 新的网址（令牌在 # 后面）用同一份清单：打开 /m/，分享到 /m/share
        let m: serde_json::Value = serde_json::from_str(&manifest("")).unwrap();
        assert_eq!((m["start_url"].as_str(), m["share_target"]["action"].as_str()), (Some("/m/"), Some("/m/share")));
    }

    /// 手机网页的 CSP：只认网页里那两段脚本（哈希对得上），别的内联脚本、外面的脚本都不认
    #[test]
    fn page_csp_allows_only_our_scripts() {
        use base64::Engine;
        use sha2::{Digest, Sha256};
        let csp = page_csp(&hyper::HeaderMap::new());
        let hash = |s: &str| format!("'sha256-{}'", base64::engine::general_purpose::STANDARD.encode(Sha256::digest(s.as_bytes())));
        assert!(csp.contains(&hash(include_str!("../web/nacl-fast.min.js"))), "{csp}");
        let html = page();
        // 浏览器按 \n 换行算脚本的哈希：网页里不能留 \r（Windows 上检出来的 app.html 是 \r\n）
        assert!(!html.contains('\r'));
        let app = html.rsplit("<script>").next().unwrap().split("</script>").next().unwrap();
        assert!(csp.contains(&hash(app)));
        assert_eq!(csp.matches("'sha256-").count(), 2);
        let script = csp.split(';').find(|d| d.trim().starts_with("script-src")).unwrap();
        assert!(!script.contains("'unsafe-inline'") && !script.contains("'self'") && !script.contains("'unsafe-eval'"), "{script}");
        for bad in ["http:", "https:", "*"] {
            assert!(!csp.contains(bad), "{bad} in {csp}");
        }
        assert!(csp.contains("default-src 'none'") && csp.contains("frame-ancestors 'none'") && csp.contains("connect-src 'self'"));
        // iOS 15 的 Safari 不认 'wasm-unsafe-eval'，给它 'unsafe-eval'（不然算不了 Argon2、输不了同步密码）
        let mut h = hyper::HeaderMap::new();
        h.insert(header::USER_AGENT, "Mozilla/5.0 (iPhone; CPU iPhone OS 15_7 like Mac OS X) Version/15.6".parse().unwrap());
        assert!(page_csp(&h).contains("'unsafe-eval'") && !page_csp(&h).contains("'wasm-unsafe-eval'"));
    }

    #[test]
    fn app_browsers() {
        let ua = |s: &'static str| {
            let mut h = hyper::HeaderMap::new();
            h.insert(header::USER_AGENT, hyper::header::HeaderValue::from_static(s));
            h
        };
        let wx = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 MicroMessenger/8.0.47(0x18002f2c) NetType/WIFI Language/zh_CN";
        let qq = "Mozilla/5.0 (Linux; Android 13; V2196A) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/98.0 MQQBrowser/6.2 TBS/046613 Mobile Safari/537.36 V1_AND_SQ_8.9.80_4614_YYB_D QQ/8.9.80.12440 NetType/WIFI";
        assert!(in_app(&ua(wx)));
        assert!(in_app(&ua(qq)));
        for ok in [
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1",
            "Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Mobile Safari/537.36",
            "Mozilla/5.0 (Linux; U; Android 13; zh-cn) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/109.0 MQQBrowser/14.5 Mobile Safari/537.36",
        ] {
            assert!(!in_app(&ua(ok)), "{ok}");
        }
        assert!(!in_app(&hyper::HeaderMap::new()));
        // 落地页自己也不让收录
        assert!(OPEN_IN_BROWSER.contains("noindex"));
        // 清单只给安卓的 Chrome、Edge、三星浏览器
        for yes in [
            "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.6723.58 Mobile Safari/537.36",
            "Mozilla/5.0 (Linux; Android 14; SM-X910) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.6723.58 Safari/537.36",
            "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.6723.58 Mobile Safari/537.36 EdgA/130.0.2849.80",
            "Mozilla/5.0 (Linux; Android 14; SAMSUNG SM-S921B) AppleWebKit/537.36 (KHTML, like Gecko) SamsungBrowser/25.0 Chrome/121.0.6167.178 Mobile Safari/537.36",
        ] {
            assert!(wants_manifest(&ua(yes)), "{yes}");
        }
        // iPhone、iPad（报成 Mac 的 Safari）、iPhone 上的 Chrome、国产浏览器、WebView、火狐、电脑都不给
        for no in [
            "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Safari/605.1.15",
            "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/130.0.6723.90 Mobile/15E148 Safari/604.1",
            "Mozilla/5.0 (Linux; U; Android 13; zh-CN; 22081212C Build/TKQ1.220829.002) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/100.0.4896.58 Quark/6.4.5.420 Mobile Safari/537.36",
            "Mozilla/5.0 (Linux; U; Android 13; zh-CN; V2241A Build/TP1A.220624.014) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/100.0.4896.58 UCBrowser/16.0.0.1260 Mobile Safari/537.36",
            "Mozilla/5.0 (Linux; Android 12; HarmonyOS; NOH-AN00; HMSCore 6.11.0.302) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/99.0.4844.88 HuaweiBrowser/14.0.0.320 Mobile Safari/537.36",
            "Mozilla/5.0 (Linux; U; Android 13; zh-cn; 2211133C Build/TKQ1.220905.001) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/109.0.5414.118 Mobile Safari/537.36 XiaoMi/MiuiBrowser/17.8.120626",
            "Mozilla/5.0 (Linux; Android 13; V2241A; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/87.0.4280.141 Mobile Safari/537.36 VivoBrowser/14.5.310.1",
            "Mozilla/5.0 (Linux; U; Android 13; zh-cn; PHB110 Build/TP1A.220905.001) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/90.0.4430.61 Mobile Safari/537.36 HeyTapBrowser/40.8.312.1",
            "Mozilla/5.0 (Linux; U; Android 13; zh-cn) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/109.0 MQQBrowser/14.5 Mobile Safari/537.36",
            "Mozilla/5.0 (Android 14; Mobile; rv:132.0) Gecko/132.0 Firefox/132.0",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.6723.58 Safari/537.36 Edg/130.0.2849.80",
        ] {
            assert!(!wants_manifest(&ua(no)), "{no}");
        }
        assert!(!wants_manifest(&hyper::HeaderMap::new()));
    }

    #[test]
    fn ip_behind_proxy() {
        let mut h = hyper::HeaderMap::new();
        let local: IpAddr = "127.0.0.1".parse().unwrap();
        let far: IpAddr = "8.8.8.8".parse().unwrap();
        // 没有反向代理的头：就是连过来的那个
        assert_eq!(real_ip(&h, local), local);
        // X-Forwarded-For 认最后一个（反向代理加的）；第一个是浏览器自己写的，能随便编
        h.insert("x-forwarded-for", "1.2.3.4, 10.0.0.1".parse().unwrap());
        assert_eq!(real_ip(&h, local), "10.0.0.1".parse::<IpAddr>().unwrap());
        h.insert("x-real-ip", "5.6.7.8".parse().unwrap());
        assert_eq!(real_ip(&h, local), "5.6.7.8".parse::<IpAddr>().unwrap());
        // 不是从本机来的连接，头是别人随便写的，不信
        assert_eq!(real_ip(&h, far), far);
        // X-Real-IP 写坏了：退回去看 X-Forwarded-For；两个都没有能认的 IP，就是连过来的那个
        h.insert("x-real-ip", hyper::header::HeaderValue::from_static("bad"));
        assert_eq!(real_ip(&h, local), "10.0.0.1".parse::<IpAddr>().unwrap());
        h.insert("x-forwarded-for", hyper::header::HeaderValue::from_static("unknown"));
        assert_eq!(real_ip(&h, local), local);
        // 攻击：没有 X-Real-IP，浏览器自己在 X-Forwarded-For 前面写一个假的，想冒充别人（或者躲开屏蔽）
        let mut h = hyper::HeaderMap::new();
        h.insert("x-forwarded-for", "8.8.8.8, 203.0.113.9".parse().unwrap());
        assert_eq!(real_ip(&h, local), "203.0.113.9".parse::<IpAddr>().unwrap());
    }
}
