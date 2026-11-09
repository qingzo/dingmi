// 本地规则逐句核对 parse.cases.ts（需求-0.2 8.6：必须全部通过）
import { beforeAll, describe, expect, it } from "vitest";
import { setNoteCats, setTags, repeatLabel } from "@shared/util";
import { CASES } from "./parse.cases";
import { fixLunar, normalizeAI, parseLocal, type ParsedReminder } from "./parse";
import type { Device } from "./api";

const NOW = "2026-11-05 10:00";
const dt = (s: string) => {
  const [d, t] = s.split(" ");
  const [y, m, dd] = d.split("-").map(Number);
  const [h, mi] = t.split(":").map(Number);
  return new Date(y, m - 1, dd, h, mi);
};

beforeAll(() => {
  setTags([]);
  setNoteCats([]);
});

describe("本地规则：测试句子", () => {
  it("不少于 300 句", () => expect(CASES.length).toBeGreaterThanOrEqual(300));
  for (const c of CASES) {
    const now = c.now ?? NOW;
    it(`${c.say}${c.now ? `（${c.now}）` : ""}`, () => {
      const devices: Device[] = (c.devices ?? []).map(([uid, name]) => ({ uid, name, me: false }));
      const items = parseLocal(c.say, dt(now), devices);
      const rems = items.filter((i): i is ParsedReminder => i.type === "reminder");
      const got = rems.map((x) => ({
        title: x.title,
        due: x.due,
        kind: x.repeat.kind,
        urgent: x.urgent,
        guess: !!x.guess,
      }));
      const want = c.want.map((w) => ({
        title: w.title,
        due: w.due.length === 11 ? `${now.slice(0, 4)}-${w.due}` : w.due,
        kind: w.repeat?.kind ?? "none",
        urgent: w.urgent ?? false,
        guess: !!w.guess,
      }));
      expect(got).toEqual(want);
      expect(items.length - rems.length).toBe(c.notes ?? 0);
      c.want.forEach((w, i) => {
        const x = rems[i];
        if (w.repeat) for (const [k, v] of Object.entries(w.repeat)) expect(x.repeat[k as keyof typeof x.repeat], k).toEqual(v);
        if (w.detail) expect(x.detail).toContain(w.detail);
        if (typeof w.guess === "string") expect(x.guess).toBe(w.guess);
      });
      if (c.only) for (const x of rems) expect(x.only_on).toEqual(c.only);
    });
  }
});

describe("别的", () => {
  it("分类：还信用卡是周期", () => {
    setTags([
      { id: "work", name: "工作", color: "blue" },
      { id: "cycle", name: "周期", color: "purple" },
    ]);
    const [a] = parseLocal("每月15号还信用卡", dt(NOW)) as ParsedReminder[];
    const [b] = parseLocal("明天下午3点开会", dt(NOW)) as ParsedReminder[];
    setTags([]);
    expect([a.tag, b.tag]).toEqual(["cycle", "work"]);
  });

  it("分类：吃药是生活，吃火锅是喜好", () => {
    setTags([
      { id: "life", name: "生活", color: "orange" },
      { id: "like", name: "喜好", color: "pink" },
    ]);
    const [a] = parseLocal("每天中午12点半吃药", dt(NOW)) as ParsedReminder[];
    const [b] = parseLocal("周六晚上6点吃火锅", dt(NOW)) as ParsedReminder[];
    setTags([]);
    expect([a.tag, b.tag]).toEqual(["life", "like"]);
  });

  it("分类：有「健康」时吃药、跑步、体检算健康，吃火锅还是喜好", () => {
    setTags([
      { id: "life", name: "生活", color: "orange" },
      { id: "health", name: "健康", color: "green" },
      { id: "like", name: "喜好", color: "pink" },
    ]);
    const tags = ["每天中午12点半吃药", "明早7点跑步", "下周三上午去医院体检", "周六晚上6点吃火锅", "明天下午取快递"].map(
      (s) => (parseLocal(s, dt(NOW)) as ParsedReminder[])[0].tag,
    );
    setTags([]);
    expect(tags).toEqual(["health", "health", "health", "like", "life"]);
  });

  it("每年农历：卡片上同时写农历和公历", () => {
    const [x] = parseLocal("每年农历八月十五回家", dt(NOW)) as ParsedReminder[];
    expect(repeatLabel(x.repeat, x.due)).toBe("每年农历八月十五 · 9月15日");
  });

  it("新周期的写法", () => {
    const due = "2026-11-06 09:00";
    const label = (r: object) => repeatLabel({ kind: "none", days: [], monthday: 0, every: 0, ...r } as never, due);
    expect(label({ kind: "weekly", days: [5], every: 2 })).toBe("每两周的周五");
    expect(label({ kind: "weekly", days: [1, 3], every: 3 })).toBe("每 3 周的周一、三");
    expect(label({ kind: "yearly", month: 3, monthday: 5 })).toBe("每年 3 月 5 日");
    expect(label({ kind: "month_last" })).toBe("每月最后一天");
    expect(label({ kind: "month_nth", nth: -1, days: [5] })).toBe("每月最后一个周五");
    expect(label({ kind: "month_nth", nth: 1, days: [1] })).toBe("每月第一个周一");
    expect(label({ kind: "lunar_yearly", month: 1, monthday: 3 })).toBe("每年农历正月初三 · 11月6日");
  });

  it("AI 返回的新周期类型和字段都认", () => {
    const items = normalizeAI(
      {
        items: [
          { type: "reminder", title: "回家", due: "2027-09-15 09:00", repeat: { kind: "lunar_yearly", month: 8, monthday: 15 } },
          { type: "reminder", title: "团建", due: "2026-11-27 09:00", repeat: { kind: "month_nth", nth: -1, days: [5] }, vague: false },
          { type: "reminder", title: "聚聚", due: "2026-11-06 09:00", repeat: { kind: "weird", nth: 9, month: 13 }, vague: true },
        ],
      },
      [],
      dt(NOW),
    ) as ParsedReminder[];
    expect(items.map((i) => [i.repeat.kind, i.repeat.month, i.repeat.nth, i.guess])).toEqual([
      ["lunar_yearly", 8, 0, ""],
      ["month_nth", 0, -1, ""],
      ["none", 0, 0, "没听清具体时间，先定在明天 9:00"],
    ]);
  });

  it("AI 给的周期提醒，第一次自己算", () => {
    const rem = (due: string, repeat: object) => ({ type: "reminder", title: "x", due, repeat });
    const items = normalizeAI(
      {
        items: [
          // 农历九月廿七就是今天，AI 算成了明天
          rem("2026-11-06 20:00", { kind: "lunar_yearly", month: 9, monthday: 27 }),
          // 11 月没有 31 号，AI 跳到了 12 月
          rem("2026-12-31 09:00", { kind: "monthly", monthday: 31 }),
          // 今天的已经过了
          rem("2026-11-05 09:00", { kind: "daily" }),
          // 从下周一开始每天：照 AI 给的开始
          rem("2026-11-09 07:00", { kind: "daily" }),
          // 每年的今天，AI 没填几月几号
          rem("2026-11-05 20:00", { kind: "yearly" }),
        ],
      },
      [],
      dt(NOW),
    ) as ParsedReminder[];
    expect(items.map((i) => i.due)).toEqual(["2026-11-05 20:00", "2026-11-30 09:00", "2026-11-06 09:00", "2026-11-09 07:00", "2026-11-05 20:00"]);
    expect([items[4].repeat.month, items[4].repeat.monthday]).toEqual([11, 5]);
  });

  it("说了农历的，日子和周期用本地规则算的", () => {
    const ai = (due: string, repeat: object) => normalizeAI({ items: [{ type: "reminder", title: "x", due, repeat }] }, [], dt(NOW));
    // AI 差了一天、把一次的当成了每年
    const [a] = fixLunar(ai("2026-11-24 09:00", { kind: "none" }), "农历十月十五去庙里", dt(NOW)) as ParsedReminder[];
    const [b] = fixLunar(ai("2027-09-15 09:00", { kind: "lunar_yearly", month: 8, monthday: 15 }), "农历八月十五", dt(NOW)) as ParsedReminder[];
    expect([a.due, b.due, b.repeat.kind]).toEqual(["2026-11-23 09:00", "2027-09-15 09:00", "none"]);
    // 没说农历的不动
    const [c] = fixLunar(ai("2026-11-24 09:00", { kind: "none" }), "24号去庙里", dt(NOW)) as ParsedReminder[];
    expect(c.due).toBe("2026-11-24 09:00");
  });

  it("闪念输入接着补时间，预览马上变", () => {
    const a = parseLocal("改天请老王吃饭", dt(NOW))[0] as ParsedReminder;
    const b = parseLocal("改天请老王吃饭周三", dt(NOW))[0] as ParsedReminder;
    expect([a.guess !== "", b.guess]).toEqual([true, ""]);
    expect(b.due).toBe("2026-11-11 09:00");
  });
});
