// 提醒编辑里"重复"那一栏 0.2 加的几种（每几周、每年、每月第几个周几、每年农历）：电脑和手机的编辑界面共用
import { LUNAR_MONTHS, lunarDayName, toLunar } from "./lunar";
import { nthOf, weekday } from "./recur";
import type { Repeat, RepeatKind } from "./types";

export const REPEAT_KINDS: [RepeatKind, string][] = [
  ["none", "不重复"],
  ["daily", "每天"],
  ["weekdays", "工作日（周一到周五）"],
  ["weekly", "每周（或每几周）"],
  ["monthly", "每月几号"],
  ["month_last", "每月最后一天"],
  ["month_nth", "每月第几个周几"],
  ["yearly", "每年"],
  ["lunar_yearly", "每年农历"],
  ["interval", "每几天"],
];

const WD = ["一", "二", "三", "四", "五", "六", "日"];
const NTH: [number, string][] = [[1, "第一个"], [2, "第二个"], [3, "第三个"], [4, "第四个"], [5, "第五个"], [-1, "最后一个"]];
const opts = (list: [number, string][], sel: number) => list.map(([v, l]) => `<option value="${v}" ${v === sel ? "selected" : ""}>${l}</option>`).join("");
const range = (n: number, label: (i: number) => string): [number, string][] => Array.from({ length: n }, (_, i) => [i + 1, label(i + 1)]);

/** 这几种的输入框，row 是一行的 class（电脑 inline-row，手机 inline）。没填的按开始日期 due 给默认值 */
export function repeatExtrasHtml(r: Repeat, kind: string, due: Date, row: string): string {
  const l = toLunar(due);
  const show = (k: string) => (kind === k ? "" : "hidden");
  const num = (id: string, min: number, max: number, v: number) => `<input class="field" type="number" inputmode="numeric" min="${min}" max="${max}" id="${id}" value="${v}">`;
  const sel = (id: string, html: string) => `<select class="field" id="${id}">${html}</select>`;
  return `
    <div class="${row}" id="ed-wevery" ${show("weekly")}>每 ${num("ed-wn", 1, 8, Math.max(1, r.every || 1))} 周（2 就是隔一周，从最近的那天开始数）</div>
    <div class="${row}" id="ed-nth" ${show("month_nth")}>每月 ${sel("ed-nth-n", opts(NTH, r.nth || nthOf(due)))} ${sel("ed-nth-wd", opts(range(7, (i) => "周" + WD[i - 1]), r.days[0] || weekday(due)))}</div>
    <div class="${row}" id="ed-year" ${show("yearly")}>每年 ${num("ed-ym", 1, 12, r.month || due.getMonth() + 1)} 月 ${num("ed-yd", 1, 31, r.monthday || due.getDate())} 日（2 月 29 号在平年是 28 号）</div>
    <div class="${row}" id="ed-lunar" ${show("lunar_yearly")}>每年农历 ${sel("ed-lm", opts(range(12, (i) => LUNAR_MONTHS[i - 1]), r.month || l?.month || 1))} ${sel("ed-ld", opts(range(30, lunarDayName), r.monthday || l?.day || 1))}（闰月不算；没有三十的月份在二十九）</div>`;
}

/** 换了"重复"的种类：只显示用得到的那一栏 */
export function showRepeatExtras(root: ParentNode, kind: string) {
  const ids: Record<string, string> = { "ed-wevery": "weekly", "ed-nth": "month_nth", "ed-year": "yearly", "ed-lunar": "lunar_yearly" };
  for (const [id, k] of Object.entries(ids)) (root.querySelector(`#${id}`) as HTMLElement).hidden = kind !== k;
}

/** 读出这几种要存的字段；别的种类返回空 */
export function readRepeatExtras(root: ParentNode, kind: string): Partial<Repeat> {
  const v = (id: string) => Number((root.querySelector(`#${id}`) as HTMLInputElement | HTMLSelectElement).value) || 0;
  const clamp = (n: number, min: number, max: number) => Math.min(max, Math.max(min, n));
  switch (kind) {
    case "weekly": {
      const n = clamp(v("ed-wn"), 1, 8);
      return { every: n > 1 ? n : 0 };
    }
    case "month_nth":
      return { nth: v("ed-nth-n") === -1 ? -1 : clamp(v("ed-nth-n"), 1, 5), days: [clamp(v("ed-nth-wd"), 1, 7)] };
    case "yearly":
      return { month: clamp(v("ed-ym"), 1, 12), monthday: clamp(v("ed-yd"), 1, 31) };
    case "lunar_yearly":
      return { month: clamp(v("ed-lm"), 1, 12), monthday: clamp(v("ed-ld"), 1, 30) };
    default:
      return {};
  }
}
