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
    /// 提示音（需求-1.x 第 42.5 节）：dingmi 叮咪 / bright 清脆 / marimba 木琴 / soft 柔和 / bell 铃铛 / alarm 闹钟 / custom 自己选的文件
    pub sound_kind: String,
    /// 紧急的用哪种；空着是和上面一样、响一点
    pub sound_urgent: String,
    /// 音量 0–100
    pub sound_volume: u32,
    /// 一直响到点了卡片为止（最多 1 分钟）；默认关，怕打扰（作者定）
    pub sound_loop: bool,
    /// 自己选的声音文件，复制到数据目录的 sounds/ 里（只属于这台电脑；别的电脑没有这个文件就用叮咪）
    pub sound_file: String,
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
    /// 提醒加图片时新建的那篇笔记放进哪个笔记分类（需求-1.x 第 45 节）；空着是不分类，默认「提醒附图」
    pub pic_cat: String,
    /// 「全部」里也显示 pic_cat 里的笔记；默认不显示，免得提醒的图片把笔记本堆满
    pub pic_cat_all: bool,
    /// 已经加过「提醒附图」、把以前提醒的图片笔记搬进去了（只属于这台电脑，见 upgrade_pic_cat）
    pub pic_cat_ready: bool,
    /// 升级时搬了几篇，笔记本顶上说一声，点了「知道了」清零（只属于这台电脑）
    pub pic_cat_moved: u32,
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
    /// 快捷框空着时「最近用过」显示几排（1–3，一排 8 个；常用里放的另外在下面全显示；只属于这台电脑）
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
    /// 网盘文件夹同步：没有改动时隔几小时更新一次「这台电脑最后出现的时间」，1 / 6 / 12，别的数当 6（只属于这台电脑）。
    /// 坚果云这类网盘每传一个文件弹一次通知，嫌烦的选长一点（2026-11-18 用户反馈后加的）
    pub folder_beat_hours: u32,
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
    /// 这个分类的提示音（只有提醒分类用，需求-1.x 第 42.5 节）；空着跟默认。旧版本没有这一项
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sound: String,
    /// 上一层（只有笔记分类用：笔记文件夹，需求-1.x 第 45.3 节）；None 是最外层。旧版本没有这一项，平着显示
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

/// 笔记文件夹最多几个、几层（需求-1.x 第 45.3 节）；提醒分类还是 12 个
pub const MAX_NOTE_CATS: usize = 60;
pub const MAX_NOTE_DEPTH: usize = 3;

/// 笔记文件夹从最外层到它自己的名字（需求-1.x 第 45.3 节）；上一层找不到的当最外层，绕成圈的最多走 MAX_NOTE_DEPTH 层
pub fn note_cat_path<'a>(list: &'a [TagDef], id: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut cur = list.iter().find(|t| t.id == id);
    while let Some(t) = cur {
        out.insert(0, t.name.as_str());
        if out.len() >= MAX_NOTE_DEPTH {
            break;
        }
        cur = t.parent.as_deref().and_then(|p| list.iter().find(|x| x.id == p));
    }
    out
}

/// 笔记文件夹删掉一个：里面的子文件夹放回上一层，回上一层的 id（None 是最外层）。没有这个文件夹回 Err(())
pub fn remove_note_cat(s: &mut Settings, id: &str) -> Result<Option<String>, ()> {
    let i = s.note_cats.iter().position(|t| t.id == id).ok_or(())?;
    let up = s.note_cats.remove(i).parent;
    for t in s.note_cats.iter_mut().filter(|t| t.parent.as_deref() == Some(id)) {
        t.parent = up.clone();
    }
    s.fix_pic_cat();
    Ok(up)
}

fn defs(list: &[(&str, &str, &str)]) -> Vec<TagDef> {
    list.iter().map(|&(id, name, color)| TagDef { id: id.into(), name: name.into(), color: color.into(), sound: String::new(), parent: None }).collect()
}

pub fn default_tags() -> Vec<TagDef> {
    defs(&[("work", "工作", "blue"), ("life", "生活", "orange"), ("health", "健康", "green"), ("like", "喜好", "pink"), ("cycle", "周期", "purple")])
}

pub fn default_note_cats() -> Vec<TagDef> {
    defs(&[("toread", "待看", "orange"), ("idea", "灵感", "purple"), ("ref", "资料", "teal"), (PIC_CAT, "提醒附图", "gray")])
}

/// 放提醒图片的笔记分类（1.0.13 加的，需求-1.x 第 45 节）
pub const PIC_CAT: &str = "rpic";

fn pic_cat_def() -> TagDef {
    default_note_cats().into_iter().find(|t| t.id == PIC_CAT).unwrap()
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
            sound_kind: "dingmi".into(),
            sound_urgent: String::new(),
            sound_volume: 70,
            sound_loop: false,
            sound_file: String::new(),
            autostart: true,
            wrap: true,
            wrap_time: "18:00".into(),
            inbox_auto: false,
            sync_ai_key: true,
            flash_selection: true,
            tags: default_tags(),
            note_cats: default_note_cats(),
            note_cats_ready: false,
            pic_cat: PIC_CAT.into(),
            pic_cat_all: false,
            pic_cat_ready: false,
            pic_cat_moved: 0,
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
            folder_beat_hours: 6,
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
    /// 提示音的几项（1.0.10 加的），旧版本传来的没有，同上
    #[serde(default)]
    pub sound_kind: Option<String>,
    #[serde(default)]
    pub sound_urgent: Option<String>,
    #[serde(default)]
    pub sound_volume: Option<u32>,
    #[serde(default)]
    pub sound_loop: Option<bool>,
    /// 提醒的图片放哪个分类、「全部」里显不显示（1.0.13 加的），旧版本传来的没有，同上
    #[serde(default)]
    pub pic_cat: Option<String>,
    #[serde(default)]
    pub pic_cat_all: Option<bool>,
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
            sound_kind: Some(self.sound_kind.clone()),
            sound_urgent: Some(self.sound_urgent.clone()),
            sound_volume: Some(self.sound_volume),
            sound_loop: Some(self.sound_loop),
            pic_cat: Some(self.pic_cat.clone()),
            pic_cat_all: Some(self.pic_cat_all),
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
        if let Some(k) = &s.sound_kind {
            self.sound_kind = k.clone();
        }
        if let Some(k) = &s.sound_urgent {
            self.sound_urgent = k.clone();
        }
        if let Some(v) = s.sound_volume {
            self.sound_volume = v.min(100);
        }
        if let Some(l) = s.sound_loop {
            self.sound_loop = l;
        }
        if let Some(p) = &s.pic_cat {
            self.pic_cat = p.clone();
        }
        if let Some(a) = s.pic_cat_all {
            self.pic_cat_all = a;
        }
        // 传来的笔记分类里没有「提醒附图」，可图片还放在那（旧版本传来的、那台电脑还没升级时存的）：放回来，
        // 不然这些笔记看着像没分类。真删掉了的，删的时候已经改成不分类了（fix_pic_cat）
        if self.pic_cat == PIC_CAT && !self.note_cats.iter().any(|t| t.id == PIC_CAT) {
            self.note_cats.push(pic_cat_def());
        }
        self.fix_pic_cat();
    }

    /// 放提醒图片的分类被删掉了：以后不分类
    pub fn fix_pic_cat(&mut self) {
        if !self.pic_cat.is_empty() && !self.note_cats.iter().any(|t| t.id == self.pic_cat) {
            self.pic_cat.clear();
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

/// 1.0.13 加了「提醒附图」（需求-1.x 第 45 节）：笔记分类里补上它，以前提醒加图片时建的笔记（关联了提醒、没有分类、
/// 正文只有图片）搬进去，正文有字的不动。只做一次，回搬了几篇。
/// 几台电脑各自做，结果一样；分类只存本机，免得盖掉别的电脑上刚删掉它的设置（下次改共用设置时一起传出去）
pub fn upgrade_pic_cat(c: &Connection, s: &mut Settings) -> rusqlite::Result<u32> {
    if s.pic_cat_ready {
        return Ok(0);
    }
    if s.pic_cat == PIC_CAT && !s.note_cats.iter().any(|t| t.id == PIC_CAT) {
        s.note_cats.push(pic_cat_def());
    }
    s.fix_pic_cat();
    let mut moved = 0;
    if !s.pic_cat.is_empty() {
        let ids: Vec<(i64, String)> = {
            let mut st = c.prepare("SELECT id, body FROM notes WHERE tag IS NULL AND reminder_id IS NOT NULL AND deleted_at IS NULL AND purged=0")?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for (id, body) in ids {
            if only_pics(&body) {
                db::set_note_tag(c, id, Some(&s.pic_cat))?;
                moved += 1;
            }
        }
    }
    s.pic_cat_ready = true;
    s.pic_cat_moved = moved;
    save_local(c, s)?;
    Ok(moved)
}

/// 正文只有图片（`![](att:文件名)` 一张接一张，中间只有空白）
fn only_pics(body: &str) -> bool {
    let mut rest = body.trim_start();
    if rest.is_empty() {
        return false;
    }
    while !rest.is_empty() {
        let Some(r) = rest.strip_prefix("![") else { return false };
        let Some(close) = r.find("](att:") else { return false };
        if r[..close].contains(']') {
            return false;
        }
        let r = &r[close + 6..];
        let Some(end) = r.find(')') else { return false };
        let name = &r[..end];
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
            return false;
        }
        rest = r[end + 1..].trim_start();
    }
    true
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
        // （放图片的分类也删了，存的时候 fix_pic_cat 会改成不分类）
        a.note_cats.truncate(1);
        a.fix_pic_cat();
        c.merge_shared(&a.shared());
        assert_eq!(c.note_cats.len(), 1);
        let old: Shared = serde_json::from_str(&serde_json::to_string(&Settings::default().shared()).unwrap().replace(r#","note_cats":"#, r#","y":"#).replace(r#""pic_cat":"#, r#""z":"#)).unwrap();
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
        // 提示音跟着走，自己选的声音文件不走；旧版本传来的没有，保留自己的（需求-1.x 第 42.5 节）
        a.sound_kind = "alarm".into();
        a.sound_urgent = "bell".into();
        a.sound_volume = 100;
        a.sound_loop = true;
        a.sound_file = r"C:\a\sounds\custom.mp3".into();
        c.merge_shared(&a.shared());
        assert_eq!((c.sound_kind.as_str(), c.sound_urgent.as_str(), c.sound_volume, c.sound_loop, c.sound_file.as_str()), ("alarm", "bell", 100, true, ""));
        c.merge_shared(&old);
        assert_eq!((c.sound_kind.as_str(), c.sound_volume, c.sound_loop), ("alarm", 100, true));
        // 分类上的提示音：旧版本存的没有这一项；没设的不写出去
        let t: TagDef = serde_json::from_str(r#"{"id":"work","name":"工作","color":"blue"}"#).unwrap();
        assert_eq!(t.sound, "");
        assert!(!serde_json::to_string(&t).unwrap().contains("sound"));
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
        assert_eq!(ids, ["toread", "idea", "ref", "rpic", "like", "work"]);
        assert_eq!(s.note_cats[4].name, "喜好");
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

    #[test]
    fn upgrade_moves_reminder_pics() {
        let dir = std::env::temp_dir().join(format!("dingmi-piccat-{}", uuid::Uuid::new_v4().simple()));
        let c = db::open(&dir).unwrap();
        let rid = db::upsert_reminder(&c, &db::Reminder { title: "报表修改".into(), due_at: "2026-11-26 15:45".into(), ..Default::default() }).unwrap();
        let add = |body: &str, tag: Option<&str>, rem: bool| {
            let n = db::Note { title: "x".into(), body: body.into(), tag: tag.map(Into::into), reminder_id: rem.then_some(rid), ..Default::default() };
            db::upsert_note(&c, &n).unwrap()
        };
        let pics = add("![](att:a1.png)\n![](att:b-2.jpg)\n", None, true);
        let text = add("交周报\n\n![](att:c.png)", None, true);
        let tagged = add("![](att:d.png)", Some("ref"), true);
        let loose = add("![](att:e.png)", None, false);
        let empty = add("", None, true);
        let tag = |id: i64| db::list_notes(&c).unwrap().into_iter().find(|n| n.id == id).unwrap().tag;
        let before = db::list_notes(&c).unwrap().into_iter().find(|n| n.id == pics).unwrap().updated_at;

        // 1.0.12 的设置：笔记分类里没有「提醒附图」，补在后面；只搬关联了提醒、没分类、正文只有图片的
        let mut s: Settings = serde_json::from_str(r#"{"note_cats":[{"id":"toread","name":"待看","color":"orange"}],"note_cats_ready":true}"#).unwrap();
        assert_eq!(upgrade_pic_cat(&c, &mut s).unwrap(), 1);
        assert_eq!(s.note_cats.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["toread", "rpic"]);
        assert_eq!(tag(pics).as_deref(), Some("rpic"));
        assert_eq!([tag(text), tag(tagged), tag(loose), tag(empty)], [None, Some("ref".into()), None, None]);
        // 搬了不算改过内容；本机记着搬了几篇；分类只存本机，不当成共用设置的改动
        assert_eq!(db::list_notes(&c).unwrap().into_iter().find(|n| n.id == pics).unwrap().updated_at, before);
        let saved = load(&c).unwrap();
        assert!(saved.pic_cat_ready && saved.pic_cat_moved == 1 && !saved.pic_cat_all);
        assert!(db::shared_get(&c, SHARED_UID).is_none());
        // 只做一次：删掉「提醒附图」以后不会再加回来
        s.note_cats.truncate(1);
        s.fix_pic_cat();
        assert_eq!(upgrade_pic_cat(&c, &mut s).unwrap(), 0);
        assert_eq!((s.note_cats.len(), s.pic_cat.as_str()), (1, ""));

        assert!(only_pics(" ![图](att:x.png)  ![](att:y_1.webp) "));
        assert!(!only_pics("") && !only_pics("![](att:x.png) 说明") && !only_pics("![](https://a.com/x.png)") && !only_pics("[文件](att:x.pdf)"));
    }

    #[test]
    fn pic_cat_follows_shared() {
        let mut a = Settings { pic_cat: "ref".into(), pic_cat_all: true, ..Default::default() };
        let mut b = Settings::default();
        b.merge_shared(&a.shared());
        assert_eq!((b.pic_cat.as_str(), b.pic_cat_all), ("ref", true));
        // 删掉了放图片的分类：存的时候改成不分类，传过去也是
        a.pic_cat = PIC_CAT.into();
        a.note_cats.retain(|t| t.id != PIC_CAT);
        a.fix_pic_cat();
        b.merge_shared(&a.shared());
        assert_eq!(b.pic_cat, "");
        // 换成了别的分类又被删掉：同样不分类
        a.pic_cat = "gone".into();
        b.merge_shared(&a.shared());
        assert_eq!(b.pic_cat, "");
        // 那台电脑还没升级时存的：说放在「提醒附图」、分类里却没有，放回来
        let mut stale = Settings::default();
        stale.note_cats.retain(|t| t.id != PIC_CAT);
        let mut d = Settings::default();
        d.merge_shared(&stale.shared());
        assert_eq!(d.pic_cat, PIC_CAT);
        assert!(d.note_cats.iter().any(|t| t.id == PIC_CAT));
        // 旧版本传来的：没有这两项，笔记分类里也没有「提醒附图」；保留自己的，「提醒附图」放回去
        let mut c = Settings::default();
        let mut old = Settings::default().shared();
        old.pic_cat = None;
        old.pic_cat_all = None;
        old.note_cats.as_mut().unwrap().retain(|t| t.id != PIC_CAT);
        c.merge_shared(&old);
        assert_eq!(c.pic_cat, PIC_CAT);
        assert!(c.note_cats.iter().any(|t| t.id == PIC_CAT));
    }

    #[test]
    fn note_folders() {
        let f = |id: &str, name: &str, parent: Option<&str>| TagDef { id: id.into(), name: name.into(), color: "blue".into(), sound: String::new(), parent: parent.map(Into::into) };
        let mut s = Settings { note_cats: vec![f("work", "工作", None), f("cs", "长寿医院", Some("work")), f("rep", "月报", Some("cs")), f("ref", "资料", None)], ..Default::default() };
        assert_eq!(note_cat_path(&s.note_cats, "rep"), ["工作", "长寿医院", "月报"]);
        assert_eq!(note_cat_path(&s.note_cats, "ref"), ["资料"]);
        assert!(note_cat_path(&s.note_cats, "gone").is_empty());
        // 上一层找不到的当最外层；绕成圈的不会一直走下去
        s.note_cats.push(f("x", "孤儿", Some("nope")));
        assert_eq!(note_cat_path(&s.note_cats, "x"), ["孤儿"]);
        let lp = vec![f("a", "甲", Some("b")), f("b", "乙", Some("a"))];
        assert_eq!(note_cat_path(&lp, "a").len(), MAX_NOTE_DEPTH);
        // 删掉中间一层：子文件夹放回上一层，回上一层的 id
        assert_eq!(remove_note_cat(&mut s, "cs"), Ok(Some("work".into())));
        assert_eq!(s.note_cats.iter().find(|t| t.id == "rep").unwrap().parent.as_deref(), Some("work"));
        assert_eq!(remove_note_cat(&mut s, "work"), Ok(None));
        assert_eq!(s.note_cats.iter().find(|t| t.id == "rep").unwrap().parent, None);
        assert_eq!(remove_note_cat(&mut s, "work"), Err(()));
        // 旧版本存的没有 parent；没有上一层的不写出去
        let t: TagDef = serde_json::from_str(r#"{"id":"ref","name":"资料","color":"teal"}"#).unwrap();
        assert_eq!(t.parent, None);
        assert!(!serde_json::to_string(&t).unwrap().contains("parent"));
    }
}
