// 提醒的子项存在备注里（需求-1.x 第 42.1 节）
import { describe, expect, it } from "vitest";
import { detailText, joinSubs, resetSubs, splitSubs, subsLabel } from "./md";

describe("子项", () => {
  const d = "带上身份证\n- [x] 订票\n- [ ] 收拾行李\n- [ ] 叫车";

  it("拆开：别的文字和子项", () => {
    expect(splitSubs(d)).toEqual({
      text: "带上身份证",
      subs: [
        { text: "订票", done: true },
        { text: "收拾行李", done: false },
        { text: "叫车", done: false },
      ],
    });
    expect(splitSubs("")).toEqual({ text: "", subs: [] });
  });

  it("拼回去：文字在前，空着的子项不要", () => {
    const { text, subs } = splitSubs(d);
    expect(joinSubs(text, subs)).toBe(d);
    expect(joinSubs("  ", [{ text: "订票", done: false }, { text: " ", done: true }])).toBe("- [ ] 订票");
    expect(joinSubs("只有文字\n", [])).toBe("只有文字");
  });

  it("一行显示和勾了几个", () => {
    expect(detailText(d)).toBe("带上身份证");
    expect(detailText("第一行\n第二行")).toBe("第一行 第二行");
    expect(subsLabel(d)).toBe("1/3");
    expect(subsLabel("没有子项")).toBe("");
  });

  it("重复的完成一次：都变回没勾", () => {
    expect(resetSubs(d)).toBe("带上身份证\n- [ ] 订票\n- [ ] 收拾行李\n- [ ] 叫车");
    expect(resetSubs("[x] 不是子项")).toBe("[x] 不是子项");
  });
});
