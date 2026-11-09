// 抽屉里「什么时候」一开始写的话，本地规则要能原样认回同一个时间
import { describe, expect, it } from "vitest";
import { parseLocal } from "./parse";
import { whenText } from "./when";

const now = new Date(2026, 10, 5, 10, 30);

describe("whenText 和 parseLocal 对得上", () => {
  for (const due of ["2026-11-05 14:00", "2026-11-06 09:30", "2026-11-12 08:00", "2026-12-31 23:45", "2027-01-03 08:00", "2027-11-01 18:00"]) {
    it(due, () => {
      const text = whenText(due, now);
      const it = parseLocal(text, now).find((x) => x.type === "reminder");
      expect(it?.type === "reminder" ? it.due : null, text).toBe(due);
    });
  }
});
