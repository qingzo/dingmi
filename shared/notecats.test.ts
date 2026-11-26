// 笔记文件夹（需求-1.x 第 45.3 节）：笔记分类多记一个上一层，按树排、算层数和路径
import { describe, expect, it } from "vitest";
import { noteCatAll, noteCatDepth, noteCatKids, noteCatList, noteCatPath, noteCatUp, setNoteCats } from "./util";

const f = (id: string, name: string, parent?: string) => ({ id, name, color: "blue", ...(parent ? { parent } : {}) });

describe("笔记文件夹", () => {
  it("按树排：每个文件夹后面紧跟它下面的", () => {
    setNoteCats([f("rep", "月报", "cs"), f("work", "工作"), f("ref", "资料"), f("cs", "长寿医院", "work"), f("soft", "软件", "ref")]);
    expect(noteCatList().map((t) => t.id)).toEqual(["work", "cs", "rep", "ref", "soft"]);
    expect(noteCatKids(null).map((t) => t.id)).toEqual(["work", "ref"]);
    expect(noteCatKids("work").map((t) => t.id)).toEqual(["cs"]);
  });

  it("层数、路径、连下面几层", () => {
    setNoteCats([f("work", "工作"), f("cs", "长寿医院", "work"), f("rep", "月报", "cs")]);
    expect([noteCatDepth("work"), noteCatDepth("cs"), noteCatDepth("rep"), noteCatDepth(null)]).toEqual([0, 1, 2, 0]);
    expect(noteCatPath("rep")).toBe("工作 / 长寿医院 / 月报");
    expect(noteCatPath("cs", "/")).toBe("工作/长寿医院");
    expect(noteCatPath("gone")).toBe("");
    expect(noteCatAll("work")).toEqual(["work", "cs", "rep"]);
    expect(noteCatUp("cs")?.id).toBe("work");
  });

  it("上一层被删了的当最外层；旧版本存的没有上一层", () => {
    setNoteCats([f("x", "孤儿", "nope"), { id: "ref", name: "资料", color: "teal" }]);
    expect(noteCatKids(null).map((t) => t.id)).toEqual(["x", "ref"]);
    expect([noteCatDepth("x"), noteCatPath("x")]).toEqual([0, "孤儿"]);
  });

  it("绕成圈的（不该有）也不会卡住", () => {
    setNoteCats([f("a", "甲", "b"), f("b", "乙", "a")]);
    expect(noteCatList()).toHaveLength(2);
    expect(noteCatDepth("a")).toBeLessThanOrEqual(3);
    expect(noteCatPath("a").split(" / ").length).toBeLessThanOrEqual(3);
    expect(noteCatAll("a").length).toBeLessThanOrEqual(4);
  });
});
