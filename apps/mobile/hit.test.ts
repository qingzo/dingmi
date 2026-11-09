// 今天页认出时间（需求-0.4 第 31 节）：什么时候出「记成提醒」、出几条、存成什么样
import { describe, expect, it } from "vitest";
import { hitToReminder, reminderHits, type HitState } from "./hit";

const now = new Date(2026, 10, 6, 10, 0); // 2026-11-06 周四 10:00
const on: HitState = { unlocked: true, noteMode: false, files: 0 };
const hits = (text: string, st: HitState = on) => reminderHits(text, st, now);

describe("什么时候出", () => {
  it("输过同步密码、发送模式、没带图片文件、认出带时间的提醒", () => {
    const got = hits("明天下午3点和老王对方案");
    expect(got).toHaveLength(1);
    expect(got[0].title).toBe("和老王对方案");
    expect(got[0].due).toBe("2026-11-07 15:00");
    expect(got[0].guess).toBe("");
  });

  it("不出：没输同步密码、存成笔记、带了图片文件、空的", () => {
    const text = "明天下午3点和老王对方案";
    expect(hits(text, { ...on, unlocked: false })).toEqual([]);
    expect(hits(text, { ...on, noteMode: true })).toEqual([]);
    expect(hits(text, { ...on, files: 1 })).toEqual([]);
    expect(hits("   ")).toEqual([]);
  });

  it("不出：没有时间的话（认成笔记）", () => {
    expect(hits("这篇文章写得不错")).toEqual([]);
    expect(hits("https://example.com/a/b")).toEqual([]);
  });
});

describe("出几条", () => {
  it("一句话里几件事，一件一条", () => {
    const got = hits("每天早上8点吃药，下周去体检");
    expect(got.map((h) => h.title)).toEqual(["吃药", "去体检"]);
    expect(got[0].repeat.kind).toBe("daily");
    // 没说几点：时间是先定的，带着说明
    expect(got[1].guess).not.toBe("");
  });
});

describe("存成什么样", () => {
  it("和新建提醒一样的字段；周期的排到下一次；先定的时间标成含糊", () => {
    const [pill, check] = hits("每天早上8点吃药，下周去体检");
    const r = hitToReminder(pill, now);
    expect(r.title).toBe("吃药");
    expect(r.repeat).toEqual({ kind: "daily", days: [], monthday: 0, every: 0, month: 0, nth: 0 });
    expect(r.due_at).toBe("2026-11-07 08:00");
    expect(r.done).toBe(false);
    expect(r.only_on).toEqual([]);
    expect(r.vague).toBe(false);
    expect(r.uid).not.toBe(hitToReminder(pill, now).uid);
    expect(hitToReminder(check, now).vague).toBe(true);
  });
});
