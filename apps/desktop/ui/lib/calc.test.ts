import { describe, expect, it } from "vitest";
import { calc } from "./calc";

const v = (s: string) => calc(s)?.value;

describe("随手算", () => {
  it("四则运算、括号、小数", () => {
    expect(v("128*3.5")).toBe("448");
    expect(v("1+2*3")).toBe("7");
    expect(v("(1+2)/3")).toBe("1");
    expect(v("10-2-3")).toBe("5");
    expect(v(".5+.25")).toBe("0.75");
    expect(v("-3+5")).toBe("2");
    expect(v("2*-3")).toBe("-6");
  });

  it("× ÷、全角、末尾的 =、空格", () => {
    expect(v("6×7")).toBe("42");
    expect(v("8÷4")).toBe("2");
    expect(v("３＋４")).toBe("7");
    expect(v("3＋4")).toBe("7");
    expect(v("（1+2）＊3")).toBe("9");
    expect(v("1+1=")).toBe("2");
    expect(v(" 2 * 3 = ")).toBe("6");
  });

  it("百分号和乘方", () => {
    expect(v("200*15%")).toBe("30");
    expect(v("50%")).toBe("0.5");
    expect(v("2^10")).toBe("1024");
    expect(v("2^3^2")).toBe("512");
    expect(v("-2^2")).toBe("-4");
  });

  it("最多 10 位有效数字，去掉末尾的 0，大数加千分位", () => {
    expect(v("1/3")).toBe("0.3333333333");
    expect(v("0.1+0.2")).toBe("0.3");
    expect(calc("1234567*1000")).toMatchObject({ value: "1234567000", shown: "1,234,567,000" });
    expect(calc("12345.5*2")?.shown).toBe("24,691");
  });

  it("除以 0 算不了", () => {
    expect(calc("1/0")).toMatchObject({ value: null });
    expect(calc("0/0")).toMatchObject({ value: null });
  });

  it("不是算式的不算", () => {
    for (const s of ["448", "-5", "(5)", "3 点开会", "wx", "1+", "(1+2", "1++", "2026年", "abc+1", "", "+", "%"]) expect(calc(s), s).toBeNull();
  });

  it("显示的算式", () => {
    expect(calc("128*3.5")?.expr).toBe("128 × 3.5");
    expect(calc("10-2/4")?.expr).toBe("10 - 2 ÷ 4");
    expect(calc("2*-3")?.expr).toBe("2 × -3");
  });
});
