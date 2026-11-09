// 把一句话拆成提醒/笔记。优先用 AI，没有 Key 或出错时用这里的本地规则兜底。
// 本地规则认哪些说法见 docs/方案/需求-0.2.md 第 8 节，测试句子在 parse.cases.ts（npm test）。
import { api, errText, newNote, newReminder, type Device, type Note, type Reminder, type Repeat, type Tag } from "./api";
import { addDays, fmtDT, noteCatDef, noteCatList, parseDT, tagDef, tagList } from "@shared/util";
import { lunarLabel, toLunar } from "@shared/lunar";
import { lastDayOfMonth, normalize, weekday } from "@shared/recur";

export interface ParsedReminder {
  type: "reminder";
  title: string;
  detail: string;
  tag: Tag | null;
  due: string;
  repeat: Repeat;
  urgent: boolean;
  /** 只在这几台电脑上提醒（设备 uid），空表示每台都提醒 */
  only_on: string[];
  /** 说得含糊、时间是先定的："没听清具体时间，先定在明天 9:00"。空表示说清楚了 */
  guess: string;
}
export interface ParsedNote {
  type: "note";
  title: string;
  body: string;
  tag: Tag | null;
  link: boolean;
}
export type ParsedItem = ParsedReminder | ParsedNote;

export interface ParseResult {
  items: ParsedItem[];
  source: "ai" | "local";
  /** AI 失败时的原因 */
  error?: string;
}

const CN: Record<string, number> = { 零: 0, 〇: 0, 一: 1, 二: 2, 两: 2, 三: 3, 四: 4, 五: 5, 六: 6, 七: 7, 八: 8, 九: 9 };
const CNUM = "[零〇一二两三四五六七八九十]{1,3}";
const NUMS = `\\d{1,2}|${CNUM}`;
const NUM = `(${NUMS})`;
const WDMAP: Record<string, number> = { 一: 1, 二: 2, 三: 3, 四: 4, 五: 5, 六: 6, 日: 7, 天: 7, "1": 1, "2": 2, "3": 3, "4": 4, "5": 5, "6": 6, "7": 7 };
const WEEK = "(?:周|星期|礼拜)";
/** 每周几里的一天：「每周1、3、5」也认，但钟点里的数字不算——
 *  挨着的（「每周一10点」「每周二四19:00」的 1 是钟点），隔着空格的也一样（「每周二 4 点」的 4 是钟点，不是周四） */
const WDCH = "(?:[一二三四五六日天]|[1-7](?!\\d|\\s*[:：.点时]))";
const PERIODS = "凌晨|早上|早晨|清早|上午|中午|下午|傍晚|晚上|夜里|夜间|深夜|半夜";
/** 只说了"早上""晚上"时的钟点 */
const PERIOD_HOUR: Record<string, number> = { 凌晨: 6, 早上: 9, 早晨: 9, 清早: 9, 上午: 9, 中午: 12, 下午: 15, 傍晚: 18, 晚上: 20, 夜里: 22, 夜间: 22, 深夜: 23, 半夜: 23 };
const MINW = `半|一刻|三刻|\\d{1,2}|${CNUM}`;
/** 一个钟点：可以带"下午"，3 点 / 3 点半 / 15:30。分组：时段、时、分（冒号后）、分（点后） */
const CLOCK = `(?:(${PERIODS})\\s*)?(\\d{1,2}|${CNUM})(?:\\s*[:：]\\s*(\\d{2})|\\s*[点时](?:钟|整)?(?:\\s*(${MINW})\\s*分?)?)`;
/** 同上，但"点"可以省（时间段的一头："3-5 点"） */
const CLOCK_OPT = `(?:(${PERIODS})\\s*)?(\\d{1,2}|${CNUM})(?:\\s*[:：]\\s*(\\d{2})|\\s*([点时])(?:钟|整)?(?:\\s*(${MINW})\\s*分?)?)?`;
const URGENT = /(?<!不)(?:有?急事|紧急|加急(?!件)|很急|非常重要|特别重要|很重要|千万|务必)/g;
const WD_CN = ["日", "一", "二", "三", "四", "五", "六"];
/** 截止只说了日期、截止就是今天时，当成几点截止 */
const DEADLINE_TODAY_HOUR = 18;
/** 切掉的时间说法换成这个记号，最后收拾标题时再决定留不留空格 */
const CUT = "\u0001";

export function cnNum(s: string): number | null {
  if (/^\d+$/.test(s)) return +s;
  const m = s.match(/^([一二两三四五六七八九])?十([一二三四五六七八九])?$/);
  if (m) return (m[1] ? CN[m[1]] : 1) * 10 + (m[2] ? CN[m[2]] : 0);
  if (s.length === 1 && s in CN) return CN[s];
  return null;
}

const TAG_WORDS: [Tag, RegExp][] = [
  ["cycle", /还款|信用卡|房租|水电|燃气|账单|订阅|续费|物业|社保|缴费|交费/],
  ["work", /会议|开会|方案|报告|项目|客户|老板|领导|同事|周报|日报|月报|PPT|ppt|邮件|加班|面试|合同|需求|上线|评审|汇报|对接/],
  // 健康排在喜好、生活前面；删掉了「健康」的话，跑步、吃药这些照旧算喜好、生活
  ["health", /体检|看病|医院|医生|挂号|复查|复诊|吃药|服药|打针|疫苗|牙医|看牙|洗牙|血压|血糖|跑步|健身|锻炼|运动|瑜伽|游泳|拉伸|喝水|维生素|鱼油|早睡|午睡|眼保健操|减肥|称体重/],
  ["like", /跑步|健身|瑜伽|游泳|电影|游戏|看书|读书|吃(?!药)|餐厅|旅行|旅游|音乐|演唱会|球|展览|咖啡|追剧|练琴/],
  ["life", /妈|爸|家|买|取|快递|洗|做饭|打扫|医院|体检|看病|吃药|孩子|老婆|老公|朋友|生日|理发|浇水|倒垃圾/],
];

/** 按字面猜分类：默认的几个用上面的词表，自己加的看有没有提到分类的名字。分类删掉了就不猜它 */
export function guessTag(s: string): Tag | null {
  const hit = TAG_WORDS.find(([id, re]) => tagDef(id) && re.test(s));
  if (hit) return hit[0];
  return tagList().find((t) => !TAG_WORDS.some(([id]) => id === t.id) && t.name.length >= 2 && s.includes(t.name))?.id ?? null;
}

/** 笔记分类：提到了分类的名字（"灵感：……"）就是它；只发了一个链接的算「待看」（还在的话） */
export function guessNoteCat(s: string): Tag | null {
  const named = noteCatList().find((t) => t.name.length >= 2 && s.includes(t.name));
  if (named) return named.id;
  return /^\s*https?:\/\/\S+\s*$/.test(s) && noteCatDef("toread") ? "toread" : null;
}

const startOfDay = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate());
const escRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const sameDay = (a: Date, b: Date) => startOfDay(a).getTime() === startOfDay(b).getTime();
const at = (d: Date, h: number, m: number) => new Date(d.getFullYear(), d.getMonth(), d.getDate(), h, m);
const addMin = (d: Date, n: number) => new Date(d.getTime() + n * 60000);
/** 往后取整到 5 分钟 */
const ceil5 = (d: Date) => {
  const out = new Date(d);
  out.setSeconds(0, 0);
  out.setMinutes(Math.ceil(out.getMinutes() / 5) * 5);
  return out;
};
const pad = (n: number) => String(n).padStart(2, "0");
const hm = (d: Date) => `${pad(d.getHours())}:${pad(d.getMinutes())}`;
/** 9:00、15:30（说明里的写法，小时不补 0） */
const hmShort = (d: Date) => `${d.getHours()}:${pad(d.getMinutes())}`;
/** 11月7日（周五） */
const mdw = (d: Date) => `${d.getMonth() + 1}月${d.getDate()}日（周${WD_CN[d.getDay()]}）`;
/** 今天 / 明天 / 后天 / 11月8日（周日） */
function dayWord(d: Date, now: Date): string {
  const diff = Math.round((startOfDay(d).getTime() - startOfDay(now).getTime()) / 86400000);
  return ["今天", "明天", "后天"][diff] ?? mdw(d);
}
const whenWord = (d: Date, now: Date) => {
  const w = dayWord(d, now);
  return `${w}${w.endsWith("）") ? "" : " "}${hmShort(d)}`;
};
const addMonths = (d: Date, n: number) => {
  const first = new Date(d.getFullYear(), d.getMonth() + n, 1, d.getHours(), d.getMinutes());
  return new Date(first.getFullYear(), first.getMonth(), Math.min(d.getDate(), lastDayOfMonth(first)), d.getHours(), d.getMinutes());
};
const monthEnd = (y: number, m: number) => new Date(y, m + 1, 0);
const validMD = (m: number, d: number) => m >= 1 && m <= 12 && d >= 1 && d <= 31;
/** 公历月日：没写年份，已经过了就是明年 */
const nextMD = (today: Date, m: number, d: number) => {
  const x = new Date(today.getFullYear(), m - 1, d);
  return x < today ? new Date(today.getFullYear() + 1, m - 1, d) : x;
};
const weekdayOf = (d: Date) => d.getDay() || 7;

/** 设备名里表示地点的词："公司电脑" → 公司、单位、办公室；"家里的电脑" → 家里、家 */
function placeWords(name: string): string[] {
  const words = new Set<string>();
  const base = name.replace(/的|电脑|笔记本|台式机?|主机|机器|pc|laptop/gi, "").replace(/^[\s_-]+|[\s_-]+$/g, "");
  if (base) words.add(base);
  if (/家/.test(name)) words.add("家");
  if (/公司|单位|办公/.test(name)) ["公司", "单位", "办公室"].forEach((w) => words.add(w));
  return [...words];
}

/** "到公司提醒我""回家后提醒"：说的是哪台电脑。phrase 是要从标题里去掉的那段地点说法 */
export function matchPlace(text: string, devices: Device[]): { only_on: string[]; phrase: string } {
  let best: { uid: string; phrase: string; len: number } | null = null;
  if (devices.length > 1) {
    for (const d of devices) {
      for (const w of placeWords(d.name)) {
        // 地点后面紧跟着"提醒""叫我""记得"才算，免得"明天去公司开会"被当成只在公司提醒
        const re = new RegExp(`(?:在|到|回|去)\\s*${escRe(w)}(?:里面?)?(?:的?电脑上?)?(?:以后|之后|后|的时候|时)?(?=\\s*(?:再|就)?(?:提醒|叫我|告诉我|记得))`);
        const m = text.match(re);
        if (m && (!best || w.length > best.len)) best = { uid: d.uid, phrase: m[0], len: w.length };
      }
    }
  }
  return best ? { only_on: [best.uid], phrase: best.phrase } : { only_on: [], phrase: "" };
}

/** 一个钟点：h 已经按"下午""晚上"换成 24 小时制（晚上 12 点是 24） */
interface Clock {
  h: number;
  m: number;
  /** 说了"下午""晚上"这类时段 */
  period: string;
  /** 写成 9:00 这样的，就是 24 小时制，不再猜是不是下午 */
  exact?: boolean;
}

function minuteOf(s: string | undefined): number {
  if (!s) return 0;
  if (s === "半") return 30;
  if (s === "一刻") return 15;
  if (s === "三刻") return 45;
  return cnNum(s) ?? 0;
}

function toClock(period: string, h: number, m: number, exact = false): Clock | null {
  if (h > 24 || m > 59) return null;
  if (exact && !period) return { h, m, period, exact };
  if (/下午|傍晚|晚上|夜里|夜间|深夜/.test(period) && h < 12) h += 12;
  else if (period === "中午" && h < 6) h += 12;
  else if (/晚上|夜里|夜间|深夜|半夜/.test(period) && h === 12) h = 24;
  else if (/凌晨|半夜/.test(period) && h === 12) h = 0;
  return { h, m, period };
}

/** CLOCK 的匹配结果（从 i 开始的 4 个分组）→ 钟点 */
function clockOf(g: (string | undefined)[], i: number, inherit = ""): Clock | null {
  const h = cnNum(g[i + 1] ?? "");
  if (h === null) return null;
  return toClock(g[i] ?? inherit, h, g[i + 2] ? +g[i + 2]! : minuteOf(g[i + 3]), !!g[i + 2]);
}

/** 一段话（一件事）理解出来的样子 */
interface One {
  items: ParsedReminder[];
  /** 说了时间（含糊的也算） */
  hasWhen: boolean;
  title: string;
  /** 说了哪天，下一件事没说哪天时沿用 */
  date: Date | null;
}

/** 本地规则：能识别常见的中文时间说法；devices 给了的话，也认"到公司提醒我"这类说法 */
export function parseLocal(input: string, now = new Date(), devices: Device[] = []): ParsedItem[] {
  let text = input.trim();
  if (!text) return [];
  const place = matchPlace(text, devices);
  if (place.phrase) text = text.replace(place.phrase, " ").trim();

  // "顺便记一下……" 后面的部分单独存成笔记
  let noteText = "";
  const nm = text.match(/[，,。；;\s]*(?:顺便|另外|还有)?(?:帮我)?记(?:一下|下|住)[:：，,]?\s*(.+)$/s);
  if (nm && nm.index! > 0) {
    noteText = nm[1].trim();
    text = text.slice(0, nm.index).trim();
  }

  const items: ParsedItem[] = [];
  for (const one of splitEvents(text, now)) {
    for (const r of one.items) items.push({ ...r, only_on: place.only_on });
  }

  if (noteText || !items.length) {
    const body = noteText || input.trim();
    const first = body.split("\n")[0];
    items.push({
      type: "note",
      title: first.length > 20 ? first.slice(0, 20) + "…" : first,
      body,
      tag: guessNoteCat(body),
      link: !!items.length,
    });
  }
  return items;
}

/** 用逗号、分号分开、各自带时间的几件事拆开；没带时间的那段跟着前一件事 */
function splitEvents(text: string, now: Date): One[] {
  const segs = text.split(/[，,；;。！!？?\n]+/).map((s) => s.trim()).filter(Boolean);
  if (segs.length > 1) {
    const groups: { text: string; timed: boolean; titled: boolean }[] = [];
    for (const s of segs) {
      const p = parseOne(s, now, null);
      const last = groups[groups.length - 1];
      // 带时间、有事情的一段另起一件事；只有时间没有事情的（"周六下午"）补给上一件
      if (!last || (p.hasWhen && p.title && last.timed && last.titled)) groups.push({ text: s, timed: p.hasWhen, titled: !!p.title });
      else {
        last.text += "，" + s;
        last.timed ||= p.hasWhen;
        last.titled ||= !!p.title;
      }
    }
    if (groups.filter((g) => g.timed).length >= 2) {
      const out: One[] = [];
      let prev: Date | null = null;
      for (const g of groups) {
        const one = parseOne(g.text, now, prev);
        prev = one.date ?? prev;
        out.push(one);
      }
      return out;
    }
  }
  return [parseOne(text, now, null)];
}

/** 理解一件事。prevDate：上一件事说的哪天（"明天上午开会，下午交报告"） */
function parseOne(text: string, now: Date, prevDate: Date | null): One {
  let t = text;
  /** 找到第一个符合的说法，换成记号；accept 不认的跳过接着找 */
  /** keep：切掉后留下的字（"农历生日正月初三"留下"生日"） */
  const cut = (re: RegExp, accept?: (m: RegExpMatchArray, before: string, after: string) => boolean, keep?: (m: RegExpMatchArray) => string) => {
    const g = new RegExp(re.source, re.flags.includes("g") ? re.flags : re.flags + "g");
    for (const m of t.matchAll(g)) {
      const before = t.slice(0, m.index), after = t.slice(m.index! + m[0].length);
      if (accept && !accept(m, before, after)) continue;
      t = before + CUT + (keep?.(m) ?? "") + CUT + after;
      return m;
    }
    return null;
  };
  let m: RegExpMatchArray | null;
  const today = startOfDay(now);

  // 紧急
  let urgent = false;
  while (cut(URGENT)) urgent = true;

  // 说得含糊
  let vague: "tomorrow" | "days" | "month" | "week" | null = null;
  if (cut(/改天|回头|有空(?:的时候|时|了|再)?|有时间(?:的时候|时|了)?/)) vague = "tomorrow";
  else if (cut(/过(?:几|些)天/)) vague = "days";

  // ---------- 周期 ----------
  let repeat: Repeat = { kind: "none", days: [], monthday: 0, every: 0, month: 0, nth: 0 };
  let date: Date | null = null;
  /** 说的是周几（没加"下"），今天已经过了那个点就往后挪一周 */
  let bareWeekday = false;
  /** 月底、年底：这次的已经过了，换成下一个 */
  let roll: Date | null = null;
  let periodHint = "";
  let lunarNote = "";
  let monthdays: number[] = [];
  const LM = "(十一|十二|正|冬|腊|[一二三四五六七八九十]|\\d{1,2})";
  const LD = "(初[一二三四五六七八九十]|廿[一二三四五六七八九]|二十[一二三四五六七八九]|三十|二十|十[一二三四五六七八九]|十|\\d{1,2}|[一二三四五六七八九])";
  if ((m = cut(new RegExp(`(每年的?)?\\s*(?:阴历|农历)\\s*(生日|忌日|纪念日)?\\s*(?:是\\s*)?(闰)?${LM}\\s*月\\s*${LD}\\s*[日号]?`), undefined, (x) => x[2] ?? ""))) {
    const lm = ({ 正: 1, 冬: 11, 腊: 12 } as Record<string, number>)[m[4]] ?? cnNum(m[4]) ?? 1;
    const ld = cnNum(m[5].replace(/^初/, "").replace(/^廿/, "二十")) ?? 1;
    if (lm >= 1 && lm <= 12 && ld >= 1 && ld <= 30) {
      const rule: Repeat = { ...repeat, kind: "lunar_yearly", month: lm, monthday: ld };
      // 说了"每年"或者是生日、忌日这类每年都有的，就每年提醒；不然只提醒一次（最近的那个）
      if (m[1] || m[2] || /生日|忌日|纪念日/.test(text)) repeat = rule;
      else {
        date = startOfDay(normalize(rule, at(today, 0, 0), addMin(today, -1)));
        lunarNote = "农历" + lunarLabel(lm, ld);
      }
    }
  } else if (cut(/每年的?今天/)) repeat = { ...repeat, kind: "yearly", month: now.getMonth() + 1, monthday: now.getDate() };
  else if ((m = cut(new RegExp(`每年的?\\s*${NUM}\\s*月\\s*${NUM}\\s*[日号]?`)))) {
    const mon = cnNum(m[1]) ?? 0, day = cnNum(m[2]) ?? 0;
    if (validMD(mon, day)) repeat = { ...repeat, kind: "yearly", month: mon, monthday: day };
  } else if (cut(/每个?月的?(?:最后一天|月底|月末|底)/)) repeat = { ...repeat, kind: "month_last" };
  else if ((m = cut(new RegExp(`每个?月的?(?:第\\s*([一二三四五1-5])\\s*个|(最后一?)个?)\\s*${WEEK}([一二三四五六日天])`)))) {
    repeat = { ...repeat, kind: "month_nth", nth: m[2] !== undefined ? -1 : cnNum(m[1]) ?? 1, days: [WDMAP[m[3]]] };
  } else if ((m = cut(new RegExp(`(?:隔周|每隔\\s*(${NUMS})\\s*个?${WEEK}|每\\s*(${NUMS})\\s*个?${WEEK})的?\\s*(?:${WEEK}(${WDCH}(?:\\s*[、，,和及与]?\\s*${WEEK}?${WDCH})*))?`)))) {
    // 隔周、每隔一周 = 每两周，每隔两周 = 每三周（隔开几周再来一次）
    const n = m[1] !== undefined ? (cnNum(m[1]) ?? 1) + 1 : m[2] !== undefined ? cnNum(m[2]) ?? 1 : 2;
    const days = m[3] ? [...new Set([...m[3]].map((c) => WDMAP[c]).filter(Boolean))].sort() : [weekdayOf(now)];
    repeat = { ...repeat, kind: "weekly", days, every: n > 1 ? n : 0 };
  } else if ((m = cut(new RegExp(`每(隔)?\\s*${NUM}\\s*天`)))) {
    // 每 3 天 = 3 天一次；每隔一天 = 隔开一天 = 2 天一次
    repeat = { ...repeat, kind: "interval", every: (cnNum(m[2]) ?? 1) + (m[1] ? 1 : 0) };
  }
  else if ((m = cut(new RegExp(`每个?${WEEK}(${WDCH}(?:\\s*[、，,和及与]?\\s*${WEEK}?${WDCH})*)`)))) {
    const days = [...new Set([...m[1]].map((c) => WDMAP[c]).filter(Boolean))].sort();
    repeat = { ...repeat, kind: "weekly", days };
  } else if ((m = cut(new RegExp(`每个?月的?\\s*${NUM}\\s*[号日]?(?:\\s*(?:和|、|，|,|跟|及)\\s*${NUM}\\s*[号日]?)*(?<=[号日])`)))) {
    // 每月 1 号和 15 号：拆成两条
    const mds = [...m[0].matchAll(new RegExp(NUM, "g"))].map((x) => cnNum(x[1]) ?? 0).filter((d) => d >= 1 && d <= 31);
    monthdays = [...new Set(mds)];
    repeat = { ...repeat, kind: "monthly", monthday: monthdays[0] ?? 1 };
  }
  else if (cut(/(?:每个?)?工作日(?!志)(?:每天)?|周一到周五(?:每天)?|周一至周五(?:每天)?/)) repeat = { ...repeat, kind: "weekdays" };
  else if ((m = cut(/每(?:天|日|晚|早)(?:的)?/))) {
    repeat = { ...repeat, kind: "daily" };
    if (/晚/.test(m[0])) periodHint = "晚上";
    if (/早/.test(m[0])) periodHint = "早上";
  } else if (cut(new RegExp(`每${WEEK}`))) repeat = { ...repeat, kind: "weekly", days: [weekdayOf(now)] };
  const recurring = repeat.kind !== "none";

  // 提前量
  let ahead = 0;
  if ((m = cut(new RegExp(`提前\\s*(${NUMS}|半)\\s*个?\\s*(分钟|小时|钟头|天|${WEEK})\\s*(?:提醒我?|叫我|告诉我|通知我)?`)))) {
    const n = m[1] === "半" ? 0.5 : cnNum(m[1]) ?? 0;
    ahead = n * (m[2] === "分钟" ? 1 : m[2] === "天" ? 1440 : /周|星期|礼拜/.test(m[2]) ? 10080 : 60);
  }

  // ---------- 带年份的日期（先认，免得"2026-03-05"被当成时间段） ----------
  if (!date) {
    if ((m = cut(new RegExp(`(\\d{4})\\s*年\\s*${NUM}\\s*月\\s*${NUM}\\s*[日号]?`))) || (m = cut(/(?<!\d)(\d{4})\s*[-/.]\s*(\d{1,2})\s*[-/.]\s*(\d{1,2})(?!\d)/))) {
      const mon = cnNum(m[2]) ?? 0, day = cnNum(m[3]) ?? 0;
      if (validMD(mon, day)) date = new Date(+m[1], mon - 1, day);
    } else if ((m = cut(new RegExp(`明年\\s*${NUM}\\s*月\\s*${NUM}\\s*[日号]?`)))) {
      const mon = cnNum(m[1]) ?? 0, day = cnNum(m[2]) ?? 0;
      if (validMD(mon, day)) date = new Date(now.getFullYear() + 1, mon - 1, day);
    }
  }

  // ---------- 时间段：下午 3 点到 5 点、3-5 点、15:00-17:00 ----------
  let clocks: Clock[] = [];
  let rangeLen = 0;
  if (
    (m = cut(new RegExp(`从?\\s*${CLOCK_OPT}\\s*(?:到|至|-|~|～|—|–|－)\\s*${CLOCK_OPT}`), (x) => {
      // 后一头带"点"或冒号（或者前一头是 15:00 这样），不然"3-5"是 3 月 5 号、"1到2个"是个数
      return x[8] !== undefined || x[9] !== undefined || x[3] !== undefined;
    }))
  ) {
    const g = [...m];
    const start = toClock(g[1] ?? "", cnNum(g[2])!, g[3] ? +g[3] : minuteOf(g[5]), !!g[3]);
    const end = toClock(g[6] ?? g[1] ?? "", cnNum(g[7])!, g[8] ? +g[8] : minuteOf(g[10]), !!g[8]);
    if (start && end) {
      let len = end.h * 60 + end.m - (start.h * 60 + start.m);
      if (len <= 0 && end.h < 12) len += 720;
      if (len > 0) {
        clocks = [start];
        rangeLen = len;
      }
    }
  }

  // ---------- 相对时间 ----------
  let rel: Date | null = null;
  let relDay: Date | null = null;
  /** "一会儿""晚点"：句子里另说了哪天就不算 */
  let softRel = false;
  const REL_UNIT = "分钟|小时|钟头|刻钟";
  const relMin = (n: string, half: string | undefined, unit: string) => {
    let v = n === "半" ? 0.5 : /\./.test(n) ? parseFloat(n) : cnNum(n) ?? 0;
    if (half) v += 0.5;
    return v * (unit === "分钟" ? 1 : unit === "刻钟" ? 15 : 60);
  };
  if (
    (m = cut(new RegExp(`(${NUMS}|半|\\d+\\.\\d+)\\s*个?\\s*(半)?\\s*(${REL_UNIT})\\s*(?:以后|之后|后)`))) ||
    (m = cut(new RegExp(`过\\s*(${NUMS}|半|\\d+\\.\\d+)\\s*个?\\s*(半)?\\s*(${REL_UNIT})`)))
  ) {
    rel = addMin(now, relMin(m[1], m[2], m[3]));
  } else if ((m = cut(new RegExp(`(${NUMS}|半)\\s*个?\\s*(天|日|${WEEK}|月|年)\\s*(?:以后|之后|后)`)))) {
    // 三天后、一周后、两个月后：那天的同一时刻（另说了钟点就用那个）
    const n = m[1] === "半" ? 0.5 : cnNum(m[1]) ?? 0;
    const u = m[2];
    relDay =
      u === "天" || u === "日" ? addDays(now, n) : /周|星期|礼拜/.test(u) ? addDays(now, n * 7) : u === "月" ? (n === 0.5 ? addDays(now, 15) : addMonths(now, n)) : addMonths(now, n * 12);
  } else if ((m = cut(new RegExp(`过\\s*(${NUMS})\\s*天`)))) date = addDays(today, cnNum(m[1]) ?? 2);
  else if (cut(/(?:过|等|待)?\s*一会儿?(?!议)|[等待]会儿?(?!议)/)) [rel, softRel] = [addMin(now, 30), true];
  else if (cut(/(?<![今明昨每傍])晚点儿?(?!名)/)) [rel, softRel] = [ceil5(addMin(now, 120)), true];

  // ---------- 日期 ----------
  if (!date && !relDay && (!rel || softRel)) {
    if (cut(/大后天/)) date = addDays(today, 3);
    else if (cut(/后天/)) date = addDays(today, 2);
    else if ((m = cut(/明天|明早|明晚|明儿|明日|明夜/))) {
      date = addDays(today, 1);
      if (/晚|夜/.test(m[0])) periodHint = "晚上";
      if (/早/.test(m[0])) periodHint = "早上";
    } else if ((m = cut(/今天|今早|今晚|今儿|今日|今夜/))) {
      date = today;
      if (/晚|夜/.test(m[0])) periodHint = "晚上";
      if (/早/.test(m[0])) periodHint = "早上";
    } else if ((m = cut(/(下下|下个?|这个?|本)?\s*周末/))) {
      // 周末是这周六，今天已经是周日就是今天
      const wd = weekdayOf(now);
      const sat = addDays(today, 6 - wd);
      date = m[1]?.startsWith("下下") ? addDays(sat, 14) : m[1]?.startsWith("下") ? addDays(sat, 7) : wd === 7 ? today : sat;
    } else if ((m = cut(new RegExp(`(下下|下个?|这个?|本)?\\s*${WEEK}([一二三四五六日天])`)))) {
      const monday = addDays(today, 1 - weekdayOf(now));
      const weeks = m[1]?.startsWith("下下") ? 2 : m[1]?.startsWith("下") ? 1 : 0;
      date = addDays(monday, weeks * 7 + WDMAP[m[2]] - 1);
      if (!m[1]) {
        bareWeekday = true;
        if (date < today) date = addDays(date, 7);
      }
    } else if ((m = cut(/下个?月的?(?:底|末)/))) date = monthEnd(now.getFullYear(), now.getMonth() + 1);
    else if ((m = cut(new RegExp(`(下|这|本)个?月的?\\s*${NUM}\\s*[号日]`)))) {
      const day = cnNum(m[2]) ?? 1;
      date = new Date(now.getFullYear(), now.getMonth() + (m[1] === "下" ? 1 : 0), day);
      if (date < today) date = new Date(now.getFullYear(), now.getMonth() + 1, day);
    } else if (cut(/下个?月初?/)) {
      date = new Date(now.getFullYear(), now.getMonth() + 1, 1);
      if (!/月初/.test(text)) vague = "month";
    } else if (cut(new RegExp(`下个?${WEEK}(?![一二三四五六日天末])`))) {
      date = addDays(today, 8 - weekdayOf(now));
      vague = "week";
    } else if (cut(/(?:这个?|本)?月(?:底|末)/)) {
      date = monthEnd(now.getFullYear(), now.getMonth());
      roll = monthEnd(now.getFullYear(), now.getMonth() + 1);
    } else if (cut(/(?:这个?|本)?月初/)) date = new Date(now.getFullYear(), now.getMonth() + 1, 1);
    else if (cut(/(?:今年)?年(?:底|末)/)) {
      date = new Date(now.getFullYear(), 11, 31);
      roll = new Date(now.getFullYear() + 1, 11, 31);
    } else if ((m = cut(new RegExp(`${NUM}\\s*月\\s*${NUM}\\s*[日号]?`)))) {
      const mon = cnNum(m[1]) ?? 0, day = cnNum(m[2]) ?? 0;
      if (validMD(mon, day)) date = nextMD(today, mon, day);
    } else if ((m = cut(/(?<![\d/.:：-])(\d{1,2})\s*\/\s*(\d{1,2})(?![\d/.])/, (x) => validMD(+x[1], +x[2])))) date = nextMD(today, +m[1], +m[2]);
    else if ((m = cut(/(?<![\d/.:：-])(\d{1,2})-(\d{1,2})(?![\d/.:：-]|\s*[点时])/, (x) => validMD(+x[1], +x[2])))) date = nextMD(today, +m[1], +m[2]);
    else if (
      // 3.5：只有后面紧跟钟点或"号"时才算日期，"买 3.5 斤苹果"不算
      (m = cut(new RegExp(`(?<![\\d.])(\\d{1,2})\\.(\\d{1,2})(?:\\s*[号日]|(?=\\s*(?:${PERIODS}|\\d{1,2}\\s*[:：点时]|${CNUM}\\s*[点时])))`), (x) => validMD(+x[1], +x[2])))
    )
      date = nextMD(today, +m[1], +m[2]);
    else if ((m = cut(new RegExp(`(?<![月\\d./])${NUM}\\s*[号日](?![子线楼门床位机桌房厅柜元])`)))) {
      const day = cnNum(m[1]) ?? 0;
      if (day >= 1 && day <= 31) {
        date = new Date(now.getFullYear(), now.getMonth(), day);
        if (date < today) date = new Date(now.getFullYear(), now.getMonth() + 1, day);
      }
    }
  }
  const explicitDate = !!date;
  if (date && softRel) rel = null;

  // ---------- 钟点 ----------
  if (!clocks.length) {
    // 几个钟点："明天 9 点和下午 3 点各提醒一次""每天早晚各一次"
    const ONE = `(?:(?:${PERIODS})\\s*)?(?:\\d{1,2}|${CNUM})(?:\\s*[:：]\\s*\\d{2}|\\s*[点时](?:钟|整)?(?:\\s*(?:${MINW})\\s*分?)?)|(?:${PERIODS})`;
    if ((m = cut(/早中晚(?=\s*各)/))) clocks = ["早上", "中午", "晚上"].map((p) => toClock(p, PERIOD_HOUR[p], 0)!);
    else if ((m = cut(/早晚(?=\s*各)/))) clocks = ["早上", "晚上"].map((p) => toClock(p, PERIOD_HOUR[p], 0)!);
    else if ((m = cut(new RegExp(`(?:${ONE})(?:\\s*(?:和|跟|与|及|以及|还有|、|，|,)\\s*(?:${ONE}))+`)))) {
      let inherit = periodHint;
      for (const part of m[0].split(/\s*(?:和|跟|与|以及|及|还有|、|，|,)\s*/)) {
        const pm = part.match(new RegExp(`^${CLOCK}$`));
        const c = pm ? clockOf([...pm], 1, inherit) : PERIOD_HOUR[part] !== undefined ? toClock(part, PERIOD_HOUR[part], 0) : null;
        if (c) {
          clocks.push(c);
          inherit = c.period || inherit;
        }
      }
    }
    // "各提醒一次""各吃一次药"：去掉"各""一次"，留下中间的字
    if (clocks.length) cut(/各([^，,\s]{0,4}?)(?:一次|一遍|1次)/, undefined, (x) => x[1]);
  }
  if (!clocks.length) {
    m = cut(new RegExp(CLOCK), (x, before, after) => {
      // "一点点""早一点"不是一点钟
      if (/^[一两]$/.test(x[2]) && !x[1] && (/[早晚快慢多少差好有这那些点]$/.test(before) || /^[点儿心]/.test(after))) return false;
      return true;
    });
    let period = m?.[1] ?? "";
    if (!period) {
      // 单说"早""晚"：后面紧跟着认出来的钟点才算（"早8点"），"早一点睡"不算
      const pm = cut(new RegExp(PERIODS)) ?? (m ? cut(new RegExp(`(?<![今明每])[早晚](?=${CUT})`)) : null);
      period = pm ? (pm[0] === "早" ? "早上" : pm[0] === "晚" ? "晚上" : pm[0]) : periodHint;
    }
    if (m) {
      const c = clockOf([...m], 1, period);
      if (c) clocks = [c];
    } else if (period) clocks = [toClock(period, PERIOD_HOUR[period] ?? 9, 0)!];
  }
  const hasClock = clocks.length > 0;

  // ---------- 截止："周五前交报告""11 月 10 号之前续费" ----------
  let deadline = false;
  if (!recurring && (explicitDate || hasClock)) {
    if (cut(new RegExp(`(?<=${CUT})\\s*(?:之前|以前|前)(?![天台面后])`)) || cut(new RegExp(`截止(?:到|日期是?|时间是?)?\\s*(?=${CUT})|(?<=${CUT})\\s*(?:截止|为止)`))) deadline = true;
  }

  const wantsReminder = /提醒|记得|别忘|叫我|闹钟|通知我/.test(text);
  const hasWhen = !!(rel || relDay || date || hasClock || recurring || vague);
  const title = clean(t);
  const one: One = { items: [], hasWhen, title, date: explicitDate ? date : null };
  if (!hasWhen && !wantsReminder) return one;

  // ---------- 算时间 ----------
  const base = date ?? (hasClock && prevDate ? prevDate : today);
  const sameDate = !!date || (hasClock && !!prevDate);
  const details: string[] = [];
  if (lunarNote) details.push(lunarNote);
  let guess = "";
  const dues: Date[] = [];
  /** 周期提醒：每条自己的规则（每月 1 号和 15 号是两条） */
  const repeats: [Repeat, Date][] = [];

  if (rel) dues.push(rel);
  else if (relDay) {
    dues.push(...(hasClock ? clocks.map((c) => at(relDay!, c.h, c.m)) : [new Date(relDay.getFullYear(), relDay.getMonth(), relDay.getDate(), now.getHours(), now.getMinutes())]));
  } else if (recurring) {
    for (const md of monthdays.length > 1 ? monthdays : [0]) {
      const r = md ? { ...repeat, monthday: md } : repeat;
      for (const c of hasClock ? clocks : [{ h: 9, m: 0, period: "" }]) {
        // 隔周：从最近的那一天开始算
        const first = r.kind === "weekly" && r.every > 1 ? normalize({ ...r, every: 0 }, at(base, c.h, c.m), now) : at(base, c.h, c.m);
        repeats.push([r, normalize(r, first, now)]);
      }
    }
  } else if (deadline) {
    // 截止：带了钟点提前 2 小时；只说了日期，前一天 15:00；截止就是今天的，截止前 2 小时
    const c = clocks[0];
    let dl = c ? at(base, c.h, c.m) : sameDay(base, now) ? at(base, DEADLINE_TODAY_HOUR, 0) : at(base, 9, 0);
    // "5 点前"说在上午：是下午 5 点
    if (c && dl < now && !c.period && !c.exact && c.h < 12 && addMin(dl, 720) > now) dl = addMin(dl, 720);
    let due = ahead ? addMin(c ? dl : at(base, 9, 0), -ahead) : c || sameDay(base, now) ? addMin(dl, -120) : at(addDays(base, -1), 15, 0);
    if (due <= now) due = ceil5(addMin(now, 10));
    dues.push(due);
    details.push(`截止：${mdw(dl)}${c ? hm(dl) : ""}`);
    ahead = 0;
  } else if (hasClock) {
    for (const c of clocks) {
      let due = at(base, c.h, c.m);
      if (due < now && sameDay(base, now)) {
        // "8点提醒我"说在下午：先试晚上 8 点，再顺延
        if (!c.period && !c.exact && c.h < 12 && due < now && addMin(due, 720) > now) due = addMin(due, 720);
        else if (bareWeekday) due = addDays(due, 7);
        else if (!sameDate) due = addDays(due, 1);
      }
      dues.push(due);
    }
  } else if (date) dues.push(at(date, 9, 0));
  else if (vague) dues.push(at(today, 9, 0));
  else {
    // 只说了"提醒我"：一小时后
    const due = ceil5(addMin(now, 60));
    dues.push(due);
    guess = `没说什么时候，先定在 1 小时后（${hmShort(due)}）`;
  }

  if (roll && dues[0] && dues[0] <= now) dues[0] = at(roll, dues[0].getHours(), dues[0].getMinutes());

  // 含糊的说法：说了别的日期就用那个，不然先定一个
  if (vague && !recurring && !explicitDate && !rel && !relDay) {
    const day = vague === "tomorrow" ? addDays(today, 1) : vague === "days" ? addDays(today, 3) : startOfDay(date ?? today);
    for (let i = 0; i < dues.length; i++) dues[i] = at(day, hasClock ? dues[i].getHours() : 9, hasClock ? dues[i].getMinutes() : 0);
    guess = `没听清具体时间，先定在${whenWord(dues[0], now)}`;
  } else if (vague === "month" || vague === "week") guess = `没听清具体时间，先定在${whenWord(dues[0], now)}`;

  for (const d of dues) repeats.push([repeat, d]);
  if (rangeLen && repeats[0]) details.push(`${hm(repeats[0][1])}–${hm(addMin(repeats[0][1], rangeLen))}`);
  const tag = recurring && tagDef("cycle") && /还款|账单|缴费|续费|房租/.test(text) ? "cycle" : guessTag(text);
  for (const [rule, due0] of repeats) {
    let due = due0;
    const detail = [...details];
    if (ahead) {
      detail.push(`事情本身在 ${due.getMonth() + 1}月${due.getDate()}日 ${hm(due)}`);
      due = addMin(due, -ahead);
    }
    one.items.push({ type: "reminder", title: title || "时间到了", detail: detail.join("，"), tag, due: fmtDT(due), repeat: rule, urgent, only_on: [], guess });
  }
  return one;
}

const CJK_PUNCT = "，。、；：！？（）《》「」“”‘’【】";

/** 去掉时间说法后的标题：去掉"提醒我"这类词，收拾多出来的空格和标点 */
function clean(s: string): string {
  const C = CUT;
  let out = s
    .replace(/提醒一下我?|提醒我|提醒|通知我|告诉我|叫我|记得要?|别忘了?|别忘记|不要忘了?|不要忘记|帮我|一下|到时候|的时候/g, C)
    // "从下周一开始每天跑步"、"在周五交报告"、"7 点开始开会"：跟着时间的这几个字一起去掉
    .replace(new RegExp(`(?:从|在|于)\\s*(?=${C})`, "g"), C)
    .replace(new RegExp(`(?<=${C})\\s*(?:开始|起)(?![来床飞])`, "g"), C)
    .replace(new RegExp(`(?<=${C})\\s*的(?=[\\u4e00-\\u9fff])`, "g"), C);
  // 切掉的地方：原来有空格就留一个空格，两边都是英文、数字也留一个，别的直接接上
  out = out.replace(new RegExp(`[\\s${C}]*${C}[\\s${C}]*`, "g"), (seg, i: number, all: string) => {
    const prev = all[i - 1] ?? "", next = all[i + seg.length] ?? "";
    if (!prev || !next) return "";
    return /\s/.test(seg) || (/[\w]/.test(prev) && /[\w]/.test(next)) ? " " : "";
  });
  return out
    .replace(new RegExp(`\\s*([${CJK_PUNCT}])\\s*`, "g"), "$1")
    .replace(/([，、；：,;:])(?:\s*[，、；：,;:])+/g, "$1")
    .replace(/[（(]\s*[)）]/g, "")
    .replace(/^[\s，,。.、；;:：!！?？)）\-—~～]+/, "")
    .replace(/^(?:要|得|需要|还要|是(?!否))(?=..)/, "")
    .replace(/[\s，,。.、；;:：!！的了吧啊哦呀嘛呢\-—~～]+$/, "")
    .replace(/\s{2,}/g, " ")
    .trim();
}

const KINDS = new Set(["none", "daily", "weekdays", "weekly", "monthly", "interval", "yearly", "month_last", "month_nth", "lunar_yearly"]);
const int = (v: unknown, min: number, max: number) => (typeof v === "number" && Number.isInteger(v) && v >= min && v <= max ? v : 0);

/** 每年几号、每月几号、每周几、农历这几种，日子由规则定死了 */
const FIXED_DAY = new Set(["weekly", "monthly", "yearly", "month_last", "month_nth", "lunar_yearly"]);

/**
 * AI 给的周期提醒，第一次自己算：AI 算农历、跨月、"今天的已经过了"常常算错。
 * 日子定死的几种从今天起找第一个对上的，只用 AI 给的钟点；每天、工作日、每隔几天照 AI 给的开始（"从下周一开始每天跑步"），过了就往后推。
 * AI 漏填的几号、几月、周几，按它给的日期补上
 */
function firstDue(r: Repeat, due: Date, now: Date): Date {
  if (r.kind === "yearly") {
    r.month ||= due.getMonth() + 1;
    r.monthday ||= due.getDate();
  } else if (r.kind === "monthly") r.monthday ||= due.getDate();
  else if (r.kind === "weekly" && !r.days.length) r.days = [weekday(due)];
  else if (r.kind === "lunar_yearly" && !(r.month && r.monthday)) {
    const l = toLunar(due);
    if (l) {
      r.month ||= l.month;
      r.monthday ||= l.day;
    }
  }
  if (!FIXED_DAY.has(r.kind)) return normalize(r, due, now);
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate(), due.getHours(), due.getMinutes());
  const start = due < today ? due : today;
  // 每两周：从最近的那一天开始数（和本地规则一样）
  const first = r.kind === "weekly" && r.every > 1 ? normalize({ ...r, every: 0 }, start, now) : start;
  return normalize(r, first, now);
}

/** 校验 AI 返回的 JSON，丢掉不合格的条目。AI 说的电脑名（on）换成设备 uid */
export function normalizeAI(raw: unknown, devices: Device[] = [], now = new Date()): ParsedItem[] {
  const list = (raw as { items?: unknown[] })?.items;
  if (!Array.isArray(list)) return [];
  const out: ParsedItem[] = [];
  for (const it of list as Record<string, any>[]) {
    const tag = tagDef(it?.tag) ? (it.tag as Tag) : null;
    if (it?.type === "reminder" && typeof it.title === "string" && parseDT(it.due)) {
      const r = it.repeat ?? {};
      const repeat: Repeat = {
        kind: KINDS.has(r.kind) ? r.kind : "none",
        days: Array.isArray(r.days) ? r.days.filter((d: unknown) => typeof d === "number" && d >= 1 && d <= 7) : [],
        monthday: int(r.monthday, 0, 31),
        every: int(r.every, 0, 3650),
        month: int(r.month, 0, 12),
        nth: r.nth === -1 ? -1 : int(r.nth, 0, 5),
      };
      const at = parseDT(it.due)!;
      const due = repeat.kind === "none" ? fmtDT(at) : fmtDT(firstDue(repeat, at, now));
      out.push({
        type: "reminder",
        title: it.title.trim(),
        detail: typeof it.detail === "string" ? it.detail : "",
        tag,
        due,
        repeat,
        urgent: !!it.urgent,
        only_on: Array.isArray(it.on)
          ? devices.filter((d) => it.on.some((n: unknown) => typeof n === "string" && n.trim() === d.name)).map((d) => d.uid)
          : [],
        // AI 说含糊的，和本地规则一样标出来
        guess: it.vague === true ? `没听清具体时间，先定在${whenWord(parseDT(due)!, now)}` : "",
      });
    } else if (it?.type === "note" && (it.title || it.body)) {
      const body = String(it.body ?? it.title);
      out.push({ type: "note", title: String(it.title ?? ""), body, tag: noteCatDef(it.tag) ? (it.tag as Tag) : guessNoteCat(body), link: !!it.link });
    }
  }
  return out;
}

/** 说了农历的，日子和周期用本地规则算的（AI 换算农历常常差几天）。两边拆出的提醒条数一样才换 */
export function fixLunar(items: ParsedItem[], text: string, now: Date, devices: Device[] = []): ParsedItem[] {
  if (!/农历|阴历/.test(text)) return items;
  const local = parseLocal(text, now, devices).filter((i): i is ParsedReminder => i.type === "reminder");
  const rems = items.filter((i): i is ParsedReminder => i.type === "reminder");
  if (local.length !== rems.length) return items;
  rems.forEach((r, i) => {
    r.due = local[i].due;
    r.repeat = local[i].repeat;
  });
  return items;
}

/** 有 Key 先问 AI，失败就用本地规则。devices：同步过的电脑，用来理解"到公司提醒我" */
export async function smartParse(text: string, useAI: boolean, timeoutMs = 20000, devices: Device[] = []): Promise<ParseResult> {
  const local = () => parseLocal(text, new Date(), devices);
  if (useAI) {
    try {
      const raw = await Promise.race([
        api.aiParse(text, devices.map((d) => d.name)),
        new Promise((_, rej) => setTimeout(() => rej("AI 响应太慢，先用本地规则"), timeoutMs)),
      ]);
      const items = fixLunar(normalizeAI(raw, devices), text, new Date(), devices);
      // AI 没认出地点、本地规则认出来了，用本地的
      const place = matchPlace(text, devices);
      for (const it of items) if (it.type === "reminder" && !it.only_on.length) it.only_on = place.only_on;
      if (items.length) return { items, source: "ai" };
      return { items: local(), source: "local", error: "AI 没理解出内容" };
    } catch (e) {
      return { items: local(), source: "local", error: errText(e) };
    }
  }
  return { items: local(), source: "local" };
}

/** 保存拆出来的条目；笔记 link=true 时挂到同一批的第一条提醒上。说得含糊的提醒标上 vague。回存好的（快捷框撤销时用） */
export async function saveParsed(items: ParsedItem[]): Promise<{ reminders: Reminder[]; notes: Note[] }> {
  const reminders: Reminder[] = [];
  const notes: Note[] = [];
  for (const it of items) {
    if (it.type !== "reminder") continue;
    const saved = await api.saveReminder(
      newReminder({ title: it.title, detail: it.detail, tag: it.tag, due_at: it.due, repeat: it.repeat, urgent: it.urgent, only_on: it.only_on, vague: !!it.guess }),
    );
    reminders.push(saved);
  }
  const firstReminderId = reminders[0]?.id ?? null;
  for (const it of items) {
    if (it.type !== "note") continue;
    notes.push(await api.saveNote(newNote({ title: it.title, body: it.body, tag: it.tag, reminder_id: it.link ? firstReminderId : null })));
  }
  return { reminders, notes };
}
