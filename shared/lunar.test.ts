import { describe, expect, it } from "vitest";
import cases from "./lunar.cases.json";
import { fromLunar, leapMonth, lunarDayName, lunarLabel, monthDays, toLunar } from "./lunar";

const ymd = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;

describe("农历（和 lunar.rs 用同一份测试日期）", () => {
  it("测试日期够多", () => expect(cases.cases.length).toBeGreaterThanOrEqual(20));
  for (const [s, y, m, d, leap] of cases.cases as [string, number, number, number, boolean][]) {
    it(s, () => {
      const [yy, mm, dd] = s.split("-").map(Number);
      expect(toLunar(new Date(yy, mm - 1, dd))).toEqual({ year: y, month: m, day: d, leap });
      expect(ymd(fromLunar(y, m, d, leap)!)).toBe(s);
    });
  }
  it("范围外、没有的日子", () => {
    expect(toLunar(new Date(1900, 0, 30))).toBeNull();
    expect(toLunar(new Date(2101, 1, 1))).toBeNull();
    expect(fromLunar(2025, 5, 1, true)).toBeNull();
    expect(fromLunar(2025, 12, 30)).toBeNull();
    expect(leapMonth(2025)).toBe(6);
    expect(monthDays(2025, 6, true)).toBe(29);
  });
  it("每天往后一天，农历也连着往后", () => {
    let prev = toLunar(new Date(2019, 0, 1))!;
    for (let i = 1; i < 3000; i++) {
      const cur = toLunar(new Date(2019, 0, 1 + i))!;
      if (cur.day !== 1) expect(cur.day).toBe(prev.day + 1);
      else expect(prev.day).toBe(monthDays(prev.year, prev.month, prev.leap));
      prev = cur;
    }
  });
  it("写法", () => {
    expect(lunarLabel(8, 15)).toBe("八月十五");
    expect(lunarLabel(1, 3)).toBe("正月初三");
    expect(lunarLabel(4, 1, true)).toBe("闰四月初一");
    expect([1, 10, 11, 19, 20, 21, 29, 30].map(lunarDayName)).toEqual(["初一", "初十", "十一", "十九", "二十", "廿一", "廿九", "三十"]);
  });
});
