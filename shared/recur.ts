// 周期提醒：算下一次该响的时间。和电脑端 src-tauri/src/recur.rs 一样，格式见 docs/方案/同步方案.md 4.2
import { monthDays, toLunar } from "./lunar";

/** 只用到这几个字段；month、nth 是 0.2 加的，旧记录没有 */
export interface RepeatRule {
  kind: string;
  days: number[];
  monthday: number;
  every: number;
  month?: number;
  nth?: number;
}

export const isRecurring = (r: RepeatRule) => !!r.kind && r.kind !== "none";

export const lastDayOfMonth = (d: Date) => new Date(d.getFullYear(), d.getMonth() + 1, 0).getDate();
/** 1 = 周一 … 7 = 周日 */
export const weekday = (d: Date) => d.getDay() || 7;
const dateOnly = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate());
const daysBetween = (a: Date, b: Date) => Math.round((dateOnly(b).getTime() - dateOnly(a).getTime()) / 86400000);
const monday = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate() - weekday(d) + 1);
/** 这天是这个月的第几个周几 */
export const nthOf = (d: Date) => Math.ceil(d.getDate() / 7);

/** base 是这条提醒第一次的日期，没填的字段（几号、几月、第几个）沿用它 */
export function matches(r: RepeatRule, base: Date, d: Date): boolean {
  switch (r.kind) {
    case "daily":
      return true;
    case "weekdays":
      return weekday(d) <= 5;
    case "weekly": {
      if (!(r.days.length ? r.days.includes(weekday(d)) : weekday(d) === weekday(base))) return false;
      // 每几周：从 base 当天或之后第一个对上的那天所在的那一周开始数
      const n = Math.max(1, r.every);
      if (n === 1) return true;
      let anchor = base;
      for (let i = 0; i < 7 && r.days.length && !r.days.includes(weekday(anchor)); i++) anchor = new Date(base.getFullYear(), base.getMonth(), base.getDate() + i + 1);
      const weeks = Math.round(daysBetween(monday(anchor), monday(d)) / 7);
      return ((weeks % n) + n) % n === 0;
    }
    case "monthly": {
      const want = r.monthday || base.getDate();
      return d.getDate() === Math.min(want, lastDayOfMonth(d));
    }
    case "interval": {
      const diff = daysBetween(base, d);
      return diff >= 0 && diff % Math.max(1, r.every) === 0;
    }
    case "yearly": {
      const m = r.month || base.getMonth() + 1;
      const md = r.monthday || base.getDate();
      // 2 月 29 号在平年是 28 号
      return d.getMonth() + 1 === m && d.getDate() === Math.min(md, lastDayOfMonth(d));
    }
    case "month_last":
      return d.getDate() === lastDayOfMonth(d);
    case "month_nth": {
      if (weekday(d) !== (r.days[0] || weekday(base))) return false;
      const nth = r.nth || nthOf(base);
      return nth === -1 ? d.getDate() + 7 > lastDayOfMonth(d) : nthOf(d) === nth;
    }
    case "lunar_yearly": {
      const l = toLunar(d);
      // 闰月不算，只在平常那个月提醒
      if (!l || l.leap) return false;
      const b = r.month && r.monthday ? null : toLunar(base);
      const m = r.month || b?.month || 1;
      const md = r.monthday || b?.day || 1;
      // 那个月没有三十就在二十九
      return l.month === m && l.day === Math.min(md, monthDays(l.year, l.month));
    }
    default:
      return false;
  }
}

/** 严格晚于 after 的下一次，沿用 due 的钟点 */
export function nextAfter(r: RepeatRule, due: Date, after: Date): Date | null {
  if (!isRecurring(r)) return null;
  let d = dateOnly(after);
  for (let i = 0; i < 800; i++) {
    const cand = new Date(d.getFullYear(), d.getMonth(), d.getDate(), due.getHours(), due.getMinutes());
    if (cand > after && matches(r, due, d)) return cand;
    d = new Date(d.getFullYear(), d.getMonth(), d.getDate() + 1);
  }
  return null;
}

/** 新建或修改周期提醒时，开始时间对齐到第一个符合规则、且不早于现在的时间 */
export function normalize(r: RepeatRule, due: Date, now = new Date()): Date {
  if (!isRecurring(r)) return due;
  const after = due > now ? new Date(due.getTime() - 60000) : now;
  return nextAfter(r, due, after) ?? due;
}
