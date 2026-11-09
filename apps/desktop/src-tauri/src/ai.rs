//! 调用 OpenAI 兼容的对话接口（默认 DeepSeek）。

use crate::settings::{Settings, TagDef};
use serde_json::{json, Value};
use std::time::Duration;

pub async fn chat(s: &Settings, system: &str, user: &str, want_json: bool) -> Result<String, String> {
    chat_timeout(s, system, user, want_json, 30).await
}

/// 同 chat，等得更久（整理长笔记时 AI 要写很多字）
pub async fn chat_timeout(s: &Settings, system: &str, user: &str, want_json: bool, secs: u64) -> Result<String, String> {
    let key = s.ai_key.trim();
    if key.is_empty() {
        return Err("还没有填写 AI 的 API Key，可以在「设置」里填".into());
    }
    let base = if s.ai_base.trim().is_empty() {
        "https://api.deepseek.com"
    } else {
        s.ai_base.trim().trim_end_matches('/')
    };
    let model = if s.ai_model.trim().is_empty() { "deepseek-chat" } else { s.ai_model.trim() };
    let mut body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user }
        ],
        "temperature": 0.3,
        "stream": false
    });
    if want_json {
        body["response_format"] = json!({ "type": "json_object" });
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(secs))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(format!("{base}/chat/completions"))
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("连不上 AI 服务，请检查网络或接口地址（{e}）"))?;
    let status = resp.status();
    let v: Value = resp
        .json()
        .await
        .map_err(|e| format!("AI 服务返回的内容看不懂（{e}）"))?;
    if !status.is_success() {
        let msg = v["error"]["message"].as_str().unwrap_or("");
        return Err(match status.as_u16() {
            401 => "API Key 不对，请到「设置」里检查".into(),
            402 => "AI 账户余额不足".into(),
            code => format!("AI 服务出错了（{code}）{msg}"),
        });
    }
    v["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "AI 没有返回内容".into())
}

const PARSE_PROMPT: &str = r#"你是一个提醒和笔记助手。用户会随手写一句或几句话，你要把它拆成"提醒"和"笔记"。
只输出 JSON，格式：
{"items":[
  {"type":"reminder","title":"简短的事情","detail":"补充说明，可为空","tag":"{TAG_IDS}","due":"YYYY-MM-DD HH:MM","repeat":{"kind":"none|daily|weekdays|weekly|monthly|interval|yearly|month_last|month_nth|lunar_yearly","days":[1-7],"monthday":0,"every":0,"month":0,"nth":0},"urgent":false,"vague":false,"on":[]},
  {"type":"note","title":"笔记标题","body":"笔记正文，可用 Markdown","tag":"{NOTE_IDS}","link":true}
]}
规则：
- 有时间说法（哪怕只有日期、没写要做什么，比如"农历八月十五""本周日休息""3点"），或者说了"提醒我/记得"的，是 reminder；去掉时间以后什么都不剩（"3点""农历八月十五""过5分钟提醒我"）时 title 填"时间到了"，还剩下事情就用事情（"本周日休息"是"休息"，"年底体检"是"体检"，"过三天再说"是"再说"）。没有任何时间说法的信息、想法、事实，是 note："写工作日志""早一点睡觉""差一点忘了交水费""晚点名之前到""坐1号线去公司""帮我记一下"里没有时间（"工作日志""早一点""差一点""晚点名""1号线""3号楼"都不是时间说法，"记一下"不是"提醒我"）。"奶奶的农历生日是冬月十二"这类说生日、纪念日的，是每年的 reminder。一句话可以同时拆出两种。
- due 用 24 小时制本地时间。没说具体钟点时：早上/上午 09:00，中午 12:00，下午 15:00，傍晚 18:00，晚上 20:00；只说了日期（包括"今天"）、或者周期提醒没说钟点，都是 09:00。
- 已经过了的时间：
  - 没说日期、只说了钟点，按顺序看：①今天这个钟点还没到，就是今天（现在 07:00 说"9点开会"是今天 09:00）；②已经过了，没说上午下午的整点（"8点""九点"，1–11 点），今天晚上的这个钟点还没到，就是今天晚上（现在 10:00 说"8点开会"是今天 20:00，现在 21:00 说"10点睡觉"是今天 22:00）；③晚上的也过了（现在 21:00 说"8点"是明天 08:00），或者写成"8:30"这样带冒号的，或者说了早上/上午/下午/晚上的（"早上提醒我锻炼""上午9点到11点半培训""下午3点开会"），就是明天。周期提醒（"每天8点吃药"）不往晚上猜，就是 08:00。"晚上12点"是第二天 00:00。
  - 说了"今天""今早"，或者说的那一天就是今天的（今天 30 号说"30号交水费"），就用今天，钟点过了也照填。但"月底""年底"今天就是、09:00 已经过了的，算下个月底、明年底。
  - 周几："周X"是这一周的周X；已经过了、或者就是今天但钟点已经过了，就是下周的。一周从周一开始，"下周X"是下一周的周X（今天是周日时"下周一"就是明天）。
  - 没写年份的日期今年已经过了，就是明年；只说了几号，这个月已经过了，就是下个月的。写了年份的（"2026-03-05""2026年3月5日"）照写的填，过了也不改。"3-5""3/5""3.5号"都是 3 月 5 号。
  - 一句话里后面的时间没说日期，就和前面的同一天（"明天下午3点开会，晚上8点给妈妈打电话"都是明天）。
- 相对时间："三天后""一周后""两周后""一个月后""一年后"是那天的现在这个钟点（现在 10:00 就是 10:00，不是 09:00；另说了钟点就用那个），那个月没有这一天就用那个月最后一天；"过两天""过三天"是 2 天、3 天后的 09:00；"一会儿/等会儿/待会儿"是 30 分钟后；"晚点"是 2 小时后（取整到 5 分钟）。
- 日期："周末/这周末"是这周六（今天是周日就是今天），"下周末"是下一周的周六（今天是周日时就是 6 天后）；"月底"是这个月最后一天（已经过了就是下个月的）；"月初"是下个月 1 号；"年底"是 12 月 31 号（已经过了就是明年的）。"3.5"只有后面紧跟钟点或"号"时才是 3 月 5 号，"买 3.5 斤苹果"不是日期。"工作日志"不是工作日。
- "提前一小时提醒"这类说法：due 是事件时间减去提前量，detail 里写上事件本身的时间。
- 截止时间：只有带"前、之前、以前、截止"的才是（"周五前交报告""11 月 10 号之前续费""12月1日截止报名"）。"明天交报告""25号交房租""月底交房租"不是截止，就按上面的规则，只说了日期就是那天 09:00。截止的（"截止到周五提交"也是）：截止日期是明天或以后、只说了日期，提醒在截止的前一天 15:00（"明天之前回复"是今天 15:00），这个时间已经过了就 10 分钟后；带了钟点就提前 2 小时；截止就是今天、没说钟点的（"今天之前交表"），当成今天 18:00 截止，提前 2 小时；已经不到 2 小时就 10 分钟后（现在 10:00 说"今天11点前回复邮件"、现在 16:00 说"今天下午5点前交"，都是 10 分钟后）。detail 写"截止：11月7日（周五）"。用户另说了提前多久就照说的来，提前的是几天（"提前两天提醒"）就是那天 09:00。
- 时间段（"下午 3 点到 5 点开会""15:00-17:00"）：due 是开始时间，detail 写"15:00–17:00"。
- 一句话几个时间："明天 9 点和下午 3 点各提醒一次""每天早晚各一次""每天早晚各吃一次药""每天早上8点和晚上8点""每月1号和15号"拆成几条（几个时间就输出几个 item），标题一样；用逗号、分号分开、各自带时间的几件事拆成几条，各用各的标题。
- 紧急：句子里有"紧急、加急、很急、急事、很重要、非常重要、特别重要、千万、务必"，urgent=true，标题里去掉这些词。"加急件""不紧急"这类不是在说这件事急，不算。
- 说得含糊时先定一个时间，vague=true，只有这几种："改天、回头、有空时、有时间的时候"是明天 09:00；"下周"（没说周几）是下周一 09:00；"过几天"是 3 天后 09:00；"下个月"（没说几号，"下个月提醒我换季"也是）是下个月 1 号 09:00；只说了"提醒我"、或者只说了地点（"到公司提醒我"）、或者只说了"等会议结束"这类没法算的时间，是 1 小时后、往后取整到 5 分钟（现在 10:00 就是 11:00，10:02 是 11:05）。别的都是 vague=false："一会儿、晚点、周末、月底、3点"这些都有固定的算法；"改天请老王吃饭 周三"后面补了时间，也不算含糊。
- 周期：句子里出现"每、每个、每逢、天天"才是周期，没有这几个字 kind 一定是 none：工资、房租、农历日子也一样，"周三开会""周一上午10点例会""月初发工资""周一交周报""农历八月十五"都只提醒一次。每天=daily；工作日=weekdays；每周几=weekly，days 用 1=周一…7=周日，没说周几就填今天是周几；每两周、隔周、每隔一周=weekly 加 every=2，每隔两周=every 3（"每隔 N"是隔开 N 个再来一次，所以是 N+1；从最近的那一天开始算）；每月几号=monthly，monthday 填几号；每月最后一天、每个月月底=month_last；每月第几个周几=month_nth，nth 填 1–5，最后一个填 -1，days 填那个周几（"每月最后一个周日"是 month_nth、nth=-1、days=[7]，不是 month_last）；每年几月几号、每年的今天=yearly，month、monthday 填公历月日（每年的今天就填今天的）；每年农历几月几日（农历生日）=lunar_yearly，month、monthday 一定照用户说的农历月日填，不能是 0（八月十五 → month=8、monthday=15；正月=1、冬月=11、腊月=12，初三=3，廿七=27）；每 N 天=interval，every 填 N；每隔 N 天=interval，every 填 N+1（每隔一天=every 2，每隔3天=every 4）。不重复用 none，用不到的字段填 0 或空数组。周期提醒的 due 填第一次、还没过的时间（今天的已经过了就是下一次）；日子由程序按 repeat 重新算，最要紧的是 repeat 的字段和 due 的钟点填对，农历的日期算不准也没关系。
{TAG_RULE}
- 笔记 link=true 表示和同一次拆出的提醒相关。
- title 简短自然，去掉"提醒我""记得"这类词和时间说法，也不要留下多余的空格和标点。不要编造用户没说的信息。
- 如果给了"用户的电脑"列表，而用户说了在哪里提醒（"到公司提醒我""回家后提醒""在家里电脑上提醒"），on 填对应的电脑名，必须从列表里原样选；没说地点就是空数组。title 里不要保留这类地点说法。事情本身带着地点的（"明天去公司开会""下午回家做饭"）不是在说在哪里提醒，on 是空数组，标题照原样留着"去公司""回家"。"#;

/// 拆分用的提示词，分类换成用户自己设的：提醒用提醒分类，笔记用笔记分类
pub fn parse_prompt(tags: &[TagDef], note_cats: &[TagDef]) -> String {
    let ids = |list: &[TagDef]| list.iter().map(|t| t.id.as_str()).chain(["null"]).collect::<Vec<_>>().join("|");
    let rule = |what: &str, list: &[TagDef]| {
        if list.is_empty() {
            format!("- 用户没有设{what}分类，{what}的 tag 一律 null。")
        } else {
            let names: Vec<String> = list.iter().map(|t| format!("{}={}", t.name, t.id)).collect();
            format!("- {what}的 tag 从用户的{what}分类里选最贴切的，填 id（名字=id）：{}。都不合适就 null。", names.join("，"))
        }
    };
    PARSE_PROMPT
        .replace("{TAG_IDS}", &ids(tags))
        .replace("{NOTE_IDS}", &ids(note_cats))
        .replace("{TAG_RULE}", &format!("{}\n{}", rule("提醒", tags), rule("笔记", note_cats)))
}

/// 「让 AI 整理」：只理格式，不动内容
pub const TIDY_PROMPT: &str = r#"你帮用户整理一条笔记的格式。用户的笔记常常是随手粘贴来的：句子被硬换行打断、空行很多、符号杂乱、没有层次。
只输出 JSON：{"title":"标题","body":"整理后的正文"}
规则：
- 只整理格式，不增删内容、不改写意思、不总结、不翻译、不加评论。原文里的每个信息点（数字、名字、日期、网址、电话）都要留着。
- 网址、IP、端口、账号、密码、密钥、口令、命令、代码、文件路径、型号这类内容一个字符都不能改：原样照抄，大小写、标点、空格都不动，也不要改成全角。
- 正文只用这几种写法：# 小标题（最多到 ##）、- 列表、1. 编号列表、- [ ] 待办（原文里已经勾上的写 - [x]）、> 引用、**粗体**、`行内代码`、[文字](网址)、表格。不要用 ``` 代码块。
- 把被硬换行打断的句子接起来；去掉多余的空行和行首行尾的空格；明显是并列的几项整理成列表；内容多时按意思分段，必要时加小标题。中文句子里的标点用中文全角（上一条说的那些内容除外）。
- 几样东西各有好几项可以对照的信息（比如几个方案的价格和配置、几天的行程安排、几个人的联系方式），整理成表格：第一行是表头，第二行是 | --- | --- | 这样的分隔行，每行格子一样多；第一列放每样东西的名字（列多时会显示成卡片，第一列是卡片标题），第二列放最能说明它是什么的一项；格子里本来有竖线的写成 \|。一格里有好几项的（比如配置里的 CPU、内存、磁盘），用 <br> 分开，不要真的换行。原文已经是表格的保持表格。每样东西只有一两句话的用列表，不要硬做成表格。
- 单独一行的 ![...](att:...) 是图片、[...](att:...) 是文件附件，原样保留那一整行，不要改动，也不要和别的行合并。
- title：用户给了标题就原样保留；没给就根据内容起一个不超过 20 个字的标题。
- 用户这次可能提了「整理要求」（比如整理成表格、按时间排好、只分段）：按要求来，要求和上面的写法规则冲突时以要求为准；但仍然不能删掉或改动信息，也只能用上面列的那几种写法。
- 原文已经很规整的话，原样返回。"#;

/// AI 整理后，原文里单独一行的图片、文件附件（att:）要是不见了，补在最后，免得附件从笔记里丢掉
pub fn keep_attachments(original: &str, tidied: &str) -> String {
    let mut out = tidied.trim_end().to_string();
    for line in original.lines().map(str::trim) {
        let is_att = (line.starts_with("![") || line.starts_with('[')) && line.contains("](att:") && line.ends_with(')');
        let Some(at) = line.find("(att:") else { continue };
        if is_att && !out.contains(&line[at..]) {
            out.push('\n');
            out.push_str(line);
        }
    }
    out
}

pub const MORNING_PROMPT: &str = r#"你是用户自己的晨间小助手，语气像一个很熟的朋友，温和、简洁、具体。
根据给出的今天日程和最近笔记，写一段 2~3 句的中文开场白（不超过 90 个字）。
要点：点出今天有几件事、最要紧的是哪件以及原因；如果有关联的长期目标或没整理的笔记，可以顺带提一句。
不要使用列表、表情符号或 Markdown，直接输出这段话。"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{default_note_cats, default_tags};

    #[test]
    fn tidy_keeps_attachments() {
        let original = "随手贴的\n![](att:a.png)\n[报价.pdf](att:b.pdf)\n不是附件 [x](att:c.png) 在句子里";
        // AI 把图片留着、漏了文件：文件补在最后，句子里的不算附件行
        let tidied = "# 随手贴的\n\n![](att:a.png)\n";
        assert_eq!(keep_attachments(original, tidied), "# 随手贴的\n\n![](att:a.png)\n[报价.pdf](att:b.pdf)");
        assert_eq!(keep_attachments("没有附件", "没有附件\n"), "没有附件");
    }

    #[test]
    fn prompt_uses_own_tags() {
        let mut tags = default_tags();
        tags.push(TagDef { id: "tx1".into(), name: "学习".into(), color: "green".into() });
        let p = parse_prompt(&tags, &default_note_cats());
        assert!(p.contains(r#""tag":"work|life|health|like|cycle|tx1|null""#));
        assert!(p.contains("学习=tx1"));
        // 笔记用笔记分类
        assert!(p.contains(r#""tag":"toread|idea|ref|null","link""#));
        assert!(p.contains("灵感=idea"));
        assert!(!p.contains("{TAG") && !p.contains("{NOTE"));
        assert!(parse_prompt(&[], &[]).contains(r#""tag":"null""#));
        // 0.2 的新周期类型和字段都写进去了
        for k in ["yearly", "month_last", "month_nth", "lunar_yearly", r#""nth":0"#, r#""vague":false"#] {
            assert!(p.contains(k), "{k}");
        }
    }
}
