// 实时预览（UI 设计 U7）：写出来就是排好版的样子，只有光标所在的那一段显示 Markdown 符号。
// 这里只看文字、算出哪些符号藏起来、哪些换成小部件（勾选框、圆点、网站名、分隔线、图片、文件、表格），
// 不碰 CodeMirror，方便测试（live.test.ts）；editor.ts 把结果变成装饰。
// 认的写法跟着 shared/md.ts 的 renderMd 走（表格的编号也和它一样，表格的看法记在编号上），多认一个 ``` 代码块和 --- 分隔线

import { findTables } from "@shared/md";
import { T, type SlashId } from "../text/notes";

export type Widget =
  | { t: "check"; done: boolean }
  | { t: "bullet" }
  | { t: "host"; url: string }
  | { t: "hr" }
  | { t: "image"; file: string; alt: string }
  | { t: "file"; file: string; name: string }
  | { t: "table"; n: number; src: string };

/** 位置都是在整篇里的 */
export type Span =
  /** 整行的样子（at 是行首） */
  | { k: "line"; at: number; cls: string }
  /** 藏起来的符号 */
  | { k: "hide"; from: number; to: number }
  /** 行内的样子；href：点了打开的网址 */
  | { k: "mark"; from: number; to: number; cls: string; href?: string }
  /** 行内换成小部件 */
  | { k: "swap"; from: number; to: number; w: Widget }
  /** 整行（几行）换成一块 */
  | { k: "block"; from: number; to: number; w: Widget }
  /** 光标在图片、文件那一行时，源码下面照样显示图片、文件 */
  | { k: "after"; at: number; w: Widget }
  /** 待办（at 是行尾）：后面放「设提醒」或者提醒的时间 */
  | { k: "todo"; at: number; text: string; done: boolean };

/** 灰色等宽的 Markdown 符号（光标所在的那一段） */
export const RAW = "lp-raw";

const HEADING = /^(#{1,3})(\s+)/;
const TODO = /^(\s*)[-*+]\s+\[( |x|X)\](\s|$)/;
const BULLET = /^(\s*)[-*+]\s+/;
/** 编号列表：1. / 1) 后面要有空格，1、可以不要（同 mdformat.ts） */
const ORDERED = /^(\s*)\d+(?:[.)]\s+|、\s*)/;
const QUOTE = /^>\s?/;
const HR = /^\s*(?:-{3,}|\*{3,}|_{3,})\s*$/;
const FENCE = /^\s*(```|~~~)/;
const IMAGE = /^!\[([^\]]*)\]\(att:([\w.-]+)\)$/;
const FILE = /^\[([^\]]+)\]\(att:([\w.-]+)\)$/;
/** 同 md.ts：[文字](网址) 或者光秃秃的网址 */
const LINK = /\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)|(https?:\/\/[^\s<>"，。！？、；：」』）】)]+)/g;
const CODE = /`([^`\n]+)`/g;
const BOLD = /\*\*([^*\n]+)\*\*/g;
const STRIKE = /~~([^~\n]+)~~/g;

/** 一行里的行内写法：代码、链接、网址、粗体、删除线。base 是这一行在整篇里的位置 */
export function inlineSpans(text: string, base: number, active: boolean): Span[] {
  const out: Span[] = [];
  /** 符号：光标不在这里时藏起来，在时显示成灰色等宽 */
  const sym = (from: number, to: number) => {
    if (to > from) out.push(active ? { k: "mark", from: base + from, to: base + to, cls: RAW } : { k: "hide", from: base + from, to: base + to });
  };
  // 代码、网址里的字不再认粗体这些：先盖住
  let masked = text;
  const mask = (from: number, to: number) => (masked = masked.slice(0, from) + "\0".repeat(to - from) + masked.slice(to));
  for (const m of text.matchAll(CODE)) {
    const a = m.index!;
    const b = a + m[0].length;
    sym(a, a + 1);
    out.push({ k: "mark", from: base + a + 1, to: base + b - 1, cls: "lp-code" });
    sym(b - 1, b);
    mask(a, b);
  }
  for (const m of masked.matchAll(LINK)) {
    const a = m.index!;
    const b = a + m[0].length;
    if (m[2]) {
      // [文字](网址)：只留文字
      const url = text.slice(a + m[1].length + 3, b - 1);
      sym(a, a + 1);
      out.push({ k: "mark", from: base + a + 1, to: base + a + 1 + m[1].length, cls: "lp-link", href: url });
      sym(a + 1 + m[1].length, b);
      mask(a + 1 + m[1].length, b);
    } else {
      const url = text.slice(a, b);
      // 光秃秃的网址：只显示网站名 ↗
      if (active) out.push({ k: "mark", from: base + a, to: base + b, cls: "lp-url", href: url });
      else out.push({ k: "swap", from: base + a, to: base + b, w: { t: "host", url } });
      mask(a, b);
    }
  }
  const pair = (re: RegExp, n: number, cls: string) => {
    for (const m of masked.matchAll(re)) {
      const a = m.index!;
      const b = a + m[0].length;
      sym(a, a + n);
      out.push({ k: "mark", from: base + a + n, to: base + b - n, cls });
      sym(b - n, b);
    }
  };
  pair(BOLD, 2, "lp-b");
  pair(STRIKE, 2, "lp-s");
  return out;
}

/**
 * 整篇算一遍。active(i)：第 i 行（从 0 开始）是不是光标所在的那一段，是的话显示符号。
 * 表格、代码块是一整块：光标在里面任何一行，整块都显示源码
 */
export function scan(doc: string, active: (line: number) => boolean): Span[] {
  const lines = doc.split("\n");
  const at: number[] = [];
  let pos = 0;
  for (const l of lines) {
    at.push(pos);
    pos += l.length + 1;
  }
  const end = (i: number) => at[i] + lines[i].length;
  const anyActive = (a: number, b: number) => {
    for (let i = a; i <= b; i++) if (active(i)) return true;
    return false;
  };
  const out: Span[] = [];
  const line = (i: number, cls: string) => out.push({ k: "line", at: at[i], cls });
  const sym = (i: number, from: number, to: number, on: boolean) => {
    if (to > from) out.push(on ? { k: "mark", from: at[i] + from, to: at[i] + to, cls: RAW } : { k: "hide", from: at[i] + from, to: at[i] + to });
  };

  // 表格（和 renderMd 一样的编号）
  const tables = new Map(findTables(doc).map((t, n) => [t.start, { n, last: t.start + t.lines - 1 }]));

  for (let i = 0; i < lines.length; i++) {
    const text = lines[i];
    // ``` 代码块：里面原样显示，用等宽字
    if (FENCE.test(text)) {
      let j = i + 1;
      while (j < lines.length && !FENCE.test(lines[j])) j++;
      const last = Math.min(j, lines.length - 1);
      const on = anyActive(i, last);
      for (let k = i; k <= last; k++) {
        line(k, "lp-codeblock" + (k === i ? " lp-codefirst" : "") + (k === last ? " lp-codelast" : ""));
        if ((k === i || (k === last && j < lines.length)) && lines[k].length) sym(k, 0, lines[k].length, on);
      }
      i = last;
      continue;
    }
    const tbl = tables.get(i);
    if (tbl) {
      if (anyActive(i, tbl.last)) for (let k = i; k <= tbl.last; k++) line(k, "lp-tablerow");
      else out.push({ k: "block", from: at[i], to: end(tbl.last), w: { t: "table", n: tbl.n, src: doc.slice(at[i], end(tbl.last)) } });
      i = tbl.last;
      continue;
    }
    const on = active(i);
    const trimmed = text.trim();
    let m: RegExpMatchArray | null;
    // 单独一行的图片、文件
    if ((m = trimmed.match(IMAGE)) || (m = trimmed.match(FILE))) {
      const w: Widget = trimmed.startsWith("!") ? { t: "image", file: m[2], alt: m[1] } : { t: "file", file: m[2], name: m[1] };
      if (on) {
        out.push({ k: "mark", from: at[i], to: end(i), cls: RAW });
        out.push({ k: "after", at: end(i), w });
      } else out.push({ k: "block", from: at[i], to: end(i), w });
      continue;
    }
    if (HR.test(text)) {
      line(i, "lp-hrline");
      if (on) out.push({ k: "mark", from: at[i], to: end(i), cls: RAW });
      else out.push({ k: "swap", from: at[i], to: end(i), w: { t: "hr" } });
      continue;
    }
    let rest = 0;
    if ((m = text.match(HEADING))) {
      line(i, `lp-h${m[1].length}`);
      sym(i, 0, m[0].length, on);
      rest = m[0].length;
    } else if ((m = text.match(TODO))) {
      const done = m[2] !== " ";
      line(i, "lp-todo" + (done ? " lp-done" : ""));
      const a = m[1].length;
      const b = m[0].length;
      if (on) out.push({ k: "mark", from: at[i] + a, to: at[i] + b, cls: RAW });
      else out.push({ k: "swap", from: at[i] + a, to: at[i] + b, w: { t: "check", done } });
      out.push({ k: "todo", at: end(i), text: text.slice(b).trim(), done });
      rest = b;
    } else if ((m = text.match(BULLET))) {
      line(i, "lp-li");
      const a = m[1].length;
      if (on) out.push({ k: "mark", from: at[i] + a, to: at[i] + m[0].length, cls: RAW });
      else out.push({ k: "swap", from: at[i] + a, to: at[i] + m[0].length, w: { t: "bullet" } });
      rest = m[0].length;
    } else if ((m = text.match(ORDERED))) {
      line(i, "lp-ol");
      out.push({ k: "mark", from: at[i] + m[1].length, to: at[i] + m[0].length, cls: "lp-olnum" });
      rest = m[0].length;
    } else if ((m = text.match(QUOTE))) {
      line(i, "lp-quote");
      sym(i, 0, m[0].length, on);
      rest = m[0].length;
    }
    out.push(...inlineSpans(text.slice(rest), at[i] + rest, on));
  }
  return out;
}

// ---------- 行首打 / 的插入菜单 ----------

/** 光标前面这一段（从行首到光标）是不是「/ + 要找的字」，是的话回要找的字。
 * 中文输入法里按 / 打出来的是「、」，行首的「、」也算 */
export function slashQuery(before: string): string | null {
  const m = before.match(/^\s*[/、]([^\s/、]{0,8})$/);
  return m ? m[1] : null;
}

/** 按打的字筛插入菜单：名字、Markdown 写法、拼音都认 */
export function slashItems(q: string): SlashId[] {
  const s = q.trim().toLowerCase();
  return T.slash.filter((it) => !s || it.name.startsWith(s) || it.md.toLowerCase().startsWith(s) || it.keys.split(" ").some((k) => k.startsWith(s))).map((it) => it.id);
}
