// 实时预览接到 CodeMirror 上：live.ts 算出来的藏符号、换小部件变成装饰；小部件自己的样子和点击。
// 装饰放在 StateField 里（整块换掉的表格、图片要这样）；光标所在的行、编辑器有没有焦点变了才重算

import { Facet, StateEffect, StateField, type EditorState, type Range } from "@codemirror/state";
import { Decoration, EditorView, ViewPlugin, WidgetType, type DecorationSet } from "@codemirror/view";
import { attachmentSrc, fileInfoOf, findTables, renderTable } from "@shared/md";
import { captureDrafts, tableClick, tableInput, tableKey, tableStates, type TableCtx } from "@shared/tables";
import { icon, type IconName } from "@shared/icons";
import { copyText, esc } from "@shared/util";
import { api, errText } from "./api";
import { gridAt, replaceAll, setTodoAt } from "./edcmd";
import { copyImage, openLightbox } from "./lightbox";
import { scan, type Span, type Widget } from "./live";
import { T } from "../text/notes";

/** 一条待办：字和在第几行（从 0 开始，同 md.ts 的 Todo） */
export interface TodoRef {
  text: string;
  line: number;
}

export interface LiveConf {
  /** 这条笔记的 uid：表格的看法记在它上面 */
  uid: string;
  say: (text: string) => void;
  /** 待办设过提醒的：回提醒时间的说法（「周三 10:00」）；没设过回 null */
  todoReminder?: (text: string) => string | null;
  /** 点了待办后面的「设提醒」或者提醒时间 */
  onTodoRemind?: (todo: TodoRef, anchor: HTMLElement) => void;
  /** 勾上 / 取消了一条待办（正文已经改好了） */
  onTodoToggle?: (todo: TodoRef & { done: boolean }) => void;
}

export const liveConf = Facet.define<LiveConf, LiveConf>({ combine: (v) => v[0] ?? { uid: "", say: () => {} } });
/** 编辑器得到 / 失去焦点：没有焦点时整篇都排好版 */
const focusFx = StateEffect.define<boolean>();
/** 重画小部件（提醒、附件下载完了） */
export const refreshFx = StateEffect.define<null>();

interface Live {
  deco: DecorationSet;
  focused: boolean;
  /** 光标所在的行（从 0 开始） */
  active: Set<number>;
  key: string;
  ver: number;
}

function activeLines(state: EditorState, focused: boolean): Set<number> {
  const out = new Set<number>();
  if (!focused) return out;
  for (const r of state.selection.ranges) {
    const a = state.doc.lineAt(r.from).number - 1;
    const b = state.doc.lineAt(r.to).number - 1;
    for (let i = a; i <= b; i++) out.add(i);
  }
  return out;
}

// ---------- 小部件 ----------

/** 小部件现在在第几行（从 1 开始）。文字改过以后小部件可能还是原来那个，所以点的时候再算 */
const lineOf = (view: EditorView, dom: HTMLElement) => view.state.doc.lineAt(view.posAtDOM(dom)).number;

/** 去掉第 lineNo 行（从 1 开始），连同换行 */
function removeLine(view: EditorView, lineNo: number) {
  const doc = view.state.doc;
  const l = doc.line(lineNo);
  const from = lineNo < doc.lines ? l.from : Math.max(0, l.from - 1);
  const to = lineNo < doc.lines ? l.to + 1 : l.to;
  view.dispatch({ changes: { from, to }, userEvent: "delete" });
}

/** 按下去时不让编辑器挪光标、不丢焦点；松开时做事 */
function press(el: HTMLElement, fn: (e: MouseEvent) => void) {
  el.addEventListener("mousedown", (e) => {
    e.preventDefault();
    e.stopPropagation();
  });
  el.addEventListener("click", (e) => {
    e.preventDefault();
    e.stopPropagation();
    fn(e);
  });
}

class CheckW extends WidgetType {
  constructor(readonly done: boolean) {
    super();
  }
  eq(o: CheckW) {
    return o.done === this.done;
  }
  toDOM(view: EditorView) {
    const el = document.createElement("span");
    el.className = "lp-cb" + (this.done ? " on" : "");
    el.setAttribute("role", "checkbox");
    el.setAttribute("aria-checked", String(this.done));
    el.title = T.checkTip(this.done);
    el.innerHTML = this.done ? icon("check") : "";
    press(el, () => {
      const n = lineOf(view, el);
      const r = setTodoAt(view, n);
      if (r) view.state.facet(liveConf).onTodoToggle?.({ text: r.text, line: n - 1, done: r.done });
    });
    return el;
  }
}

class BulletW extends WidgetType {
  eq() {
    return true;
  }
  toDOM() {
    const el = document.createElement("span");
    el.className = "lp-bullet";
    return el;
  }
  ignoreEvent() {
    return false;
  }
}

class HrW extends WidgetType {
  eq() {
    return true;
  }
  toDOM() {
    const el = document.createElement("span");
    el.className = "lp-hr";
    return el;
  }
  ignoreEvent() {
    return false;
  }
}

/** 光秃秃的网址：网站名 ↗，点了用浏览器打开 */
class HostW extends WidgetType {
  constructor(readonly url: string) {
    super();
  }
  eq(o: HostW) {
    return o.url === this.url;
  }
  toDOM(view: EditorView) {
    const el = document.createElement("span");
    el.className = "lp-host";
    let host = this.url;
    try {
      host = new URL(this.url).host;
    } catch {
      // 不像网址就原样显示
    }
    el.title = T.linkTip(this.url);
    el.innerHTML = `${icon(/(^|\.)github\.com$/.test(host) ? "github" : "globe")}${esc(host)} ↗`;
    press(el, () => openUrl(this.url, view.state.facet(liveConf).say));
    return el;
  }
}

export const openUrl = (url: string, say: (t: string) => void) => void api.openUrl(url).catch((err) => say(errText(err)));

/** 待办后面：设过提醒显示时间（橘色小胶囊），没设过鼠标停上去出现虚线的「设提醒」 */
class TodoW extends WidgetType {
  constructor(readonly label: string | null) {
    super();
  }
  eq(o: TodoW) {
    return o.label === this.label;
  }
  toDOM(view: EditorView) {
    const el = document.createElement("button");
    el.type = "button";
    el.className = this.label ? "lp-rmd" : "lp-setr";
    el.title = this.label ? T.remindTip : T.setRemindTip;
    el.innerHTML = this.label ? `${icon("bell")}${esc(this.label)}` : `${icon("bell-plus")}${T.setRemind}`;
    press(el, () => {
      const n = lineOf(view, el);
      const text = view.state.doc.line(n).text.replace(/^\s*[-*+]\s+\[[ xX]\]\s*/, "").trim();
      view.state.facet(liveConf).onTodoRemind?.({ text, line: n - 1 }, el);
    });
    return el;
  }
}

/** 下载失败（还没下载到）的图片：重画时再试 */
const waiting = new Set<string>();

/** 这篇里所有的图片，看大图时左右切 */
function pics(state: EditorState) {
  const out: { file: string; alt: string }[] = [];
  for (const l of state.doc.toString().split("\n")) {
    const m = l.trim().match(/^!\[([^\]]*)\]\(att:([\w.-]+)\)$/);
    if (m) out.push({ file: m[2], alt: m[1] });
  }
  return out;
}

const toolBtn = (act: string, ic: IconName, tip: string) => `<button type="button" data-act="${act}" title="${tip}" aria-label="${tip}">${icon(ic)}</button>`;

class ImageW extends WidgetType {
  constructor(
    readonly file: string,
    readonly alt: string,
    readonly ver: number,
  ) {
    super();
  }
  eq(o: ImageW) {
    return o.file === this.file && o.alt === this.alt && (o.ver === this.ver || !waiting.has(this.file));
  }
  toDOM(view: EditorView) {
    const say = view.state.facet(liveConf).say;
    const el = document.createElement("div");
    el.className = "lp-img";
    // 只在别的设备上的（叮咪同步不传太大的）：读不到时说清楚在哪，不说「正在下载」
    const wait = fileInfoOf(this.file)?.away ?? T.imgLoading;
    el.innerHTML = `<img alt="${esc(this.alt)}" title="${T.imgView}"><div class="lp-imgwait">${esc(wait)}</div><div class="lp-tools">${toolBtn("view", "maximize-2", T.imgView)}${toolBtn(
      "copy",
      "copy",
      T.imgCopy,
    )}${toolBtn("del", "trash-2", T.imgDelete)}</div>`;
    const img = el.querySelector("img")!;
    img.onload = () => {
      waiting.delete(this.file);
      el.classList.remove("wait");
      view.requestMeasure();
    };
    img.onerror = () => {
      waiting.add(this.file);
      el.classList.add("wait");
      view.requestMeasure();
    };
    img.src = attachmentSrc(this.file);
    const look = () => {
      const all = pics(view.state);
      openLightbox(all, Math.max(0, all.findIndex((p) => p.file === this.file)), say);
    };
    press(el, (e) => {
      const act = (e.target as HTMLElement).closest<HTMLElement>("[data-act]")?.dataset.act;
      if (act === "del") return removeLine(view, lineOf(view, el));
      if (el.classList.contains("wait")) return;
      if (act === "copy") copyImage(this.file).then(() => say(T.imgCopied), (err) => say(errText(err)));
      else look();
    });
    return el;
  }
}

/** 能直接打开的类型（白名单，同 Rust 的 links::safe_to_open）；别的只在文件夹里选中 */
export const OPENABLE = /\.(png|jpe?g|gif|webp|bmp|heic|heif|pdf|txt|md|docx|xlsx|pptx|mp3|m4a|wav|flac|aac|ogg|opus|mp4|m4v|mov|webm|mkv|avi)$/i;
/** 文件类型的小方块用哪个分类色 */
const KIND: [RegExp, string][] = [
  [/^pdf$/, "red"],
  [/^(docx?|rtf|odt|wps)$/, "blue"],
  [/^(xlsx?|csv|ods|et)$/, "green"],
  [/^(pptx?|odp|dps|key)$/, "orange"],
  [/^(zip|rar|7z|tar|gz)$/, "purple"],
  [/^(txt|md|log|json|xml|ya?ml)$/, "teal"],
];

class FileW extends WidgetType {
  constructor(
    readonly file: string,
    readonly name: string,
    readonly ver: number,
  ) {
    super();
  }
  eq(o: FileW) {
    return o.file === this.file && o.name === this.name && o.ver === this.ver;
  }
  toDOM(view: EditorView) {
    const say = view.state.facet(liveConf).say;
    const info = fileInfoOf(this.file);
    const ext = (this.name.match(/\.([a-z0-9]{1,5})$/i)?.[1] ?? "").toLowerCase();
    const run = !OPENABLE.test(this.name) || !OPENABLE.test(this.file);
    const c = run ? "gray" : (KIND.find(([re]) => re.test(ext))?.[1] ?? "gray");
    const local = info?.local ?? true;
    const el = document.createElement("div");
    el.className = "lp-file" + (run ? " warn" : "");
    el.title = this.name;
    el.innerHTML = `<span class="fi" style="--c:var(--tc-${c})">${esc((ext || "file").toUpperCase().slice(0, 4))}</span><div class="m"><b>${esc(this.name)}</b><small>${
      run ? `${icon("shield-alert")}${T.fileRunnable}` : `${local ? "" : icon("cloud")}${esc(info?.away ? T.fileMetaAway(info.size, info.away) : T.fileMeta(info?.size, local))}`
    }</small><div class="prog" hidden><i></i></div></div><span class="st" hidden>${T.fileDownloading}</span><button type="button" class="btn sm" data-act="open">${
      run ? T.fileReveal : T.fileOpen
    }</button><button type="button" class="btn sm ghost icon lp-fdel" data-act="del" title="${T.fileDelete}" aria-label="${T.fileDelete}">${icon("trash-2")}</button>`;
    press(el, async (e) => {
      const act = (e.target as HTMLElement).closest<HTMLElement>("[data-act]")?.dataset.act;
      if (act === "del") return removeLine(view, lineOf(view, el));
      if (el.classList.contains("busy")) return;
      // 还没下载的先下载（带进度条），再打开
      const busy = (on: boolean) => {
        el.classList.toggle("busy", on && !local);
        el.querySelector<HTMLElement>(".prog")!.hidden = !(on && !local);
        el.querySelector<HTMLElement>(".st")!.hidden = !(on && !local);
      };
      busy(true);
      try {
        if (await api.openAttachment(this.file)) say(T.fileRevealed);
      } catch (err) {
        say(errText(err));
      } finally {
        busy(false);
      }
    });
    return el;
  }
}

/** 正文里第 n 张表格的改动怎么写回去 */
function tableCtx(view: EditorView): TableCtx {
  const conf = view.state.facet(liveConf);
  return {
    uid: conf.uid,
    body: () => view.state.doc.toString(),
    render: () => renderTables(view),
    save: async (body) => replaceAll(view, body),
    toast: conf.say,
  };
}

const tableHtml = (view: EditorView, n: number) => {
  const t = findTables(view.state.doc.toString())[n];
  return t ? renderTable(t, n, tableStates(view.state.facet(liveConf).uid)(n)) : "";
};

/** 表格换了看法、展开了一行这些：重画编辑器里的表格（不改正文） */
function renderTables(view: EditorView) {
  const uid = view.state.facet(liveConf).uid;
  for (const el of view.contentDOM.querySelectorAll<HTMLElement>(".lp-table")) {
    const body = el.querySelector<HTMLElement>(".lp-tbody")!;
    // 改到一半的一行，重画后还在
    captureDrafts(body, uid);
    body.innerHTML = tableHtml(view, Number(el.dataset.n));
  }
  view.requestMeasure();
}

class TableW extends WidgetType {
  constructor(
    readonly n: number,
    readonly src: string,
  ) {
    super();
  }
  eq(o: TableW) {
    return o.n === this.n && o.src === this.src;
  }
  toDOM(view: EditorView) {
    const say = view.state.facet(liveConf).say;
    const el = document.createElement("div");
    el.className = "lp-table dmd";
    el.dataset.n = String(this.n);
    el.innerHTML = `<div class="lp-ttools"><button type="button" data-tsrc title="${T.tableSourceTip}">${icon("code")}${T.tableSource}</button><button type="button" data-tgridx title="${
      T.tableGridTip
    }">${icon("sheet")}${T.tableGridShort}</button></div><div class="lp-tbody">${tableHtml(view, this.n)}</div>`;
    /** 表格在正文里从第几行开始（从 1 开始） */
    const first = () => view.state.doc.lineAt(view.posAtDOM(el)).number;
    el.addEventListener("mousedown", (e) => {
      // 输入框、改一行的格子里要能点进去；别处不让编辑器挪光标
      if (!(e.target as HTMLElement).closest("input, textarea")) e.preventDefault();
    });
    el.addEventListener("click", async (e) => {
      const t = e.target as HTMLElement;
      if (t.closest("[data-tsrc]")) {
        const l = view.state.doc.line(first() + 2 <= view.state.doc.lines ? first() + 2 : first());
        view.dispatch({ selection: { anchor: l.from + Math.min(2, l.length) }, scrollIntoView: true });
        view.focus();
        return;
      }
      if (t.closest("[data-tgridx], [data-tgrid]")) return void gridAt(view, say, first());
      const cp = t.closest<HTMLElement>("[data-copy]");
      if (cp) return void copyText(cp.dataset.copy!).then(() => say(T.copied(cp.dataset.copy!.replace(/\s+/g, " "))));
      const link = t.closest<HTMLElement>("[data-href]");
      if (link) return openUrl(link.dataset.href!, say);
      await tableClick(t, tableCtx(view));
    });
    el.addEventListener("input", (e) => tableInput(e.target as HTMLElement, tableCtx(view)));
    el.addEventListener("keydown", (e) => {
      tableKey(e, tableCtx(view));
      // 表格里的输入框打字，不给编辑器
      e.stopPropagation();
    });
    return el;
  }
}

// ---------- 装饰 ----------

const hide = Decoration.replace({});
const lineDeco = new Map<string, Decoration>();
const lineOf2 = (cls: string) => lineDeco.get(cls) ?? (lineDeco.set(cls, Decoration.line({ class: cls })), lineDeco.get(cls)!);
const markDeco = new Map<string, Decoration>();
const mark = (cls: string, href?: string) => {
  if (href) return Decoration.mark({ class: cls, attributes: { "data-href": href, title: T.linkTip(href) } });
  return markDeco.get(cls) ?? (markDeco.set(cls, Decoration.mark({ class: cls })), markDeco.get(cls)!);
};

function widget(w: Widget, ver: number): WidgetType {
  switch (w.t) {
    case "check":
      return new CheckW(w.done);
    case "bullet":
      return new BulletW();
    case "hr":
      return new HrW();
    case "host":
      return new HostW(w.url);
    case "image":
      return new ImageW(w.file, w.alt, ver);
    case "file":
      return new FileW(w.file, w.name, ver);
    case "table":
      return new TableW(w.n, w.src);
  }
}

function build(state: EditorState, focused: boolean, ver: number): Live {
  const active = activeLines(state, focused);
  const conf = state.facet(liveConf);
  const r: Range<Decoration>[] = [];
  const spans: Span[] = scan(state.doc.toString(), (i) => active.has(i));
  for (const s of spans) {
    if (s.k === "line") r.push(lineOf2(s.cls).range(s.at));
    else if (s.k === "hide") r.push(hide.range(s.from, s.to));
    else if (s.k === "mark") r.push(mark(s.cls, s.href).range(s.from, s.to));
    else if (s.k === "swap") r.push(Decoration.replace({ widget: widget(s.w, ver) }).range(s.from, s.to));
    else if (s.k === "block") r.push(Decoration.replace({ widget: widget(s.w, ver), block: true }).range(s.from, s.to));
    else if (s.k === "after") r.push(Decoration.widget({ widget: widget(s.w, ver), block: true, side: 1 }).range(s.at));
    else if (s.k === "todo" && !s.done && s.text && conf.onTodoRemind) r.push(Decoration.widget({ widget: new TodoW(conf.todoReminder?.(s.text) ?? null), side: 1 }).range(s.at));
  }
  // 光标所在的那一段：淡淡的底色
  const sel = state.selection.main;
  if (focused && sel.empty) r.push(lineOf2("lp-cur").range(state.doc.lineAt(sel.head).from));
  return { deco: Decoration.set(r, true), focused, active, key: [focused, ...active].join(","), ver };
}

/** 鼠标左键按着：这期间光标换了行、编辑器拿到焦点，都先不改排版（哪行显示符号），松开再改。
 *  不然点下去的一瞬间上面的行变高变矮，鼠标稍微一动编辑器就按新的样子算位置，点到下一行去了 */
let pointerHeld = false;

const live = StateField.define<Live>({
  create: (state) => build(state, false, 0),
  update(v, tr) {
    let focused = v.focused;
    let ver = v.ver;
    for (const e of tr.effects) {
      if (e.is(focusFx)) focused = e.value;
      if (e.is(refreshFx)) ver++;
    }
    // 按着鼠标时只记下焦点，排版不动；松开时会发 refreshFx 重画
    if (pointerHeld && !tr.docChanged && ver === v.ver) return focused === v.focused ? v : { ...v, focused };
    const sel = tr.state.selection.main;
    const key = [focused, ...activeLines(tr.state, focused)].join(",") + (sel.empty ? "" : "~");
    if (!tr.docChanged && ver === v.ver && key === v.key) return v;
    const next = build(tr.state, focused, ver);
    next.key = key;
    return next;
  },
  provide: (f) => EditorView.decorations.from(f, (v) => v.deco),
});

/** 第 line 行（从 0 开始）是不是正显示着 Markdown 符号 */
export const isActiveLine = (state: EditorState, line: number) => state.field(live).active.has(line);

/** 开着的表格菜单（分组、摘要），点别处关上 */
const closeMenus = ViewPlugin.fromClass(
  class {
    constructor(readonly view: EditorView) {
      document.addEventListener("click", this.click, true);
    }
    click = (e: MouseEvent) => {
      const el = e.target as HTMLElement;
      if (!this.view.contentDOM.querySelector("[data-tmenu][aria-expanded=true]")) return;
      if (el.closest(".tmenu, [data-tmenu]")) return;
      void tableClick(el, tableCtx(this.view));
    };
    destroy() {
      document.removeEventListener("click", this.click, true);
    }
  },
);

export function livePreview(conf: LiveConf) {
  return [
    liveConf.of(conf),
    live,
    closeMenus,
    EditorView.focusChangeEffect.of((_s, focusing) => focusFx.of(focusing)),
    EditorView.domEventHandlers({
      // 排好版的链接：点一下用浏览器打开；光标所在的那一段在改字，按住 Ctrl 才打开
      mousedown(e, view) {
        if (e.button === 0 && !pointerHeld) {
          pointerHeld = true;
          window.addEventListener(
            "mouseup",
            () => {
              pointerHeld = false;
              // 按着的时候换了笔记、关了窗口的话，这个编辑器已经不在了
              if (view.dom.isConnected) view.dispatch({ effects: refreshFx.of(null) });
            },
            { once: true, capture: true },
          );
        }
        const a = (e.target as HTMLElement).closest<HTMLElement>(".lp-link[data-href], .lp-url[data-href]");
        if (!a || e.button !== 0) return false;
        const line = view.state.doc.lineAt(view.posAtDOM(a)).number - 1;
        if (!e.ctrlKey && isActiveLine(view.state, line)) return false;
        e.preventDefault();
        openUrl(a.dataset.href!, conf.say);
        return true;
      },
    }),
  ];
}
