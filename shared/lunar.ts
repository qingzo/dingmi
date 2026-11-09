// 农历：1900–2100 年。和电脑端 src-tauri/src/lunar.rs 是同一份表，两边用 shared/lunar.cases.json 核对。
//
// 每年一个数：低 4 位是闰几月（0 没有闰月）；0x8000 … 0x10 依次是正月 … 腊月，1 是大月（30 天）、0 是小月（29 天）；
// 0x10000 是闰月是不是大月。1900 年正月初一是公历 1900-01-31。

const INFO = [
  0x04bd8, 0x04ae0, 0x0a570, 0x054d5, 0x0d260, 0x0d950, 0x16554, 0x056a0, 0x09ad0, 0x055d2, // 1900
  0x04ae0, 0x0a5b6, 0x0a4d0, 0x0d250, 0x1d255, 0x0b540, 0x0d6a0, 0x0ada2, 0x095b0, 0x14977, // 1910
  0x04970, 0x0a4b0, 0x0b4b5, 0x06a50, 0x06d40, 0x1ab54, 0x02b60, 0x09570, 0x052f2, 0x04970, // 1920
  0x06566, 0x0d4a0, 0x0ea50, 0x16a95, 0x05ad0, 0x02b60, 0x186e3, 0x092e0, 0x1c8d7, 0x0c950, // 1930
  0x0d4a0, 0x1d8a6, 0x0b550, 0x056a0, 0x1a5b4, 0x025d0, 0x092d0, 0x0d2b2, 0x0a950, 0x0b557, // 1940
  0x06ca0, 0x0b550, 0x15355, 0x04da0, 0x0a5b0, 0x14573, 0x052b0, 0x0a9a8, 0x0e950, 0x06aa0, // 1950
  0x0aea6, 0x0ab50, 0x04b60, 0x0aae4, 0x0a570, 0x05260, 0x0f263, 0x0d950, 0x05b57, 0x056a0, // 1960
  0x096d0, 0x04dd5, 0x04ad0, 0x0a4d0, 0x0d4d4, 0x0d250, 0x0d558, 0x0b540, 0x0b6a0, 0x195a6, // 1970
  0x095b0, 0x049b0, 0x0a974, 0x0a4b0, 0x0b27a, 0x06a50, 0x06d40, 0x0af46, 0x0ab60, 0x09570, // 1980
  0x04af5, 0x04970, 0x064b0, 0x074a3, 0x0ea50, 0x06b58, 0x05ac0, 0x0ab60, 0x096d5, 0x092e0, // 1990
  0x0c960, 0x0d954, 0x0d4a0, 0x0da50, 0x07552, 0x056a0, 0x0abb7, 0x025d0, 0x092d0, 0x0cab5, // 2000
  0x0a950, 0x0b4a0, 0x0baa4, 0x0ad50, 0x055d9, 0x04ba0, 0x0a5b0, 0x15176, 0x052b0, 0x0a930, // 2010
  0x07954, 0x06aa0, 0x0ad50, 0x05b52, 0x04b60, 0x0a6e6, 0x0a4e0, 0x0d260, 0x0ea65, 0x0d530, // 2020
  0x05aa0, 0x076a3, 0x096d0, 0x04afb, 0x04ad0, 0x0a4d0, 0x1d0b6, 0x0d250, 0x0d520, 0x0dd45, // 2030
  0x0b5a0, 0x056d0, 0x055b2, 0x049b0, 0x0a577, 0x0a4b0, 0x0aa50, 0x1b255, 0x06d20, 0x0ada0, // 2040
  0x14b63, 0x09370, 0x049f8, 0x04970, 0x064b0, 0x168a6, 0x0ea50, 0x06b20, 0x1a6c4, 0x0aae0, // 2050
  0x092e0, 0x0d2e3, 0x0c960, 0x0d557, 0x0d4a0, 0x0da50, 0x05d55, 0x056a0, 0x0a6d0, 0x055d4, // 2060
  0x052d0, 0x0a9b8, 0x0a950, 0x0b4a0, 0x0b6a6, 0x0ad50, 0x055a0, 0x0aba4, 0x0a5b0, 0x052b0, // 2070
  0x0b273, 0x06930, 0x07337, 0x06aa0, 0x0ad50, 0x14b55, 0x04b60, 0x0a570, 0x054e4, 0x0d160, // 2080
  0x0e968, 0x0d520, 0x0daa0, 0x16aa6, 0x056d0, 0x04ae0, 0x0a9d4, 0x0a2d0, 0x0d150, 0x0f252, // 2090
  0x0d520, // 2100
];
export const LUNAR_FIRST = 1900;
export const LUNAR_LAST = 2100;

export interface LunarDate {
  year: number;
  /** 1–12；闰月也写它跟着的那个月 */
  month: number;
  day: number;
  leap: boolean;
}

/** 公历日期 → 从 1970-01-01 起第几天（不管时区） */
const dayNum = (y: number, m: number, d: number) => Math.round(Date.UTC(y, m - 1, d) / 86400000);
const START = dayNum(1900, 1, 31);

const info = (y: number) => (y >= LUNAR_FIRST && y <= LUNAR_LAST ? INFO[y - LUNAR_FIRST] : 0);

/** 这一年闰几月，0 是没有闰月 */
export const leapMonth = (y: number) => info(y) & 0xf;

/** 这个月有几天；没有这个月（不是闰月却问闰月、超出范围）是 0 */
export function monthDays(y: number, m: number, leap = false): number {
  const v = info(y);
  if (!v || m < 1 || m > 12) return 0;
  if (leap) return (v & 0xf) === m ? (v & 0x10000 ? 30 : 29) : 0;
  return v & (0x10000 >> m) ? 30 : 29;
}

function yearDays(y: number): number {
  let n = 0;
  for (let m = 1; m <= 12; m++) n += monthDays(y, m) + monthDays(y, m, true);
  return n;
}

/** 每年正月初一离 1900 年正月初一几天，最后多一个 2101 年的 */
const YEAR_START: number[] = [0];
for (let y = LUNAR_FIRST; y <= LUNAR_LAST; y++) YEAR_START.push(YEAR_START[YEAR_START.length - 1] + yearDays(y));

/** 公历 → 农历（只看年月日）；超出 1900–2100 返回 null */
export function toLunar(d: Date): LunarDate | null {
  let off = dayNum(d.getFullYear(), d.getMonth() + 1, d.getDate()) - START;
  if (off < 0 || off >= YEAR_START[YEAR_START.length - 1]) return null;
  let i = Math.min(YEAR_START.length - 2, Math.floor(off / 354));
  while (YEAR_START[i] > off) i--;
  while (YEAR_START[i + 1] <= off) i++;
  const y = LUNAR_FIRST + i;
  off -= YEAR_START[i];
  for (let m = 1; m <= 12; m++) {
    for (const leap of [false, true]) {
      const md = monthDays(y, m, leap);
      if (off < md) return { year: y, month: m, day: off + 1, leap };
      off -= md;
    }
  }
  return null;
}

/** 农历 → 公历（当天 0 点，本地时间）；没有这一天返回 null */
export function fromLunar(y: number, m: number, d: number, leap = false): Date | null {
  if (d < 1 || d > monthDays(y, m, leap)) return null;
  let off = YEAR_START[y - LUNAR_FIRST];
  for (let i = 1; i < m; i++) off += monthDays(y, i) + monthDays(y, i, true);
  if (leap) off += monthDays(y, m);
  const t = new Date((START + off + d - 1) * 86400000);
  return new Date(t.getUTCFullYear(), t.getUTCMonth(), t.getUTCDate());
}

export const LUNAR_MONTHS = ["正月", "二月", "三月", "四月", "五月", "六月", "七月", "八月", "九月", "十月", "冬月", "腊月"];
const TEN = ["初", "十", "廿", "三"];
const ONE = ["一", "二", "三", "四", "五", "六", "七", "八", "九", "十"];

/** 初一 … 初十、十一 … 二十、廿一 … 三十 */
export function lunarDayName(d: number): string {
  if (d === 10) return "初十";
  if (d === 20) return "二十";
  if (d === 30) return "三十";
  return TEN[Math.floor(d / 10)] + ONE[(d - 1) % 10];
}

/** "八月十五"、"闰四月初一" */
export const lunarLabel = (m: number, d: number, leap = false) => `${leap ? "闰" : ""}${LUNAR_MONTHS[m - 1] ?? ""}${lunarDayName(d)}`;
