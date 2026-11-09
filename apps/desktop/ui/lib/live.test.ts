// 实时预览：哪些符号什么时候藏、什么时候显示（live.ts）
import { describe, expect, it } from "vitest";
import { inlineSpans, RAW, scan, slashItems, slashQuery, type Span } from "./live";
import { whenShort } from "../text/notes";

const none = () => false;
const all = () => true;
/** 只看某几种，按位置排好，方便对 */
const pick = (spans: Span[], ...k: Span["k"][]) => spans.filter((s) => k.includes(s.k)).sort((a, b) => ("from" in a ? a.from : a.at) - ("from" in b ? b.from : b.at));
/** 藏起来以后看到的字 */
function shown(doc: string, spans: Span[]): string {
  const cut = spans.filter((s): s is Extract<Span, { k: "hide" | "swap" | "block" }> => s.k === "hide" || s.k === "swap" || s.k === "block").sort((a, b) => b.from - a.from);
  let out = doc;
  for (const s of cut) out = out.slice(0, s.from) + (s.k === "hide" ? "" : `[${s.w.t}]`) + out.slice(s.to);
  return out;
}

describe("行内", () => {
  it("粗体、删除线、代码：光标不在时藏符号", () => {
    const text = "**粗** ~~删~~ `a**b**`";
    expect(shown(text, inlineSpans(text, 0, false))).toBe("粗 删 a**b**");
  });
  it("光标在这一段时符号显示成灰色等宽，不藏", () => {
    const s = inlineSpans("客户要的**接口文档**", 0, true);
    expect(s.some((x) => x.k === "hide")).toBe(false);
    expect(pick(s, "mark").map((x) => [x.k === "mark" && x.cls, "from" in x && x.from])).toEqual([
      [RAW, 4],
      ["lp-b", 6],
      [RAW, 10],
    ]);
  });
  it("代码里的 ** 不当粗体", () => {
    const s = inlineSpans("`**x**`", 0, false);
    expect(s.filter((x) => x.k === "mark").map((x) => x.k === "mark" && x.cls)).toEqual(["lp-code"]);
  });
  it("[文字](网址)：只留文字，点了打开网址", () => {
    const text = "参考 [会议纪要](https://a.com/x)。";
    const s = inlineSpans(text, 10, false);
    expect(shown("x".repeat(10) + text, s).slice(10)).toBe("参考 会议纪要。");
    const link = s.find((x) => x.k === "mark" && x.cls === "lp-link");
    expect(link).toMatchObject({ from: 14, to: 18, href: "https://a.com/x" });
  });
  it("光秃秃的网址换成网站名，光标在时显示原样", () => {
    const text = "地址 https://github.com/a/b，看看";
    expect(pick(inlineSpans(text, 0, false), "swap")).toEqual([{ k: "swap", from: 3, to: 25, w: { t: "host", url: "https://github.com/a/b" } }]);
    expect(pick(inlineSpans(text, 0, true), "swap")).toEqual([]);
  });
  it("网址里的 ** 不当粗体", () => {
    expect(inlineSpans("https://a.com/**x**", 0, false).filter((x) => x.k === "hide")).toEqual([]);
  });
});

describe("整行", () => {
  it("标题：藏 #，光标在时显示", () => {
    const doc = "## 联系人\n正文";
    expect(shown(doc, scan(doc, none))).toBe("联系人\n正文");
    expect(pick(scan(doc, none), "line")).toEqual([{ k: "line", at: 0, cls: "lp-h2" }]);
    expect(shown(doc, scan(doc, (i) => i === 0))).toBe(doc);
  });
  it("#### 不算标题（和 renderMd 一样）", () => {
    expect(pick(scan("#### 四级", none), "line")).toEqual([]);
  });
  it("待办：- [ ] 换成勾选框，后面能设提醒；勾掉的划掉", () => {
    const doc = "- [ ] 交付时间\n- [x] 报价单";
    const s = scan(doc, none);
    expect(shown(doc, s)).toBe("[check]交付时间\n[check]报价单");
    expect(pick(s, "swap").map((x) => x.k === "swap" && x.w)).toEqual([
      { t: "check", done: false },
      { t: "check", done: true },
    ]);
    expect(pick(s, "todo")).toEqual([
      { k: "todo", at: 10, text: "交付时间", done: false },
      { k: "todo", at: 20, text: "报价单", done: true },
    ]);
    expect(pick(s, "line").map((x) => x.k === "line" && x.cls)).toEqual(["lp-todo", "lp-todo lp-done"]);
  });
  it("光标所在的待办显示 - [ ] 原样（灰色等宽）", () => {
    const s = scan("- [ ] 客户要的**接口文档**", all);
    expect(pick(s, "swap", "hide")).toEqual([]);
    expect(pick(s, "mark")[0]).toEqual({ k: "mark", from: 0, to: 6, cls: RAW });
  });
  it("缩进的待办、空待办", () => {
    expect(shown("  - [ ] 子项", scan("  - [ ] 子项", none))).toBe("  [check]子项");
    expect(pick(scan("- [ ]", none), "swap")).toHaveLength(1);
  });
  it("列表换成圆点，编号不藏", () => {
    const doc = "- 一\n* 二\n1. 三\n2、四";
    expect(shown(doc, scan(doc, none))).toBe("[bullet]一\n[bullet]二\n1. 三\n2、四");
    expect(pick(scan(doc, none), "line").map((x) => x.k === "line" && x.cls)).toEqual(["lp-li", "lp-li", "lp-ol", "lp-ol"]);
  });
  it("3.14 不是编号列表", () => {
    expect(pick(scan("3.14 是圆周率", none), "line")).toEqual([]);
  });
  it("引用：藏 >", () => {
    expect(shown("> 客户原话", scan("> 客户原话", none))).toBe("客户原话");
  });
  it("分隔线", () => {
    expect(shown("上\n---\n下", scan("上\n---\n下", none))).toBe("上\n[hr]\n下");
    expect(shown("上\n---\n下", scan("上\n---\n下", (i) => i === 1))).toBe("上\n---\n下");
  });
  it("只有光标所在的那一行显示符号", () => {
    const doc = "**一**\n**二**\n**三**";
    expect(shown(doc, scan(doc, (i) => i === 1))).toBe("一\n**二**\n三");
  });
});

describe("图片和文件", () => {
  it("单独一行的图片、文件换成一块", () => {
    const doc = "现场照片：\n![](att:a1.png)\n[合同.pdf](att:b2.pdf)";
    const s = scan(doc, none);
    expect(pick(s, "block").map((x) => x.k === "block" && x.w)).toEqual([
      { t: "image", file: "a1.png", alt: "" },
      { t: "file", file: "b2.pdf", name: "合同.pdf" },
    ]);
  });
  it("光标在图片那一行：显示源码，图片照样在下面", () => {
    const doc = "![图](att:a1.png)";
    const s = scan(doc, all);
    expect(pick(s, "block")).toEqual([]);
    expect(pick(s, "after")).toEqual([{ k: "after", at: doc.length, w: { t: "image", file: "a1.png", alt: "图" } }]);
  });
  it("句子中间的不算", () => {
    expect(pick(scan("看 ![](att:a1.png) 这个", none), "block")).toEqual([]);
  });
});

describe("表格和代码块", () => {
  const table = "| 名字 | 电话 |\n| --- | --- |\n| 老王 | 138 |";
  it("光标不在表格里：整张换成一块，编号和 renderMd 一样", () => {
    const doc = `${table}\n\n${table}`;
    const s = pick(scan(doc, none), "block");
    expect(s.map((x) => x.k === "block" && x.w.t === "table" && [x.from, x.to, x.w.n])).toEqual([
      [0, table.length, 0],
      [table.length + 2, doc.length, 1],
    ]);
  });
  it("光标在表格任何一行：整张显示源码", () => {
    const s = scan(table, (i) => i === 2);
    expect(pick(s, "block")).toEqual([]);
    expect(pick(s, "line").map((x) => x.k === "line" && x.cls)).toEqual(["lp-tablerow", "lp-tablerow", "lp-tablerow"]);
  });
  it("代码块里原样显示，光标不在时藏 ```", () => {
    const doc = "```\n**不是粗体** https://a.com\n```\n后面";
    const s = scan(doc, none);
    expect(shown(doc, s)).toBe("\n**不是粗体** https://a.com\n\n后面");
    expect(pick(s, "line").map((x) => x.k === "line" && x.cls)).toEqual(["lp-codeblock lp-codefirst", "lp-codeblock", "lp-codeblock lp-codelast"]);
    expect(shown(doc, scan(doc, (i) => i === 1))).toBe(doc);
  });
  it("没写完的代码块：只藏开头", () => {
    const doc = "```js\nlet a";
    expect(shown(doc, scan(doc, none))).toBe("\nlet a");
  });
});

describe("插入菜单", () => {
  it("行首打 / 才算", () => {
    expect(slashQuery("/")).toBe("");
    expect(slashQuery("  /表")).toBe("表");
    expect(slashQuery("、表")).toBe("表");
    expect(slashQuery("一、")).toBe(null);
    expect(slashQuery("文字/")).toBe(null);
    expect(slashQuery("/a b")).toBe(null);
    expect(slashQuery("https://")).toBe(null);
  });
  it("按名字、写法、拼音筛", () => {
    expect(slashItems("")).toHaveLength(7);
    expect(slashItems("表")).toEqual(["table"]);
    expect(slashItems("db")).toEqual(["todo"]);
    expect(slashItems("tp")).toEqual(["image"]);
    expect(slashItems("---")).toEqual(["hr"]);
    expect(slashItems("没有这个")).toEqual([]);
  });
});

describe("待办后面的提醒时间（U13 时间写法）", () => {
  // 2026-11-05 是周四
  const now = new Date(2026, 10, 5, 10, 0);
  it("今天、明天、这周、更远、跨年", () => {
    expect(whenShort("2026-11-05 14:00", now)).toBe("14:00");
    expect(whenShort("2026-11-06 08:30", now)).toBe("明天 08:30");
    expect(whenShort("2026-11-08 15:00", now)).toBe("周日 15:00");
    expect(whenShort("2026-11-03 15:00", now)).toBe("周二 15:00");
    expect(whenShort("2026-11-12 09:00", now)).toBe("11 月 12 日");
    expect(whenShort("2027-01-02 09:00", now)).toBe("2027 年 1 月 2 日");
  });
});
