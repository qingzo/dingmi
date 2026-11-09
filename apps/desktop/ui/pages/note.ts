import "../styles/ui.css";
import "../styles/note.css";
import { scaleWithFontSize } from "../lib/look";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, errText, newNote, newReminder, type Note, type Reminder, type Settings } from "../lib/api";
import { lightboxFrom } from "../lib/lightbox";
import { attView } from "../lib/official";
import { createNoteEditor, EDIT_KEYS, fillToolbar, type NoteEditor } from "../lib/editor";
import { parseLocal } from "../lib/parse";
import { askRemind } from "../lib/remindpop";
import { askTidy } from "../lib/tidyask";
import { plain, renderMd, setAttachmentDir, setFileInfo, setFileSaveButton, setTodo, shrunkMuch } from "@shared/md";

// 文件卡片旁边放「另存为…」
setFileSaveButton(true);
import { captureDrafts, tableClick, tableInput, tableKey, tableStates, type TableCtx } from "@shared/tables";
import { icon, type IconName } from "@shared/icons";
import { $, ago, copyText, debounce, esc, fireAt, isRecurring, noteCatDef, noteCatList, setNoteCats, tagColor, toast } from "@shared/util";
import { T, whenShort } from "../text/notes";

scaleWithFontSize();

// 单独的笔记窗口（U7）：左边目录，中间是笔记（工具栏、标题、分类和关联提醒、实时预览的正文），右边可以打开「对照着看」（跟着一起滚）。
// 点进去就能改，自动保存；别的电脑或主窗口改了这条笔记：这里没改过就直接换上，改过了就问用哪一版。
// 窗口开着时，主窗口里这条笔记只能看（见 main.ts 的 noteWins）。

const win = getCurrentWindow();
const id = Number(new URLSearchParams(location.search).get("id"));
const MAX_ATTACHMENT = 50 * 1024 * 1024;
const LAYOUT_KEY = "note-window";

const titleEl = $<HTMLInputElement>("#title");
const stateEl = $("#state");
const bar = $("#nbar");
const aibar = $("#aibar");
const outline = $("#outline");
const preview = $("#preview");
const col = $("#ncol");

let note: Note;
let ed: NoteEditor;
let settings: Settings | null = null;
let reminders: Reminder[] = [];
/** 上次存进去的正文：数据变了时拿它判断是不是别处改的、这里有没有没存的修改 */
let savedBody = "";
let savedTitle = "";
/** 笔记被删了：这里的修改不再保存 */
let gone = false;
/** 别处也改了、还没选用哪一版：先不自动保存，免得不等选就盖掉那边的 */
let conflict: Note | null = null;
let pendingSave = false;
/** 分类：这里选的（还没存） */
let tag: string | null = null;

// ---------- 按钮上的图标和提示 ----------

function iconBtn(sel: string, ic: IconName, tip: string) {
  const b = $(sel);
  b.innerHTML = icon(ic);
  b.title = tip;
  b.setAttribute("aria-label", tip);
}
iconBtn("#t-outline", "panel-left", T.outlineTip);
iconBtn("#t-pin", "pin", T.pin);
iconBtn("#t-preview", "panel-right", T.preview);
iconBtn("#t-focus", "fullscreen", T.focusMode);
iconBtn("#w-min", "minus", T.min);
iconBtn("#w-max", "square", T.max);
iconBtn("#w-close", "x", T.close);
iconBtn("#b-more", "ellipsis", T.more);
$("#b-tidy").innerHTML = `${icon("sparkles")}${T.aiButton}`;
titleEl.placeholder = T.title;
titleEl.setAttribute("aria-label", T.title);
outline.setAttribute("aria-label", T.outline);

// ---------- 布局：目录、对照着看开没开，窗口多大（只记在这台电脑） ----------

interface Layout {
  outline: boolean;
  preview: boolean;
  w: number;
  h: number;
}
function layout(): Layout {
  const d: Layout = { outline: true, preview: false, w: 980, h: 680 };
  try {
    return { ...d, ...JSON.parse(localStorage.getItem(LAYOUT_KEY) ?? "{}") };
  } catch {
    return d;
  }
}
function saveLayout(p: Partial<Layout>) {
  try {
    localStorage.setItem(LAYOUT_KEY, JSON.stringify({ ...layout(), ...p }));
  } catch {
    // 记不住就算了，下次用默认的
  }
}

// ---------- 保存 ----------

/** 右上角「已保存」那一小块 */
function setState(text: string, kind: "" | "bad" | "typing" = "") {
  stateEl.innerHTML = `${icon("check")}${esc(text)}`;
  stateEl.className = "saved" + (kind ? " " + kind : "");
}

/** force：选了「保留我的」，不再检查别处有没有改过 */
async function save(force = false) {
  pendingSave = false;
  if (gone) return;
  if (conflict && !force) return setState(T.pickFirst, "bad");
  const title = titleEl.value;
  const body = ed.value;
  if (body === savedBody && title === savedTitle && tag === note.tag) return setState(T.saved);
  try {
    // 存之前看一眼：上次存过之后别处改过了（通知可能还没到），就先问，不直接盖上去
    if (!force) {
      const fresh = await api.getNote(id);
      if (!fresh) return void onDataChanged();
      if (fresh.body !== savedBody) return showConflict(fresh);
    }
    const saved = await api.saveNote({ ...note, title, body, tag });
    note = saved;
    savedBody = body;
    savedTitle = title;
    setState(T.saved);
    void win.setTitle(saved.title || T.winTitle);
    $("#wtitle").textContent = saved.title;
    drawMeta();
  } catch (e) {
    setState(T.saveFail(errText(e)), "bad");
  }
}
const saveLater = debounce(() => void save(), 600);

function touch() {
  pendingSave = true;
  setState(T.saving, "typing");
  saveLater();
  refreshLater();
}

// ---------- 分类、改过的时间、关联提醒、字数 ----------

function drawMeta() {
  const cat = noteCatDef(tag);
  const linked = note.reminder_id ? reminders.find((r) => r.id === note.reminder_id) : undefined;
  const n = plain(ed?.value ?? note.body).replace(/\s/g, "").length;
  $("#meta").innerHTML = [
    noteCatList().length || cat
      ? `<button type="button" class="${cat ? "tag" : "tag nocat"}" id="m-cat" style="${cat ? tagColor(cat.color) : ""}" title="${T.catTip}" aria-haspopup="menu">${cat ? esc(cat.name) : `${icon("plus")}${T.noCat}`}</button>`
      : "",
    `<span>${esc(T.edited(ago(note.updated_at)))}</span>`,
    linked ? `<button type="button" class="lk" id="m-rem" title="${T.linkedTip}">${icon("bell")}${esc(T.linked(`${whenShort(fireAt(linked))} ${linked.title}`))}</button>` : "",
    `<span class="num">${T.words(n)}</span>`,
  ].join("");
}

/** 小菜单：贴在 anchor 下面，点了某一项或者点别处就关 */
function popMenu(anchor: HTMLElement, html: string, onPick: (el: HTMLElement) => void) {
  document.querySelector(".nmenu")?.remove();
  const m = document.createElement("div");
  m.className = "menu nmenu";
  m.setAttribute("role", "menu");
  m.innerHTML = html;
  document.body.append(m);
  const r = anchor.getBoundingClientRect();
  m.style.left = `${Math.max(8, Math.min(r.left, innerWidth - m.offsetWidth - 8))}px`;
  m.style.top = `${r.bottom + 4}px`;
  const close = (e?: Event) => {
    if (e && (m.contains(e.target as Node) || anchor.contains(e.target as Node))) return;
    m.remove();
    document.removeEventListener("pointerdown", close, true);
  };
  document.addEventListener("pointerdown", close, true);
  m.addEventListener("click", (e) => {
    const el = (e.target as HTMLElement).closest<HTMLElement>(".mi");
    if (!el) return;
    close();
    onPick(el);
  });
}

$("#meta").addEventListener("click", (e) => {
  const el = e.target as HTMLElement;
  const cat = el.closest<HTMLElement>("#m-cat");
  if (cat) {
    const list = noteCatList();
    popMenu(
      cat,
      list.length
        ? `<button class="mi" data-cat="">${T.catNone}</button>${list.map((c) => `<button class="mi" data-cat="${esc(c.id)}"><span class="tag" style="${tagColor(c.color)}">${esc(c.name)}</span>${c.id === tag ? icon("check") : ""}</button>`).join("")}`
        : `<div class="mi">${T.catEmpty}</div>`,
      (mi) => {
        if (mi.dataset.cat === undefined) return;
        tag = mi.dataset.cat || null;
        drawMeta();
        touch();
      },
    );
  }
  if (el.closest("#m-rem") && note.reminder_id) void api.openMainNav("reminders", undefined, note.reminder_id).catch((err) => toast(errText(err)));
});

// ---------- 目录、对照着看 ----------

interface Heading {
  level: number;
  text: string;
  line: number;
}
function headings(src: string): Heading[] {
  const out: Heading[] = [];
  let fence = false;
  src.split("\n").forEach((raw, line) => {
    if (/^\s*```/.test(raw)) fence = !fence;
    const m = !fence && raw.match(/^\s*(#{1,3})\s+(.+?)\s*#*\s*$/);
    if (m) out.push({ level: m[1].length, text: plain(m[2]), line });
  });
  return out;
}

function refresh() {
  const hs = headings(ed.value);
  outline.innerHTML = `<div class="oh">${T.outline}</div>${
    hs.length ? hs.map((h) => `<button class="l${h.level}" data-line="${h.line}" title="${esc(h.text)}">${esc(h.text)}</button>`).join("") : `<p class="oe">${T.outlineEmpty}</p>`
  }`;
  drawMeta();
  markOutline();
  if (!preview.hidden) renderPreview();
}
const refreshLater = debounce(refresh, 250);

/** 目录里亮着正在看的那一节 */
function markOutline() {
  if (!ed) return;
  const y = col.getBoundingClientRect().top + 60 - ed.view.documentTop;
  const line = y <= 0 ? -1 : ed.view.state.doc.lineAt(ed.view.lineBlockAtHeight(y).from).number - 1;
  const btns = [...outline.querySelectorAll<HTMLElement>("[data-line]")];
  const cur = btns.filter((b) => Number(b.dataset.line) <= line).pop();
  btns.forEach((b) => b.classList.toggle("on", b === cur));
}

/** 重画「对照着看」，停在原来滚到的地方 */
function renderPreview() {
  const top = preview.scrollTop;
  // 表格里改到一半的一行，重画后还在
  captureDrafts(preview, note.uid);
  preview.innerHTML = ed.value.trim() ? renderMd(ed.value, "", undefined, tableStates(note.uid)) : `<p class="pe">${T.previewEmpty}</p>`;
  preview.scrollTop = top;
}

/** 「对照着看」里的表格：改一行、加一行、删一行改到编辑器的正文里，照常自动保存，Ctrl + Z 能撤回 */
const tableCtx = (): TableCtx => ({
  uid: note.uid,
  body: () => ed.value,
  render: renderPreview,
  save: async (body) => ed.setValue(body),
  toast,
});

/** 「对照着看」跟着正文滚：按滚动的比例对上 */
function syncScroll() {
  markOutline();
  if (preview.hidden) return;
  const ratio = col.scrollHeight > col.clientHeight ? col.scrollTop / (col.scrollHeight - col.clientHeight) : 0;
  preview.scrollTop = ratio * (preview.scrollHeight - preview.clientHeight);
}

function applyLayout() {
  const L = layout();
  outline.hidden = !L.outline;
  preview.hidden = !L.preview;
  $("#t-outline").setAttribute("aria-pressed", String(L.outline));
  $("#t-preview").setAttribute("aria-pressed", String(L.preview));
  if (L.preview) {
    renderPreview();
    syncScroll();
  }
}

// ---------- 专注写作：全屏，只留正文 ----------

async function setFocusMode(on: boolean) {
  document.body.classList.toggle("focus", on);
  await win.setFullscreen(on).catch(() => {});
  ed.focus();
}

// ---------- 附件 ----------

async function insertFiles(files: File[]) {
  for (const f of files) {
    if (f.size > MAX_ATTACHMENT) {
      toast(T.tooBig(f.name));
      continue;
    }
    try {
      const image = f.type.startsWith("image/");
      const file = await api.saveAttachment(new Uint8Array(await f.arrayBuffer()), f.type, note.uid, image ? "" : f.name);
      const label = f.name.replace(/[[\]]/g, "_") || "文件";
      ed.insertLine(image ? `![](att:${file})` : `[${label}](att:${file})`);
    } catch (err) {
      toast(T.notSaved(f.name, errText(err)));
    }
  }
  await loadAtts();
  ed.refresh();
  ed.focus();
}

async function loadAtts() {
  const [atts, devices] = await Promise.all([api.listAttachments().catch(() => []), api.listDevices().catch(() => [])]);
  const map = new Map(atts.map((a) => [a.file, attView(a, devices)]));
  setFileInfo((f) => map.get(f));
}

// ---------- 待办和提醒 ----------

/** 这条待办设过的提醒（同一条笔记、同一行字）；all：完成了的也算 */
const todoRem = (text: string, all = false) => reminders.find((r) => r.note_uid === note.uid && r.todo === text && (all || !r.done));

async function loadReminders() {
  reminders = await api.listReminders().catch(() => reminders);
}

async function afterRemind(msg: string) {
  toast(msg);
  await loadReminders();
  ed.refresh();
  drawMeta();
}

/** 待办后面的「设提醒」/ 提醒时间：打一句话设时间，设过的能改、能不提醒了 */
async function remindTodo(text: string, anchor: HTMLElement) {
  const r = todoRem(text);
  const res = await askRemind({ anchor, due: r ? fireAt(r) : undefined, canDelete: !!r });
  if (!res) return;
  try {
    if (res === "delete" && r) {
      await api.deleteReminder(r.id);
      return afterRemind(T.rpDeleted);
    }
    if (res === "delete") return;
    if (pendingSave) await save();
    await api.saveReminder(r ? { ...r, due_at: res.due, snoozed_until: null, vague: false } : newReminder({ title: text, note_uid: note.uid, todo: text, due_at: res.due }));
    await afterRemind(T.rpSaved(whenShort(res.due)));
  } catch (err) {
    toast(T.notDone(errText(err)));
  }
}

/** 勾上 / 取消待办：设过提醒的，提醒跟着完成 / 取消完成。先存好，提醒那边就不会再去改这一行 */
async function todoToggled(text: string, done: boolean) {
  const r = todoRem(text, true);
  if (!r || isRecurring(r.repeat) || r.done === done) return;
  await save();
  try {
    await api.reminderAction(r.id, done ? "done" : "undo");
    if (done) toast(T.rpTodoDone);
    await loadReminders();
  } catch (err) {
    toast(T.notDone(errText(err)));
  }
}

/** 选中的文字设成提醒：走随手记的理解（时间、周期、紧急），关联这条笔记 */
async function remindText(text: string, anchor: HTMLElement) {
  if (!text) return;
  const parsed = parseLocal(text).find((i) => i.type === "reminder");
  const title = parsed?.type === "reminder" && parsed.title && text.includes(parsed.title) ? parsed.title : text;
  const res = await askRemind({ anchor, title, phrase: parsed ? text : "" });
  if (!res || res === "delete") return;
  try {
    if (pendingSave) await save();
    const same = parsed?.type === "reminder" && parsed.due === res.due;
    await api.saveReminder(
      newReminder({
        title,
        note_uid: note.uid,
        due_at: res.due,
        ...(same && parsed?.type === "reminder" ? { repeat: parsed.repeat, urgent: parsed.urgent, tag: parsed.tag, detail: parsed.detail } : {}),
      }),
    );
    await afterRemind(T.rpSaved(whenShort(res.due)));
  } catch (err) {
    toast(T.notDone(errText(err)));
  }
}

// ---------- 别处改了这条笔记 ----------

function showBar(kind: "warn" | "err", title: string, sub: string, btns: string) {
  bar.innerHTML = `<div class="banner ${kind}">${icon(kind === "err" ? "circle-alert" : "triangle-alert")}<div><b>${esc(title)}</b>${sub ? esc(sub) : ""}</div>${btns}</div>`;
  bar.hidden = false;
}

async function onDataChanged() {
  const [fresh] = await Promise.all([api.getNote(id).catch(() => undefined), loadReminders(), loadAtts()]);
  ed?.refresh();
  if (fresh === undefined) return;
  if (!fresh) {
    gone = true;
    setState("");
    showBar("err", T.noteGone, "", `<button class="btn sm" data-b="close">${T.closeWin}</button>`);
    return;
  }
  // 置顶跟着变；分类这里没动过的话也跟着变
  const tagUntouched = tag === note.tag;
  if (tagUntouched) tag = fresh.tag;
  note = { ...note, pinned_at: fresh.pinned_at, reminder_id: fresh.reminder_id, tag: tagUntouched ? fresh.tag : note.tag };
  $("#t-pin").setAttribute("aria-pressed", String(!!fresh.pinned_at));
  drawMeta();
  if (fresh.body === savedBody && fresh.title === note.title) return;
  if (fresh.body === savedBody) {
    // 只改了标题
    if (titleEl.value === savedTitle) {
      titleEl.value = savedTitle = fresh.title;
      note = fresh;
    }
    return;
  }
  if (ed.value === savedBody && titleEl.value === savedTitle) {
    // 这里没有没存的修改：直接换上
    note = fresh;
    savedBody = fresh.body;
    savedTitle = titleEl.value = fresh.title;
    ed.setValue(fresh.body);
    pendingSave = false;
    setState(T.usedTheirs);
    refresh();
    return;
  }
  showConflict(fresh);
}

/** 两边都改了：问用哪一版，选之前不自动保存 */
function showConflict(fresh: Note) {
  conflict = fresh;
  setState(T.pickFirst, "bad");
  showBar("warn", T.conflict, T.conflictSub, `<button class="btn sm" data-b="theirs">${T.useTheirs}</button><button class="btn sm pri" data-b="mine">${T.keepMine}</button>`);
}

bar.addEventListener("click", async (e) => {
  const b = (e.target as HTMLElement).closest<HTMLElement>("[data-b]")?.dataset.b;
  if (!b) return;
  if (b === "close") return void win.close();
  const fresh = conflict;
  if (!fresh) return;
  conflict = null;
  if (b === "theirs") {
    note = fresh;
    tag = fresh.tag;
    savedBody = fresh.body;
    savedTitle = titleEl.value = fresh.title;
    ed.setValue(fresh.body);
    pendingSave = false;
    setState(T.usedTheirs);
  } else {
    // 保留我的：当作那边的已经看过了，存一次盖上去
    savedBody = fresh.body;
    await save(true);
  }
  bar.hidden = true;
  refresh();
});

// ---------- 让 AI 整理：整理完直接换上，顶上一条「整理好了」能撤销；少了一大截的先问一句 ----------

let beforeTidy: { title: string; body: string } | null = null;
let tidyPending: { title: string; body: string } | null = null;

function showAiBar(kind: "ok" | "warn", title: string, sub: string, btns: string) {
  aibar.className = `banner ${kind}`;
  aibar.innerHTML = `${icon(kind === "ok" ? "sparkles" : "triangle-alert")}<div><b>${esc(title)}</b>${esc(sub)}</div>${btns}`;
  aibar.hidden = false;
}

const applyTidy = (r: { title: string; body: string }) => {
  beforeTidy = { title: titleEl.value, body: ed.value };
  titleEl.value = r.title;
  ed.setValue(r.body);
  touch();
  showAiBar("ok", T.aiDone, ` · ${T.aiDoneSub}`, `<button class="btn sm" data-ai="undo">${T.aiUndo}</button><button class="btn sm ghost icon" data-ai="x" aria-label="${T.close}">${icon("x")}</button>`);
};

$("#b-tidy").onclick = async () => {
  const btn = $<HTMLButtonElement>("#b-tidy");
  if (!ed.value.trim()) return toast(T.aiEmpty);
  if (!settings?.ai_key) return toast(T.aiNoKey);
  const ask = await askTidy(btn);
  if (ask === null) return;
  btn.disabled = true;
  btn.innerHTML = `<span class="spin"></span>${T.aiBusy}`;
  aibar.hidden = true;
  try {
    const r = await api.aiTidyNote(titleEl.value, ed.value, ask);
    if (shrunkMuch(ed.value, r.body)) {
      tidyPending = r;
      showAiBar("warn", T.aiShrunk, ` · ${T.aiShrunkSub}`, `<button class="btn sm" data-ai="use">${T.aiUse}</button><button class="btn sm ghost" data-ai="drop">${T.aiDrop}</button>`);
    } else applyTidy(r);
  } catch (err) {
    toast(T.aiFail(errText(err)));
  } finally {
    btn.disabled = false;
    btn.innerHTML = `${icon("sparkles")}${T.aiButton}`;
  }
};
aibar.addEventListener("click", (e) => {
  const a = (e.target as HTMLElement).closest<HTMLElement>("[data-ai]")?.dataset.ai;
  if (!a) return;
  if (a === "undo" && beforeTidy) {
    titleEl.value = beforeTidy.title;
    ed.setValue(beforeTidy.body);
    beforeTidy = null;
    touch();
  } else if (a === "use" && tidyPending) {
    applyTidy(tidyPending);
    tidyPending = null;
    return;
  }
  tidyPending = null;
  aibar.hidden = true;
});

// 没填 AI Key 时按钮灰着，鼠标停上去说去哪填
function tidyReady() {
  const b = $<HTMLButtonElement>("#b-tidy");
  b.classList.toggle("off", !settings?.ai_key);
  b.title = settings?.ai_key ? "" : T.aiNoKey;
}

// ---------- 按钮 ----------

$("#t-outline").onclick = () => {
  saveLayout({ outline: !layout().outline });
  applyLayout();
};
$("#t-preview").onclick = () => {
  saveLayout({ preview: !layout().preview });
  applyLayout();
};
$("#t-focus").onclick = () => setFocusMode(!document.body.classList.contains("focus"));
$("#t-pin").onclick = async () => {
  const on = !note.pinned_at;
  try {
    await api.pinNote(id, on);
    note.pinned_at = on ? new Date().toISOString() : null;
    $("#t-pin").setAttribute("aria-pressed", String(on));
    $("#t-pin").title = on ? T.unpin : T.pin;
    toast(on ? T.pinned : T.unpinned);
  } catch (err) {
    toast(T.notDone(errText(err)));
  }
};
$("#w-min").onclick = () => win.minimize();
$("#w-max").onclick = () => win.toggleMaximize();
$("#w-close").onclick = () => win.close();
$("#b-more").onclick = (e) =>
  popMenu(
    e.currentTarget as HTMLElement,
    `<button class="mi" data-m="fmt" title="${T.fmtTip}">${icon("wand-sparkles")}${T.fmt}<kbd>Shift Alt F</kbd></button><button class="mi" data-m="keys">${icon("keyboard")}${T.keys}</button>`,
    (mi) => {
      if (mi.dataset.m === "fmt") ed.format();
      else if (mi.dataset.m === "keys") showKeys();
    },
  );

/** 快捷键一览 */
function showKeys() {
  const s = document.createElement("div");
  s.className = "scrim";
  s.innerHTML = `<div class="dialog"><h4>${T.keys}</h4><ul class="keys">${EDIT_KEYS.split("\n").map((k) => `<li>${esc(k)}</li>`).join("")}</ul><div class="acts"><button class="btn pri">${T.close}</button></div></div>`;
  s.addEventListener("click", (e) => {
    if (e.target === s || (e.target as HTMLElement).closest("button")) s.remove();
  });
  document.body.append(s);
}

outline.addEventListener("click", (e) => {
  const b = (e.target as HTMLElement).closest<HTMLElement>("[data-line]");
  if (b) ed.gotoLine(Number(b.dataset.line));
});
// 「对照着看」里的点击：链接、复制按钮、待办的勾、表格（开着的表格菜单点别处也要关上，所以挂在整个页面上）
document.addEventListener("click", async (e) => {
  const el = e.target as HTMLElement;
  if (preview.hidden) return;
  if (preview.contains(el)) {
    const link = el.closest<HTMLElement>("[data-href]");
    if (link) return void api.openUrl(link.dataset.href!).catch((err) => toast(errText(err)));
    const cp = el.closest<HTMLElement>("[data-copy]");
    if (cp) {
      await copyText(cp.dataset.copy!);
      return toast(T.copied(cp.dataset.copy!.replace(/\s+/g, " ")));
    }
    const box = el.closest<HTMLElement>("[data-box]");
    if (box) {
      const done = !box.closest("li")?.classList.contains("checked");
      const text = box.closest("li")?.querySelector(".tt")?.textContent?.trim() ?? "";
      ed.setValue(setTodo(ed.value, Number(box.dataset.box), done));
      return void todoToggled(text, done);
    }
    const att = el.closest<HTMLElement>("[data-att]");
    if (att?.tagName === "IMG") return lightboxFrom(att, toast);
    if (att) return void api.openAttachment(att.dataset.att!).then((shown) => shown && toast(T.fileRevealed), (err) => toast(errText(err)));
    const sv = el.closest<HTMLElement>("[data-saveas]");
    if (sv) {
      const p = await api.saveAttachmentAs(sv.dataset.saveas!).catch((err) => void toast(errText(err)));
      if (p) toast(T.lbSaved(p));
      return;
    }
    const fc = el.closest<HTMLElement>("[data-file]");
    if (fc) return void api.openAttachment(fc.dataset.file!).then((shown) => shown && toast(T.fileRevealed), (err) => toast(errText(err)));
  }
  if (el.closest("#preview .tblock") || preview.querySelector("[data-tmenu][aria-expanded=true]")) await tableClick(el, tableCtx());
});
preview.addEventListener("input", (e) => tableInput(e.target as HTMLElement, tableCtx()));
preview.addEventListener("keydown", (e) => tableKey(e, tableCtx()));
titleEl.addEventListener("input", touch);
titleEl.addEventListener("keydown", (e) => {
  if (e.key === "Enter" || e.key === "ArrowDown") {
    e.preventDefault();
    ed.view.dispatch({ selection: { anchor: 0 } });
    ed.focus();
  }
});
col.addEventListener("scroll", syncScroll, { passive: true });

document.addEventListener("keydown", (e) => {
  if (e.defaultPrevented) return;
  if (e.key === "F11") {
    e.preventDefault();
    void setFocusMode(!document.body.classList.contains("focus"));
  } else if (e.key === "Escape" && document.body.classList.contains("focus") && !document.querySelector(".cm-search")) {
    void setFocusMode(false);
  } else if (e.ctrlKey && (e.key === "s" || e.key === "S")) {
    e.preventDefault();
    void save();
  } else if (e.ctrlKey && (e.key === "w" || e.key === "W")) {
    e.preventDefault();
    void win.close();
  } else if (e.ctrlKey && e.shiftKey && (e.key === "P" || e.key === "p")) {
    e.preventDefault();
    $("#t-preview").click();
  }
});

// 关窗口前先存好（点 ✕、Ctrl + W、Alt + F4 都走这里）。还在冲突、没选用哪一版的：这里的内容另存一条冲突副本，两边都不丢
void win.onCloseRequested(async () => {
  if (conflict && !gone) {
    await api.saveNote(newNote({ title: T.conflictCopy(titleEl.value || note.title), body: ed.value, tag })).catch(() => {});
    return;
  }
  if (pendingSave) await save();
});

// 记住窗口大小（最大化、全屏时不记）
void win.onResized(
  debounce(async () => {
    if ((await win.isMaximized()) || (await win.isFullscreen())) return;
    const size = await win.innerSize();
    const scale = await win.scaleFactor();
    saveLayout({ w: Math.round(size.width / scale), h: Math.round(size.height / scale) });
  }, 400),
);

// ---------- 启动 ----------

(async () => {
  const [n, s, dir] = await Promise.all([api.getNote(id), api.getSettings().catch(() => null), api.attachmentDir().catch(() => ""), loadAtts(), loadReminders()]);
  if (!n) {
    await win.close();
    return;
  }
  note = n;
  tag = n.tag;
  settings = s;
  setNoteCats(s?.note_cats);
  setAttachmentDir(dir);
  savedBody = n.body;
  savedTitle = titleEl.value = n.title;
  $("#wtitle").textContent = n.title;
  $("#t-pin").setAttribute("aria-pressed", String(!!n.pinned_at));
  $("#t-pin").title = n.pinned_at ? T.unpin : T.pin;
  tidyReady();
  let lightTools: (on: Set<string>) => void = () => {};
  ed = createNoteEditor($("#ed"), n.body, {
    uid: n.uid,
    onChange: touch,
    onFiles: (files) => void insertFiles(files),
    onMessage: toast,
    todoReminder: (text) => {
      const r = todoRem(text);
      return r ? whenShort(fireAt(r)) : null;
    },
    onTodoRemind: (t, anchor) => void remindTodo(t.text, anchor),
    onTodoToggle: (t) => void todoToggled(t.text, t.done),
    onRemindText: (text, anchor) => void remindText(text, anchor),
    onCursor: (on) => lightTools(on),
    // 钉在上面的工具栏
    topMargin: 64,
  });
  lightTools = fillToolbar($("#bar"), () => ed);
  setState(T.autoSave);
  applyLayout();
  refresh();
  void win.setTitle(n.title || T.winTitle);
  const L = layout();
  await win.setSize(new LogicalSize(L.w, L.h)).catch(() => {});
  await win.center().catch(() => {});
  await win.show();
  await win.setFocus();
  if (n.title) ed.focus();
  else titleEl.focus();
  listen("data-changed", () => void onDataChanged());
  listen<Settings>("settings-changed", (e) => {
    settings = e.payload;
    setNoteCats(e.payload.note_cats);
    tidyReady();
    drawMeta();
  });
})();
