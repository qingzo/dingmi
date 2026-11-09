// 编辑器里浮出来的两样（U7）：行首打 / 弹出的插入菜单、选中文字后上方的小工具条（含「设成提醒」）。
// 都用 CodeMirror 的 tooltip 定位

import { Prec, StateEffect, StateField, type EditorState } from "@codemirror/state";
import { EditorView, keymap, showTooltip, type Tooltip, type TooltipView } from "@codemirror/view";
import { icon, type IconName } from "@shared/icons";
import { copyText, esc } from "@shared/util";
import { bullets, heading, insertHr, insertLink, insertTable, todoLines, toggleMark, toggleTodo } from "./edcmd";
import { slashItems, slashQuery } from "./live";
import { T, type SlashId } from "../text/notes";

export interface PopOptions {
  say: (text: string) => void;
  /** 选好图片、文件（images：只要图片） */
  pickFiles: (images: boolean) => void;
  /** 选中的文字点了「设成提醒」；没有就不显示这个按钮 */
  onRemindText?: (text: string, anchor: HTMLElement) => void;
}

// ---------- / 插入菜单 ----------

interface Slash {
  /** / 在哪 */
  from: number;
  /** 选着第几项 */
  sel: number;
  tip: Tooltip;
}

const slashFx = StateEffect.define<{ sel: number } | null>();

/** 光标前面是不是「行首 / + 字」，是的话回要找的字 */
function queryAt(state: EditorState, from: number): string | null {
  const sel = state.selection.main;
  if (!sel.empty || sel.head <= from) return null;
  const line = state.doc.lineAt(from);
  if (sel.head > line.to) return null;
  const q = slashQuery(state.sliceDoc(line.from, sel.head));
  return q !== null && slashItems(q).length ? q : null;
}

function slashField(opts: PopOptions) {
  const tip = (from: number): Tooltip => ({ pos: from, above: false, create: (view) => slashView(view, opts) });
  return StateField.define<Slash | null>({
    create: () => null,
    update(v, tr) {
      for (const e of tr.effects) if (e.is(slashFx)) return e.value && v ? { ...v, sel: e.value.sel } : null;
      let next = v;
      if (!v && tr.docChanged && tr.isUserEvent("input.type")) {
        // 刚在行首打了一个 /（或者中文输入法的「、」）
        const head = tr.state.selection.main.head;
        const line = tr.state.doc.lineAt(head);
        if (slashQuery(tr.state.sliceDoc(line.from, head)) === "") next = { from: head - 1, sel: 0, tip: tip(head - 1) };
      } else if (v && tr.docChanged) {
        const from = tr.changes.mapPos(v.from);
        next = { from, sel: 0, tip: from === v.from ? v.tip : tip(from) };
      }
      if (!next || queryAt(tr.state, next.from) === null) return null;
      return next;
    },
    provide: (f) => showTooltip.from(f, (v) => v?.tip ?? null),
  });
}

const SLASH_ICONS: Record<SlashId, IconName> = { todo: "square-check", table: "table", heading: "heading", list: "list", image: "image", file: "paperclip", hr: "minus" };

function slashView(view: EditorView, opts: PopOptions): TooltipView {
  const dom = document.createElement("div");
  dom.className = "lp-pop lp-menu";
  const field = () => view.state.field(slashState(opts), false) ?? null;
  const draw = (state: EditorState) => {
    const s = state.field(slashState(opts), false);
    if (!s) return;
    const q = queryAt(state, s.from) ?? "";
    const ids = slashItems(q);
    dom.innerHTML =
      `<div class="mlab">${T.slashHead}</div>` +
      ids
        .map((id, i) => {
          const it = T.slash.find((x) => x.id === id)!;
          return `<div class="mi${i === s.sel % ids.length ? " on" : ""}" data-slash="${id}">${icon(SLASH_ICONS[id])}${esc(it.name)}${it.md ? `<small>${esc(it.md)}</small>` : ""}</div>`;
        })
        .join("");
  };
  dom.addEventListener("mousedown", (e) => {
    e.preventDefault();
    const id = (e.target as HTMLElement).closest<HTMLElement>("[data-slash]")?.dataset.slash as SlashId | undefined;
    if (id && field()) applySlash(view, opts, id);
  });
  draw(view.state);
  return { dom, update: (u) => draw(u.state) };
}

/** 去掉「/ + 字」，在这一行插进选的东西 */
function applySlash(view: EditorView, opts: PopOptions, id: SlashId) {
  const s = view.state.field(slashState(opts), false);
  if (!s) return;
  view.dispatch({ changes: { from: s.from, to: view.state.selection.main.head }, effects: slashFx.of(null), userEvent: "input" });
  const toEnd = () => view.dispatch({ selection: { anchor: view.state.doc.lineAt(view.state.selection.main.head).to } });
  if (id === "todo") (todoLines(view), toEnd());
  else if (id === "list") (bullets(view), toEnd());
  else if (id === "heading") (heading(2)(view), toEnd());
  else if (id === "hr") insertHr(view);
  else if (id === "table") insertTable(view, opts.say);
  else opts.pickFiles(id === "image");
  view.focus();
}

// 每个编辑器一个菜单状态（主窗口、笔记窗口各自的选项不同）
const fields = new WeakMap<PopOptions, StateField<Slash | null>>();
const slashState = (opts: PopOptions) => fields.get(opts) ?? (fields.set(opts, slashField(opts)), fields.get(opts)!);

function slashKeys(opts: PopOptions) {
  const f = slashState(opts);
  const move = (d: number) => (view: EditorView) => {
    const s = view.state.field(f, false);
    if (!s) return false;
    const n = slashItems(queryAt(view.state, s.from) ?? "").length;
    view.dispatch({ effects: slashFx.of({ sel: (s.sel + d + n) % n }) });
    return true;
  };
  const pick = (view: EditorView) => {
    const s = view.state.field(f, false);
    if (!s) return false;
    const ids = slashItems(queryAt(view.state, s.from) ?? "");
    applySlash(view, opts, ids[s.sel % ids.length]);
    return true;
  };
  const close = (view: EditorView) => {
    if (!view.state.field(f, false)) return false;
    view.dispatch({ effects: slashFx.of(null) });
    return true;
  };
  return Prec.highest(
    keymap.of([
      { key: "ArrowDown", run: move(1) },
      { key: "ArrowUp", run: move(-1) },
      { key: "Enter", run: pick },
      { key: "Tab", run: pick },
      { key: "Escape", run: close },
    ]),
  );
}

// ---------- 选中文字后的小工具条 ----------

const bubbleFx = StateEffect.define<boolean>();

function bubbleField(opts: PopOptions) {
  const tip = (state: EditorState): Tooltip => ({ pos: state.selection.main.from, above: true, create: (view) => bubbleView(view, opts) });
  return StateField.define<Tooltip | null>({
    create: () => null,
    update(v, tr) {
      const sel = tr.state.selection.main;
      for (const e of tr.effects) if (e.is(bubbleFx)) return e.value && !sel.empty ? tip(tr.state) : null;
      if (!v || sel.empty) return null;
      // 加了粗体这些：跟着选中的字挪
      return v.pos === sel.from ? v : tip(tr.state);
    },
    provide: (f) => showTooltip.from(f),
  });
}

const BUBBLE: [string, IconName, string][] = [
  ["bold", "bold", T.tool.bold],
  ["heading", "heading", T.tool.heading],
  ["todo", "square-check", T.tool.todo],
  ["link", "link", T.tool.link],
  ["code", "code", T.tool.code],
];

function bubbleView(view: EditorView, opts: PopOptions): TooltipView {
  const dom = document.createElement("div");
  dom.className = "lp-pop lp-bubble";
  const btn = ([id, ic, tip]: [string, IconName, string]) => `<button type="button" data-b="${id}" title="${tip}" aria-label="${tip}">${icon(ic)}</button>`;
  dom.innerHTML =
    BUBBLE.map(btn).join("") +
    `<span class="dv"></span>` +
    (opts.onRemindText ? btn(["remind", "bell-plus", T.tool.remind]).replace('type="button"', 'type="button" class="hi"') : "") +
    btn(["copy", "copy", T.tool.copy]);
  dom.addEventListener("mousedown", (e) => e.preventDefault());
  dom.addEventListener("click", (e) => {
    const b = (e.target as HTMLElement).closest<HTMLElement>("[data-b]");
    if (!b) return;
    const sel = view.state.selection.main;
    const text = view.state.sliceDoc(sel.from, sel.to);
    switch (b.dataset.b) {
      case "bold":
        toggleMark("**")(view);
        break;
      case "heading":
        heading(2)(view);
        break;
      case "todo":
        toggleTodo(view);
        break;
      case "link":
        insertLink(view);
        break;
      case "code":
        toggleMark("`")(view);
        break;
      case "remind":
        opts.onRemindText?.(text.replace(/\s+/g, " ").trim(), b);
        view.dispatch({ effects: bubbleFx.of(false) });
        return;
      case "copy":
        void copyText(text).then(() => opts.say(T.copied(text.replace(/\s+/g, " "))));
        view.dispatch({ effects: bubbleFx.of(false) });
        return;
    }
    view.focus();
  });
  return { dom };
}

export function popups(opts: PopOptions) {
  const bubble = bubbleField(opts);
  const show = (view: EditorView) => {
    // 等编辑器把选中的范围定下来再看
    setTimeout(() => {
      if (!view.state.selection.main.empty && view.hasFocus) view.dispatch({ effects: bubbleFx.of(true) });
    });
  };
  return [
    slashState(opts),
    slashKeys(opts),
    bubble,
    EditorView.domEventHandlers({
      mouseup: (_e, view) => void show(view),
      keyup: (e, view) => void (e.shiftKey || (e.ctrlKey && (e.key === "a" || e.key === "A")) ? show(view) : undefined),
    }),
  ];
}
