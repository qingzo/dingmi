//! 设置。一部分几台电脑共用（开启同步后作为一条记录同步，见 `Shared`），快捷键、开机自启这类只属于这台电脑。

use crate::db;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub hk_flash: String,
    pub hk_qnote: String,
    pub hk_main: String,
    pub hk_focus: String,
    pub ai_base: String,
    pub ai_model: String,
    pub ai_key: String,
    /// 每天第一次开机时显示开场白
    pub morning: bool,
    /// 全屏或演示时自动进入专注模式
    pub auto_focus: bool,
    pub snooze_min: u32,
    /// 提醒没处理时，每隔几分钟再响一次；0 表示不重复响
    pub ring_every_min: u32,
    pub sound: bool,
    pub autostart: bool,
    /// 每天收尾：到了 wrap_time（"HH:MM"）弹一张卡片
    pub wrap: bool,
    pub wrap_time: String,
    /// 手机发来文字后自动「整理一下」（整理完仍然要确认）
    pub inbox_auto: bool,
    /// 开启同步后 AI Key 也同步到别的电脑（加密后上传）
    pub sync_ai_key: bool,
    /// 按闪念快捷键时，把别的程序里选中的文字带进输入框
    pub flash_selection: bool,
    /// 提醒的分类，可以自己增删改；提醒上存的是 id
    pub tags: Vec<TagDef>,
    /// 笔记的分类（v0.3 起和提醒的分开），笔记上存的是 id
    pub note_cats: Vec<TagDef>,
    /// 旧数据里笔记用的提醒分类已经搬进笔记分类了（见 upgrade_note_cats）
    pub note_cats_ready: bool,
    /// 自动检查更新（只属于这台电脑）：关了就只在「设置 → 关于」里点按钮时检查
    pub auto_update: bool,
    /// 桌面组件（只属于这台电脑）
    pub widget: bool,
    /// 桌面组件里不显示的几块：next 下一条 / today 今天 / toread 待看 / pins 常用 / input 输入框
    pub widget_hide: Vec<String>,
    /// 桌面组件用毛玻璃的底（只属于这台电脑）；默认不透明，壁纸再花字也清楚（UI设计.md U1）
    pub widget_glass: bool,
    /// 毛玻璃深浅 0–100：越大叠在上面的颜色越浓、越像磨砂，越小越透（Windows 的模糊程度改不了）；50 是原来的样子
    pub widget_glass_level: u32,
    /// 启动器：放在组件上的、别名、自己加的、不显示的（只属于这台电脑，每台电脑装的程序不一样）
    pub launcher: crate::launcher::Config,
    /// 快捷框空着时「常用」显示几排（1 或 2，一排 8 个；只属于这台电脑）
    pub pin_rows: u32,
    /// 空格启动（实验，需求-1.x 第 37 节）：快捷框里打了字以后按空格也能打开选中的程序；默认关，只属于这台电脑
    pub flash_space: bool,
    /// 外观（只属于这台电脑，需求-0.2 第 29 节）：风格 warm 暖云母 / simple 简洁 / calm 安静
    pub style: String,
    /// 深浅色：system 跟随系统 / light / dark
    pub theme: String,
    /// 字号：s / m / l，只影响主窗口和笔记窗口
    pub font_size: String,
    /// 团子猫：None 表示没手动改过，跟着风格走（简洁默认关，另外两种默认开）
    pub mascot: Option<bool>,
    /// 动画：on 启用动画（默认，不管 Windows 怎么设）/ system 跟随 Windows 的「动画效果」/ reduce 减少动画
    pub motion: String,
    /// 随手算：快捷框里打算式直接出结果（需求-0.3 第 10.1 节）
    pub calc: bool,
    /// 网页关键字：快捷框里打「bd 天气」用百度搜（需求-0.3 第 10.2 节），几台电脑共用
    pub web_keys: Vec<WebKey>,
}

/// 一条网页关键字。url 里搜的内容用 {q} 代替
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WebKey {
    pub name: String,
    pub key: String,
    pub url: String,
}

pub fn default_web_keys() -> Vec<WebKey> {
    [
        ("百度", "bd", "https://www.baidu.com/s?wd={q}"),
        ("必应", "by", "https://cn.bing.com/search?q={q}"),
        ("知乎", "zh", "https://www.zhihu.com/search?type=content&q={q}"),
        ("B 站", "bl", "https://search.bilibili.com/all?keyword={q}"),
        ("淘宝", "tb", "https://s.taobao.com/search?q={q}"),
        ("京东", "jd", "https://search.jd.com/Search?keyword={q}"),
        ("GitHub", "gh", "https://github.com/search?q={q}"),
        ("百度翻译", "fy", "https://fanyi.baidu.com/#auto/zh/{q}"),
    ]
    .iter()
    .map(|&(name, key, url)| WebKey { name: name.into(), key: key.into(), url: url.into() })
    .collect()
}

/// 一个分类。默认的 id 是 work / life / health / like / cycle（health 是 0.4 加的，之前装的没有），新加的是随机的
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TagDef {
    pub id: String,
    pub name: String,
    /// 色板里的名字（blue、orange……），见 base.css 的 --tc-*
    pub color: String,
}

fn defs(list: &[(&str, &str, &str)]) -> Vec<TagDef> {
    list.iter().map(|&(id, name, color)| TagDef { id: id.into(), name: name.into(), color: color.into() }).collect()
}

pub fn default_tags() -> Vec<TagDef> {
    defs(&[("work", "工作", "blue"), ("life", "生活", "orange"), ("health", "健康", "green"), ("like", "喜好", "pink"), ("cycle", "周期", "purple")])
}

pub fn default_note_cats() -> Vec<TagDef> {
    defs(&[("toread", "待看", "orange"), ("idea", "灵感", "purple"), ("ref", "资料", "teal")])
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hk_flash: "Alt+Space".into(),
            hk_qnote: "Ctrl+Alt+N".into(),
            hk_main: "Ctrl+Alt+H".into(),
            hk_focus: "Ctrl+Alt+F".into(),
            ai_base: "https://api.deepseek.com".into(),
            ai_model: "deepseek-chat".into(),
            ai_key: String::new(),
            morning: true,
            auto_focus: true,
            snooze_min: 10,
            ring_every_min: 5,
            sound: true,
            autostart: true,
            wrap: true,
            wrap_time: "18:00".into(),
            inbox_auto: false,
            sync_ai_key: true,
            flash_selection: true,
            tags: default_tags(),
            note_cats: default_note_cats(),
            note_cats_ready: false,
            auto_update: true,
            widget: false,
            widget_hide: Vec::new(),
            widget_glass: false,
            widget_glass_level: 50,
            pin_rows: 1,
            flash_space: false,
            launcher: Default::default(),
            style: "warm".into(),
            theme: "system".into(),
            font_size: "m".into(),
            mascot: None,
            motion: "on".into(),
            calc: true,
            web_keys: default_web_keys(),
        }
    }
}

/// 几台电脑共用的设置，同步时是 uid 为 "settings" 的一条记录
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Shared {
    pub ai_base: String,
    pub ai_model: String,
    /// 关掉「同步 AI Key」或者没填时是 None，收到的电脑保留自己的
    pub ai_key: Option<String>,
    pub morning: bool,
    pub snooze_min: u32,
    pub ring_every_min: u32,
    pub sound: bool,
    pub wrap: bool,
    pub wrap_time: String,
    pub inbox_auto: bool,
    pub sync_ai_key: bool,
    /// 旧版本传来的没有这一项，收到的电脑保留自己的
    #[serde(default)]
    pub tags: Option<Vec<TagDef>>,
    /// 笔记分类（v0.3 加的），旧版本传来的没有，同上
    #[serde(default)]
    pub note_cats: Option<Vec<TagDef>>,
    /// 随手算、网页关键字（0.3 加的），旧版本传来的没有，同上
    #[serde(default)]
    pub calc: Option<bool>,
    #[serde(default)]
    pub web_keys: Option<Vec<WebKey>>,
}

pub const SHARED_UID: &str = "settings";

impl Settings {
    pub fn shared(&self) -> Shared {
        let key = self.ai_key.trim();
        Shared {
            ai_base: self.ai_base.clone(),
            ai_model: self.ai_model.clone(),
            ai_key: (self.sync_ai_key && !key.is_empty()).then(|| key.to_string()),
            morning: self.morning,
            snooze_min: self.snooze_min,
            ring_every_min: self.ring_every_min,
            sound: self.sound,
            wrap: self.wrap,
            wrap_time: self.wrap_time.clone(),
            inbox_auto: self.inbox_auto,
            sync_ai_key: self.sync_ai_key,
            tags: Some(self.tags.clone()),
            note_cats: Some(self.note_cats.clone()),
            calc: Some(self.calc),
            web_keys: Some(self.web_keys.clone()),
        }
    }

    pub fn merge_shared(&mut self, s: &Shared) {
        self.ai_base = s.ai_base.clone();
        self.ai_model = s.ai_model.clone();
        if let Some(k) = s.ai_key.as_deref().filter(|k| !k.is_empty()) {
            self.ai_key = k.to_string();
        }
        self.morning = s.morning;
        self.snooze_min = s.snooze_min;
        self.ring_every_min = s.ring_every_min;
        self.sound = s.sound;
        self.wrap = s.wrap;
        self.wrap_time = s.wrap_time.clone();
        self.inbox_auto = s.inbox_auto;
        self.sync_ai_key = s.sync_ai_key;
        if let Some(t) = &s.tags {
            self.tags = t.clone();
        }
        if let Some(t) = &s.note_cats {
            self.note_cats = t.clone();
        }
        if let Some(c) = s.calc {
            self.calc = c;
        }
        if let Some(k) = &s.web_keys {
            self.web_keys = k.clone();
        }
    }
}

pub fn load(c: &Connection) -> Option<Settings> {
    db::kv_get(c, "settings").and_then(|t| serde_json::from_str(&t).ok())
}

/// 存本机设置；共用的那部分变了就记一笔，等着同步给别的电脑
pub fn save(c: &Connection, s: &Settings) -> rusqlite::Result<()> {
    let text = serde_json::to_string(s).unwrap_or_default();
    db::kv_set(c, "settings", &text)?;
    let shared = serde_json::to_string(&s.shared()).unwrap_or_default();
    db::shared_set(c, SHARED_UID, &shared)?;
    Ok(())
}

/// 只存本机（第一次运行自动换快捷键时用）：不算改了共用设置，免得新电脑的默认设置盖掉别的电脑上的
pub fn save_local(c: &Connection, s: &Settings) -> rusqlite::Result<()> {
    db::kv_set(c, "settings", &serde_json::to_string(s).unwrap_or_default())
}

/// 从没同步过设置的旧数据（改过设置、但还没有共用记录）：补建一条版本号最小的，
/// 别的电脑上真改过的会盖过它；和默认设置一样的不建，免得新装的电脑把默认值传出去
pub fn ensure_shared(c: &Connection, s: &Settings, device: &str) -> rusqlite::Result<()> {
    let shared = s.shared();
    if shared == Settings::default().shared() || db::shared_get(c, SHARED_UID).is_some() {
        return Ok(());
    }
    let text = serde_json::to_string(&shared).unwrap_or_default();
    db::shared_init(c, SHARED_UID, &text, &format!("0000000000000-0000-{device}"))
}

/// v0.3 起笔记有自己的一套分类。旧数据里笔记上存的还是提醒分类的 id：把用到的那些按同样的 id、名字、颜色
/// 加进笔记分类（几台电脑各自搬，结果一样），笔记本身不用改。只做一次。
/// 没有要搬的（比如新装的电脑）只存本机，免得把默认设置当成改动传给别的电脑
pub fn upgrade_note_cats(c: &Connection, s: &mut Settings) -> rusqlite::Result<()> {
    if s.note_cats_ready {
        return Ok(());
    }
    let used: Vec<String> = {
        let mut st = c.prepare("SELECT DISTINCT tag FROM notes WHERE tag IS NOT NULL AND purged=0 ORDER BY tag")?;
        let rows = st.query_map([], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut moved = false;
    for id in used {
        if s.note_cats.iter().any(|t| t.id == id) {
            continue;
        }
        if let Some(t) = s.tags.iter().find(|t| t.id == id) {
            s.note_cats.push(t.clone());
            moved = true;
        }
    }
    s.note_cats_ready = true;
    if moved {
        save(c, s)
    } else {
        save_local(c, s)
    }
}

/// 别的电脑传来了新的共用设置：合进本机设置并存好
pub fn apply_shared(c: &Connection, value: &str) -> rusqlite::Result<()> {
    let Ok(shared) = serde_json::from_str::<Shared>(value) else {
        return Ok(());
    };
    let mut s = load(c).unwrap_or_default();
    s.merge_shared(&shared);
    save_local(c, &s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_part_and_merge() {
        let mut a = Settings { ai_key: "sk-a".into(), snooze_min: 15, hk_flash: "Alt+Q".into(), ..Default::default() };
        let mut b = Settings { hk_flash: "Ctrl+Space".into(), autostart: false, ..Default::default() };
        b.merge_shared(&a.shared());
        // 共用的过来了，快捷键、开机自启不动
        assert_eq!((b.ai_key.as_str(), b.snooze_min), ("sk-a", 15));
        assert_eq!((b.hk_flash.as_str(), b.autostart), ("Ctrl+Space", false));
        // 关掉同步 AI Key：不传 Key，对方保留自己的
        a.sync_ai_key = false;
        assert_eq!(a.shared().ai_key, None);
        let mut c = Settings { ai_key: "sk-c".into(), ..Default::default() };
        c.merge_shared(&a.shared());
        assert_eq!(c.ai_key, "sk-c");
        assert!(!c.sync_ai_key);
        // 分类跟着走；旧版本传来的没有分类，保留自己的
        a.tags.truncate(1);
        c.merge_shared(&a.shared());
        assert_eq!(c.tags.len(), 1);
        let old: Shared = serde_json::from_str(&serde_json::to_string(&Settings::default().shared()).unwrap().replace(r#","tags":"#, r#","x":"#)).unwrap();
        c.merge_shared(&old);
        assert_eq!(c.tags.len(), 1);
        // 笔记分类也跟着走；旧版本传来的没有笔记分类，保留自己的
        a.note_cats.truncate(1);
        c.merge_shared(&a.shared());
        assert_eq!(c.note_cats.len(), 1);
        let old: Shared = serde_json::from_str(&serde_json::to_string(&Settings::default().shared()).unwrap().replace(r#","note_cats":"#, r#","y":"#)).unwrap();
        c.merge_shared(&old);
        assert_eq!(c.note_cats.len(), 1);
        // 网页关键字、随手算也跟着走；旧版本传来的没有，保留自己的
        a.web_keys.truncate(2);
        a.calc = false;
        c.merge_shared(&a.shared());
        assert_eq!((c.web_keys.len(), c.calc), (2, false));
        let old: Shared = serde_json::from_str(r#"{"ai_base":"","ai_model":"","ai_key":null,"morning":true,"snooze_min":10,"ring_every_min":5,"sound":true,"wrap":true,"wrap_time":"18:00","inbox_auto":false,"sync_ai_key":true}"#).unwrap();
        c.merge_shared(&old);
        assert_eq!((c.web_keys.len(), c.calc), (2, false));
    }

    #[test]
    fn upgrade_moves_used_note_tags() {
        let dir = std::env::temp_dir().join(format!("dingmi-notecats-{}", uuid::Uuid::new_v4().simple()));
        let c = db::open(&dir).unwrap();
        let note = |tag: Option<&str>| db::Note { title: "x".into(), tag: tag.map(Into::into), ..Default::default() };
        for t in [Some("work"), Some("work"), Some("like"), Some("gone"), None] {
            db::upsert_note(&c, &note(t)).unwrap();
        }
        // 旧设置里没有笔记分类：用到的「工作」「喜好」原样搬过来，接在默认的后面；已经删掉的分类不搬
        let mut s: Settings = serde_json::from_str(r#"{"hk_flash":"Alt+Q"}"#).unwrap();
        upgrade_note_cats(&c, &mut s).unwrap();
        let ids: Vec<&str> = s.note_cats.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["toread", "idea", "ref", "like", "work"]);
        assert_eq!(s.note_cats[3].name, "喜好");
        assert!(load(&c).unwrap().note_cats_ready);
        // 有东西搬过来才算改了共用设置，要同步给别的电脑
        assert!(db::shared_get(&c, SHARED_UID).is_some());
        // 只做一次：之后删掉的笔记分类不会再被搬回来
        s.note_cats.truncate(3);
        upgrade_note_cats(&c, &mut s).unwrap();
        assert_eq!(s.note_cats.len(), 3);

        // 新装的电脑：没东西可搬，只存本机，不当成共用设置的改动
        let dir2 = std::env::temp_dir().join(format!("dingmi-notecats-{}", uuid::Uuid::new_v4().simple()));
        let c2 = db::open(&dir2).unwrap();
        let mut fresh = Settings::default();
        upgrade_note_cats(&c2, &mut fresh).unwrap();
        assert!(fresh.note_cats_ready && db::shared_get(&c2, SHARED_UID).is_none());
    }
}
