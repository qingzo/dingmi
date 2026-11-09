import { describe, expect, it } from "vitest";
import { spaceOpens, type SpaceCtx } from "./flashspace";

/** 默认：开着、打了 wx、选中的是排第一的微信 */
const ctx = (o: Partial<SpaceCtx> = {}): SpaceCtx => ({ on: true, composing: false, value: "wx", app: true, picked: false, top: true, webKey: false, ...o });

describe("空格启动（需求-1.x 第 37 节）", () => {
  it("程序对得很准排第一、还没打空格：空格打开", () => {
    expect(spaceOpens(ctx())).toBe(true);
  });

  it("没开开关：空格就是空格", () => {
    expect(spaceOpens(ctx({ on: false }))).toBe(false);
    expect(spaceOpens(ctx({ on: false, picked: true }))).toBe(false);
  });

  it("输入法正在选字：空格是给输入法的", () => {
    expect(spaceOpens(ctx({ composing: true }))).toBe(false);
    expect(spaceOpens(ctx({ composing: true, picked: true }))).toBe(false);
  });

  it("用方向键选中的程序：不管排第几、打没打过空格都打开", () => {
    expect(spaceOpens(ctx({ top: false, picked: true }))).toBe(true);
    expect(spaceOpens(ctx({ top: false, picked: true, value: "visual studio" }))).toBe(true);
    // 选中的是「记下来」、笔记这些不是程序的：空格照常
    expect(spaceOpens(ctx({ app: false, picked: true }))).toBe(false);
  });

  it("排第一的程序，输入里已经有空格了：照常打空格（可能在写一句话）", () => {
    expect(spaceOpens(ctx({ value: "visual studio" }))).toBe(false);
    expect(spaceOpens(ctx({ value: "wx " }))).toBe(false);
    expect(spaceOpens(ctx({ value: "wx　" }))).toBe(false);
  });

  it("不是「像程序名」排第一的（像一句话、笔记在前、鼠标停在别的程序上）：照常打空格", () => {
    expect(spaceOpens(ctx({ top: false }))).toBe(false);
    expect(spaceOpens(ctx({ app: false, top: false }))).toBe(false);
  });

  it("再打一个空格就是网页关键字（bd 天气）：空格留给关键字，方向键选中的照样打开", () => {
    expect(spaceOpens(ctx({ value: "bd", webKey: true }))).toBe(false);
    expect(spaceOpens(ctx({ value: "bd", webKey: true, picked: true }))).toBe(true);
  });

  it("空着时不归这里管", () => {
    expect(spaceOpens(ctx({ value: "" }))).toBe(false);
    expect(spaceOpens(ctx({ value: "  ", picked: true }))).toBe(false);
  });
});
