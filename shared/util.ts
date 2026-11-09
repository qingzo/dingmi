import { lunarLabel, toLunar } from "./lunar";
import type { Reminder, Repeat, Tag, TagDef } from "./types";

// 分类：在「设置 → 分类」里自己增删改，几台电脑共用。提醒和笔记各一套（v0.3 起），每个窗口拿到设置后调用 setTags / setNoteCats
export const TAG_COLORS = ["blue", "orange", "pink", "purple", "green", "teal", "red", "amber", "gray"];
let tags: TagDef[] = [];
export const setTags = (list: TagDef[] | undefined) => (tags = list ?? []);
export const tagList = () => tags;
export const tagDef = (id: Tag | null | undefined) => (id ? tags.find((t) => t.id === id) : undefined);
/** 色板里的颜色，给 .tag / .chip.tagged / 色块用的 --c */
export const tagColor = (color: string) => `--c:var(--tc-${TAG_COLORS.includes(color) ? color : "gray"})`;
const tagSpan = (t: TagDef | undefined) => (t ? `<span class="tag" style="${tagColor(t.color)}">${esc(t.name)}</span>` : "");
const chips = (list: TagDef[], sel: Tag | null) =>
  list.map((t) => `<button type="button" class="chip tagged" style="${tagColor(t.color)}" data-tag="${esc(t.id)}" aria-pressed="${sel === t.id}">${esc(t.name)}</button>`).join("");
/** 提醒上的分类小标；分类已经删掉了就不显示 */
export const tagHtml = (id: Tag | null | undefined) => tagSpan(tagDef(id));
/** 编辑提醒时选分类的一排小圆片 */
export const tagChips = (sel: Tag | null) => chips(tags, sel);

// 笔记分类：笔记上存的 tag 是笔记分类的 id
let noteCats: TagDef[] = [];
export const setNoteCats = (list: TagDef[] | undefined) => (noteCats = list ?? []);
export const noteCatList = () => noteCats;
export const noteCatDef = (id: Tag | null | undefined) => (id ? noteCats.find((t) => t.id === id) : undefined);
export const noteCatHtml = (id: Tag | null | undefined) => tagSpan(noteCatDef(id));
export const noteCatChips = (sel: Tag | null) => chips(noteCats, sel);
const WD = ["日", "一", "二", "三", "四", "五", "六"];

export const $ = <T extends Element = HTMLElement>(s: string, r: ParentNode = document) => r.querySelector(s) as T;
export const $$ = <T extends Element = HTMLElement>(s: string, r: ParentNode = document) => [...r.querySelectorAll(s)] as T[];

/** 复制到剪贴板；网页的剪贴板接口用不了时退回老办法 */
export async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.cssText = "position:fixed;opacity:0";
    document.body.append(ta);
    ta.select();
    document.execCommand("copy");
    ta.remove();
  }
}

export const esc = (s: string) =>
  s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);

const pad = (n: number) => String(n).padStart(2, "0");

export const dayStr = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
export const fmtDT = (d: Date) => `${dayStr(d)} ${pad(d.getHours())}:${pad(d.getMinutes())}`;

export function parseDT(s: string | null | undefined): Date | null {
  const m = s?.match(/^(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2})/);
  return m ? new Date(+m[1], +m[2] - 1, +m[3], +m[4], +m[5]) : null;
}

export const addDays = (d: Date, n: number) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n, d.getHours(), d.getMinutes());

export const fireAt = (r: Reminder) => r.snoozed_until ?? r.due_at;
export const timeOf = (s: string) => s.slice(11, 16);
export const weekdayCn = (d: Date) => "周" + WD[d.getDay()];

/** 今天 / 明天 / 后天 / 昨天 / 11月3日 周二 */
export function friendlyDay(s: string, withWeek = true): string {
  const d = parseDT(s);
  if (!d) return "";
  const today = new Date();
  const diff = Math.round(
    (new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime() -
      new Date(today.getFullYear(), today.getMonth(), today.getDate()).getTime()) /
      86400000,
  );
  const names: Record<number, string> = { [-1]: "昨天", 0: "今天", 1: "明天", 2: "后天" };
  if (names[diff] !== undefined) return names[diff];
  const yr = d.getFullYear() !== today.getFullYear() ? `${d.getFullYear()}年` : "";
  return `${yr}${d.getMonth() + 1}月${d.getDate()}日${withWeek ? " " + weekdayCn(d) : ""}`;
}

export const friendlyDT = (s: string) => `${friendlyDay(s)} ${timeOf(s)}`;

const NTH = ["", "第一个", "第二个", "第三个", "第四个", "第五个"];

export function repeatLabel(r: Repeat, due?: string): string {
  const d = parseDT(due);
  switch (r.kind) {
    case "daily":
      return "每天";
    case "weekdays":
      return "工作日";
    case "weekly": {
      const days = r.days.length ? [...r.days].sort() : d ? [d.getDay() || 7] : [];
      const n = Math.max(1, r.every);
      if (days.length === 7 && n === 1) return "每天";
      return (n === 1 ? "每周" : `每${n === 2 ? "两" : ` ${n} `}周的周`) + days.map((x) => WD[x % 7]).join("、");
    }
    case "monthly":
      return `每月 ${r.monthday || d?.getDate() || 1} 日`;
    case "interval":
      return `每 ${Math.max(1, r.every)} 天`;
    case "yearly":
      return `每年 ${r.month || (d ? d.getMonth() + 1 : 1)} 月 ${r.monthday || d?.getDate() || 1} 日`;
    case "month_last":
      return "每月最后一天";
    case "month_nth": {
      const nth = r.nth || (d ? Math.ceil(d.getDate() / 7) : 1);
      return `每月${nth === -1 ? "最后一个" : NTH[nth] ?? ""}周${WD[(r.days[0] || (d ? d.getDay() || 7 : 1)) % 7]}`;
    }
    case "lunar_yearly": {
      // 农历的同时写上这次是公历几号
      const l = d ? toLunar(d) : null;
      const m = r.month || l?.month || 1;
      const md = r.monthday || l?.day || 1;
      return `每年农历${lunarLabel(m, md)}${d ? ` · ${d.getMonth() + 1}月${d.getDate()}日` : ""}`;
    }
    default:
      return "";
  }
}

export const isRecurring = (r: Repeat) => !!r.kind && r.kind !== "none";

export function greeting(h = new Date().getHours()) {
  if (h < 5) return "夜深了";
  if (h < 11) return "早上好";
  if (h < 13) return "中午好";
  if (h < 18) return "下午好";
  return "晚上好";
}

/** 2 分钟前 / 3 小时前 / 昨天 / 9月14日 */
export function ago(s: string): string {
  const d = new Date(s.replace(" ", "T"));
  if (isNaN(d.getTime())) return "";
  const sec = (Date.now() - d.getTime()) / 1000;
  if (sec < 60) return "刚刚";
  if (sec < 3600) return `${Math.floor(sec / 60)} 分钟前`;
  if (dayStr(d) === dayStr(new Date())) return `今天 ${s.slice(11, 16)}`;
  if (dayStr(d) === dayStr(addDays(new Date(), -1))) return "昨天";
  const yr = d.getFullYear() !== new Date().getFullYear() ? `${d.getFullYear()}年` : "";
  return `${yr}${d.getMonth() + 1}月${d.getDate()}日`;
}

export const hotkeyText = (s: string) => (s ? s.split("+").map((p) => p.trim()).join(" + ") : "未设置");

/** 把键盘事件转成快捷键字符串，比如 "Ctrl+Alt+N"；只按了修饰键时返回 null */
export function hotkeyFromEvent(e: KeyboardEvent): { combo: string; hasMod: boolean } | null {
  // e.code 是空的：程序模拟出来的按键（比如闪念输入取选中文字时按的复制），不是用户按的
  if (["Control", "Alt", "Shift", "Meta"].includes(e.key) || !e.code) return null;
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super");
  let key = e.code;
  if (key.startsWith("Key")) key = key.slice(3);
  else if (key.startsWith("Digit")) key = key.slice(5);
  return { combo: [...mods, key].join("+"), hasMod: mods.length > 0 };
}

let toastTimer = 0;
export function toast(msg: string, el: HTMLElement = $("#toast")) {
  el.textContent = msg;
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => (el.hidden = true), 2400);
}

export function debounce<A extends unknown[]>(fn: (...a: A) => void, ms: number) {
  let t = 0;
  return (...a: A) => {
    clearTimeout(t);
    t = window.setTimeout(() => fn(...a), ms);
  };
}

/** 提醒音：两个柔和的音符；loud 时响一点、多叮一下（紧急的提醒） */
export function chime(loud = false) {
  try {
    const ctx = new AudioContext();
    const play = (freq: number, at: number) => {
      const o = ctx.createOscillator();
      const g = ctx.createGain();
      o.type = "sine";
      o.frequency.value = freq;
      g.gain.setValueAtTime(0, ctx.currentTime + at);
      g.gain.linearRampToValueAtTime(loud ? 0.42 : 0.22, ctx.currentTime + at + 0.02);
      g.gain.exponentialRampToValueAtTime(0.001, ctx.currentTime + at + 0.9);
      o.connect(g).connect(ctx.destination);
      o.start(ctx.currentTime + at);
      o.stop(ctx.currentTime + at + 1);
    };
    play(880, 0);
    play(1318.5, 0.16);
    if (loud) play(1760, 0.32);
    setTimeout(() => ctx.close(), 1800);
  } catch {
    /* 没有声卡也没关系 */
  }
}
