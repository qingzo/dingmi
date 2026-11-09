// 快捷框找程序：拼音、首字母对上时标出哪几个字
import { describe, expect, it } from "vitest";
import type { LaunchItem } from "./api";
import { EMPTY_STATS, markHit, score } from "./launch";

const app = (name: string, pys: string[][]) => ({ name, pys });
const wx = app("微信", [["wei"], ["xin"]]);
const qywx = app("企业微信", [["qi"], ["ye"], ["wei"], ["xin"]]);
const vsc = app("Visual Studio Code", [["visual"], ["studio"], ["code"]]);
const qq = app("QQ音乐", [["qq"], ["yin"], ["yue", "le"]]);
// pys 照 launcher.rs pinyin()：小写后面跟大写分开，「ToDesk」→ [to] [desk]
const todesk = app("ToDesk", [["to"], ["desk"]]);

describe("markHit", () => {
  it("名字里有这几个字", () => expect(markHit(vsc, "studio")).toBe("Visual <mark>Studio</mark> Code"));
  it("拼音首字母：两个字都标", () => expect(markHit(wx, "wx")).toBe("<mark>微信</mark>"));
  it("全拼、半拼", () => {
    expect(markHit(wx, "weixin")).toBe("<mark>微信</mark>");
    expect(markHit(wx, "wei")).toBe("<mark>微</mark>信");
  });
  it("从中间对上", () => expect(markHit(qywx, "wx")).toBe("企业<mark>微信</mark>"));
  it("英文首字母只标打了的字母", () => expect(markHit(vsc, "vsc")).toBe("<mark>V</mark>isual <mark>S</mark>tudio <mark>C</mark>ode"));
  it("英文和汉字混着", () => expect(markHit(qq, "qqyy")).toBe("<mark>QQ音乐</mark>"));
  it("多音字", () => expect(markHit(qq, "yl")).toBe("QQ<mark>音乐</mark>"));
  it("对不上就原样（转义）", () => expect(markHit(app("A<B", [["a"], ["b"]]), "zz")).toBe("A&lt;B"));
  it("大小写分开的单词（ToDesk）", () => {
    expect(markHit(todesk, "td")).toBe("<mark>T</mark>o<mark>D</mark>esk");
    expect(markHit(todesk, "tdesk")).toBe("<mark>T</mark>o<mark>Desk</mark>");
  });
});

const item = (name: string, pys: string[][]) => ({ id: `app:${name}`, name, pys, kind: "app", path: "", icon: "", junk: false, missing: false, runs: 0 }) as LaunchItem;

describe("score", () => {
  const td = item("ToDesk", todesk.pys);
  it("td 是首字母", () => expect(score("td", td, [], EMPTY_STATS)).toMatchObject({ score: 96, why: "首字母" }));
  it("从头打、跳着打都行", () => {
    for (const q of ["to", "tod", "todesk", "tdesk"]) expect(score(q, td, [], EMPTY_STATS)).not.toBeNull();
  });
  it("从后一个词开始", () => expect(score("desk", td, [], EMPTY_STATS)?.score).toBe(68));
  it("对不上", () => expect(score("tx", td, [], EMPTY_STATS)).toBeNull());
});
