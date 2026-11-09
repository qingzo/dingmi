//! 协议的版本（需求-1.x 第 36.6 节；后台、官网那一半在官方版里）。
//!
//! - 三份协议（用户协议 terms、隐私说明 privacy、叮咪同步服务协议 sync_terms）在后台改、发布，官网写一份 terms.json。
//!   检查更新时顺便取一次（同样的节奏、同样的 User-Agent，见 update.rs check），存在 kv 里。
//! - 取不到、看不懂就当没有：不报错，不影响检查更新，留着上次存的。
//! - 用在三处：「设置 → 关于」里协议那一行写「M 月 D 日更新」（30 天内）；重大修改在主窗口顶上通知一次（点「知道了」按
//!   notice.version 记下，不再显示）；开通、续费叮咪同步时带的协议版本（sync_version，没取到过就用 TERMS_FALLBACK）。

use crate::{db, AppState};
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// 官网上的 terms.json。本地测试时可以用环境变量 DINGMI_TERMS_URL 换成别的地址
const TERMS_URL: &str = "https://dingmi.app/terms.json";
/// 存在 kv 里的：上次取到的 terms.json（整理过的，见 Terms）
const KV_TERMS: &str = "terms_json";
/// 点过「知道了」的通知：["sync_terms@2026-11-15", …]
const KV_SEEN: &str = "terms_seen";
/// 「M 月 D 日更新」、通知在生效后还给几天（和官网一样是 30 天）
const RECENT_DAYS: i64 = 30;
/// terms.json 最多多大，比这大的不认
const MAX_SIZE: usize = 64 * 1024;
/// 通知那段话最多几个字（后台限 300 字，这里多留一点）
const MAX_SUMMARY: usize = 400;

/// 三份协议的名字，和 terms.json 里的键一样
pub const KEYS: [&str; 3] = ["terms", "privacy", "sync_terms"];

/// 一份协议：version 是现在生效的那版（就是生效日），updated 是最近一次发布的日子（小修改也算）
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Doc {
    pub version: String,
    pub updated: String,
    /// 最近一次重大修改：生效日起 30 天后官网就不给了
    #[serde(default)]
    pub notice: Option<Notice>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Notice {
    pub version: String,
    pub effective: String,
    pub summary: String,
}

/// 整理过的 terms.json：哪份看不懂就当那份没有
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Terms {
    #[serde(default)]
    pub terms: Option<Doc>,
    #[serde(default)]
    pub privacy: Option<Doc>,
    #[serde(default)]
    pub sync_terms: Option<Doc>,
}

impl Terms {
    fn get(&self, key: &str) -> Option<&Doc> {
        match key {
            "terms" => self.terms.as_ref(),
            "privacy" => self.privacy.as_ref(),
            "sync_terms" => self.sync_terms.as_ref(),
            _ => None,
        }
    }
}

/// 给界面的：一份协议现在要显示什么
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct DocView {
    /// 30 天内更新过：更新的日子（YYYY-MM-DD），「关于」里那一行写「M 月 D 日更新」
    pub recent: Option<String>,
    /// 还没点过「知道了」、没过期的重大修改
    pub notice: Option<Notice>,
    /// 那次重大修改已经生效了（今天 ≥ 生效日）
    pub started: bool,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct View {
    pub terms: DocView,
    pub privacy: DocView,
    pub sync_terms: DocView,
}

static CACHE: Mutex<Option<Terms>> = Mutex::new(None);

/// 开通叮咪同步时同意的协议版本：terms.json 里的 sync_terms.version，没取到过就用写死的那个
pub fn sync_version() -> String {
    CACHE
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|t| t.sync_terms.as_ref())
        .map(|d| d.version.clone())
        .unwrap_or_else(|| crate::sync::TERMS_FALLBACK.into())
}

fn date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()
}

/// 像 YYYY-MM-DD 的字符串（统一写成两位的月、日）
fn date_str(v: &Value) -> Option<String> {
    date(v.as_str()?).map(|d| d.format("%Y-%m-%d").to_string())
}

fn parse_notice(v: &Value) -> Option<Notice> {
    let version = date_str(&v["version"])?;
    let effective = date_str(&v["effective"])?;
    let summary: String = v["summary"].as_str()?.trim().chars().take(MAX_SUMMARY).collect();
    (!summary.is_empty()).then_some(Notice { version, effective, summary })
}

/// 一份协议：version 要像日期；updated 缺了就当是 version 那天；notice 不完整就当没有
fn parse_doc(v: &Value) -> Option<Doc> {
    let version = date_str(&v["version"])?;
    let updated = date_str(&v["updated"]).unwrap_or_else(|| version.clone());
    Some(Doc { version, updated, notice: v.get("notice").and_then(parse_notice) })
}

/// 读 terms.json。整个看不懂、一份能用的都没有就回 None（当没有）
pub fn parse(text: &str) -> Option<Terms> {
    let v: Value = serde_json::from_str(text).ok()?;
    v.as_object()?;
    let t = Terms { terms: parse_doc(&v["terms"]), privacy: parse_doc(&v["privacy"]), sync_terms: parse_doc(&v["sync_terms"]) };
    (t != Terms::default()).then_some(t)
}

/// 今天该显示什么。seen：点过「知道了」的（"键@notice.version"）
pub fn view(t: &Terms, today: NaiveDate, seen: &[String]) -> View {
    let one = |key: &str| {
        let Some(d) = t.get(key) else { return DocView::default() };
        let ago = |s: &str| date(s).map(|x| (today - x).num_days());
        let recent = ago(&d.updated).filter(|n| (0..=RECENT_DAYS).contains(n)).map(|_| d.updated.clone());
        // 通知：生效日起 30 天后不再显示（官网也不给了，这里再拦一道，免得存着的旧的一直在）
        let notice = d
            .notice
            .clone()
            .filter(|n| ago(&n.effective).is_some_and(|days| days <= RECENT_DAYS))
            .filter(|n| !seen.contains(&format!("{key}@{}", n.version)));
        let started = notice.as_ref().is_some_and(|n| ago(&n.effective).is_some_and(|days| days >= 0));
        DocView { recent, notice, started }
    };
    View { terms: one("terms"), privacy: one("privacy"), sync_terms: one("sync_terms") }
}

fn seen_list(c: &rusqlite::Connection) -> Vec<String> {
    db::kv_get(c, KV_SEEN).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// 现在要显示什么（「设置 → 关于」、主窗口的横幅用）
pub fn current(app: &AppHandle) -> View {
    let seen = seen_list(&app.state::<AppState>().db.lock().unwrap());
    let t = CACHE.lock().unwrap().clone().unwrap_or_default();
    view(&t, Local::now().date_naive(), &seen)
}

/// 点了「知道了」：这份协议的这次修改不再显示
pub fn mark_seen(app: &AppHandle, key: &str, version: &str) {
    if !KEYS.contains(&key) {
        return;
    }
    {
        let st = app.state::<AppState>();
        let c = st.db.lock().unwrap();
        let mut seen = seen_list(&c);
        let k = format!("{key}@{version}");
        if !seen.contains(&k) {
            seen.push(k);
            // 只留最近的几十条，够用了
            let n = seen.len().saturating_sub(30);
            seen.drain(..n);
            let _ = db::kv_set(&c, KV_SEEN, &serde_json::to_string(&seen).unwrap_or_default());
        }
    }
    let _ = app.emit("terms-changed", current(app));
}

/// 启动时读出上次存的
pub fn load(app: &AppHandle) {
    let saved = db::kv_get(&app.state::<AppState>().db.lock().unwrap(), KV_TERMS);
    *CACHE.lock().unwrap() = saved.and_then(|s| serde_json::from_str(&s).ok());
}

fn terms_url() -> String {
    std::env::var("DINGMI_TERMS_URL").ok().filter(|u| !u.trim().is_empty()).unwrap_or_else(|| TERMS_URL.into())
}

/// 取一次 terms.json：取不到、看不懂、太大都当没有，留着上次的。
/// 太大的不先整个读进来再比：说了长度的先看长度，没说或者说少了的一边读一边数，超过 MAX_SIZE 就不读了
async fn fetch(client: &reqwest::Client, url: &str) -> Option<Terms> {
    let mut resp = client.get(url).timeout(Duration::from_secs(30)).send().await.ok().filter(|r| r.status().is_success())?;
    if resp.content_length().is_some_and(|n| n > MAX_SIZE as u64) {
        return None;
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.ok()? {
        if body.len() + chunk.len() > MAX_SIZE {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    parse(std::str::from_utf8(&body).ok()?)
}

/// 检查更新时顺便取一次（update.rs check 调的）。变了就存起来、告诉主窗口
pub async fn refresh(app: &AppHandle, client: &reqwest::Client) {
    let Some(t) = fetch(client, &terms_url()).await else {
        crate::diag::log("协议", "terms.json 没取到或者看不懂，不管它");
        return;
    };
    if CACHE.lock().unwrap().as_ref() == Some(&t) {
        return;
    }
    {
        let st = app.state::<AppState>();
        let _ = db::kv_set(&st.db.lock().unwrap(), KV_TERMS, &serde_json::to_string(&t).unwrap_or_default());
    }
    *CACHE.lock().unwrap() = Some(t);
    let _ = app.emit("terms-changed", current(app));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> NaiveDate {
        date(s).unwrap()
    }

    const GOOD: &str = r#"{
        "terms":{"version":"2026-11-05","updated":"2026-11-05"},
        "privacy":{"version":"2026-10-01","updated":"2026-11-20"},
        "sync_terms":{"version":"2026-11-08","updated":"2026-11-08",
            "notice":{"version":"2026-11-15","effective":"2026-11-15","summary":"  到期后保留的天数从 30 天改成 15 天。 "}},
        "later":{"version":"2027-01-01"}
    }"#;

    #[test]
    fn good() {
        let t = parse(GOOD).unwrap();
        assert_eq!(t.terms.as_ref().unwrap().version, "2026-11-05");
        assert_eq!(t.privacy.as_ref().unwrap().updated, "2026-11-20");
        let n = t.sync_terms.as_ref().unwrap().notice.clone().unwrap();
        assert_eq!((n.version.as_str(), n.effective.as_str()), ("2026-11-15", "2026-11-15"));
        assert_eq!(n.summary, "到期后保留的天数从 30 天改成 15 天。");

        // 11 月 21 日看：隐私说明 30 天内更新过；用户协议 11-05 也在 30 天内；还没生效
        let v = view(&t, day("2026-11-21"), &[]);
        assert_eq!(v.privacy.recent.as_deref(), Some("2026-11-20"));
        assert_eq!(v.terms.recent.as_deref(), Some("2026-11-05"));
        assert_eq!(v.sync_terms.notice.as_ref().unwrap().version, "2026-11-15");
        assert!(v.sync_terms.started);
        let v = view(&t, day("2026-11-10"), &[]);
        assert!(!v.sync_terms.started);
        // 12 月 10 日：用户协议过了 30 天不写了
        let v = view(&t, day("2026-12-10"), &[]);
        assert_eq!(v.terms.recent, None);
        assert_eq!(v.privacy.recent.as_deref(), Some("2026-11-20"));
        // 点过「知道了」的不再显示，别的版本照样显示
        let v = view(&t, day("2026-11-10"), &["sync_terms@2026-11-15".into()]);
        assert_eq!(v.sync_terms.notice, None);
        let v = view(&t, day("2026-11-10"), &["sync_terms@2026-10-01".into(), "terms@2026-11-15".into()]);
        assert!(v.sync_terms.notice.is_some());
        // 存进 kv 再读出来一样
        assert_eq!(serde_json::from_str::<Terms>(&serde_json::to_string(&t).unwrap()).unwrap(), t);
    }

    #[test]
    fn missing_fields() {
        // updated 缺了当是 version 那天；version 缺了那份不认；notice 缺 summary、生效日不认
        let t = parse(
            r#"{"terms":{"version":"2026-11-05"},"privacy":{"updated":"2026-11-05"},
                "sync_terms":{"version":"2026-11-08","notice":{"version":"2026-11-15","effective":"2026-11-15"}}}"#,
        )
        .unwrap();
        assert_eq!(t.terms.as_ref().unwrap().updated, "2026-11-05");
        assert_eq!(t.privacy, None);
        assert_eq!(t.sync_terms.as_ref().unwrap().notice, None);
        let t = parse(r#"{"sync_terms":{"version":"2026-11-08","notice":{"version":"2026-11-15","summary":"x"}}}"#).unwrap();
        assert_eq!(t.sync_terms.unwrap().notice, None);
        let t = parse(r#"{"sync_terms":{"version":"2026-11-08","notice":{"version":"2026-11-15","effective":"2026-11-15","summary":"   "}}}"#).unwrap();
        assert_eq!(t.sync_terms.unwrap().notice, None);
        // 只有用户协议：没有同步协议，开通时用写死的
        let t = parse(r#"{"terms":{"version":"2026-11-05"}}"#).unwrap();
        assert_eq!(t.sync_terms, None);
        // 一份能用的都没有：当没有
        assert_eq!(parse(r#"{}"#), None);
        assert_eq!(parse(r#"{"terms":{}}"#), None);
    }

    #[test]
    fn garbage() {
        for s in ["", "not json", "[]", "null", "\"x\"", r#"{"terms":"2026-11-05"}"#, r#"{"terms":{"version":"昨天"}}"#, r#"{"terms":{"version":20261105}}"#, r#"{"terms":{"version":"2026-13-45"}}"#, "<html>404</html>"] {
            assert_eq!(parse(s), None, "{s}");
        }
        // 日期写成一位的月、日也认，统一成两位
        assert_eq!(parse(r#"{"terms":{"version":"2026-1-5"}}"#).unwrap().terms.unwrap().version, "2026-01-05");
        // 一份乱写的不影响别的
        let t = parse(r#"{"terms":{"version":"x"},"sync_terms":{"version":"2026-11-08","updated":"乱写","notice":"x"}}"#).unwrap();
        assert_eq!(t.terms, None);
        let s = t.sync_terms.unwrap();
        assert_eq!((s.updated.as_str(), s.notice), ("2026-11-08", None));
    }

    #[test]
    fn notice_expires() {
        let t = parse(GOOD).unwrap();
        // 生效日 11-15：30 天内还显示，第 31 天起不显示（官网不给了，存着的也不再显示）
        assert!(view(&t, day("2026-12-15"), &[]).sync_terms.notice.is_some());
        let v = view(&t, day("2026-12-16"), &[]);
        assert_eq!((v.sync_terms.notice, v.sync_terms.started), (None, false));
        // 生效日写错（比如写成很久以前的）也一样不显示
        let old = parse(r#"{"privacy":{"version":"2025-01-01","notice":{"version":"2025-01-01","effective":"2025-01-01","summary":"x"}}}"#).unwrap();
        assert_eq!(view(&old, day("2026-11-08"), &[]).privacy.notice, None);
        // 更新日比今天还晚（电脑的时间不对）：不写「M 月 D 日更新」
        assert_eq!(view(&t, day("2026-11-01"), &[]).sync_terms.recent, None);
    }

    #[test]
    fn version_falls_back() {
        // 没取到过：用写死的那个
        *CACHE.lock().unwrap() = None;
        assert_eq!(sync_version(), crate::sync::TERMS_FALLBACK);
        *CACHE.lock().unwrap() = parse(r#"{"terms":{"version":"2026-11-05"}}"#);
        assert_eq!(sync_version(), crate::sync::TERMS_FALLBACK);
        *CACHE.lock().unwrap() = parse(GOOD);
        assert_eq!(sync_version(), "2026-11-08");
        *CACHE.lock().unwrap() = None;
    }

    /// 在 127.0.0.1 上起一个只回一样东西的 HTTP 服务（status 200 或 404）
    fn serve_once(body: &'static str, ok: bool) -> String {
        serve(body.to_string(), ok, true)
    }

    /// 同上；length 是 false 时不说长度（读到断开为止）
    fn serve(body: String, ok: bool, length: bool) -> String {
        use std::io::{Read, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/terms.json", l.local_addr().unwrap());
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf);
                let head = if ok { "200 OK" } else { "404 Not Found" };
                let len = if length { format!("Content-Length: {}\r\n", body.len()) } else { String::new() };
                let _ = s.write_all(format!("HTTP/1.1 {head}\r\n{len}Connection: close\r\n\r\n{body}").as_bytes());
            }
        });
        url
    }

    #[tokio::test]
    async fn fetch_terms_json() {
        let c = reqwest::Client::builder().user_agent("Dingmi/0.1.0").no_proxy().build().unwrap();
        assert_eq!(fetch(&c, &serve_once(GOOD, true)).await, parse(GOOD));
        assert_eq!(fetch(&c, &serve_once("{oops", true)).await, None);
        assert_eq!(fetch(&c, &serve_once(GOOD, false)).await, None);
        // 不说长度的也能读
        assert_eq!(fetch(&c, &serve(GOOD.into(), true, false)).await, parse(GOOD));
    }

    #[tokio::test]
    async fn fetch_too_big() {
        let c = reqwest::Client::builder().user_agent("Dingmi/0.1.0").no_proxy().build().unwrap();
        // 后面补空格：还是能看懂的 JSON，只是大小不一样
        let pad = |n: usize| format!("{GOOD}{}", " ".repeat(n - GOOD.len()));
        assert_eq!(fetch(&c, &serve(pad(MAX_SIZE), true, true)).await, parse(GOOD));
        assert_eq!(fetch(&c, &serve(pad(MAX_SIZE), true, false)).await, parse(GOOD));
        // 大了一个字节：说了长度的看长度就不要；没说长度的读着读着超了也不要
        assert_eq!(fetch(&c, &serve(pad(MAX_SIZE + 1), true, true)).await, None);
        assert_eq!(fetch(&c, &serve(pad(MAX_SIZE + 1), true, false)).await, None);
        assert_eq!(fetch(&c, &serve(pad(4 * MAX_SIZE), true, false)).await, None);
    }
}
