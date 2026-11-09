// 改提醒的抽屉里「什么时候」那一栏一开始写的话：本地规则（parse.ts）能原样认回来，when.test.ts 核对
import { timeOf, parseDT } from "@shared/util";

/** 离今天几天：明天 1，昨天 -1 */
export function dayDiff(d: Date, now = new Date()) {
  const a = new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const b = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  return Math.round((a - b) / 86400000);
}

/** 今天 14:00 / 明天 09:30 / 11月6日 14:00 / 2027年1月3日 08:00 */
export function whenText(due: string, now = new Date()) {
  const d = parseDT(due);
  if (!d) return "";
  const n = dayDiff(d, now);
  const year = d.getFullYear() !== now.getFullYear() ? `${d.getFullYear()}年` : "";
  const day = n === 0 ? "今天" : n === 1 ? "明天" : `${year}${d.getMonth() + 1}月${d.getDate()}日`;
  return `${day} ${timeOf(due)}`;
}
