// 笔记的编辑器（CodeMirror 6）：编辑时显示 Markdown 原文，符号带颜色。平时看笔记是排好版的（renderMd），
// 双击或点「编辑」才进来，Esc 回去看（2026-11-06 试用后改：原来的边写边排版，点击时行高会变、容易点错行）。
// 还有：行首 / 插入菜单、选中文字的小工具条（edpop.ts）、工具栏（toolbar）、笔记内搜索替换、
// 表格里 Tab 跳格、列表自动接续、格式快捷键、一键格式化、粘贴网页 / Word / Excel 的内容自动转成 Markdown。
// 样子在 styles/editor.css（这里自己 import，哪个窗口用都一样）

import "../styles/editor.css";
import "../styles/md.css";
import { defaultKeymap, history, historyKeymap, indentLess, indentMore } from "@codemirror/commands";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { highlightSelectionMatches, openSearchPanel, search, searchKeymap, SearchQuery, setSearchQuery } from "@codemirror/search";
import { EditorSelection, EditorState, Prec, StateField, type Extension } from "@codemirror/state";
import { drawSelection, EditorView, keymap, placeholder, showPanel, type Panel } from "@codemirror/view";
import { tags as t } from "@lezer/highlight";
import { icon, type IconName } from "@shared/icons";
import { bullets, cursorInTable, formatAll, gridAt, heading, insertLink, insertTable, lineKind, numbers, replaceAll, tableMove, todoLines, toggleMark, toggleTodo } from "./edcmd";
import { popups } from "./edpop";
import type { TodoRef } from "./livedeco";
import { htmlToMd } from "./mdformat";
import { T } from "../text/notes";

export type { TodoRef } from "./livedeco";

/** 编辑器的快捷键说明（鼠标停在提示上时显示） */
export const EDIT_KEYS = T.editKeys.join("\n");

/** 工具栏、菜单能做的事 */
export type EditCmd = "heading" | "bold" | "list" | "olist" | "todo" | "table" | "image" | "file" | "link" | "code" | "strike";

export interface NoteEditor {
  readonly view: EditorView;
  readonly value: string;
  /** 整篇换掉（AI 整理、撤销整理、别处的修改），只改不一样的地方，可以 Ctrl + Z 撤回 */
  setValue(s: string): void;
  /** 在光标处单独占一行插进去（附件） */
  insertLine(text: string): void;
  /** 一键格式化整篇 */
  format(): void;
  /** 选中第一个包含 q 的地方，F3 接着找下一个 */
  find(q: string): void;
  /** 跳到第 line 行（从 0 开始），滚到顶上 */
  gotoLine(line: number): void;
  /** 光标放到第 line 行（从 0 开始）；给了 word 就放在这一行里那个词的前面，滚到中间（双击排好版的笔记进来时） */
  placeAt(line: number, word?: string): void;
  focus(): void;
  destroy(): void;
  /** 重画待办后面的提醒、图片和文件（提醒改了、附件下载完了时调） */
  refresh(): void;
  /** 工具栏上的一个按钮 */
  run(cmd: EditCmd): void;
  /** 选图片、文件加进来（images：只要图片） */
  pickFiles(images: boolean): void;
}

interface Options {
  placeholder?: string;
  onChange: () => void;
  /** 粘贴、拖进来、选的文件（图片、附件） */
  onFiles: (files: File[], view: EditorView) => void;
  /** 给用户的一句提示（比如表格在别处改过了，格子编辑的结果没写回去） */
  onMessage?: (text: string) => void;
  /** 这条笔记的 uid：表格的看法（列表 / 卡片 / 表格）记在它上面 */
  uid?: string;
  /** 待办设过提醒的：回提醒时间的说法（「周三 10:00」），没设过回 null */
  todoReminder?: (text: string) => string | null;
  /** 点了待办后面的「设提醒」或者提醒时间；不传就不显示「设提醒」 */
  onTodoRemind?: (todo: TodoRef, anchor: HTMLElement) => void;
  /** 勾上 / 取消了一条待办（正文已经改好了），设过提醒的跟着完成 */
  onTodoToggle?: (todo: TodoRef & { done: boolean }) => void;
  /** 选中文字点了「设成提醒」；不传就不显示这个按钮 */
  onRemindText?: (text: string, anchor: HTMLElement) => void;
  /** 选择、光标挪了（工具栏跟着亮） */
  onCursor?: (on: Set<string>) => void;
  /** 编辑区上面钉着多高的东西（工具栏）：跳到某一行、找到的字不要藏在它下面 */
  topMargin?: number;
  /** 按 Esc（搜索框、菜单都没开时）：回去看 */
  onEscape?: () => void;
}

// ---------- 中文界面 ----------

const PHRASES = {
  Find: "查找",
  Replace: "替换为",
  next: "下一个",
  previous: "上一个",
  all: "全部",
  "match case": "区分大小写",
  regexp: "正则",
  "by word": "整词",
  replace: "替换",
  "replace all": "全部替换",
  close: "关闭",
  "current match": "当前",
  "on line": "在第几行",
  "replaced match on line $": "替换了第 $ 行的一处",
  "replaced $ matches": "替换了 $ 处",
  "Go to line": "跳到第几行",
  go: "跳转",
};

// ---------- 源码的颜色：标题、粗体照样粗，符号淡一点，链接、代码各有颜色 ----------

const style = HighlightStyle.define([
  { tag: t.heading1, fontWeight: "700", fontSize: "1.3em" },
  { tag: t.heading2, fontWeight: "700", fontSize: "1.15em" },
  { tag: [t.heading3, t.heading4, t.heading5, t.heading6], fontWeight: "700" },
  { tag: t.strong, fontWeight: "700" },
  { tag: t.emphasis, fontStyle: "italic" },
  { tag: t.strikethrough, textDecoration: "line-through", color: "var(--ink-3)" },
  { tag: [t.processingInstruction, t.contentSeparator, t.meta], color: "var(--ink-3)" },
  { tag: [t.link, t.url], color: "var(--accent-ink)" },
  { tag: t.monospace, fontFamily: "var(--f-mono)", color: "var(--tc-purple)" },
  { tag: t.quote, color: "var(--ink-2)" },
  { tag: t.list, color: "var(--accent-ink)" },
]);

// ---------- 光标在表格里：编辑区底下一条提示，带「格子编辑」按钮 ----------

function tableTools(say: (text: string) => void): Extension {
  const panel = (view: EditorView): Panel => {
    const dom = document.createElement("div");
    dom.className = "cm-tablepanel";
    dom.innerHTML = `<span>${T.tablePanel}</span><button type="button">${icon("sheet")}${T.tableGrid}</button>`;
    dom.querySelector("button")!.addEventListener("mousedown", (e) => {
      e.preventDefault();
      gridAt(view, say);
    });
    return { dom, top: false };
  };
  const field = StateField.define<boolean>({
    create: cursorInTable,
    update: (v, tr) => (tr.docChanged || tr.selection ? cursorInTable(tr.state) : v),
    provide: (f) => showPanel.from(f, (on) => (on ? panel : null)),
  });
  return [field, Prec.high(keymap.of([{ key: "Mod-Alt-t", run: (v) => gridAt(v, say) || insertTable(v, say, true) }]))];
}

// ---------- 创建 ----------

export function createNoteEditor(parent: HTMLElement, doc: string, opts: Options): NoteEditor {
  const say = opts.onMessage ?? (() => {});
  /** Ctrl + Shift + V：这次按纯文本粘贴 */
  let plainNext = false;
  let view: EditorView;

  const fileInput = document.createElement("input");
  fileInput.type = "file";
  fileInput.multiple = true;
  fileInput.hidden = true;
  fileInput.addEventListener("change", () => {
    const files = [...(fileInput.files ?? [])];
    fileInput.value = "";
    if (files.length) opts.onFiles(files, view);
  });
  parent.append(fileInput);
  const pickFiles = (images: boolean) => {
    fileInput.accept = images ? "image/*" : "";
    fileInput.click();
  };

  const shortcuts = Prec.high(
    keymap.of([
      { key: "Tab", run: (v) => tableMove(v, 1) || indentMore(v), shift: (v) => tableMove(v, -1) || indentLess(v) },
      { key: "Enter", run: (v) => tableMove(v, 0) },
      { key: "Mod-b", run: toggleMark("**") },
      { key: "Mod-Shift-x", run: toggleMark("~~") },
      { key: "Mod-e", run: toggleMark("`") },
      { key: "Mod-k", run: insertLink },
      { key: "Mod-1", run: heading(1) },
      { key: "Mod-2", run: heading(2) },
      { key: "Mod-3", run: heading(3) },
      { key: "Mod-Shift-7", run: numbers },
      { key: "Mod-Shift-8", run: bullets },
      { key: "Mod-Enter", run: toggleTodo },
      { key: "Shift-Alt-f", run: formatAll },
      { key: "Mod-h", run: openSearchPanel },
    ]),
  );

  const extensions: Extension[] = [
    history(),
    drawSelection(),
    EditorView.lineWrapping,
    EditorView.editorAttributes.of({ class: "dm-ed" }),
    EditorState.allowMultipleSelections.of(true),
    EditorState.phrases.of(PHRASES),
    placeholder(opts.placeholder ?? T.placeholder),
    markdown({ base: markdownLanguage }),
    syntaxHighlighting(style),
    search({ top: true }),
    highlightSelectionMatches(),
    popups({ say, pickFiles, onRemindText: opts.onRemindText }),
    tableTools(say),
    EditorView.scrollMargins.of(() => ({ top: opts.topMargin ?? 0 })),
    shortcuts,
    keymap.of([...searchKeymap, ...historyKeymap, ...defaultKeymap]),
    // 最后才轮到它：搜索框、插入菜单这些先用掉 Esc
    Prec.lowest(keymap.of([{ key: "Escape", run: () => (opts.onEscape ? (opts.onEscape(), true) : false) }])),
    EditorView.updateListener.of((u) => {
      if (u.docChanged) opts.onChange();
      if (opts.onCursor && (u.docChanged || u.selectionSet)) opts.onCursor(lineKind(u.state));
    }),
    EditorView.domEventHandlers({
      keydown(e) {
        plainNext = e.ctrlKey && e.shiftKey && (e.key === "V" || e.key === "v");
        return false;
      },
      paste(e, view) {
        const files = [...(e.clipboardData?.items ?? [])]
          .filter((i) => i.kind === "file")
          .map((i) => i.getAsFile())
          .filter((f): f is File => !!f);
        if (files.length) {
          e.preventDefault();
          opts.onFiles(files, view);
          return true;
        }
        if (plainNext) {
          plainNext = false;
          return false;
        }
        const html = e.clipboardData?.getData("text/html");
        const md = html ? htmlToMd(html) : null;
        if (!md) return false;
        e.preventDefault();
        // 好几行的内容（标题、表格、列表）单独起一行，不和光标前后的字连在一起
        const { from, to } = view.state.selection.main;
        const line = view.state.doc.lineAt(from);
        const block = md.includes("\n");
        const text = (block && from > line.from ? "\n" : "") + md + (block && to < view.state.doc.lineAt(to).to ? "\n" : "");
        view.dispatch(view.state.replaceSelection(text), { userEvent: "input.paste", scrollIntoView: true });
        return true;
      },
      drop(e, view) {
        const files = [...(e.dataTransfer?.files ?? [])];
        if (!files.length) return false;
        e.preventDefault();
        const pos = view.posAtCoords({ x: e.clientX, y: e.clientY });
        if (pos != null) view.dispatch({ selection: { anchor: pos } });
        opts.onFiles(files, view);
        return true;
      },
    }),
  ];
  view = new EditorView({
    parent,
    state: EditorState.create({ doc, extensions, selection: { anchor: doc.length } }),
  });

  const run = (cmd: EditCmd) => {
    const v = view;
    switch (cmd) {
      case "heading":
        heading(2)(v);
        break;
      case "bold":
        toggleMark("**")(v);
        break;
      case "strike":
        toggleMark("~~")(v);
        break;
      case "code":
        toggleMark("`")(v);
        break;
      case "link":
        insertLink(v);
        break;
      case "list":
        bullets(v);
        break;
      case "olist":
        numbers(v);
        break;
      case "todo":
        todoLines(v);
        break;
      case "table":
        if (!gridAt(v, say)) insertTable(v, say);
        break;
      case "image":
      case "file":
        pickFiles(cmd === "image");
        return;
    }
    v.focus();
  };

  return {
    view,
    get value() {
      return view.state.doc.toString();
    },
    setValue(s) {
      replaceAll(view, s);
    },
    insertLine(text) {
      const { state } = view;
      const at = state.selection.main;
      const line = state.doc.lineAt(at.from);
      const before = at.from > line.from ? "\n" : "";
      view.dispatch(state.replaceSelection(`${before}${text}\n`), { scrollIntoView: true });
    },
    format() {
      formatAll(view);
      view.focus();
    },
    find(q) {
      const i = view.state.doc.toString().toLowerCase().indexOf(q.toLowerCase());
      view.dispatch({ effects: setSearchQuery.of(new SearchQuery({ search: q })) });
      if (i >= 0) view.dispatch({ selection: EditorSelection.range(i, i + q.length), effects: EditorView.scrollIntoView(i, { y: "center" }) });
    },
    gotoLine(line) {
      const l = view.state.doc.line(Math.min(line + 1, view.state.doc.lines));
      view.dispatch({ selection: { anchor: l.from }, effects: EditorView.scrollIntoView(l.from, { y: "start", yMargin: 12 }) });
      view.focus();
    },
    placeAt(line, word) {
      const l = view.state.doc.line(Math.min(Math.max(line, 0) + 1, view.state.doc.lines));
      const i = word ? l.text.indexOf(word) : -1;
      const at = i >= 0 ? l.from + i : l.to;
      view.dispatch({ selection: { anchor: at }, effects: EditorView.scrollIntoView(at, { y: "center" }) });
      view.focus();
    },
    focus() {
      view.focus();
    },
    destroy() {
      view.destroy();
      fileInput.remove();
    },
    // 原来边写边排版时重画待办后面的提醒、图片；现在编辑时是源码，没有要重画的
    refresh() {},
    run,
    pickFiles,
  };
}

// ---------- 工具栏（U7）：标题、粗体、列表、待办、表格 | 图片、文件、链接、代码 ----------

const TOOLS: (readonly [EditCmd, IconName, string] | null)[] = [
  ["heading", "heading", T.tool.heading],
  ["bold", "bold", T.tool.bold],
  ["list", "list", T.tool.list],
  ["todo", "square-check", T.tool.todo],
  ["table", "table", T.tool.table],
  null,
  ["image", "image", T.tool.image],
  ["file", "paperclip", T.tool.file],
  ["link", "link", T.tool.link],
  ["code", "code", T.tool.code],
];

/** 往 bar 里放格式按钮（放在最前面），bar 的其余部分（已保存、让 AI 整理）由窗口自己放。回一个函数：光标挪了时调，按钮跟着亮 */
export function fillToolbar(bar: HTMLElement, ed: () => NoteEditor | null): (on: Set<string>) => void {
  const box = document.createElement("div");
  box.className = "ed-tools";
  box.innerHTML = TOOLS.map((x) => (x ? `<button type="button" class="b" data-cmd="${x[0]}" title="${x[2]}" aria-label="${x[2]}">${icon(x[1])}</button>` : `<span class="dv"></span>`)).join("");
  // 按下去不让编辑器丢焦点
  box.addEventListener("mousedown", (e) => {
    if ((e.target as HTMLElement).closest("[data-cmd]")) e.preventDefault();
  });
  box.addEventListener("click", (e) => {
    const b = (e.target as HTMLElement).closest<HTMLElement>("[data-cmd]");
    if (b) ed()?.run(b.dataset.cmd as EditCmd);
  });
  bar.prepend(box);
  return (on) => box.querySelectorAll<HTMLElement>("[data-cmd]").forEach((b) => b.classList.toggle("on", on.has(b.dataset.cmd!)));
}
