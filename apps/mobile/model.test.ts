// 手机网页的周期提醒（代码在 shared/recur.ts，电脑端界面也用它）：和 recur.rs 的测试一样的几组
import { describe, expect, it } from "vitest";
import { complete, matches, nextAfter, normalize, overLimit, type Repeat as RepeatRule, type SReminder } from "./model";

// 叮咪同步的套餐限单个图片 / 文件多大（需求-1.0 第 32.1 节第 5 条）：图片超了压缩，文件超了传不了，自建服务器不限
describe("套餐限额", () => {
  const l = { max_image: 5 << 20, max_file: 10 << 20 };
  it("图片按图片的、文件按文件的", () => {
    expect(overLimit("image/jpeg", 4 << 20, l)).toBeNull();
    expect(overLimit("image/png", 6 << 20, l)).toBe("image");
    expect(overLimit("application/pdf", 6 << 20, l)).toBeNull();
    expect(overLimit("application/pdf", 11 << 20, l)).toBe("file");
    expect(overLimit("", 11 << 20, l)).toBe("file");
  });
  it("不限的（自建服务器、旧服务器）都放得下", () => {
    const none = { max_image: null, max_file: null };
    expect(overLimit("image/jpeg", 40 << 20, none)).toBeNull();
    expect(overLimit("video/mp4", 40 << 20, none)).toBeNull();
  });
});

const dt = (s: string) => {
  const [d, t] = s.split(" ");
  const [y, m, dd] = d.split("-").map(Number);
  const [h, mi] = t.split(":").map(Number);
  return new Date(y, m - 1, dd, h, mi);
};
const pad = (n: number) => String(n).padStart(2, "0");
const fmt = (d: Date | null) => (d ? `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}` : null);
const rep = (kind: string, p: Partial<RepeatRule> = {}): RepeatRule => ({ kind, days: [], monthday: 0, every: 0, ...p });
const next = (r: RepeatRule, due: string, after: string) => fmt(nextAfter(r, dt(due), dt(after)));
const norm = (r: RepeatRule, due: string, now: string) => fmt(normalize(r, dt(due), dt(now)));

describe("原来的几种", () => {
  it("每天、工作日、每周、每月、每隔几天", () => {
    expect(next(rep("daily"), "2026-10-31 08:30", "2026-10-31 15:00")).toBe("2026-11-01 08:30");
    expect(next(rep("weekdays"), "2026-10-30 17:50", "2026-10-30 18:00")).toBe("2026-11-02 17:50");
    expect(next(rep("weekly", { days: [2, 4, 6] }), "2026-10-31 19:30", "2026-10-31 19:31")).toBe("2026-11-03 19:30");
    expect(next(rep("monthly", { monthday: 31 }), "2026-10-31 09:00", "2026-10-31 10:00")).toBe("2026-11-30 09:00");
    expect(next(rep("interval", { every: 5 }), "2026-10-31 10:00", "2026-10-31 10:00")).toBe("2026-11-05 10:00");
    expect(norm(rep("weekly", { days: [1] }), "2026-11-02 09:00", "2026-10-31 12:00")).toBe("2026-11-02 09:00");
    expect(norm(rep("daily"), "2026-10-31 08:00", "2026-10-31 12:00")).toBe("2026-11-01 08:00");
    expect(next(rep("none"), "2026-10-31 08:00", "2026-10-31 12:00")).toBeNull();
  });
});

describe("0.2 加的", () => {
  it("每年，2 月 29 号平年在 28 号", () => {
    expect(next(rep("yearly", { month: 3, monthday: 5 }), "2026-03-05 09:00", "2026-03-05 10:00")).toBe("2027-03-05 09:00");
    const feb29 = rep("yearly", { month: 2, monthday: 29 });
    expect(next(feb29, "2024-02-29 08:00", "2024-03-01 00:00")).toBe("2025-02-28 08:00");
    expect(next(feb29, "2027-02-28 08:00", "2027-03-01 00:00")).toBe("2028-02-29 08:00");
    expect(next(rep("yearly"), "2026-11-05 09:00", "2026-11-05 10:00")).toBe("2027-11-05 09:00");
  });

  it("隔周：从第一次那周开始数", () => {
    const r = rep("weekly", { days: [5], every: 2 });
    expect(norm(r, "2026-11-05 09:00", "2026-11-05 10:00")).toBe("2026-11-06 09:00");
    expect(next(r, "2026-11-06 09:00", "2026-11-06 09:30")).toBe("2026-11-20 09:00");
    expect(next(r, "2026-11-20 09:00", "2026-11-20 09:30")).toBe("2026-12-04 09:00");
    expect(norm(rep("weekly", { days: [1], every: 2 }), "2026-11-05 09:00", "2026-11-05 10:00")).toBe("2026-11-09 09:00");
    const two = rep("weekly", { days: [1, 3], every: 2 });
    expect(next(two, "2026-11-02 09:00", "2026-11-02 09:30")).toBe("2026-11-04 09:00");
    expect(next(two, "2026-11-04 09:00", "2026-11-04 09:30")).toBe("2026-11-16 09:00");
    expect(next(rep("weekly", { days: [5], every: 1 }), "2026-11-06 09:00", "2026-11-06 09:30")).toBe("2026-11-13 09:00");
  });

  it("每月最后一天", () => {
    const r = rep("month_last");
    expect(norm(r, "2026-11-05 09:00", "2026-11-05 10:00")).toBe("2026-11-30 09:00");
    expect(next(r, "2026-11-30 09:00", "2026-11-30 10:00")).toBe("2026-12-31 09:00");
    expect(next(r, "2027-01-31 09:00", "2027-01-31 10:00")).toBe("2027-02-28 09:00");
    expect(next(r, "2028-01-31 09:00", "2028-01-31 10:00")).toBe("2028-02-29 09:00");
  });

  it("每月第几个周几", () => {
    const first = rep("month_nth", { nth: 1, days: [1] });
    expect(norm(first, "2026-11-05 09:00", "2026-11-05 10:00")).toBe("2026-12-07 09:00");
    expect(next(first, "2026-12-07 09:00", "2026-12-07 10:00")).toBe("2027-01-04 09:00");
    const last = rep("month_nth", { nth: -1, days: [5] });
    expect(norm(last, "2026-11-05 09:00", "2026-11-05 10:00")).toBe("2026-11-27 09:00");
    expect(next(last, "2026-11-27 09:00", "2026-11-27 10:00")).toBe("2026-12-25 09:00");
    expect(norm(rep("month_nth", { nth: 5, days: [4] }), "2026-11-05 09:00", "2026-11-05 10:00")).toBe("2026-12-31 09:00");
    expect(next(rep("month_nth"), "2026-11-05 09:00", "2026-11-05 10:00")).toBe("2026-12-03 09:00");
  });

  it("每年农历：闰月不响，没有三十就在二十九", () => {
    const mid = rep("lunar_yearly", { month: 8, monthday: 15 });
    expect(norm(mid, "2026-08-01 20:00", "2026-08-01 10:00")).toBe("2026-09-25 20:00");
    expect(next(mid, "2026-09-25 20:00", "2026-09-25 21:00")).toBe("2027-09-15 20:00");
    expect(next(rep("lunar_yearly", { month: 6, monthday: 1 }), "2025-06-25 09:00", "2025-06-25 10:00")).toBe("2026-07-14 09:00");
    const eve = rep("lunar_yearly", { month: 12, monthday: 30 });
    expect(next(eve, "2024-02-09 09:00", "2024-02-09 10:00")).toBe("2025-01-28 09:00");
    expect(next(eve, "2025-01-28 09:00", "2025-01-28 10:00")).toBe("2026-02-16 09:00");
    expect(next(rep("lunar_yearly"), "2026-11-05 09:00", "2026-11-05 10:00")).toBe("2027-10-26 09:00");
    expect(matches(mid, dt("2026-09-25 00:00"), dt("2026-09-25 00:00"))).toBe(true);
  });

  it("旧记录没有 month、nth", () => {
    const old = JSON.parse('{"kind":"weekly","days":[5],"monthday":0,"every":0}') as RepeatRule;
    expect(next(old, "2026-11-06 09:00", "2026-11-06 10:00")).toBe("2026-11-13 09:00");
  });

  it("手机上完成一条每年农历的提醒，排到明年", () => {
    const r = { due_at: "2026-09-25 20:00", repeat: rep("lunar_yearly", { month: 8, monthday: 15 }), snoozed_until: null } as unknown as SReminder;
    expect(complete(r, dt("2026-09-25 20:05")).due_at).toBe("2027-09-15 20:00");
  });
});
