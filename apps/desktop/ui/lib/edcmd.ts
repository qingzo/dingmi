// 笔记编辑器的改字命令：格式快捷键、工具栏、/ 插入菜单、选中文字的小工具条都用这些；
// 还有表格里 Tab 跳格、回车加一行、格子编辑、一键格式化

import { EditorSelection, EditorState, type ChangeSpec } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";
import { formatMarkdown, formatTable, splitRow, tableLen } from "./mdformat";
import { blankTable, openTableGrid } from "./tablegrid";
import { T } from "../text/notes";

// ---------- 表格 ----------

/** 文档里第 lineNo 行（从 1 开始）所在的表格：从第几行到第几行；不在表格里返回 null */
export function tableAround(state: EditorState, lineNo: number): { first: number; last: number } | null {
  const doc = state.doc;
  if (!doc.line(lineNo).text.includes("|")) return null;
  let first = lineNo;
  while (first > 1 && lineNo - first < 500 && doc.line(first - 1).text.includes("|")) first--;
  // 往上连着的带 | 的行里，找一个能当表头的
  const texts: string[] = [];
  const end = Math.min(doc.lines, lineNo + 500);
  for (let i = first; i <= end; i++) texts.push(doc.line(i).text);
  for (let i = 0; first + i <= lineNo; i++) {
    const len = tableLen(texts, i);
    if (len && first + i + len - 1 >= lineNo) return { first: first + i, last: first + i + len - 1 };
    if (len) i += len - 1;
  }
  return null;
}

export const cursorInTable = (s: EditorState) => !!tableAround(s, s.doc.lineAt(s.selection.main.head).number);

/** 一行表格里各个 | 的位置（不算 \|） */
function pipes(text: string): number[] {
  const out: number[] = [];
  for (let i = 0; i < text.length; i++) if (text[i] === "|" && text[i - 1] !== "\\") out.push(i);
  return out;
}

/** 光标在这一行的第几格（从 0 开始） */
function cellIndex(text: string, col: number): number {
  const p = pipes(text);
  const lead = text.trimStart().startsWith("|") ? 1 : 0;
  return Math.max(0, p.filter((x) => x < col).length - lead);
}

/** 排好的表格行里第 k 格的内容范围（行内的位置） */
function cellRange(text: string, k: number): [number, number] {
  const p = pipes(text);
  const a = p[k] ?? p[p.length - 2] ?? 0;
  const b = p[k + 1] ?? text.length;
  const inner = text.slice(a + 1, b);
  const s = a + 1 + (inner.length - inner.trimStart().length);
  const e = b - (inner.length - inner.trimEnd().length);
  return s <= e ? [s, e] : [a + 2, a + 2];
}

/**
 * 表格里的 Tab / Shift + Tab / 回车：先把整张表格排整齐，再把光标放到目标格子。
 * dir：1 下一格，-1 上一格，0 回车（在下面加一行）
 */
export function tableMove(view: EditorView, dir: 1 | -1 | 0): boolean {
  const { state } = view;
  const sel = state.selection.main;
  const line = state.doc.lineAt(sel.head);
  const tbl = tableAround(state, line.number);
  if (!tbl) return false;
  // 回车：表格里任何位置都是在下面加一行（格子里换行会把表格拆坏，要换行写 <br>）
  if (dir === 0 && !sel.empty) return false;
  const rows: string[] = [];
  for (let i = tbl.first; i <= tbl.last; i++) rows.push(state.doc.line(i).text);
  let r = line.number - tbl.first;
  let k = cellIndex(line.text, sel.head - line.from);
  const ncol = Math.max(...rows.map((x) => splitRow(x).length));
  // 回车在最后一行、而且这一行是空的：去掉这一行，跳出表格
  if (dir === 0 && r === rows.length - 1 && r > 1 && splitRow(rows[r]).every((c) => !c)) {
    const from = state.doc.line(tbl.last - 1).to;
    view.dispatch({ changes: { from, to: line.to, insert: "\n" }, selection: { anchor: from + 1 }, scrollIntoView: true });
    return true;
  }
  if (dir === 0) {
    // 在表头、分隔行上回车：新行加在分隔行下面
    r = Math.max(2, r + 1);
    rows.splice(r, 0, "|" + " |".repeat(ncol));
    k = 0;
  } else {
    k += dir;
    if (k >= ncol) (k = 0), r++;
    if (k < 0) (k = ncol - 1), r--;
    if (r === 1) r += dir; // 跳过分隔行
    if (r < 0) return true;
    if (r >= rows.length) rows.push("|" + " |".repeat(ncol)); // 最后一格再按 Tab：加一行
  }
  const out = formatTable(rows);
  const from = state.doc.line(tbl.first).from;
  const to = state.doc.line(tbl.last).to;
  let pos = from;
  for (let i = 0; i < r; i++) pos += out[i].length + 1;
  const [a, b] = cellRange(out[r], k);
  view.dispatch({ changes: { from, to, insert: out.join("\n") }, selection: EditorSelection.range(pos + a, pos + b), scrollIntoView: true, userEvent: "input" });
  return true;
}

/** 第 lineNo 行（从 1 开始）所在的表格用格子编辑器打开，点完成后写回原处；不在表格里返回 false */
export function gridAt(view: EditorView, say: (text: string) => void, lineNo = view.state.doc.lineAt(view.state.selection.main.head).number): boolean {
  const { state } = view;
  const tbl = tableAround(state, lineNo);
  if (!tbl) return false;
  const from = state.doc.line(tbl.first).from;
  const old = state.sliceDoc(from, state.doc.line(tbl.last).to);
  openTableGrid({
    lines: old.split("\n"),
    onDone: (next) => {
      // 开着格子编辑器时正文可能变了（比如同步过来的修改）：按原来的内容找回那张表格
      const doc = view.state.doc.toString();
      const at = doc.slice(from, from + old.length) === old ? from : doc.indexOf(old);
      if (at < 0) return say(T.tableChanged);
      view.dispatch({ changes: { from: at, to: at + old.length, insert: next.join("\n") }, selection: { anchor: at }, userEvent: "input", scrollIntoView: true });
      view.focus();
    },
  });
  return true;
}

/** 在光标这里插一张 3 列 2 行的空表格（这一行有字就插在下面），选中第一格。grid：插好后用格子编辑器打开 */
export function insertTable(view: EditorView, say: (text: string) => void, grid = false): boolean {
  const { state } = view;
  const line = state.doc.lineAt(state.selection.main.head);
  const has = !!line.text.trim();
  const at = has ? line.to : line.from;
  const rows = blankTable();
  const lead = has ? "\n\n" : "";
  const next = line.number < state.doc.lines ? "" : "\n";
  const first = at + lead.length + rows[0].indexOf("列");
  view.dispatch({
    changes: { from: at, to: has ? at : line.to, insert: lead + rows.join("\n") + next },
    selection: EditorSelection.range(first, first + 3),
    userEvent: "input",
    scrollIntoView: true,
  });
  return grid ? gridAt(view, say) : true;
}

// ---------- 行内 ----------

/** **粗体**、~~删除线~~、`代码`：选中的文字两边加上记号，已经有了就去掉 */
export const toggleMark = (mark: string) => (view: EditorView) => {
  const { state } = view;
  const l = mark.length;
  view.dispatch(
    state.changeByRange((range) => {
      const text = state.sliceDoc(range.from, range.to);
      if (state.sliceDoc(range.from - l, range.from) === mark && state.sliceDoc(range.to, range.to + l) === mark)
        return {
          changes: [
            { from: range.from - l, to: range.from },
            { from: range.to, to: range.to + l },
          ],
          range: EditorSelection.range(range.from - l, range.to - l),
        };
      if (text.length >= 2 * l && text.startsWith(mark) && text.endsWith(mark))
        return { changes: { from: range.from, to: range.to, insert: text.slice(l, -l) }, range: EditorSelection.range(range.from, range.to - 2 * l) };
      return {
        changes: [
          { from: range.from, insert: mark },
          { from: range.to, insert: mark },
        ],
        range: EditorSelection.range(range.from + l, range.to + l),
      };
    }),
    { userEvent: "input" },
  );
  return true;
};

/** Ctrl + K：选中的是网址就变成 [](网址)，否则变成 [文字]()，光标放在要填的地方 */
export const insertLink = (view: EditorView) => {
  const { state } = view;
  view.dispatch(
    state.changeByRange((range) => {
      const text = state.sliceDoc(range.from, range.to);
      if (/^https?:\/\/\S+$/.test(text)) return { changes: { from: range.from, to: range.to, insert: `[](${text})` }, range: EditorSelection.cursor(range.from + 1) };
      const insert = `[${text}]()`;
      return { changes: { from: range.from, to: range.to, insert }, range: EditorSelection.cursor(range.from + insert.length - 1) };
    }),
    { userEvent: "input" },
  );
  return true;
};

// ---------- 整行 ----------

/** 选中的每一行（光标所在行）交给 fn 改写 */
function mapLines(view: EditorView, fn: (lines: string[]) => string[]) {
  const { state } = view;
  const changes: ChangeSpec[] = [];
  const seen = new Set<number>();
  for (const range of state.selection.ranges) {
    const a = state.doc.lineAt(range.from).number;
    const b = state.doc.lineAt(range.to).number;
    const nums = [];
    for (let i = a; i <= b; i++) if (!seen.has(i)) (seen.add(i), nums.push(i));
    const lines = nums.map((i) => state.doc.line(i));
    const next = fn(lines.map((l) => l.text));
    lines.forEach((l, i) => next[i] !== l.text && changes.push({ from: l.from, to: l.to, insert: next[i] }));
  }
  if (changes.length) view.dispatch({ changes, userEvent: "input" });
  return true;
}

const PREFIX = /^(\s*)(#{1,3}\s+|>\s?|[-*+]\s+\[[ xX]\]\s*|[-*+]\s+|\d+[.)]\s+|\d+、\s*)?/;
const strip = (s: string) => s.replace(PREFIX, "$1");

/** Ctrl + 1/2/3：设成几级标题，已经是这一级就变回正文 */
export const heading = (level: number) => (view: EditorView) =>
  mapLines(view, (ls) => {
    const mark = "#".repeat(level) + " ";
    const all = ls.every((l) => l.trimStart().startsWith(mark) && !l.trimStart().startsWith(mark + "#"));
    return ls.map((l) => (all ? strip(l) : l.trim() || ls.length === 1 ? strip(l).replace(/^(\s*)/, `$1${mark}`) : l));
  });

/** Ctrl + Shift + 8 圆点列表、Ctrl + Shift + 7 编号列表、引用：都已经是了就去掉 */
export const bullets = (view: EditorView) =>
  mapLines(view, (ls) => {
    const all = ls.every((l) => /^\s*[-*+]\s+(?!\[[ xX]\])/.test(l));
    return ls.map((l) => (all ? strip(l) : l.trim() || ls.length === 1 ? strip(l).replace(/^(\s*)/, "$1- ") : l));
  });
export const numbers = (view: EditorView) =>
  mapLines(view, (ls) => {
    const all = ls.every((l) => /^\s*\d+([.)]\s+|、)/.test(l));
    let n = 1;
    return ls.map((l) => (all ? strip(l) : l.trim() || ls.length === 1 ? strip(l).replace(/^(\s*)/, `$1${n++}. `) : l));
  });
export const quote = (view: EditorView) =>
  mapLines(view, (ls) => {
    const all = ls.every((l) => /^>/.test(l));
    return ls.map((l) => (all ? l.replace(/^>\s?/, "") : l.trim() || ls.length === 1 ? "> " + strip(l).trimStart() : l));
  });

/** Ctrl + Enter：勾上 / 取消待办；不是待办的行变成待办 */
export const toggleTodo = (view: EditorView) =>
  mapLines(view, (ls) =>
    ls.map((l) => {
      const m = l.match(/^(\s*[-*+]\s+\[)([ xX])(\].*)$/);
      if (m) return m[1] + (m[2] === " " ? "x" : " ") + m[3];
      return strip(l).replace(/^(\s*)/, "$1- [ ] ");
    }),
  );

/** 工具栏的待办按钮：不是待办的变成待办，已经是待办的变回正文（不勾选） */
export const todoLines = (view: EditorView) =>
  mapLines(view, (ls) => {
    const all = ls.every((l) => /^\s*[-*+]\s+\[[ xX]\]/.test(l));
    return ls.map((l) => (all ? strip(l) : l.trim() || ls.length === 1 ? strip(l).replace(/^(\s*)/, "$1- [ ] ") : l));
  });

/** 分隔线：单独一行 ---，光标放到下面一行 */
export function insertHr(view: EditorView) {
  const { state } = view;
  const line = state.doc.lineAt(state.selection.main.head);
  const has = !!line.text.trim();
  const insert = (has ? "\n" : "") + "---\n";
  const from = has ? line.to : line.from;
  view.dispatch({ changes: { from, to: line.to, insert }, selection: { anchor: from + insert.length }, userEvent: "input", scrollIntoView: true });
  return true;
}

/** 勾上 / 取消第 lineNo 行（从 1 开始）的待办；回这一行待办的字和勾没勾上，不是待办回 null */
export function setTodoAt(view: EditorView, lineNo: number, done?: boolean): { text: string; done: boolean } | null {
  const line = view.state.doc.line(lineNo);
  const m = line.text.match(/^(\s*[-*+]\s+\[)([ xX])\](.*)$/);
  if (!m) return null;
  const next = done ?? m[2] === " ";
  const at = line.from + m[1].length;
  view.dispatch({ changes: { from: at, to: at + 1, insert: next ? "x" : " " }, userEvent: "input" });
  return { text: m[3].trim(), done: next };
}

/** 一键格式化整篇，光标尽量留在原来那一行 */
export function formatAll(view: EditorView) {
  const before = view.state.doc.toString();
  const after = formatMarkdown(before);
  if (after === before) return true;
  const line = view.state.doc.lineAt(view.state.selection.main.head).number;
  const lines = after.split("\n");
  let pos = 0;
  for (let i = 0; i < Math.min(line - 1, lines.length - 1); i++) pos += lines[i].length + 1;
  view.dispatch({ changes: { from: 0, to: before.length, insert: after }, selection: { anchor: Math.min(pos, after.length) }, userEvent: "input", scrollIntoView: true });
  return true;
}

/** 整篇换成 next，只改不一样的那一段（光标、滚动位置不乱跳） */
export function replaceAll(view: EditorView, next: string, userEvent = "input") {
  const cur = view.state.doc.toString();
  if (cur === next) return;
  let a = 0;
  const max = Math.min(cur.length, next.length);
  while (a < max && cur.charCodeAt(a) === next.charCodeAt(a)) a++;
  let b = 0;
  while (b < max - a && cur.charCodeAt(cur.length - 1 - b) === next.charCodeAt(next.length - 1 - b)) b++;
  view.dispatch({ changes: { from: a, to: cur.length - b, insert: next.slice(a, next.length - b) }, userEvent });
}

/** 光标所在的那一行是什么：工具栏上对应的按钮亮着 */
export function lineKind(state: EditorState): Set<string> {
  const line = state.doc.lineAt(state.selection.main.head);
  const t = line.text;
  const on = new Set<string>();
  if (/^#{1,3}\s/.test(t)) on.add("heading");
  if (/^\s*[-*+]\s+\[[ xX]\]/.test(t)) on.add("todo");
  else if (/^\s*[-*+]\s+/.test(t)) on.add("list");
  if (tableAround(state, line.number)) on.add("table");
  return on;
}
