// 主窗口（UI设计.md U6、design/ui/u6-main.html）：标题栏、侧栏（今天、提醒、笔记本、提醒分类、专注、设置）和右边的内容面板。
// 这里有今天、提醒、笔记本的列表、回收站、改提醒的抽屉、Ctrl K 搜索；设置在 settings.ts（U9）。
// 笔记的正文和编辑器照旧用 @shared/md、lib/editor.ts（另有人按 U7 在改），这里只换外面的框
import "../styles/ui.css";
import "../styles/mainwin.css";
import { mascotOn, scaleWithFontSize } from "../lib/look";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  api, errText, newNote, newReminder, type AttachmentInfo, type Checklist, type ChecklistId, type FocusInfo, type InboxEntry, type Nav, type Note, type Reminder, type Repeat,
  type RepeatKind, type Settings, type SyncStatus, type Tag, type TermsView, type TrashKind, type UpdateStatus,
} from "../lib/api";
import { attachmentSrc, findTables, highlight, inline, plain, renderMd, setAttachmentDir, setFileInfo, setFileSaveButton, setTableGridButton, setTodo, shrunkMuch, snippet, todos } from "@shared/md";
import { captureDrafts, tableClick, tableInput, tableKey, tableStates, type TableCtx } from "@shared/tables";
import { lightboxFrom } from "../lib/lightbox";
import { attView } from "../lib/official";
import { createNoteEditor, EDIT_KEYS, fillToolbar, type NoteEditor } from "../lib/editor";
import { askRemind } from "../lib/remindpop";
import { openTableGrid } from "../lib/tablegrid";
import { parseLocal, saveParsed, smartParse, type ParsedItem } from "../lib/parse";
import { askTidy } from "../lib/tidyask";
import { dayDiff, whenText } from "../lib/when";
import { icon, type IconName } from "@shared/icons";
import { appIcon, cat, type CatMood } from "@shared/mascot";
import {
  $, $$, addDays, ago, copyText, dayStr, debounce, esc, fireAt, fmtDT, greeting, hotkeyText, isRecurring, noteCatDef, noteCatHtml,
  noteCatList, parseDT, repeatLabel, setNoteCats, setTags, tagColor, tagHtml, tagList, timeOf, weekdayCn,
} from "@shared/util";
import { readRepeatExtras, REPEAT_KINDS, repeatExtrasHtml, showRepeatExtras } from "@shared/repeat-edit";
import { armed, hideModal, hooks, M, modalOpen, S, type View } from "./main-state";
import {
  focusSettingsSearch, openSettings, pct, recordKey, renderAboutRows, renderSettings, settingsOnChange, settingsOnSync, showUpdateNotes, startUpdate, stopRecording, TOO_OLD,
} from "./settings";
import { expiringSoon, showRenew } from "./settings-sync";
import { T } from "../text/main";
import { T as TN, whenShort } from "../text/notes";

scaleWithFontSize();

/** 附件单个最大多大（和电脑端、手机网页、服务器一致） */
const MAX_ATTACHMENT = 50 * 1024 * 1024;
/** 手机发来的先显示几条，多了收起来 */
const INBOX_SHOW = 3;
/** 回收站里放多少天 */
const TRASH_DAYS = 30;
/** 搜索每组最多几条 */
const MAX_HITS = 6;

const appWin = getCurrentWindow();
const I = (n: IconName, cls = "") => icon(n, cls);

// ---------- 框上的字和图标：index.html 里只有框，字从 text/main.ts 填 ----------

type UiKey = keyof typeof T.ui;
$$("[data-t]").forEach((el) => (el.textContent = T.ui[el.dataset.t as UiKey]));
$$("[data-tt]").forEach((el) => {
  el.title = T.ui[el.dataset.tt as UiKey];
  if (!el.textContent?.trim()) el.setAttribute("aria-label", el.title);
});
$$<HTMLInputElement>("[data-tp]").forEach((el) => (el.placeholder = T.ui[el.dataset.tp as UiKey]));
$$("[data-ic]").forEach((el) => el.insertAdjacentHTML("afterbegin", I(el.dataset.ic as IconName)));
// 左上角用程序图标（暖橘方块），不跟风格变
$("#logo").innerHTML = appIcon(22);

$("#w-min").onclick = () => appWin.minimize();
$("#w-max").onclick = () => appWin.toggleMaximize();
$("#w-close").onclick = () => appWin.close();

// ---------- 小提示：底下中间一条，能带「撤销」 ----------

let sayTimer = 0;
function say(msg: string, undo?: () => unknown) {
  const el = $("#toast");
  el.innerHTML = `<span>${esc(msg)}</span>${undo ? `<button class="undo">${T.undo}</button>` : ""}`;
  el.hidden = false;
  if (undo)
    el.querySelector<HTMLElement>(".undo")!.onclick = () => {
      el.hidden = true;
      void Promise.resolve(undo()).catch((err) => say(errText(err)));
    };
  clearTimeout(sayTimer);
  sayTimer = window.setTimeout(() => (el.hidden = true), undo ? 5000 : 2400);
}

// ---------- 数据 ----------

async function load() {
  let atts: AttachmentInfo[];
  [S.reminders, S.notes, S.trash, S.inbox, S.devices, atts] = await Promise.all([
    api.listReminders(),
    api.listNotes(),
    api.listTrash(),
    api.inboxList(),
    api.listDevices(),
    api.listAttachments(),
  ]);
  S.atts = new Map(atts.map((a) => [a.file, a]));
  setFileInfo((f) => {
    const a = S.atts.get(f);
    return a && attView(a, S.devices);
  });
  renderAll();
  autoOrganize();
}

/** 点了文件：用默认程序打开；文件在别的设备上、这台还没下载的，先从服务器下载（几十 MB 要等一会儿） */
async function openFile(card: HTMLElement) {
  const file = card.dataset.file!;
  if (card.classList.contains("busy")) return;
  const info = S.atts.get(file);
  const fm = card.querySelector(".fm");
  if (info && !info.local) {
    card.classList.add("busy");
    if (fm) fm.textContent = T.downloading;
  }
  try {
    if (await api.openAttachment(file)) say(TN.fileRevealed);
  } catch (err) {
    say(errText(err));
  } finally {
    card.classList.remove("busy");
  }
}

/** 「只在 公司电脑」：指定了设备的提醒 */
function onlyOnLabel(only: string[]) {
  return T.onlyOn(only.map((u) => S.devices.find((d) => d.uid === u)?.name ?? T.somePc).join("、"));
}

function renderAll() {
  renderCounts();
  renderToday();
  renderInbox();
  renderReminders();
  renderNotes();
  renderTodos();
  renderTrash();
}

// ---------- 时间怎么写（U13：今天写「14:00」，这周写「周三 15:00」，更远写「11 月 12 日」，跨年才写年份） ----------

const byFire = (a: Reminder, b: Reminder) => fireAt(a).localeCompare(fireAt(b));
const todayKey = () => dayStr(new Date());

const dateCn = (d: Date) => (d.getFullYear() === new Date().getFullYear() ? T.date(d.getMonth() + 1, d.getDate()) : T.dateY(d.getFullYear(), d.getMonth() + 1, d.getDate()));

/** 列表右边那一栏。loose：不在「今天」「明天」组里，明天也要写出来 */
function timeCol(f: string, loose = false): string {
  const d = parseDT(f);
  if (!d) return "";
  const n = dayDiff(d);
  if (n === 0 || (n === 1 && !loose)) return timeOf(f);
  if (n === 1) return `${T.tomorrow} ${timeOf(f)}`;
  if (n === -1) return `${T.yesterday} ${timeOf(f)}`;
  if (n > 1 && n < 7) return `${weekdayCn(d)} ${timeOf(f)}`;
  return dateCn(d);
}

/** 写全：11 月 6 日 周四 14:00 */
function fullWhen(f: string) {
  const d = parseDT(f);
  return d ? `${dateCn(d)} ${weekdayCn(d)} ${timeOf(f)}` : "";
}

/** 做完的时间：今天写钟点，别的天写日期 */
function doneTime(r: Reminder) {
  const s = r.completed_at ?? r.last_done_at ?? fireAt(r);
  const d = parseDT(s);
  return d && dayDiff(d) !== 0 ? dateCn(d) : timeOf(s);
}

function todayData() {
  const now = new Date();
  const tk = todayKey();
  const tm = dayStr(addDays(now, 1));
  const active = S.reminders.filter((r) => !r.done);
  const late = active.filter((r) => (parseDT(fireAt(r)) ?? now) < now).sort(byFire);
  const today = active.filter((r) => fireAt(r).slice(0, 10) === tk && (parseDT(fireAt(r)) ?? now) >= now).sort(byFire);
  const doneToday = S.reminders.filter(
    (r) => (r.done && r.completed_at?.startsWith(tk)) || (isRecurring(r.repeat) && r.last_done_at?.startsWith(tk)),
  );
  const tomorrow = active.filter((r) => fireAt(r).slice(0, 10) === tm).sort(byFire);
  return { late, today, doneToday, tomorrow };
}

const count = (n: number) => (n ? String(n) : "");
const matchTag = (r: Reminder, t: Tag) => r.tag === t || (t === "cycle" && isRecurring(r.repeat));

function renderCounts() {
  const { late, today } = todayData();
  const active = S.reminders.filter((r) => !r.done);
  $("#n-today").textContent = count(late.length + today.length);
  $("#n-rem").textContent = count(active.length);
  $("#n-notes").textContent = count(S.notes.length);
  // 侧栏的提醒分类：点一下只看这一类
  $("#side-tags").innerHTML = tagList()
    .map((t) => {
      const on = S.view === "reminders" && S.rTag === t.id;
      return `<button class="sb-cat${on ? " on" : ""}" data-tagnav="${esc(t.id)}" style="${tagColor(t.color)}" title="${esc(t.name)}"><span class="sq"></span><span class="lb">${esc(t.name)}</span><span class="n num">${count(active.filter((r) => matchTag(r, t.id)).length)}</span></button>`;
    })
    .join("");
  // 侧栏高亮（回收站从设置里进去，仍然亮着「设置」；点了分类时亮分类）
  const cur = S.view === "trash" ? "settings" : S.view;
  $$(".sbar .nv[data-view]").forEach((b) => {
    if (b.dataset.view === cur && !(cur === "reminders" && S.rTag)) b.setAttribute("aria-current", "page");
    else b.removeAttribute("aria-current");
  });
}

// ---------- 提醒的一行 ----------

/** 抽屉里正在改的那条，列表里标出来 */
let editingRid = 0;

/** 一行提醒：圆形完成勾、标题和补充说明、小图标、分类、时间；鼠标停上去时间换成改、推迟、删除 */
function remRow(r: Reminder, o: { done?: boolean; loose?: boolean } = {}) {
  const f = fireAt(r);
  const fd = parseDT(f);
  const late = !o.done && !!fd && fd < new Date();
  const rec = isRecurring(r.repeat);
  const note = r.note_uid ? S.notes.find((n) => n.uid === r.note_uid) : null;
  const ico = (n: IconName, tip: string) => `<span class="ico" title="${esc(tip)}">${I(n)}</span>`;
  const icons = [
    note ? ico("notebook-pen", T.fromNote(note.title)) : "",
    !o.done && r.snoozed_until ? ico("alarm-clock", T.snoozed(r.snooze_count)) : "",
    rec ? ico("repeat", repeatLabel(r.repeat, r.due_at)) : "",
    r.only_on.length ? ico("monitor", onlyOnLabel(r.only_on)) : "",
    r.state === 2 && !o.done ? ico("moon", T.held) : "",
  ].join("");
  const tags = [
    r.vague && !o.done ? `<span class="tag" style="--c:var(--accent)" title="${esc(T.vagueTip)}">${T.vague}</span>` : "",
    r.urgent ? `<span class="tag" style="--c:var(--danger)">${I("zap")}${T.urgent}</span>` : "",
    r.tag && !(rec && r.tag === "cycle") ? tagHtml(r.tag) : "",
  ].join("");
  const btn = (act: string, ic: IconName, tip: string) => `<button class="btn sm ghost icon" data-ract="${act}" title="${tip}" aria-label="${tip}">${I(ic)}</button>`;
  const acts = o.done ? "" : `<span class="hacts">${btn("edit", "pencil", T.edit)}${btn("snooze", "alarm-clock", T.snooze)}${btn("del", "trash-2", T.del)}</span>`;
  const time = o.done ? doneTime(r) : timeCol(f, o.loose);
  return `<div class="it${o.done ? " done" : ""}${r.id === editingRid ? " sel" : ""}" data-rid="${r.id}">
    <button class="task${r.urgent && !o.done ? " urg" : ""}${o.done ? " on" : ""}" data-chk="${r.id}" data-done="${o.done ? 1 : 0}" aria-label="${o.done ? T.undoDone : T.markDone}">${I("check")}</button>
    <span class="t">${esc(r.title)}${r.detail ? `<small>${esc(r.detail)}</small>` : ""}</span>${icons}${tags}<span class="tm num${late ? " late" : ""}">${time}</span>${acts}</div>`;
}

/** 一组：组名（过点了是红的）、几条 */
function grp(title: string, rows: string, o: { n?: number; late?: boolean } = {}) {
  return `<div class="grp"><h5${o.late ? ' class="late"' : ""}>${o.late ? I("circle-alert") : ""}${esc(title)}${o.n ? ` · ${o.n}` : ""}</h5>${rows}</div>`;
}

/** 空白页：团子猫 + 一句话（关掉团子猫时只有字）。同一种空白页重画时猫不再重新动一遍 */
const played = new Set<string>();
function emptyHtml(mood: CatMood, title: string, sub = "", key: string = mood) {
  const play = mood === "sleep" || !played.has(key);
  played.add(key);
  const pic = mascotOn() ? `<span class="cat mascot${play ? " play" : ""}">${cat(mood)}</span>` : "";
  return `<div class="empty">${pic}<b>${title}</b>${sub ? `<span>${sub}</span>` : ""}</div>`;
}

/** 快捷框的快捷键，写成几个按键的样子 */
function flashKeys() {
  const hk = S.settings?.hk_flash;
  return hk ? hotkeyText(hk).split(" + ").map((k) => `<kbd>${esc(k)}</kbd>`).join(" + ") : "";
}

// ---------- 今天 ----------

function renderToday() {
  const now = new Date();
  const mascot = mascotOn();
  const { late, today, doneToday, tomorrow } = todayData();
  const left = late.length + today.length;
  $("#greet").textContent = greeting(now.getHours());
  $("#today-sub").textContent = T.todaySub(dateCn(now), weekdayCn(now), left);
  capHint();

  // 今天：没做的和今天做完的放在一起，按时间排
  const todayRows = [...today.map((r) => ({ r, done: false, k: timeOf(fireAt(r)) })), ...doneToday.map((r) => ({ r, done: true, k: doneTime(r) }))]
    .sort((a, b) => a.k.localeCompare(b.k))
    .map((x) => remRow(x.r, { done: x.done }))
    .join("");
  const tmDate = addDays(now, 1);
  const recent = S.notes.slice(0, 3);
  let html = late.length ? grp(T.late, late.map((r) => remRow(r)).join(""), { late: true }) : "";
  if (left) {
    html += grp(T.today, todayRows);
    if (tomorrow.length) html += grp(T.tomorrowFull(dateCn(tmDate), weekdayCn(tmDate)), tomorrow.map((r) => remRow(r)).join(""));
    html += `<div class="recent">${recent.length ? `${I("file-text", "s16")}${T.recent}${recent.map((n) => `<button data-open-note="${n.id}">${esc(n.title || T.blankNote)}</button>`).join("·")}` : ""}<button class="add" data-newnote>${I("plus", "s16")}${T.newNoteShort}</button></div>`;
  } else {
    const keys = flashKeys();
    html += doneToday.length
      ? emptyHtml("happy", T.allDone(mascot), T.allDoneSub(doneToday.length, mascot), "today-done")
      : emptyHtml("sleep", T.free(mascot), keys ? T.freeSub(keys, mascot) : "");
    if (doneToday.length) html += grp(T.today, todayRows);
    if (tomorrow.length || recent.length) {
      const notes = recent
        .map((n) => `<button class="nrow" data-open-note="${n.id}">${I("file-text", "s16")}<span><b>${esc(n.title || T.blankNote)}</b><small>${esc(plain(n.body).slice(0, 60))}</small></span></button>`)
        .join("");
      html += `<div class="two">${grp(T.tomorrow, tomorrow.map((r) => remRow(r)).join("") || `<p class="none">${T.tomorrowFree}</p>`)}${recent.length ? grp(T.recentTitle, notes) : ""}</div>`;
    }
  }
  $("#today-list").innerHTML = html;
  renderFocus();
}

/** 侧栏的专注：开没开、怎么开的、攒了几条 */
function renderFocus() {
  const f = S.focus;
  $("#focus-sw").setAttribute("aria-checked", String(f.active));
  $("#focus-row").classList.toggle("on", f.active);
  $("#focus-desc").textContent = f.active ? T.focusOn(f.manual, f.held) : S.settings?.auto_focus ? T.focusAuto : "";
  $("#focus-row").title = T.focusTip;
}

async function toggleFocus() {
  if (S.focus.active && !S.focus.manual) return say(T.focusFull);
  S.focus = await api.setFocus(!S.focus.manual);
  renderFocus();
  say(S.focus.manual ? T.focusStarted : T.focusEnded);
}
$("#focus-sw").onclick = toggleFocus;
$("#focus-ico").onclick = toggleFocus;

// ---------- 手机发来的：今天页顶上，有东西才出现 ----------

function inboxRow(e: InboxEntry) {
  const isFile = (f: string) => !!S.atts.get(f) && !S.atts.get(f)!.mime.startsWith("image/");
  const imgs = e.images.filter((f) => !isFile(f));
  const files = e.images.filter(isFile);
  const pics = imgs.map((f) => `<img class="thumb" src="${esc(attachmentSrc(f))}" data-att="${esc(f)}" alt="" title="${T.seeImage}">`).join("");
  const fileBtns = files
    .map((f) => {
      const a = S.atts.get(f)!;
      return `<button class="ib-file" data-file="${esc(f)}" title="${T.openFile}">${I("paperclip", "s16")}<span>${esc(a.name || T.file)}</span><span class="fm">${a.only_on ? esc(attView(a, S.devices).away ?? "") : a.local ? "" : T.notHere}</span></button>`;
    })
    .join("");
  const what = e.text ? esc(e.text) : imgs.length ? T.images(imgs.length) : "";
  const parse = e.text && !e.images.length ? `<button class="btn sm" data-iact="parse">${T.tidyOne}</button>` : "";
  return `<div class="ib" data-ibx="${e.id}">${pics}<span class="t" title="${esc(e.text)}"><span class="tx">${what}</span>${fileBtns}<small>${esc(e.phone || T.phone)} · ${ago(e.sent_at ?? e.received_at)}</small></span>
    ${parse}<button class="btn sm ghost" data-iact="note">${T.toNote}</button><button class="btn sm ghost icon" data-iact="del" title="${T.del}" aria-label="${T.del}">${I("trash-2")}</button></div>`;
}

function renderInbox() {
  const box = $("#inbox");
  if (!S.inbox.length) {
    box.hidden = true;
    box.innerHTML = "";
    return;
  }
  const texts = S.inbox.filter((e) => e.text && !e.images.length);
  const shown = S.inboxOpen ? S.inbox : S.inbox.slice(0, INBOX_SHOW);
  const more = S.inbox.length - shown.length;
  box.hidden = false;
  box.innerHTML = `<h4>${I("smartphone", "s16")}${T.inboxTitle(S.inbox.length)}<span class="sp"></span>${texts.length > 1 ? `<button class="btn sm" data-iact="all">${T.tidyAll}</button>` : ""}</h4>
    ${shown.map(inboxRow).join("")}
    ${more > 0 ? `<button class="ib-more" data-iact="more">${T.moreInbox(more)}</button>` : S.inbox.length > INBOX_SHOW ? `<button class="ib-more" data-iact="less">${T.fold}</button>` : ""}`;
}

$("#inbox").addEventListener("click", async (e) => {
  const b = (e.target as HTMLElement).closest<HTMLButtonElement>("[data-iact]");
  if (!b) return;
  const act = b.dataset.iact;
  if (act === "more" || act === "less") {
    S.inboxOpen = act === "more";
    return renderInbox();
  }
  if (act === "all") {
    const list = S.inbox.filter((x) => x.text && !x.images.length);
    S.inboxParsing = list.map((x) => x.id);
    return startParse(list.map((x) => x.text).join("\n"));
  }
  const id = Number(b.closest<HTMLElement>("[data-ibx]")?.dataset.ibx);
  const entry = S.inbox.find((x) => x.id === id);
  if (!entry) return;
  switch (act) {
    case "parse":
      S.inboxParsing = [id];
      startParse(entry.text);
      break;
    case "note": {
      b.disabled = true;
      b.textContent = T.saving;
      try {
        const nid = await api.inboxToNote(id);
        await load();
        openNote(nid);
        say(T.savedToNote);
      } catch (err) {
        b.disabled = false;
        b.textContent = T.toNote;
        say(T.notSaved(errText(err)));
      }
      break;
    }
    case "del":
      if (!armed(b, T.sureDel)) return;
      await api.inboxDelete(id);
      say(T.deleted);
      break;
  }
});

/** 设置里开了「收到就自动整理」：手机发来的文字自动走一遍「整理一下」，结果仍然要确认 */
const autoTried = new Set<number>();
function autoOrganize() {
  if (!S.settings?.inbox_auto || S.parsing || S.parsed || cap.value.trim() || S.view !== "today") return;
  const e = S.inbox.find((x) => x.text && !x.images.length && !autoTried.has(x.id));
  if (!e) return;
  autoTried.add(e.id);
  S.inboxParsing = [e.id];
  startParse(e.text);
}

// ---------- 上手清单（需求-0.2 第 5.2 节）：今天页顶上一条虚线框，做过的变绿划掉；点一项看说明，✕ 关掉（「设置 → 关于」里找回） ----------

let checklist: Checklist | null = null;
/** 点开看说明的那一项 */
let onbOpen: ChecklistId | null = null;

function renderChecklist() {
  const box = $("#onb");
  const c = checklist;
  if (!c || c.hidden) {
    box.hidden = true;
    box.innerHTML = "";
    return;
  }
  const open = c.items.find((i) => i.id === onbOpen);
  const act = open && T.onb.act[open.id];
  box.hidden = false;
  box.innerHTML = `<div class="onb-prog"><b>${T.onb.prog(c.done, c.total)}</b><i class="prog"><i style="width:${Math.round((c.done / Math.max(1, c.total)) * 100)}%"></i></i></div>
    <div class="steps">${c.items.map((i) => `<button class="st${i.done ? " done" : ""}${i.id === onbOpen ? " on" : ""}" data-ck="${i.id}" aria-expanded="${i.id === onbOpen}">${i.done ? I("check") : ""}${T.onb.name[i.id]}</button>`).join("")}</div>
    <button class="btn sm ghost icon" data-ck="x" title="${T.onb.hide}" aria-label="${T.onb.hide}">${I("x")}</button>
    ${open ? `<div class="onb-more"><span>${T.onb.tip[open.id](hotkeyText(S.settings?.hk_flash ?? ""), hotkeyText(S.settings?.hk_qnote ?? ""))}</span>${act ? `<button class="btn sm" data-cka="${open.id}">${act}</button>` : ""}</div>` : ""}`;
}

$("#onb").addEventListener("click", async (e) => {
  const el = e.target as HTMLElement;
  const a = el.closest<HTMLElement>("[data-cka]")?.dataset.cka as ChecklistId | undefined;
  if (a) return void checklistAct(a).catch((err) => say(errText(err)));
  const ck = el.closest<HTMLElement>("[data-ck]")?.dataset.ck;
  if (!ck) return;
  if (ck === "x") {
    checklist = await api.checklistHide(true).catch(() => checklist);
    say(T.onb.hidden);
  } else onbOpen = onbOpen === ck ? null : (ck as ChecklistId);
  renderChecklist();
});

/** 说明后面的按钮：试一下、去设置 */
async function checklistAct(id: ChecklistId) {
  switch (id) {
    case "flash":
    case "launch":
      return api.openWindow("flash");
    case "card":
      await api.saveReminder(newReminder({ title: T.onb.cardTitle, due_at: fmtDT(new Date(Date.now() + 60000)) }));
      return say(T.onb.cardSet);
    case "note":
      return newNoteHere();
    // 去设置：直接跳到那一行
    case "selection":
      return openSettings("general", "flash_selection");
    case "sync":
      return openSettings("sync", "way");
    case "ai":
      return openSettings("ai", "ai_key");
  }
}

// ---------- 随手记 ----------

const cap = $<HTMLTextAreaElement>("#cap");

/** 输入框跟着内容变高（最多 6 行） */
function fitCap() {
  cap.style.height = "auto";
  cap.style.height = `${Math.min(cap.scrollHeight, 132)}px`;
}

/** 输入框右边的按键提示 */
function capHint() {
  const hint = $("#cap-hint");
  hint.innerHTML = `<kbd>Enter</kbd>${T.capLook}<kbd>Ctrl Enter</kbd>${T.capSave}`;
  hint.title = T.capTip(!!S.settings?.ai_key, S.settings?.hk_flash ? hotkeyText(S.settings.hk_flash) : "");
}

function startParse(text: string) {
  cap.value = text;
  fitCap();
  $('.pg[data-view="today"] .vb').scrollTop = 0;
  void runParse();
}

async function runParse() {
  const text = cap.value.trim();
  if (!text) return cap.focus();
  if (S.parsing) return;
  S.parsing = true;
  const box = $("#parse");
  box.hidden = false;
  box.innerHTML = `<div class="pp-h"><span class="spin"></span>${S.settings?.ai_key ? T.aiThinking : T.thinking}</div>`;
  S.parsed = await smartParse(text, !!S.settings?.ai_key, undefined, S.devices);
  S.parsing = false;
  renderParse();
}

/** Ctrl + 回车：不看预览，直接记下来 */
async function saveNow() {
  if (S.parsed) return saveParsedItems();
  const text = cap.value.trim();
  if (!text || S.parsing) return;
  S.parsing = true;
  try {
    S.parsed = await smartParse(text, !!S.settings?.ai_key, undefined, S.devices);
  } finally {
    S.parsing = false;
  }
  await saveParsedItems();
}

function renderParse() {
  const box = $("#parse");
  const p = S.parsed;
  if (!p || !p.items.length) {
    box.hidden = true;
    box.innerHTML = "";
    return;
  }
  const n = p.items.length;
  const head = p.source === "ai" ? T.parsedAi(n) : p.error ? `<span class="warn">${esc(p.error)}</span>${T.parsedFallback(n)}` : T.parsedLocal(n);
  const hasRem = p.items.some((i) => i.type === "reminder");
  box.hidden = false;
  box.innerHTML = `
    <div class="pp-h">${I("sparkles", "s16")}<span>${head}</span></div>
    ${p.items.map((it, i) => parsedRow(it, i, hasRem)).join("")}
    <div class="pp-f"><button class="btn ghost" data-p="cancel">${T.cancel}</button><button class="btn pri" data-p="save">${T.saveAll}</button></div>`;
}

function parsedRow(it: ParsedItem, i: number, hasRem: boolean) {
  const btns = `<span class="pa"><button class="btn sm ghost icon" data-p="edit" data-i="${i}" title="${T.edit}" aria-label="${T.edit}">${I("pencil")}</button><button class="btn sm ghost icon" data-p="drop" data-i="${i}" title="${T.drop}" aria-label="${T.drop}">${I("x")}</button></span>`;
  if (it.type === "reminder") {
    const tags = [
      isRecurring(it.repeat) ? `<span class="tag" style="--c:var(--tc-purple)">${I("repeat")}${repeatLabel(it.repeat, it.due)}</span>` : "",
      it.urgent ? `<span class="tag" style="--c:var(--danger)">${I("zap")}${T.urgent}</span>` : "",
      it.only_on.length ? `<span class="tag" style="--c:var(--tc-gray)">${I("monitor")}${esc(onlyOnLabel(it.only_on))}</span>` : "",
      tagHtml(it.tag),
    ].join("");
    return `<div class="pp-row">${I("bell", "s16")}<span class="t"><b>${esc(it.title)}</b><small>${T.remindAt(fullWhen(it.due))}${it.detail ? " · " + esc(it.detail) : ""}${it.guess ? ` · <span class="warn">${esc(it.guess)}</span>` : ""}</small></span>${tags}${btns}</div>`;
  }
  const body = plain(it.body);
  return `<div class="pp-row">${I("file-text", "s16")}<span class="t"><b>${esc(it.title)}</b><small>${esc(body.length > 60 ? body.slice(0, 60) + "…" : body)}${it.link && hasRem ? T.linkAbove : ""}</small></span>${noteCatHtml(it.tag)}${btns}</div>`;
}

async function saveParsedItems(btn?: HTMLButtonElement) {
  const items = S.parsed?.items ?? [];
  if (!items.length) return;
  const rem = items.filter((x) => x.type === "reminder").length;
  try {
    if (btn) btn.disabled = true;
    await saveParsed(items);
    for (const id of S.inboxParsing) await api.inboxDone(id).catch(() => {});
    S.inboxParsing = [];
    S.parsed = null;
    cap.value = "";
    fitCap();
    renderParse();
    say(T.savedItems(rem, items.length - rem));
  } catch (err) {
    if (btn) btn.disabled = false;
    say(T.notSaved(errText(err)));
  }
}

$("#parse").addEventListener("click", (e) => {
  const b = (e.target as HTMLElement).closest<HTMLButtonElement>("[data-p]");
  if (!b || !S.parsed) return;
  const i = Number(b.dataset.i);
  const items = S.parsed.items;
  switch (b.dataset.p) {
    case "cancel":
      S.parsed = null;
      S.inboxParsing = [];
      renderParse();
      break;
    case "drop":
      items.splice(i, 1);
      renderParse();
      break;
    case "edit": {
      const it = items[i];
      if (it.type === "reminder") {
        openEditor(
          newReminder({ title: it.title, detail: it.detail, tag: it.tag, due_at: it.due, repeat: it.repeat, urgent: it.urgent, only_on: it.only_on, vague: !!it.guess }),
          {
            draft: (r) => {
              // 改过时间就不算含糊了
              const guess = r.due_at === it.due ? it.guess : "";
              items[i] = { type: "reminder", title: r.title, detail: r.detail, tag: r.tag, due: r.due_at, repeat: r.repeat, urgent: r.urgent, only_on: r.only_on, guess };
              renderParse();
            },
          },
        );
      } else {
        openNoteDraft(it, (n) => {
          items[i] = n;
          renderParse();
        });
      }
      break;
    }
    case "save":
      void saveParsedItems(b);
      break;
  }
});

// 回车先看看理解成什么，Ctrl + 回车直接记下来，Shift + 回车换行（输入法正在选字时的回车不算）
cap.addEventListener("keydown", (e) => {
  if (e.key !== "Enter" || e.shiftKey || e.isComposing) return;
  e.preventDefault();
  if (e.ctrlKey) void saveNow();
  else void runParse();
});
cap.addEventListener("input", fitCap);
$("#manual").onclick = () => {
  const t = cap.value.trim();
  openEditor(newReminder({ title: t.length <= 40 ? t : "" }), {
    afterSave: () => {
      cap.value = "";
      fitCap();
    },
  });
};
$("#new-rem").onclick = () => openEditor(newReminder({ tag: S.rTag }));

// ---------- 改提醒、新建提醒：从右边滑出的抽屉 ----------

const drawer = $<HTMLFormElement>("#drawer");
const ddim = $("#ddim");
let drawerClose: (() => void) | null = null;
// 抽屉里的点击由抽屉自己管（点过的按钮可能已经随抽屉关掉被拿走了），不再往外传
drawer.addEventListener("click", (e) => e.stopPropagation());

function showDrawer(html: string) {
  closeMenu();
  drawer.innerHTML = html;
  drawer.hidden = false;
  ddim.hidden = false;
  drawer.classList.remove("in");
  void drawer.offsetWidth;
  drawer.classList.add("in");
  setTimeout(() => drawer.querySelector<HTMLElement>("input, textarea")?.focus(), 30);
}

function hideDrawer() {
  drawer.hidden = true;
  ddim.hidden = true;
  drawer.innerHTML = "";
  drawer.onsubmit = null;
  drawer.onclick = null;
  drawerClose = null;
  editingRid = 0;
  $$(".it.sel").forEach((x) => x.classList.remove("sel"));
}
ddim.onclick = () => drawerClose?.();

function nextHour() {
  const d = new Date();
  d.setHours(d.getHours() + 1, 0, 0, 0);
  return fmtDT(d);
}

const WEEK = ["一", "二", "三", "四", "五", "六", "日"];

/** 「重复」那一块：选种类，每周选哪几天、每月几号、每隔几天，和 0.2 加的几种（shared/repeat-edit.ts） */
function repeatBoxHtml(rp: Repeat, due: string) {
  const kind = (rp.kind || "none") as RepeatKind;
  const dueDate = parseDT(due) ?? new Date();
  const days = rp.days.length ? rp.days : [((dueDate.getDay() + 6) % 7) + 1];
  return `<select class="field" id="ed-kind" aria-label="${T.d.repeat}">${REPEAT_KINDS.map(([k, l]) => `<option value="${k}" ${k === kind ? "selected" : ""}>${l}</option>`).join("")}</select>
    ${repeatExtrasHtml(rp, kind, dueDate, "inline-row")}
    <div class="rowf" id="ed-week" ${kind === "weekly" ? "" : "hidden"}>${WEEK.map((w, i) => `<button type="button" class="chipsel" data-wd="${i + 1}" aria-pressed="${days.includes(i + 1)}">${T.d.weekday(w)}</button>`).join("")}</div>
    <div class="inline-row" id="ed-month" ${kind === "monthly" ? "" : "hidden"}>${T.d.monthly(`<input class="field" type="number" min="1" max="31" id="ed-monthday" value="${rp.monthday || dueDate.getDate()}">`)}</div>
    <div class="inline-row" id="ed-int" ${kind === "interval" ? "" : "hidden"}>${T.d.interval(`<input class="field" type="number" min="1" max="365" id="ed-every" value="${rp.every || 2}">`)}</div>`;
}

/** 读出「重复」那一块；每周没选哪天时回 null */
function readRepeat(box: HTMLElement): Repeat | null {
  const k = $<HTMLSelectElement>("#ed-kind", box).value as RepeatKind;
  const days = $$("#ed-week [data-wd]", box).filter((c) => c.getAttribute("aria-pressed") === "true").map((c) => Number(c.dataset.wd));
  if (k === "weekly" && !days.length) return null;
  return {
    kind: k,
    days: k === "weekly" ? days : [],
    monthday: k === "monthly" ? Math.min(31, Math.max(1, Number($<HTMLInputElement>("#ed-monthday", box).value) || 1)) : 0,
    every: k === "interval" ? Math.max(1, Number($<HTMLInputElement>("#ed-every", box).value) || 1) : 0,
    month: 0,
    nth: 0,
    ...readRepeatExtras(box, k),
  };
}

/** 只能选一个的小胶囊：再点一次取消 */
function pickOne(box: HTMLElement, b: HTMLElement) {
  const on = b.getAttribute("aria-pressed") !== "true";
  $$("[aria-pressed]", box).forEach((c) => c.setAttribute("aria-pressed", "false"));
  b.setAttribute("aria-pressed", String(on));
}

const catChips = (list: { id: string; name: string; color: string }[], sel: Tag | null) =>
  list.map((t) => `<button type="button" class="chipsel" data-tag="${esc(t.id)}" style="${tagColor(t.color)}" aria-pressed="${sel === t.id}"><span class="sq"></span>${esc(t.name)}</button>`).join("");

/** draft：改随手记整理出来的草稿，只回传结果不存；afterSave：真正存好后 */
function openEditor(r: Reminder, o: { draft?: (r: Reminder) => void; afterSave?: () => void } = {}) {
  let due = r.due_at || nextHour();
  /** 「什么时候」改过；没改过就用原来的时间（过了点的也不会被挪到明天） */
  let dirty = false;
  let understood = true;
  let guess = "";
  const note = r.note_uid ? S.notes.find((n) => n.uid === r.note_uid) : null;
  const devs = S.devices.length > 1;
  $$(".it.sel").forEach((x) => x.classList.remove("sel"));
  editingRid = r.id;
  $$(`.it[data-rid="${r.id}"]`).forEach((x) => x.classList.add("sel"));
  showDrawer(`
    <div class="dh"><b>${r.id ? T.d.editTitle : T.d.newTitle}</b><button type="button" class="btn sm ghost icon" data-dclose title="${T.close}" aria-label="${T.close}">${I("x")}</button></div>
    <div class="db">
      <label class="fl"><span>${T.d.what}</span><input class="field big" id="ed-title" value="${esc(r.title)}" placeholder="${T.d.whatPh}"></label>
      <div class="fl"><label for="ed-when">${T.d.when}</label>
        <div class="field when" id="ed-whenbox"><input id="ed-when" value="${esc(whenText(due))}" placeholder="${T.d.whenPh}" autocomplete="off" spellcheck="false"><span class="pv" id="ed-pv"></span><button type="button" class="pick" id="ed-pick" title="${T.d.pick}" aria-label="${T.d.pick}">${I("calendar-days", "s16")}</button></div>
        <div class="rowf" id="ed-dt" hidden><input class="field" type="date" id="ed-date" aria-label="${T.d.date}"><input class="field" type="time" id="ed-time" aria-label="${T.d.time}"></div>
        <span class="fhelp warn" id="ed-guess"></span>
      </div>
      <div class="rowf"><button type="button" class="chipsel" id="ed-rep" aria-expanded="false">${I("repeat")}<span id="ed-rep-t"></span></button></div>
      <div class="repbox" id="ed-repbox" hidden>${repeatBoxHtml(r.repeat, due)}</div>
      <label class="fl"><span>${T.d.detail}</span><textarea class="field" id="ed-detail" rows="2" placeholder="${T.d.detailPh}">${esc(r.detail)}</textarea></label>
      ${tagList().length ? `<div class="fl"><span>${T.d.cat}</span><div class="rowf" id="ed-tags">${catChips(tagList(), r.tag)}</div></div>` : ""}
      <div class="swrow"><div>${T.urgent}<small>${T.d.urgentSub}</small></div><button type="button" class="tog" role="switch" id="ed-urgent" aria-checked="${r.urgent}" aria-label="${T.urgent}"></button></div>
      ${devs ? `<div class="fl"><span>${T.d.where}</span><div class="rowf" id="ed-devs"><button type="button" class="chipsel" data-dev="" aria-pressed="${!r.only_on.length}">${T.d.everyPc}</button>${S.devices.map((d) => `<button type="button" class="chipsel" data-dev="${esc(d.uid)}" aria-pressed="${r.only_on.includes(d.uid)}">${esc(d.name)}${d.me ? T.d.thisPc : ""}</button>`).join("")}</div></div>` : ""}
      ${note ? `<div class="fl"><span>${T.d.note}</span><button type="button" class="field linkn" data-open-note="${note.id}">${I("notebook-pen", "s16")}<span>${esc(note.title || T.blankNote)}</span></button></div>` : ""}
      <p class="fhelp err" id="ed-err"></p>
    </div>
    <div class="df">${r.id && !o.draft ? `<button type="button" class="btn ghost danger" id="ed-del">${I("trash-2")}${T.del}</button>` : ""}<span class="sp"></span><button type="button" class="btn" data-dclose>${T.cancel}</button><button type="submit" class="btn pri">${o.draft ? T.d.done : T.d.save}</button></div>`);

  const repBox = $("#ed-repbox");
  const when = $<HTMLInputElement>("#ed-when");
  const pv = $("#ed-pv");
  const repLabel = () => {
    const now = readRepeat(repBox);
    $("#ed-rep-t").textContent = now && isRecurring(now) ? repeatLabel(now, due) : now ? T.d.noRepeat : T.d.pickDays;
  };
  const showWhen = () => {
    pv.textContent = understood ? `→ ${fullWhen(due)}` : T.d.notUnderstood;
    pv.classList.toggle("bad", !understood);
    $("#ed-whenbox").classList.toggle("err", !understood);
    $("#ed-guess").textContent = understood && guess ? guess : r.vague && !dirty ? T.d.vagueNow : "";
    repLabel();
  };
  const bindKind = () => {
    const kindSel = $<HTMLSelectElement>("#ed-kind");
    kindSel.onchange = () => {
      const k = kindSel.value;
      $("#ed-week").hidden = k !== "weekly";
      $("#ed-month").hidden = k !== "monthly";
      $("#ed-int").hidden = k !== "interval";
      showRepeatExtras(repBox, k);
      repLabel();
    };
  };
  bindKind();
  showWhen();

  // 打一句话：本地规则理解成几点，右边马上写出来；说了「每周三」这类的，重复也跟着换
  when.addEventListener("input", () => {
    dirty = true;
    const it = parseLocal(when.value, new Date(), S.devices).find((x) => x.type === "reminder");
    understood = it?.type === "reminder";
    if (it?.type === "reminder") {
      due = it.due;
      guess = it.guess;
      if (isRecurring(it.repeat)) {
        repBox.innerHTML = repeatBoxHtml(it.repeat, due);
        bindKind();
      }
    }
    showWhen();
  });
  // 自己选日期和时间
  $("#ed-pick").onclick = () => {
    const dt = $("#ed-dt");
    dt.hidden = !dt.hidden;
    $<HTMLInputElement>("#ed-date").value = due.slice(0, 10);
    $<HTMLInputElement>("#ed-time").value = due.slice(11, 16);
  };
  const fromPicker = () => {
    const d = $<HTMLInputElement>("#ed-date").value;
    const t = $<HTMLInputElement>("#ed-time").value;
    if (!d || !t) return;
    due = `${d} ${t}`;
    dirty = true;
    understood = true;
    guess = "";
    when.value = whenText(due);
    showWhen();
  };
  $("#ed-date").onchange = fromPicker;
  $("#ed-time").onchange = fromPicker;

  drawer.onclick = async (e) => {
    const el = e.target as HTMLElement;
    if (el.closest("[data-dclose]")) return hideDrawer();
    if (el.closest("#ed-rep")) {
      repBox.hidden = !repBox.hidden;
      $("#ed-rep").setAttribute("aria-expanded", String(!repBox.hidden));
      return;
    }
    const wd = el.closest<HTMLElement>("[data-wd]");
    if (wd) {
      wd.setAttribute("aria-pressed", String(wd.getAttribute("aria-pressed") !== "true"));
      return repLabel();
    }
    const tg = el.closest<HTMLElement>("#ed-tags [data-tag]");
    if (tg) return pickOne($("#ed-tags"), tg);
    const dev = el.closest<HTMLElement>("[data-dev]");
    if (dev) {
      // 「每台都提醒」和具体哪几台只能选一边
      const all = $('#ed-devs [data-dev=""]');
      if (!dev.dataset.dev) $$("#ed-devs [data-dev]").forEach((c) => c.setAttribute("aria-pressed", String(c === all)));
      else {
        dev.setAttribute("aria-pressed", String(dev.getAttribute("aria-pressed") !== "true"));
        const any = $$("#ed-devs [data-dev]").some((c) => c.dataset.dev && c.getAttribute("aria-pressed") === "true");
        all.setAttribute("aria-pressed", String(!any));
      }
      return;
    }
    const sw = el.closest<HTMLElement>("#ed-urgent");
    if (sw) return sw.setAttribute("aria-checked", String(sw.getAttribute("aria-checked") !== "true"));
    const on = el.closest<HTMLElement>("[data-open-note]");
    if (on) {
      hideDrawer();
      return openNote(Number(on.dataset.openNote));
    }
    if (el.closest("#ed-del")) {
      hideDrawer();
      await deleteReminder(r.id);
    }
  };

  drawer.onsubmit = async (e) => {
    e.preventDefault();
    const err = $("#ed-err");
    const title = $<HTMLInputElement>("#ed-title").value.trim();
    if (!title) return void (err.textContent = T.d.needTitle);
    if (!understood) return void (err.textContent = T.d.needWhen);
    const repeat = readRepeat(repBox);
    if (!repeat) return void (err.textContent = T.d.needDays);
    const tagEl = $$("#ed-tags [data-tag]").find((c) => c.getAttribute("aria-pressed") === "true");
    const out: Reminder = {
      ...r,
      title,
      detail: $<HTMLTextAreaElement>("#ed-detail").value.trim(),
      due_at: due,
      // 改过时间就不算含糊了；新打的话又说得含糊，就还是含糊
      vague: dirty ? !!guess : r.vague && due === r.due_at,
      repeat,
      tag: (tagEl?.dataset.tag as Tag) ?? null,
      urgent: $("#ed-urgent").getAttribute("aria-checked") === "true",
      // 只有一台电脑时不显示这一栏，保持原样
      only_on: devs ? $$("#ed-devs [data-dev]").filter((c) => c.dataset.dev && c.getAttribute("aria-pressed") === "true").map((c) => c.dataset.dev!) : r.only_on,
    };
    if (o.draft) {
      o.draft(out);
      hideDrawer();
      return;
    }
    try {
      const saved = await api.saveReminder(out);
      hideDrawer();
      o.afterSave?.();
      say(T.savedRem(fullWhen(fireAt(saved))));
    } catch (ex) {
      err.textContent = errText(ex);
    }
  };
  drawerClose = hideDrawer;
}

/** 改随手记整理出来的一条笔记（还没存） */
function openNoteDraft(it: Extract<ParsedItem, { type: "note" }>, done: (n: Extract<ParsedItem, { type: "note" }>) => void) {
  showDrawer(`
    <div class="dh"><b>${T.d.noteTitle}</b><button type="button" class="btn sm ghost icon" data-dclose title="${T.close}" aria-label="${T.close}">${I("x")}</button></div>
    <div class="db">
      <label class="fl"><span>${T.d.title}</span><input class="field big" id="nd-title" value="${esc(it.title)}"></label>
      <label class="fl"><span>${T.d.body}</span><textarea class="field" id="nd-body" rows="10">${esc(it.body)}</textarea></label>
      ${noteCatList().length ? `<div class="fl"><span>${T.d.cat}</span><div class="rowf" id="nd-tags">${catChips(noteCatList(), it.tag)}</div></div>` : ""}
    </div>
    <div class="df"><span class="sp"></span><button type="button" class="btn" data-dclose>${T.cancel}</button><button type="submit" class="btn pri">${T.d.done}</button></div>`);
  drawer.onclick = (e) => {
    const el = e.target as HTMLElement;
    if (el.closest("[data-dclose]")) return hideDrawer();
    const tg = el.closest<HTMLElement>("#nd-tags [data-tag]");
    if (tg) pickOne($("#nd-tags"), tg);
  };
  drawer.onsubmit = (e) => {
    e.preventDefault();
    const tagEl = $$("#nd-tags [data-tag]").find((c) => c.getAttribute("aria-pressed") === "true");
    done({ ...it, title: $<HTMLInputElement>("#nd-title").value.trim() || it.title, body: $<HTMLTextAreaElement>("#nd-body").value, tag: (tagEl?.dataset.tag as Tag) ?? null });
    hideDrawer();
  };
  drawerClose = hideDrawer;
}

// ---------- 一行提醒上的动作：完成、推迟、删除（删除能撤销） ----------

async function deleteReminder(id: number) {
  try {
    await api.deleteReminder(id);
    say(T.deletedToTrash, () => api.trashAction("reminder", id, "restore"));
  } catch (err) {
    say(errText(err));
  }
}

async function doneClick(chk: HTMLElement) {
  const id = Number(chk.dataset.chk);
  const r = S.reminders.find((x) => x.id === id);
  if (!r) return;
  if (chk.dataset.done === "1") {
    if (isRecurring(r.repeat)) return say(T.cycleUndo);
    await api.reminderAction(id, "undo").catch((err) => say(errText(err)));
    return;
  }
  chk.classList.add("on");
  const saved = await api.reminderAction(id, "done").catch((err) => void say(errText(err)));
  if (!saved) return chk.classList.remove("on");
  if (isRecurring(saved.repeat)) say(T.doneCycle(fullWhen(saved.due_at)));
  else say(T.done(mascotOn()), () => api.reminderAction(id, "undo"));
}

// 小菜单（推迟、笔记的 ⋯）：贴着按钮弹出，点外面关掉
const menu = $("#menu");
function closeMenu() {
  menu.hidden = true;
  menu.innerHTML = "";
  menu.onclick = null;
}

function placeMenu(btn: HTMLElement) {
  menu.hidden = false;
  const b = btn.getBoundingClientRect();
  const w = menu.offsetWidth;
  menu.style.left = `${Math.max(8, Math.min(b.right - w, innerWidth - w - 8))}px`;
  menu.style.top = `${Math.max(8, Math.min(b.bottom + 4, innerHeight - menu.offsetHeight - 8))}px`;
}

/** 推迟：N 分钟后、明天再说、推迟到…（打一句话） */
function snoozeMenu(btn: HTMLElement, id: number) {
  const min = S.settings?.snooze_min ?? 10;
  menu.innerHTML = `<button class="mi" data-snz="snooze">${I("alarm-clock")}${T.later(min)}</button>
    <button class="mi" data-snz="tomorrow">${I("sunrise")}${T.tomorrowSay}</button>
    <button class="mi" data-snz="to">${I("calendar-days")}${T.snoozeTo}</button>`;
  placeMenu(btn);
  menu.onclick = async (e) => {
    const m = (e.target as HTMLElement).closest<HTMLElement>("[data-snz]");
    if (!m) return;
    if (m.dataset.snz === "to") return snoozeTo(btn, id);
    closeMenu();
    const saved = await api.reminderAction(id, m.dataset.snz as "snooze" | "tomorrow").catch((err) => void say(errText(err)));
    if (saved) say(T.snoozedTo(timeCol(fireAt(saved), true)));
  };
}

/** 推迟到…：打一句话，实时写出理解成几点（和提醒卡片的「推迟到…」一样） */
function snoozeTo(btn: HTMLElement, id: number) {
  menu.innerHTML = `<div class="snz"><label class="field"><input id="snz-in" placeholder="${T.snoozePh}" autocomplete="off" spellcheck="false" aria-label="${T.snoozeTo}"></label>
    <span class="fhelp" id="snz-pv"></span><button class="btn sm pri" id="snz-go" disabled>${T.snoozeGo}</button></div>`;
  placeMenu(btn);
  const inp = $<HTMLInputElement>("#snz-in");
  const go = $<HTMLButtonElement>("#snz-go");
  let at: Date | null = null;
  inp.oninput = () => {
    const it = parseLocal(inp.value, new Date(), S.devices).find((x) => x.type === "reminder");
    const d = it?.type === "reminder" ? parseDT(it.due) : null;
    at = d && d > new Date() ? d : null;
    $("#snz-pv").textContent = !inp.value.trim() ? "" : at ? `→ ${fullWhen(fmtDT(at))}` : d ? T.snoozePast : T.d.notUnderstood;
    go.disabled = !at;
    go.textContent = at ? T.snoozeGoAt(timeCol(fmtDT(at), true)) : T.snoozeGo;
  };
  const apply = async () => {
    if (!at) return;
    const until = fmtDT(at);
    closeMenu();
    const saved = await api.reminderAction(id, "snooze_to", until).catch((err) => void say(errText(err)));
    if (saved) say(T.snoozedTo(timeCol(until, true)));
  };
  go.onclick = apply;
  inp.onkeydown = (e) => {
    if (e.key === "Enter" && !e.isComposing) void apply();
  };
  inp.focus();
}

document.addEventListener("mousedown", (e) => {
  if (!menu.hidden && !menu.contains(e.target as Node)) closeMenu();
});

// 内容区里的点击：完成勾、一行上的按钮、点一行改提醒；图片、文件、链接、笔记里的表格
$("#main").addEventListener("click", async (e) => {
  const el = e.target as HTMLElement;
  // 编辑器里的图片、文件、链接、表格由编辑器自己管
  if (el.closest("#drawer, #pal, .cm-editor")) return;
  const att = el.closest<HTMLElement>("[data-att]");
  // 图片用自己的看大图（← → 切同一条里的别的图片）；不是图片的附件用系统的程序打开
  if (att?.tagName === "IMG") return lightboxFrom(att, say);
  if (att) return void api.openAttachment(att.dataset.att!).then((shown) => shown && say(TN.fileRevealed), (err) => say(errText(err)));
  // 文件卡片旁边的「另存为…」：用原来的名字存一份
  const sv = el.closest<HTMLElement>("[data-saveas]");
  if (sv) {
    const p = await api.saveAttachmentAs(sv.dataset.saveas!).catch((err) => void say(errText(err)));
    if (p) say(TN.lbSaved(p));
    return;
  }
  const fc = el.closest<HTMLElement>("[data-file]");
  if (fc) return void openFile(fc);
  // 笔记里格子、卡片、链接旁边的复制按钮
  const cp = el.closest<HTMLElement>("[data-copy]");
  if (cp) {
    await copyText(cp.dataset.copy!);
    const t = cp.dataset.copy!.replace(/\s+/g, " ");
    return say(T.copied(t.length > 30 ? t.slice(0, 30) + "…" : t));
  }
  // 笔记里表格右上角的「格子编辑」
  const tg = el.closest<HTMLElement>("#nb-doc [data-tgrid]");
  if (tg) return gridFromDoc(Number(tg.dataset.tgrid));
  // 笔记里的表格：切换样子、搜索、分组、展开、改一行
  if (el.closest("#nb-doc .tblock") || document.querySelector("#nb-doc [data-tmenu][aria-expanded=true]")) {
    const ctx = noteTableCtx();
    if (ctx && (await tableClick(el, ctx))) return;
  }
  const link = el.closest<HTMLElement>("[data-href]");
  if (link) return void api.openUrl(link.dataset.href!).catch((err) => say(errText(err)));
  const chk = el.closest<HTMLElement>("[data-chk]");
  if (chk) return void doneClick(chk);
  const ract = el.closest<HTMLElement>("[data-ract]");
  const row = el.closest<HTMLElement>("[data-rid]");
  const r = row ? S.reminders.find((x) => x.id === Number(row.dataset.rid)) : null;
  if (ract && r) {
    if (ract.dataset.ract === "snooze") return snoozeMenu(ract, r.id);
    if (ract.dataset.ract === "del") return void deleteReminder(r.id);
  }
  if (r) return openEditor(structuredClone(r));
  const on = el.closest<HTMLElement>("[data-open-note]");
  if (on) return openNote(Number(on.dataset.openNote));
  if (el.closest("[data-newnote]")) newNoteHere();
});

// ---------- 提醒页 ----------

function renderReminders() {
  // 正看着的分类被删掉了：回到全部
  if (S.rTag && !tagList().some((t) => t.id === S.rTag)) S.rTag = null;
  const inTag = (list: Reminder[]) => (S.rTag ? list.filter((r) => matchTag(r, S.rTag!)) : list);
  const todo = inTag(S.reminders.filter((r) => !r.done));
  $$("#rmode [data-rmode]").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.rmode === S.rMode)));
  const t = S.rTag ? tagList().find((x) => x.id === S.rTag) : null;
  $("#rem-tag").innerHTML = t ? `<button class="tag" style="${tagColor(t.color)}" title="${T.allCats}">${esc(t.name)}${I("x")}</button>` : "";
  $("#rem-sub").textContent = todo.length ? T.remLeft(todo.length) : "";

  const now = new Date();
  let html = "";
  if (S.rMode === "done") {
    const list = inTag(S.reminders.filter((r) => r.done))
      .sort((a, b) => (b.completed_at ?? "").localeCompare(a.completed_at ?? ""))
      .slice(0, 200);
    const tk = todayKey();
    const yk = dayStr(addDays(now, -1));
    const parts: [string, (d: string) => boolean][] = [[T.today, (d) => d === tk], [T.yesterday, (d) => d === yk], [T.earlier, (d) => d < yk]];
    for (const [title, test] of parts) {
      const rows = list.filter((r) => test((r.completed_at ?? "").slice(0, 10)));
      if (rows.length) html += grp(title, rows.map((r) => remRow(r, { done: true })).join(""), { n: rows.length });
    }
    if (!html) html = emptyHtml("normal", T.noDone, T.noDoneSub, "rem-done");
  } else if (S.rMode === "cycle") {
    const list = todo.filter((r) => isRecurring(r.repeat)).sort(byFire);
    const kinds: [string, RepeatKind[]][] = [
      [T.cyDaily, ["daily", "weekdays", "interval"]],
      [T.cyWeekly, ["weekly"]],
      [T.cyMonthly, ["monthly", "month_last", "month_nth"]],
      [T.cyYearly, ["yearly", "lunar_yearly"]],
    ];
    for (const [title, ks] of kinds) {
      const rows = list.filter((r) => ks.includes(r.repeat.kind));
      if (rows.length) html += grp(title, rows.map((r) => remRow(r, { loose: true })).join(""), { n: rows.length });
    }
    if (!html) html = emptyHtml("normal", T.noCycle, T.noCycleSub, "rem-cycle");
  } else {
    // 按时间分组：过点了、今天、明天、这周、以后（周期的按下一次）
    const day = (n: number) => dayStr(addDays(now, n));
    const buckets: [string, (r: Reminder) => boolean, boolean][] = [
      [T.late, (r) => (parseDT(fireAt(r)) ?? now) < now, true],
      [T.today, (r) => fireAt(r).slice(0, 10) === day(0), false],
      [T.tomorrow, (r) => fireAt(r).slice(0, 10) === day(1), false],
      [T.thisWeek, (r) => fireAt(r).slice(0, 10) > day(1) && fireAt(r).slice(0, 10) < day(7), false],
      [T.later2, (r) => fireAt(r).slice(0, 10) >= day(7), false],
    ];
    let rest = [...todo].sort(byFire);
    for (const [title, test, late] of buckets) {
      const rows = rest.filter(test);
      rest = rest.filter((r) => !test(r));
      if (rows.length) html += grp(title, rows.map((r) => remRow(r)).join(""), { n: rows.length, late });
    }
    if (!html) html = S.rTag ? emptyHtml("normal", T.noRemInTag, "", "rem-tag") : emptyHtml("sleep", T.noRem(mascotOn()), T.noRemSub);
  }
  $("#rem-groups").innerHTML = html;
}

$("#rmode").addEventListener("click", (e) => {
  const b = (e.target as HTMLElement).closest<HTMLElement>("[data-rmode]");
  if (!b) return;
  S.rMode = b.dataset.rmode as typeof S.rMode;
  renderReminders();
});
$("#rem-tag").addEventListener("click", () => {
  S.rTag = null;
  renderReminders();
  renderCounts();
});

// 侧栏：几个页面、提醒分类（只看这一类，再点一下回到全部）、管理分类
$(".sbar").addEventListener("click", (e) => {
  const el = e.target as HTMLElement;
  if (el.closest("#tags-edit")) return openSettings("remind", "tags");
  const tag = el.closest<HTMLElement>("[data-tagnav]");
  if (tag) {
    S.rTag = S.view === "reminders" && S.rTag === tag.dataset.tagnav ? null : tag.dataset.tagnav!;
    if (S.rMode === "done") S.rMode = "todo";
    return go("reminders");
  }
  const nav = el.closest<HTMLElement>(".nv[data-view]");
  if (!nav) return;
  if (nav.dataset.view === "reminders") S.rTag = null;
  if (nav.dataset.view === "notes" && S.view !== "notes") S.todoView = false;
  go(nav.dataset.view as View);
});

// ---------- 笔记本：左边列表，右边正文；顶上切「笔记 / 待办」 ----------

function filteredNotes() {
  const q = S.q.trim().toLowerCase();
  const inCat = S.nCat === "all" ? S.notes : S.notes.filter((n) => (S.nCat === "none" ? !noteCatDef(n.tag) : n.tag === S.nCat));
  return q ? inCat.filter((n) => n.title.toLowerCase().includes(q) || n.body.toLowerCase().includes(q)) : inCat;
}

/** 笔记本列表上面那一排：全部、各个笔记分类、没分类的（有的话）。点一下筛，再点一下回到全部 */
function renderNoteCats() {
  if (S.nCat !== "all" && S.nCat !== "none" && !noteCatDef(S.nCat)) S.nCat = "all";
  const none = S.notes.filter((x) => !noteCatDef(x.tag)).length;
  const items: [string, string, string][] = [
    ["all", T.all, ""],
    ...noteCatList().map((t) => [t.id, t.name, tagColor(t.color)] as [string, string, string]),
    ...(none && none < S.notes.length ? [["none", T.noCat, ""] as [string, string, string]] : []),
  ];
  $("#nb-cats").innerHTML = items
    .map(([id, name, color]) => `<button class="nbc" data-ncat="${esc(id)}" ${color ? `style="${color}"` : ""} aria-pressed="${S.nCat === id}">${color ? '<i class="sq"></i>' : ""}${esc(name)}</button>`)
    .join("");
}

$("#nb-cats").addEventListener("click", (e) => {
  const c = (e.target as HTMLElement).closest<HTMLElement>("[data-ncat]");
  if (!c) return;
  if (S.editing) finishEdit();
  S.nCat = c.dataset.ncat === S.nCat ? "all" : c.dataset.ncat!;
  renderNotes();
});

/** 主窗口里的「新笔记」「记一条笔记」：在笔记本里直接写（快速记笔记小窗口留给快捷键和托盘）。
 * 正在按分类筛的话，新笔记就放在那个分类里；什么都没写就走开，不会留下空笔记 */
function newNoteHere() {
  if (S.editing) finishEdit();
  S.q = "";
  $<HTMLInputElement>("#nb-search").value = "";
  S.todoView = false;
  const cat = S.nCat !== "all" && S.nCat !== "none" ? S.nCat : null;
  freshNote = newNote({ tag: cat });
  go("notes");
  document.querySelector<HTMLElement>("#ne-title")?.focus();
}
$("#new-note").onclick = newNoteHere;

/** 列表里一条笔记下面的小图标：有提醒、有图片、有表格、有文件 */
function noteIcons(n: Note) {
  const rem = !!n.reminder_id || S.reminders.some((r) => r.note_uid === n.uid && !r.done);
  const ico = (on: boolean, ic: IconName, tip: string) => (on ? `<span title="${tip}">${I(ic)}</span>` : "");
  return (
    ico(rem, "bell", T.hasRem) +
    ico(/!\[[^\]]*\]\(att:/.test(n.body), "image", T.hasImg) +
    ico(/^\s*\|.*\|\s*$/m.test(n.body), "table", T.hasTable) +
    ico(/(?<!!)\[[^\]]*\]\(att:/.test(n.body), "paperclip", T.hasFile)
  );
}

function renderNotes() {
  renderNoteCats();
  const q = S.q.trim();
  const ql = q.toLowerCase();
  const list = filteredNotes();
  if (!S.todoView) $("#nb-count").textContent = q ? T.found(list.length) : S.notes.length ? T.notesN(S.notes.length) : "";
  if (!freshNote && !(S.editing && edNote?.id === S.noteId) && list.length && !list.some((n) => n.id === S.noteId)) S.noteId = list[0].id;
  $("#nb-items").innerHTML = list.length
    ? list
        .map((n) => {
          const inBody = !!q && !n.title.toLowerCase().includes(ql);
          const cat = noteCatDef(n.tag);
          return `<button class="nbi${n.id === S.noteId && !freshNote ? " on" : ""}" data-nid="${n.id}" ${cat ? `style="${tagColor(cat.color)}"` : ""} title="${T.dblOpen}">
            <b>${cat ? `<span class="sq" title="${esc(cat.name)}"></span>` : ""}<span class="tt">${highlight(esc(n.title || T.blankNote), q)}</span>${n.pinned_at ? I("pin", "pin") : ""}</b>
            <small class="${inBody ? "hit" : ""}">${snippet(n.body, q, 80) || T.blankNote}</small>
            <span class="mt">${ago(n.updated_at)}${noteIcons(n)}</span></button>`;
        })
        .join("")
    : q
      ? emptyHtml("normal", T.notFound(mascotOn()), "", "nb-q")
      : S.nCat !== "all" && S.notes.length
        ? `<p class="none">${T.catEmpty}</p>`
        : emptyHtml("normal", T.nbEmpty, T.nbEmptySub(S.settings?.hk_qnote ? hotkeyText(S.settings.hk_qnote) : ""), "nb-empty");
  // 右边：只在看着笔记本时才放编辑器
  if (S.view === "notes" && !S.todoView) {
    if (S.editing) syncOpenNote();
    else renderNoteDoc();
  }
}

/** 只看笔记时点了表格上的「格子编辑」：改完写回这张表格（第 n 张）。开着单独窗口的，那边会收到修改 */
function gridFromDoc(n: number) {
  const ctx = noteTableCtx();
  const t = ctx && findTables(ctx.body())[n];
  if (!ctx || !t) return;
  const lines = ctx.body().split("\n");
  const old = lines.slice(t.start, t.start + t.lines);
  openTableGrid({
    lines: old,
    onDone: async (next) => {
      // 改的时候正文可能变了：还是第 n 张、内容也没变才写回
      const body = ctx.body().split("\n");
      const now = findTables(body.join("\n"))[n];
      if (!now || body.slice(now.start, now.start + now.lines).join("\n") !== old.join("\n")) return say(T.tableChanged);
      body.splice(now.start, now.lines, ...next);
      await ctx.save(body.join("\n")).catch((err) => say(T.notSaved(errText(err))));
      renderNoteDoc();
    },
  });
}

/** 正在看的这条笔记里的表格怎么重画、怎么保存 */
function noteTableCtx(): TableCtx | null {
  const n = S.notes.find((x) => x.id === S.noteId);
  if (!n) return null;
  const cur = () => S.notes.find((x) => x.id === n.id) ?? n;
  return {
    uid: n.uid,
    body: () => cur().body,
    render: renderNoteDoc,
    save: async (body) => {
      const saved = await api.saveNote({ ...cur(), body });
      Object.assign(cur(), saved);
    },
    toast: (m: string) => say(m),
  };
}

/** 右边那一栏：点进去就能改（U7，没有单独的查看 / 编辑状态，自动保存）。
 *  这条笔记开着单独的窗口时，这里只看（排好版的），免得两边互相覆盖 */
function renderNoteDoc() {
  const doc = $("#nb-doc");
  // 表格里改到一半的一行，重画后还在
  if (doc.dataset.uid) captureDrafts(doc, doc.dataset.uid);
  if (S.editing) finishEdit();
  const n = freshNote ?? S.notes.find((x) => x.id === S.noteId);
  doc.dataset.uid = "";
  if (!n || (!freshNote && !filteredNotes().some((x) => x.id === n.id))) {
    doc.innerHTML = S.notes.length ? `<p class="doc-none">${T.pickNote}</p>` : "";
    return;
  }
  if (n.id && S.noteWins.has(n.id)) {
    doc.dataset.uid = n.uid;
    doc.innerHTML = `<div class="winnote">${I("app-window")}<span>${T.inWindow}</span><button class="btn sm" data-nact="win">${T.goWindow}</button></div>
      <h4 class="rtitle">${highlight(esc(n.title || T.blankNote), S.q.trim())}</h4>
      <div class="dmd">${n.body.trim() ? renderMd(n.body, S.q.trim(), undefined, tableStates(n.uid)) : `<p class="doc-none">${T.blankNote}</p>`}</div>`;
    return;
  }
  // 新笔记直接写；别的平时只看，双击或点「编辑」才改
  if (n === freshNote || (n.id && editNoteId === n.id)) mountEditor(n);
  else mountView(n);
}

/** 待办后面：设过提醒的显示时间（点了改），没设过的「设提醒」 */
function todoExtra(n: Note) {
  return (t: { line: number; text: string }) => {
    const r = todoReminder(n, t.text);
    return r && !r.done
      ? `<button class="tag" style="--c:var(--accent)" data-todo-open="${r.id}" title="${T.editRem}">${I("bell")}${timeCol(fireAt(r), true)}</button>`
      : `<button class="btn sm ghost setr" data-todo-set="${n.id}" data-line="${t.line}" title="${T.setRemTip}">${I("bell-plus")}${T.setRem}</button>`;
  };
}

/** 只看：排好版的样子，能勾待办、设提醒、点链接、切表格的看法、复制；双击、点「编辑」、按回车进编辑 */
function mountView(n: Note) {
  const doc = $("#nb-doc");
  const keep = doc.dataset.view === String(n.id) ? doc.scrollTop : 0;
  noteEd?.destroy();
  noteEd = null;
  S.editing = false;
  edNote = n;
  edTag = n.tag;
  savedBody = n.body;
  savedTitle = n.title;
  conflict = null;
  lightTools = () => {};
  doc.dataset.uid = n.uid;
  doc.dataset.view = String(n.id);
  const q = S.q.trim();
  doc.innerHTML = `
    <div class="ed-bar view" id="ne-bar"><span class="sp"></span><span class="saved" id="ne-state"></span><button class="btn sm" id="ne-tidy" data-nact="tidy">${I("sparkles")}${TN.aiButton}</button><button class="btn sm pri" data-nact="edit" title="${TN.editTip}">${I("pencil")}${TN.edit}</button><button class="btn sm ghost icon" data-nact="more" title="${TN.more}" aria-label="${TN.more}">${I("ellipsis")}</button></div>
    <div class="ntitle rd" data-ln="-1" title="${TN.editTip}">${highlight(esc(n.title || T.blankNote), q)}</div>
    <div class="meta" id="ne-meta"></div>
    <div class="dmd ne-view" id="ne-view" tabindex="0" title="${TN.editTip}">${n.body.trim() ? renderMd(n.body, q, todoExtra(n), tableStates(n.uid)) : `<p class="doc-none" data-ln="0">${TN.viewEmpty}</p>`}</div>`;
  tidyReady();
  drawMeta();
  doc.scrollTop = keep;
}

/** 进编辑：at 是双击的那一行（-1 是标题） */
function enterEdit(at?: { line: number; word?: string }) {
  const n = edNote ?? S.notes.find((x) => x.id === S.noteId);
  if (!n || !n.id) return;
  editNoteId = n.id;
  editAt = at ?? null;
  renderNoteDoc();
}

/** 改完回去看：先存好，用刚写的内容画（不等存完的通知） */
function leaveEdit() {
  const n = edNote;
  if (!n || !noteEd) return;
  const title = $<HTMLInputElement>("#ne-title").value;
  const body = noteEd.value;
  finishEdit();
  editNoteId = null;
  if (!n.id) return renderNoteDoc();
  mountView({ ...n, title, body, tag: edTag });
}

/** 笔记右上角 ⋯：置顶、在单独的窗口里编辑、格式化、快捷键、删除（能撤销） */
function noteMenu(btn: HTMLElement, n: Note) {
  menu.innerHTML = `${n.id ? `<button class="mi" data-nm="pin">${I("pin")}${n.pinned_at ? T.unpin : T.pin}</button><button class="mi" data-nm="win">${I("app-window")}${T.openWindow}</button>` : ""}
    <button class="mi" data-nm="fmt" title="${TN.fmtTip}">${I("wand-sparkles")}${TN.fmt}<kbd>Shift Alt F</kbd></button>
    <button class="mi" data-nm="keys">${I("keyboard")}${TN.keys}</button>
    ${n.id ? `<div class="msep"></div><button class="mi danger" data-nm="del">${I("trash-2")}${T.del}</button>` : ""}`;
  placeMenu(btn);
  menu.onclick = async (e) => {
    const m = (e.target as HTMLElement).closest<HTMLElement>("[data-nm]")?.dataset.nm;
    if (!m) return;
    closeMenu();
    if (m === "pin") {
      await api.pinNote(n.id, !n.pinned_at).catch((err) => say(errText(err)));
      say(n.pinned_at ? T.unpinned : T.pinnedNow);
    } else if (m === "win") openNoteWindow(n.id);
    else if (m === "fmt") {
      if (!noteEd) enterEdit();
      noteEd?.format();
    }
    else if (m === "keys") showKeys();
    else {
      dropEdit();
      await api.deleteNote(n.id).catch((err) => say(errText(err)));
      say(T.deletedToTrash, () => api.trashAction("note", n.id, "restore"));
    }
  };
}

/** 编辑器的快捷键一览 */
function showKeys() {
  const box = $("#dlg");
  box.innerHTML = `<div class="dialog"><h4>${TN.keys}</h4><ul class="keys">${EDIT_KEYS.split("\n").map((k) => `<li>${esc(k)}</li>`).join("")}</ul><div class="acts"><button class="btn pri" data-dlg="no">${T.close}</button></div></div>`;
  box.hidden = false;
  box.onclick = (e) => {
    if (e.target === box || (e.target as HTMLElement).closest("[data-dlg]")) box.hidden = true;
  };
}

// ---------- 笔记里的待办 ----------

/** 这条待办设过的提醒（同一条笔记、同一行字） */
const todoReminder = (n: Note, text: string) => S.reminders.find((r) => r.note_uid === n.uid && r.todo === text);

/** 勾上 / 取消一条待办。设过提醒的，提醒也跟着完成 / 取消完成（笔记那一行由提醒那边改） */
async function toggleTodo(n: Note, line: number) {
  const t = todos(n.body).find((x) => x.line === line);
  if (!t) return;
  const r = todoReminder(n, t.text);
  if (r && !isRecurring(r.repeat) && r.done === t.done) {
    await api.reminderAction(r.id, t.done ? "undo" : "done");
    if (!t.done) say(T.todoAndRem);
    return;
  }
  await api.saveNote({ ...n, body: setTodo(n.body, line, !t.done) });
}

function setTodoReminder(n: Note, line: number) {
  const t = todos(n.body).find((x) => x.line === line);
  if (t) openEditor(newReminder({ title: t.text, note_uid: n.uid, todo: t.text }));
}

/** 待办：所有笔记里没勾掉的 - [ ]，按笔记分组。方形的勾（它属于笔记），和提醒的圆形完成勾分开 */
function renderTodos() {
  const groups = S.notes.map((n) => ({ n, list: todos(n.body).filter((t) => !t.done) })).filter((g) => g.list.length);
  const total = groups.reduce((a, g) => a + g.list.length, 0);
  $("#nb-todo-btn").textContent = total ? T.todosN(total) : T.ui.todos;
  $$("#nb-mode [data-mode]").forEach((b) => b.setAttribute("aria-pressed", String((b.dataset.mode === "todos") === S.todoView)));
  $("#nbk").hidden = S.todoView;
  const box = $("#nb-todos");
  box.hidden = !S.todoView;
  if (!S.todoView) return;
  $("#nb-count").textContent = total ? T.todosLeft(total) : "";
  box.innerHTML = groups.length
    ? groups
        .map(
          ({ n, list }) => `<div class="grp"><h5><button class="lnk" data-open-note="${n.id}">${I("file-text")}${esc(n.title || T.blankNote)}</button></h5>
          ${list
            .map((t) => {
              const r = todoReminder(n, t.text);
              const rem =
                r && !r.done
                  ? `<button class="tag" style="--c:var(--accent)" data-todo-open="${r.id}" title="${T.editRem}">${I("bell")}${timeCol(fireAt(r), true)}</button>`
                  : `<button class="btn sm ghost setr" data-todo-set="${n.id}" data-line="${t.line}" title="${T.setRemTip}">${I("bell-plus")}${T.setRem}</button>`;
              return `<div class="it todo"><button class="chk" data-tbox="${n.id}" data-line="${t.line}" aria-label="${T.tick}">${I("check")}</button><span class="t">${inline(t.text)}</span>${rem}</div>`;
            })
            .join("")}</div>`,
        )
        .join("")
    : emptyHtml("happy", T.noTodos(mascotOn()), T.noTodosSub, "todos");
}

/** 笔记本里切换「笔记 / 待办」 */
function showTodos(on: boolean) {
  if (S.editing) finishEdit();
  S.todoView = on;
  go("notes");
}

$("#nb-mode").addEventListener("click", (e) => {
  const b = (e.target as HTMLElement).closest<HTMLElement>("[data-mode]");
  if (b) showTodos(b.dataset.mode === "todos");
});

/** 笔记正文和待办页里：勾选框、设提醒、改提醒 */
async function todoClick(e: Event) {
  const el = e.target as HTMLElement;
  if (el.closest(".cm-editor")) return;
  const box = el.closest<HTMLElement>("[data-box], [data-tbox]");
  const set = el.closest<HTMLElement>("[data-todo-set]");
  const open = el.closest<HTMLElement>("[data-todo-open]");
  const noteOf = (id: string | undefined) => S.notes.find((x) => x.id === Number(id));
  if (box) {
    const n = box.dataset.tbox ? noteOf(box.dataset.tbox) : S.notes.find((x) => x.id === S.noteId);
    const line = Number(box.dataset.box ?? box.dataset.line);
    if (box.dataset.tbox) box.classList.add("on");
    if (n) await toggleTodo(n, line).catch((err) => say(errText(err)));
  } else if (set) {
    const n = noteOf(set.dataset.todoSet);
    if (n) setTodoReminder(n, Number(set.dataset.line));
  } else if (open) {
    const r = S.reminders.find((x) => x.id === Number(open.dataset.todoOpen));
    if (r) openEditor(structuredClone(r));
  } else return;
  e.stopPropagation();
}
$("#nb-todos").addEventListener("click", todoClick);
// 只看的那种（开着单独窗口时）里的表格
$("#nb-doc").addEventListener("input", (e) => {
  const ctx = !S.editing && noteTableCtx();
  if (ctx) tableInput(e.target as HTMLElement, ctx);
});
$("#nb-doc").addEventListener("keydown", (e) => {
  const ctx = !S.editing && noteTableCtx();
  if (ctx) tableKey(e, ctx);
});
$("#nb-doc").addEventListener("click", todoClick);
// 只看时：双击进编辑，光标落在双击的那一行、那个词前面；按回车也进编辑
$("#nb-doc").addEventListener("dblclick", (e) => {
  if (S.editing || !edNote) return;
  const el = e.target as HTMLElement;
  if (el.closest("button, a, input, textarea, .tblock, [data-att], [data-file]")) return;
  const blk = el.closest<HTMLElement>("[data-ln]");
  if (!blk && !el.closest("#ne-view")) return;
  const word = window.getSelection()?.toString().trim() || undefined;
  window.getSelection()?.removeAllRanges();
  enterEdit({ line: blk ? Number(blk.dataset.ln) : 0, word });
});
$("#nb-doc").addEventListener("keydown", (e) => {
  if (S.editing || !edNote || e.key !== "Enter" || e.isComposing) return;
  if ((e.target as HTMLElement).closest("#ne-view")) {
    e.preventDefault();
    enterEdit();
  }
});

function openNote(id: number) {
  if (S.editing) finishEdit();
  if (id !== S.noteId) editNoteId = null;
  S.noteId = id;
  freshNote = null;
  S.todoView = false;
  S.q = "";
  $<HTMLInputElement>("#nb-search").value = "";
  go("notes");
}

$("#nb-search").addEventListener("input", (e) => {
  S.q = (e.target as HTMLInputElement).value;
  if (S.editing) finishEdit();
  freshNote = null;
  renderNotes();
});
// 单击打开（右边直接能改），双击在单独的窗口里编辑。第一下单击会重画列表，所以不用 dblclick（落在被换掉的元素上收不到），看点击次数
$("#nb-items").addEventListener("click", (e) => {
  const b = (e.target as HTMLElement).closest<HTMLElement>("[data-nid]");
  if (!b) return;
  const id = Number(b.dataset.nid);
  if (e.detail >= 2) return openNoteWindow(id);
  if (id === S.noteId && !freshNote && S.editing) return;
  if (S.editing) finishEdit();
  if (id !== S.noteId) editNoteId = null;
  S.noteId = id;
  freshNote = null;
  renderNotes();
});

/** 在单独的窗口里编辑：这里正在改这条的话先存好，换成只看 */
function openNoteWindow(id: number) {
  if (edNote?.id === id) {
    finishEdit();
    S.noteWins.add(id);
    renderNoteDoc();
  }
  void api.openNoteWindow(id).catch((err) => say(T.notOpened(errText(err))));
}

// ---------- 右边的编辑器（lib/editor.ts，和单独的笔记窗口同一个），自动保存 ----------

/** 正在右边改的笔记；新笔记第一次存之前 id 是 0 */
let edNote: Note | null = null;
let noteEd: NoteEditor | null = null;
/** 点了「新笔记」、还没存的那条 */
let freshNote: Note | null = null;
/** 正在改的是哪条笔记（双击、点「编辑」进来的）；别的笔记、没进编辑时右边只看排好版的 */
let editNoteId: number | null = null;
/** 进编辑时光标放在哪（双击的那一行、那个词；-1 是标题） */
let editAt: { line: number; word?: string } | null = null;
/** 上次存进去的：数据变了时拿它判断是不是别处改的、这里有没有没存的修改 */
let savedBody = "";
let savedTitle = "";
/** 分类：这里选的（还没存） */
let edTag: Tag | null = null;
let pendingSave = false;
/** 别处也改了、还没选用哪一版：先不自动保存，免得不等选就盖掉那边的 */
let conflict: Note | null = null;
let lightTools: (on: Set<string>) => void = () => {};

/** 工具栏右边「已保存」那一小块 */
function setState(text: string, kind: "" | "bad" | "typing" = "") {
  const el = document.querySelector<HTMLElement>("#ne-state");
  if (!el) return;
  el.innerHTML = text ? `${I("check")}${esc(text)}` : "";
  el.className = "saved" + (kind ? " " + kind : "");
}

/** force：选了「保留我的」，不再看别处改没改过；even：还没写东西的新笔记也先存一下（放附件、设提醒要挂在它下面） */
async function saveNote(o: { force?: boolean; even?: boolean } = {}) {
  pendingSave = false;
  const n = edNote;
  const ed = noteEd;
  if (!n) return;
  if (conflict && !o.force) return setState(TN.pickFirst, "bad");
  // 只看时（只改了分类）：标题、正文就是存过的
  const title = ed ? $<HTMLInputElement>("#ne-title").value : savedTitle;
  const body = ed ? ed.value : savedBody;
  const tag = edTag;
  // 新笔记什么都没写：先不存，走开也不会留下一条空笔记
  if (!n.id && !o.even && !title.trim() && !body.trim()) return;
  if (n.id && body === savedBody && title === savedTitle && tag === n.tag) return setState(TN.saved);
  try {
    // 存之前看一眼：上次存过之后别处改过了（通知可能还没到），就先问，不直接盖上去
    if (n.id && !o.force) {
      const fresh = await api.getNote(n.id);
      if (fresh && fresh.body !== savedBody && edNote === n) return showConflict(fresh);
    }
    const saved = await api.saveNote({ ...n, title, body, tag });
    if (edNote !== n) return;
    edNote = saved;
    savedBody = body;
    savedTitle = title;
    if (!n.id) {
      S.noteId = saved.id;
      freshNote = null;
      editNoteId = saved.id;
    }
    setState(TN.saved);
  } catch (e) {
    setState(TN.saveFail(errText(e)), "bad");
  }
}
const saveLater = debounce(() => void saveNote(), 600);

function touch() {
  pendingSave = true;
  setState(TN.saving, "typing");
  saveLater();
}

/** 两边都改了：问用哪一版，选之前不自动保存 */
function showConflict(fresh: Note) {
  conflict = fresh;
  setState(TN.pickFirst, "bad");
  const bar = $("#ne-conf");
  bar.innerHTML = `${I("triangle-alert")}<div><b>${esc(TN.conflict)}</b>${esc(T.conflictSub)}</div><button class="btn sm" data-cf="theirs">${TN.useTheirs}</button><button class="btn sm pri" data-cf="mine">${TN.keepMine}</button>`;
  bar.hidden = false;
}

/** 分类、改过的时间、置顶、关联提醒 */
function drawMeta() {
  const n = edNote;
  const box = document.querySelector<HTMLElement>("#ne-meta");
  if (!n || !box) return;
  const cat = noteCatDef(edTag);
  const linked = n.reminder_id ? S.reminders.find((r) => r.id === n.reminder_id) : undefined;
  box.innerHTML = [
    noteCatList().length || cat
      ? `<button type="button" class="${cat ? "tag" : "tag nocat"}" data-nact="cat" style="${cat ? tagColor(cat.color) : ""}" title="${TN.catTip}" aria-haspopup="menu">${cat ? esc(cat.name) : `${I("plus")}${TN.noCat}`}</button>`
      : "",
    n.id ? `<span>${esc(TN.edited(ago(n.updated_at)))}</span>` : "",
    n.pinned_at ? `<span>${I("pin")}${T.pinned}</span>` : "",
    linked ? `<button type="button" class="lnk" data-nact="rem" title="${T.editRem}">${I("bell")}${esc(TN.linked(`${whenShort(fireAt(linked))} ${linked.title}`))}</button>` : "",
  ].join("");
}

// 没填 AI Key 时「让 AI 整理」灰着，鼠标停上去说去哪填
function tidyReady() {
  const b = document.querySelector<HTMLButtonElement>("#ne-tidy");
  if (!b) return;
  b.classList.toggle("off", !S.settings?.ai_key);
  b.title = S.settings?.ai_key ? "" : TN.aiNoKey;
}

function mountEditor(n: Note) {
  noteEd?.destroy();
  edNote = n;
  edTag = n.tag;
  savedBody = n.body;
  savedTitle = n.title;
  pendingSave = false;
  conflict = null;
  beforeTidy = tidyPending = null;
  const doc = $("#nb-doc");
  doc.dataset.uid = "";
  doc.innerHTML = `
    <div class="ed-bar" id="ne-bar"><span class="sp"></span><span class="saved" id="ne-state"></span><button class="btn sm" id="ne-tidy" data-nact="tidy">${I("sparkles")}${TN.aiButton}</button>${n.id ? `<button class="btn sm pri" data-nact="done" title="${TN.doneTip}">${I("check")}${TN.doneEdit}</button>` : ""}<button class="btn sm ghost icon" data-nact="more" title="${TN.more}" aria-label="${TN.more}">${I("ellipsis")}</button></div>
    <div class="banner warn" id="ne-conf" hidden></div>
    <div class="banner ok" id="ne-ai" hidden></div>
    <input id="ne-title" class="ntitle" value="${esc(n.title)}" placeholder="${TN.title}" aria-label="${TN.title}" autocomplete="off" spellcheck="false">
    <div class="meta" id="ne-meta"></div>
    <div class="ne" id="ne-body"></div>`;
  setState(n.id ? TN.autoSave : "");
  tidyReady();
  drawMeta();
  noteEd = createNoteEditor($("#ne-body"), n.body, {
    uid: n.uid || undefined,
    onChange: touch,
    onFiles: (files) => void insertFiles(files),
    onMessage: (m) => say(m),
    todoReminder: (text) => {
      const r = edTodoRem(text);
      return r ? whenShort(fireAt(r)) : null;
    },
    onTodoRemind: (t, anchor) => void remindTodo(t.text, anchor),
    onTodoToggle: (t) => void todoToggled(t.text, t.done),
    onRemindText: (text, anchor) => void remindText(text, anchor),
    onCursor: (on) => lightTools(on),
    // 钉在上面的工具栏
    topMargin: 56,
    onEscape: () => leaveEdit(),
  });
  lightTools = fillToolbar($("#ne-bar"), () => noteEd);
  S.editing = true;
  const title = $<HTMLInputElement>("#ne-title");
  title.addEventListener("input", touch);
  title.addEventListener("keydown", (e) => {
    if ((e.key === "Enter" && !e.isComposing) || e.key === "ArrowDown") {
      e.preventDefault();
      noteEd?.view.dispatch({ selection: { anchor: 0 } });
      noteEd?.focus();
    }
  });
  // 双击进来的：光标放到双击的地方（-1 是标题）；笔记本里正在搜：直接选中第一处，F3 接着找
  const at = editAt;
  editAt = null;
  const q = S.q.trim();
  if (at && at.line < 0) title.focus();
  else if (at) noteEd.placeAt(at.line, at.word);
  else if (q && n.id) noteEd.find(q);
  else if (n.id) noteEd.focus();
}

/** 收起右边的编辑器：没存的先存；还在冲突、没选用哪一版的，这里的内容另存一条冲突副本，两边都不丢 */
function finishEdit() {
  if (conflict && noteEd) void api.saveNote(newNote({ title: TN.conflictCopy($<HTMLInputElement>("#ne-title").value || edNote?.title || ""), body: noteEd.value, tag: edTag })).catch(() => {});
  else if (pendingSave) void saveNote();
  noteEd?.destroy();
  noteEd = null;
  edNote = null;
  conflict = null;
  lightTools = () => {};
  S.editing = false;
}

/** 不存了（笔记删掉了）直接收起 */
function dropEdit() {
  pendingSave = false;
  conflict = null;
  finishEdit();
}

/** 列表重画时：右边开着的这条在别处改了。这里没改过就直接换上，改过了就问用哪一版 */
function syncOpenNote() {
  const n = edNote;
  if (!n) return;
  // 只看的：换上最新的（别处改了、勾了待办）
  if (!noteEd) {
    const fresh = S.notes.find((x) => x.id === n.id);
    if (!fresh || S.noteWins.has(n.id)) return renderNoteDoc();
    if (fresh.body !== savedBody || fresh.title !== savedTitle || fresh.tag !== n.tag || fresh.pinned_at !== n.pinned_at || fresh.reminder_id !== n.reminder_id) mountView(fresh);
    else drawMeta();
    return;
  }
  noteEd.refresh();
  tidyReady();
  if (!n.id) return drawMeta();
  const fresh = S.notes.find((x) => x.id === n.id);
  if (!fresh || S.noteWins.has(n.id)) {
    // 别处删掉了，或者在单独的窗口里打开了
    if (!fresh) dropEdit();
    return renderNoteDoc();
  }
  const tagUntouched = edTag === n.tag;
  if (tagUntouched) edTag = fresh.tag;
  edNote = { ...n, pinned_at: fresh.pinned_at, reminder_id: fresh.reminder_id, updated_at: fresh.updated_at, tag: tagUntouched ? fresh.tag : n.tag };
  drawMeta();
  if (fresh.body === savedBody && fresh.title === savedTitle) return;
  const title = $<HTMLInputElement>("#ne-title");
  if (fresh.body === savedBody) {
    // 只改了标题
    if (title.value === savedTitle) {
      savedTitle = title.value = fresh.title;
      edNote = { ...edNote, title: fresh.title };
    }
    return;
  }
  if (noteEd.value === savedBody && title.value === savedTitle) {
    edNote = { ...fresh };
    savedBody = fresh.body;
    savedTitle = title.value = fresh.title;
    noteEd.setValue(fresh.body);
    pendingSave = false;
    setState(TN.usedTheirs);
    return;
  }
  showConflict(fresh);
}

// ---------- 附件、待办和提醒（和单独的笔记窗口一样） ----------

async function insertFiles(files: File[]) {
  for (const f of files) {
    if (f.size > MAX_ATTACHMENT) {
      say(TN.tooBig(f.name));
      continue;
    }
    try {
      if (!edNote?.id) await saveNote({ even: true });
      const n = edNote;
      if (!n?.id || !noteEd) return;
      const image = f.type.startsWith("image/");
      const file = await api.saveAttachment(new Uint8Array(await f.arrayBuffer()), f.type, n.uid, image ? "" : f.name);
      const label = f.name.replace(/[[\]]/g, "_") || T.file;
      noteEd.insertLine(image ? `![](att:${file})` : `[${label}](att:${file})`);
    } catch (err) {
      say(TN.notSaved(f.name, errText(err)));
    }
  }
  noteEd?.focus();
}

/** 右边这条笔记里，这条待办设过的提醒；all：完成了的也算 */
const edTodoRem = (text: string, all = false) => S.reminders.find((r) => !!edNote?.uid && r.note_uid === edNote.uid && r.todo === text && (all || !r.done));

/** 待办后面的「设提醒」/ 提醒时间：打一句话设时间，设过的能改、能不提醒了 */
async function remindTodo(text: string, anchor: HTMLElement) {
  const r = edTodoRem(text);
  const res = await askRemind({ anchor, due: r ? fireAt(r) : undefined, canDelete: !!r });
  if (!res) return;
  try {
    if (res === "delete") {
      if (r) await api.deleteReminder(r.id);
      return void (r && say(TN.rpDeleted));
    }
    if (pendingSave || !edNote?.id) await saveNote({ even: true });
    const n = edNote;
    if (!n?.id) return;
    await api.saveReminder(r ? { ...r, due_at: res.due, snoozed_until: null, vague: false } : newReminder({ title: text, note_uid: n.uid, todo: text, due_at: res.due }));
    say(TN.rpSaved(whenShort(res.due)));
  } catch (err) {
    say(TN.notDone(errText(err)));
  }
}

/** 勾上 / 取消待办：设过提醒的，提醒跟着完成 / 取消完成。先存好，提醒那边就不会再去改这一行 */
async function todoToggled(text: string, done: boolean) {
  const r = edTodoRem(text, true);
  if (!r || isRecurring(r.repeat) || r.done === done) return;
  await saveNote();
  try {
    await api.reminderAction(r.id, done ? "done" : "undo");
    if (done) say(TN.rpTodoDone);
  } catch (err) {
    say(TN.notDone(errText(err)));
  }
}

/** 选中的文字设成提醒：走随手记的理解（时间、周期、紧急），关联这条笔记 */
async function remindText(text: string, anchor: HTMLElement) {
  if (!text) return;
  const parsed = parseLocal(text, new Date(), S.devices).find((i) => i.type === "reminder");
  const title = parsed?.type === "reminder" && parsed.title && text.includes(parsed.title) ? parsed.title : text;
  const res = await askRemind({ anchor, title, phrase: parsed ? text : "" });
  if (!res || res === "delete") return;
  try {
    if (pendingSave || !edNote?.id) await saveNote({ even: true });
    const n = edNote;
    if (!n?.id) return;
    const same = parsed?.type === "reminder" && parsed.due === res.due;
    await api.saveReminder(
      newReminder({
        title,
        note_uid: n.uid,
        due_at: res.due,
        ...(same && parsed?.type === "reminder" ? { repeat: parsed.repeat, urgent: parsed.urgent, tag: parsed.tag, detail: parsed.detail } : {}),
      }),
    );
    say(TN.rpSaved(whenShort(res.due)));
  } catch (err) {
    say(TN.notDone(errText(err)));
  }
}

// ---------- 让 AI 整理：整理完直接换上，上面一条「整理好了」能撤销；少了一大截的先问一句 ----------

let beforeTidy: { title: string; body: string } | null = null;
let tidyPending: { title: string; body: string } | null = null;

function showAiBar(kind: "ok" | "warn", title: string, sub: string, btns: string) {
  const bar = $("#ne-ai");
  bar.className = `banner ${kind}`;
  bar.innerHTML = `${I(kind === "ok" ? "sparkles" : "triangle-alert")}<div><b>${esc(title)}</b>${esc(sub)}</div>${btns}`;
  bar.hidden = false;
}

function applyTidy(r: { title: string; body: string }) {
  const title = $<HTMLInputElement>("#ne-title");
  if (!noteEd) return;
  beforeTidy = { title: title.value, body: noteEd.value };
  title.value = r.title;
  noteEd.setValue(r.body);
  touch();
  showAiBar("ok", TN.aiDone, ` · ${TN.aiDoneSub}`, `<button class="btn sm" data-ai="undo">${TN.aiUndo}</button><button class="btn sm ghost icon" data-ai="x" aria-label="${T.close}">${I("x")}</button>`);
}

async function tidyNote(btn: HTMLButtonElement) {
  const ed = noteEd;
  if (!ed) return;
  if (!ed.value.trim()) return say(TN.aiEmpty);
  if (!S.settings?.ai_key) return say(TN.aiNoKey);
  const ask = await askTidy(btn);
  if (ask === null) return;
  btn.disabled = true;
  btn.innerHTML = `<span class="spin"></span>${TN.aiBusy}`;
  $("#ne-ai").hidden = true;
  try {
    const r = await api.aiTidyNote($<HTMLInputElement>("#ne-title").value, ed.value, ask);
    if (noteEd !== ed) return;
    if (shrunkMuch(ed.value, r.body)) {
      tidyPending = r;
      showAiBar("warn", TN.aiShrunk, ` · ${TN.aiShrunkSub}`, `<button class="btn sm" data-ai="use">${TN.aiUse}</button><button class="btn sm ghost" data-ai="drop">${TN.aiDrop}</button>`);
    } else applyTidy(r);
  } catch (err) {
    say(TN.aiFail(errText(err)));
  } finally {
    btn.disabled = false;
    btn.innerHTML = `${I("sparkles")}${TN.aiButton}`;
  }
}

/** 笔记分类：一个小菜单 */
function catMenu(btn: HTMLElement) {
  const list = noteCatList();
  menu.innerHTML = list.length
    ? `<button class="mi" data-cat="">${TN.catNone}</button>${list.map((c) => `<button class="mi" data-cat="${esc(c.id)}"><span class="tag" style="${tagColor(c.color)}">${esc(c.name)}</span>${c.id === edTag ? I("check") : ""}</button>`).join("")}`
    : `<div class="mi">${TN.catEmpty}</div>`;
  placeMenu(btn);
  menu.onclick = (e) => {
    const mi = (e.target as HTMLElement).closest<HTMLElement>("[data-cat]");
    if (!mi) return;
    closeMenu();
    edTag = mi.dataset.cat || null;
    drawMeta();
    touch();
  };
}

$("#nb-doc").addEventListener("click", async (e) => {
  const el = e.target as HTMLElement;
  const cf = el.closest<HTMLElement>("[data-cf]")?.dataset.cf;
  if (cf && conflict) {
    const fresh = conflict;
    conflict = null;
    $("#ne-conf").hidden = true;
    if (cf === "theirs") {
      edNote = fresh;
      edTag = fresh.tag;
      savedBody = fresh.body;
      savedTitle = $<HTMLInputElement>("#ne-title").value = fresh.title;
      noteEd?.setValue(fresh.body);
      pendingSave = false;
      setState(TN.usedTheirs);
    } else {
      // 保留我的：当作那边的已经看过了，存一次盖上去
      savedBody = fresh.body;
      await saveNote({ force: true });
    }
    return;
  }
  const ai = el.closest<HTMLElement>("[data-ai]")?.dataset.ai;
  if (ai) {
    if (ai === "undo" && beforeTidy && noteEd) {
      $<HTMLInputElement>("#ne-title").value = beforeTidy.title;
      noteEd.setValue(beforeTidy.body);
      beforeTidy = null;
      touch();
    } else if (ai === "use" && tidyPending) {
      applyTidy(tidyPending);
      tidyPending = null;
      return;
    }
    tidyPending = null;
    $("#ne-ai").hidden = true;
    return;
  }
  const b = el.closest<HTMLElement>("[data-nact]");
  if (!b) return;
  const n = edNote ?? S.notes.find((x) => x.id === S.noteId);
  switch (b.dataset.nact) {
    case "edit":
      enterEdit();
      break;
    case "done":
      leaveEdit();
      break;
    case "tidy": {
      // 只看时先进编辑（整理完要能撤销、能接着改）
      if (!noteEd) enterEdit();
      const t = document.querySelector<HTMLButtonElement>("#ne-tidy");
      if (t && noteEd) void tidyNote(t);
      break;
    }
    case "more":
      if (n) noteMenu(b, n);
      break;
    case "win":
      openNoteWindow(S.noteId);
      break;
    case "cat":
      catMenu(b);
      break;
    case "rem": {
      const r = S.reminders.find((x) => x.id === n?.reminder_id);
      if (r) openEditor(structuredClone(r));
      break;
    }
  }
});

// ---------- 回收站（从 设置 → 数据 进来） ----------

function trashLeft(deletedAt: string) {
  const d = new Date(deletedAt.replace(" ", "T"));
  const left = TRASH_DAYS - Math.floor((Date.now() - d.getTime()) / 86400000);
  return left > 0 ? T.daysLeft(left) : T.goingSoon;
}

function renderTrash() {
  const items = [
    ...S.trash.reminders.map((r) => ({ kind: "reminder" as TrashKind, id: r.id, title: r.title, at: r.deleted_at ?? "", sub: T.trashRem(timeCol(r.due_at, true)) })),
    ...S.trash.notes.map((n) => ({ kind: "note" as TrashKind, id: n.id, title: n.title || T.blankNote, at: n.deleted_at ?? "", sub: T.trashNote })),
  ].sort((a, b) => b.at.localeCompare(a.at));
  $<HTMLButtonElement>("#trash-empty").disabled = !items.length;
  const row = (it: (typeof items)[number]) => `<div class="it" data-kind="${it.kind}" data-id="${it.id}">
      <span class="ico">${I(it.kind === "reminder" ? "bell" : "file-text")}</span>
      <span class="t">${esc(it.title)}<small>${esc(it.sub)}</small></span>
      <span class="tm">${trashLeft(it.at)}</span>
      <span class="hacts"><button class="btn sm" data-tact="restore">${T.restore}</button><button class="btn sm ghost danger" data-tact="purge">${T.purge}</button></span></div>`;
  const tk = todayKey();
  const today = items.filter((x) => x.at.startsWith(tk));
  const older = items.filter((x) => !x.at.startsWith(tk));
  $("#trash-list").innerHTML = items.length
    ? (today.length ? grp(T.trashToday, today.map(row).join("")) : "") + (older.length ? grp(T.earlier, older.map(row).join("")) : "")
    : emptyHtml("happy", T.trashEmpty(mascotOn()), "", "trash");
}

$("#trash-list").addEventListener("click", async (e) => {
  const b = (e.target as HTMLElement).closest<HTMLElement>("[data-tact]");
  const row = b?.closest<HTMLElement>("[data-kind]");
  if (!b || !row) return;
  const action = b.dataset.tact as "restore" | "purge";
  const kind = row.dataset.kind as TrashKind;
  if (action === "purge" && !armed(b, T.surePurge)) return;
  try {
    await api.trashAction(kind, Number(row.dataset.id), action);
    say(action === "purge" ? T.purged : kind === "reminder" ? T.restoredRem : T.restoredNote);
  } catch (err) {
    say(errText(err));
  }
});
$("#trash-back").onclick = () => go("settings");
$("#trash-empty").onclick = () => {
  const n = S.trash.reminders.length + S.trash.notes.length;
  confirmBox(T.emptyAsk, T.emptyWhy(n), T.purge, async () => {
    try {
      say(T.purgedN(await api.emptyTrash()));
    } catch (err) {
      say(errText(err));
    }
  });
};

/** 要确认的弹窗（U2：按钮写动作，不写「确定」） */
function confirmBox(title: string, text: string, goLabel: string, run: () => void) {
  const box = $("#dlg");
  box.innerHTML = `<div class="dialog" role="alertdialog" aria-labelledby="dlg-t"><h4 id="dlg-t">${esc(title)}</h4><p>${esc(text)}</p>
    <div class="acts"><button class="btn" data-dlg="no">${T.cancel}</button><button class="btn danger solid" data-dlg="go">${esc(goLabel)}</button></div></div>`;
  box.hidden = false;
  box.querySelector<HTMLElement>('[data-dlg="no"]')!.focus();
  box.onclick = (e) => {
    const a = (e.target as HTMLElement).closest<HTMLElement>("[data-dlg]")?.dataset.dlg;
    if (!a && e.target !== box) return;
    box.hidden = true;
    if (a === "go") run();
  };
}

// ---------- Ctrl K：笔记和提醒一起搜，在窗口中间弹出（样子同快捷框，搜法和快捷框里的 ? 一样） ----------

type Hit = { kind: "note"; n: Note } | { kind: "rem"; r: Reminder };
let hits: Hit[] = [];
let hitSel = 0;
const pal = $("#pal");
const palIn = $<HTMLInputElement>("#pal-in");

function openSearch() {
  closeMenu();
  pal.hidden = false;
  $("#pdim").hidden = false;
  palIn.value = "";
  hitSel = 0;
  renderSearch();
  palIn.focus();
}

function closeSearch() {
  pal.hidden = true;
  $("#pdim").hidden = true;
}

/** 标题和正文都找：笔记一组；提醒一组，没做完的在前、做完的在后 */
function findHits(q: string) {
  const ql = q.toLowerCase();
  const has = (...s: string[]) => s.some((x) => x.toLowerCase().includes(ql));
  const rems = S.reminders.filter((r) => has(r.title, r.detail));
  return {
    notes: S.notes.filter((n) => has(n.title, n.body)).slice(0, MAX_HITS),
    rems: [...rems.filter((r) => !r.done), ...rems.filter((r) => r.done)].slice(0, MAX_HITS),
  };
}

function renderSearch() {
  const q = palIn.value.trim();
  const res = $("#pal-res");
  if (!q) {
    // 还没打字：最近的几条笔记
    hits = S.notes.slice(0, 4).map((n): Hit => ({ kind: "note", n }));
    res.innerHTML = hits.length ? `<div class="lab">${T.recentTitle}</div>${hits.map((h, i) => hitRow(h, i, "")).join("")}` : `<p class="none">${T.searchTip}</p>`;
    return paintHits();
  }
  const { notes, rems } = findHits(q);
  hits = [...notes.map((n): Hit => ({ kind: "note", n })), ...rems.map((r): Hit => ({ kind: "rem", r }))];
  hitSel = Math.min(hitSel, Math.max(0, hits.length - 1));
  res.innerHTML = hits.length
    ? (notes.length ? `<div class="lab">${T.notesHit(notes.length)}</div>` + notes.map((n, i) => hitRow({ kind: "note", n }, i, q)).join("") : "") +
      (rems.length ? `<div class="lab">${T.remsHit(rems.length)}</div>` + rems.map((r, i) => hitRow({ kind: "rem", r }, notes.length + i, q)).join("") : "")
    : emptyHtml("normal", T.notFound(mascotOn()), "", "search");
  paintHits();
}

function hitRow(h: Hit, i: number, q: string) {
  if (h.kind === "note") {
    const n = h.n;
    return `<div class="r" data-hit="${i}"><span class="ic note">${I("file-text")}</span><div class="m"><b>${highlight(esc(n.title || T.blankNote), q)}</b><small>${snippet(n.body, q) || T.blankNote}</small></div><kbd>↵</kbd></div>`;
  }
  const r = h.r;
  const sub = [r.done ? T.doneWord : timeCol(fireAt(r), true), isRecurring(r.repeat) ? repeatLabel(r.repeat, r.due_at) : "", tagList().find((t) => t.id === r.tag)?.name ?? ""]
    .filter(Boolean)
    .join(" · ");
  return `<div class="r" data-hit="${i}"><span class="ic rem">${I(isRecurring(r.repeat) ? "repeat" : "bell")}</span><div class="m"><b>${highlight(esc(r.title), q)}</b><small>${esc(sub)}</small></div><kbd>↵</kbd></div>`;
}

function paintHits() {
  $$("#pal-res [data-hit]").forEach((el) => el.classList.toggle("sel", Number(el.dataset.hit) === hitSel));
  document.querySelector(`#pal-res [data-hit="${hitSel}"]`)?.scrollIntoView({ block: "nearest" });
}

function openHit(h: Hit | undefined) {
  if (!h) return;
  closeSearch();
  if (h.kind === "note") openNote(h.n.id);
  else {
    go("reminders");
    openEditor(structuredClone(h.r));
  }
}

palIn.addEventListener("input", () => {
  hitSel = 0;
  renderSearch();
});
palIn.addEventListener("keydown", (e) => {
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    if (!hits.length) return;
    hitSel = (hitSel + (e.key === "ArrowDown" ? 1 : -1) + hits.length) % hits.length;
    paintHits();
  } else if (e.key === "Enter" && !e.isComposing) {
    e.preventDefault();
    openHit(hits[hitSel]);
  }
});
$("#pal-res").addEventListener("click", (e) => {
  const r = (e.target as HTMLElement).closest<HTMLElement>("[data-hit]");
  if (r) openHit(hits[Number(r.dataset.hit)]);
});
$("#pdim").onclick = closeSearch;
$("#tsearch").onclick = openSearch;

// ---------- 顶上的横幅：新版本、同步出问题、快捷键被占、协议重大修改；一次只显示最要紧的一条 ----------

let hotkeyNotice: string | null = null;
/** 这次打开时点过 ✕ 的（同步、快捷键） */
const bannerOff = new Set<string>();

type Banner = { kind: string; cls: string; ic: IconName; title: string; text?: string; prog?: number; btns: string; x: boolean };

/** 协议的官网地址（「看看全文」）：现在生效的那版，有没生效的新版时页面顶上有链接过去（SITE.md 4.5） */
const TERMS_URL: Record<keyof TermsView, string> = {
  sync_terms: "https://dingmi.app/sync/terms/",
  terms: "https://dingmi.app/terms/",
  privacy: "https://dingmi.app/privacy/",
};
/** 横幅上现在是哪份协议的通知：点「知道了」时按它记下 */
let termsShown: { key: keyof TermsView; version: string } | null = null;

/** 协议重大修改的通知（需求-1.x 第 36.6 节）：每份每次只通知一次；叮咪同步服务协议只给开着叮咪同步的电脑 */
function termsBanner(): Banner | null {
  termsShown = null;
  const t = S.terms;
  if (!t) return null;
  const syncOn = !!S.sync?.official && S.sync.state !== "off";
  for (const key of ["sync_terms", "terms", "privacy"] as const) {
    const d = t[key];
    const n = d.notice;
    if (!n || (key === "sync_terms" && !syncOn)) continue;
    termsShown = { key, version: n.version };
    const name = T.bn.termsName[key];
    const btn = (a: string, label: string, pri = false) => `<button class="btn sm${pri ? " pri" : ""}" data-bn="${a}">${label}</button>`;
    return {
      kind: "terms",
      cls: "info",
      ic: "file-text",
      title: d.started ? T.bn.termsLive(name, n.effective) : T.bn.termsSoon(name, n.effective),
      text: `${n.summary}${/[。！？.!?]$/.test(n.summary) ? "" : "。"}${T.bn.termsWhatIf(key === "sync_terms", d.started)}`,
      btns: btn("terms-read", T.bn.termsRead) + btn("terms-ok", T.bn.termsOk, true),
      x: false,
    };
  }
  return null;
}

function pickBanner(): Banner | null {
  const u = S.update;
  const s = S.sync;
  const has = !!u?.version && u.state !== "idle";
  const btn = (a: string, label: string, pri = false) => `<button class="btn sm${pri ? " pri" : ""}" data-bn="${a}">${label}</button>`;
  const prog = u?.state === "downloading" ? pct(u) : undefined;
  if (u?.too_old) return { kind: "old", cls: "err", ic: "circle-alert", title: TOO_OLD, prog, btns: has && u.state !== "downloading" ? btn("update", u.state === "ready" ? T.bn.restart : T.bn.update, true) : "", x: false };
  if (u && has && (u.state === "downloading" || u.state === "ready") && !u.dismissed)
    return { kind: "upd", cls: "info", ic: "download", title: u.state === "ready" ? T.bn.ready(u.version!) : T.bn.downloading(u.version!, pct(u)), prog, btns: u.state === "ready" ? btn("update", T.bn.restart, true) : "", x: true };
  if (s && !bannerOff.has("sync")) {
    // 叮咪同步：数据删了、到期了、快到期了（需求-1.0 第 18.4 节），带「续费」
    const a = s.official ? s.account : null;
    if (s.official && s.refused === "deleted") return { kind: "sync", cls: "err", ic: "cloud-off", title: T.bn.deleted, text: T.bn.deletedWhy, btns: btn("renew", T.bn.renew, true), x: true };
    if (s.official && s.state === "expired")
      return { kind: "sync", cls: "err", ic: "cloud-off", title: T.bn.expired, text: T.bn.expiredWhy(a?.delete_on ?? null), btns: btn("renew", T.bn.renew, true), x: true };
    if (a && expiringSoon(a)) return { kind: "sync", cls: "warn", ic: "alarm-clock", title: T.bn.expiring(a.days_left ?? 0), btns: btn("renew", T.bn.renew), x: true };
    if (s.state === "locked") return { kind: "sync", cls: "warn", ic: "lock", title: T.bn.locked, btns: btn("sync", T.bn.unlock), x: true };
    if (s.state === "keys_lost") return { kind: "sync", cls: "warn", ic: "key-round", title: T.bn.keysLost, btns: btn("sync", T.bn.rejoin), x: true };
    if (s.state === "outdated") return { kind: "sync", cls: "warn", ic: "cloud-off", title: T.bn.outdated, btns: "", x: true };
    if (s.state === "offline" && s.kind === "folder") return { kind: "sync", cls: "warn", ic: "folder-x", title: T.bn.lost, text: T.bn.lostWhy, btns: btn("sync", T.bn.look), x: true };
  }
  if (hotkeyNotice && !bannerOff.has("hk")) return { kind: "hk", cls: "warn", ic: "triangle-alert", title: hotkeyNotice, btns: btn("keys", T.bn.toSettings), x: true };
  const terms = termsBanner();
  if (terms) return terms;
  if (u && has && !u.dismissed) {
    const line = u.notes?.split("\n").find((l) => l.trim())?.replace(/^[-*#\s]+/, "");
    return { kind: "upd", cls: u.error ? "warn" : "info", ic: "info", title: u.error ?? T.bn.newVer(u.version!), text: u.error ? undefined : line, btns: btn("notes", T.bn.notes) + btn("update", u.error ? T.bn.retry : T.bn.update, true), x: true };
  }
  return null;
}

function renderBanner() {
  const box = $("#banner");
  const b = pickBanner();
  box.hidden = !b;
  if (!b) return void (box.innerHTML = "");
  box.className = `banner ${b.cls}`;
  box.dataset.kind = b.kind;
  box.innerHTML = `${I(b.ic)}<div><b>${esc(b.title)}</b>${b.text ? `<span>${esc(b.text)}</span>` : ""}${b.prog !== undefined ? `<i class="prog"><i style="width:${b.prog}%"></i></i>` : ""}</div>${b.btns}${b.x ? `<button class="btn sm ghost icon" data-bn="x" title="${T.bn.close}" aria-label="${T.bn.close}">${I("x")}</button>` : ""}`;
}

$("#banner").addEventListener("click", (e) => {
  const a = (e.target as HTMLElement).closest<HTMLElement>("[data-bn]")?.dataset.bn;
  const kind = $("#banner").dataset.kind ?? "";
  if (a === "update") startUpdate();
  else if (a === "notes") showUpdateNotes();
  else if (a === "sync") openSettings("sync");
  else if (a === "renew") {
    // 续费：先到 设置 → 同步，再弹出填兑换码（数据删了的是走重新开通）
    openSettings("sync", "account");
    showRenew();
  }
  else if (a === "keys") openSettings("general", "hk_flash");
  else if (a === "terms-read" && termsShown) void api.openUrl(TERMS_URL[termsShown.key]).catch((err) => say(errText(err)));
  else if (a === "terms-ok" && termsShown) {
    // 知道了：记在这台电脑上，这次修改不再显示（先在这里拿掉，terms-changed 随后再来一份）
    const { key, version } = termsShown;
    if (S.terms) S.terms = { ...S.terms, [key]: { ...S.terms[key], notice: null } };
    void api.termsSeen(key, version).catch(() => {});
    renderBanner();
  }
  else if (a === "x") {
    // 新版本的 ✕：这个版本不再提示，出了更新的版本再提示；别的这次打开时不再提示
    if (kind === "upd") void api.updateDismiss();
    else bannerOff.add(kind);
    renderBanner();
  }
});

async function checkNotice() {
  hotkeyNotice = await api.hotkeyNotice().catch(() => null);
  renderBanner();
}

function renderUpdate() {
  renderBanner();
  renderAboutRows();
}

// ---------- 标题栏右边的同步状态：已同步（绿点）/ 正在同步（橘点）/ 出问题（红点） ----------

function renderTitleSync() {
  const s = S.sync;
  const box = $("#tsync");
  box.hidden = !s || s.state === "off";
  if (!s || s.state === "off") return;
  const soon = s.official && expiringSoon(s.account);
  const [cls, text] =
    s.official && s.refused === "deleted" ? ["bad", T.sy.deleted]
    : s.state === "expired" ? ["bad", T.sy.expired]
    : s.state === "online" && soon && !s.error ? ["warn", T.sy.expiring(s.account?.days_left ?? 0)]
    : s.state === "online" ? (s.error ? ["bad", T.sy.pushFail] : ["ok", T.sy.ok])
    : s.state === "offline" ? ["bad", s.kind === "folder" ? T.sy.noFolder : T.sy.offline]
    : s.state === "outdated" ? ["bad", T.sy.outdated]
    : s.state === "locked" ? ["bad", T.sy.locked]
    : s.state === "keys_lost" ? ["bad", T.sy.keysLost]
    : s.state === "keys_retry" ? ["bad", T.sy.keysRetry]
    : ["busy", s.kind === "folder" ? T.sy.syncing : T.sy.connecting];
  box.className = `tb-sync ${cls}`;
  $("#tsync-text").textContent = text;
  box.title = [s.device, s.server, s.error].filter(Boolean).join(" · ");
}
$("#tsync").onclick = () => openSettings("sync");

// ---------- 导航 ----------

function go(view: View) {
  if (S.view === "notes" && view !== "notes") {
    if (S.editing) finishEdit();
    // 点了「新笔记」又走开、什么都没写的：不留着
    freshNote = null;
  }
  stopRecording();
  closeMenu();
  S.view = view;
  $$(".pg").forEach((v) => (v.hidden = v.dataset.view !== view));
  $$(".pg:not([hidden]) .vb").forEach((s) => (s.scrollTop = 0));
  if (view === "settings") void renderSettings();
  if (view === "reminders") renderReminders();
  if (view === "notes") {
    renderTodos();
    if (!S.todoView) renderNotes();
  }
  renderCounts();
}

hooks.go = go;
hooks.renderAll = renderAll;
hooks.afterSave = () => {
  renderToday();
  void checkNotice();
};
hooks.renderUpdate = renderUpdate;

/** 从别的窗口跳过来：打开某条笔记、某条提醒，或者某一页 */
function applyNav(nav: Nav) {
  const r = nav.reminderId ? S.reminders.find((x) => x.id === nav.reminderId) : null;
  if (r) {
    go("reminders");
    openEditor(structuredClone(r));
  } else if (nav.noteId) openNote(nav.noteId);
  else {
    // 桌面组件里点「还有 N 条」：笔记本按分类筛好；点「组件设置」：直接到那一页
    if (nav.view === "notes" && nav.sub) S.nCat = nav.sub;
    if (nav.view === "settings") return openSettings(nav.sub, nav.row);
    if (nav.view === "todos") return showTodos(true);
    if (nav.view === "notes") S.todoView = false;
    go((["today", "reminders", "notes", "settings", "trash"].includes(nav.view) ? nav.view : "today") as View);
  }
}

// 键盘：录快捷键、Esc 关掉最上面那层、Ctrl + K 搜索、Ctrl + F 搜笔记、抽屉里 Ctrl + 回车保存
document.addEventListener(
  "keydown",
  async (e) => {
    if (await recordKey(e)) return;
    const inEditor = !!(e.target as HTMLElement).closest?.(".cm-editor, .tg-wrap");
    if (e.key === "Escape") {
      if (!menu.hidden) return closeMenu();
      if (!$("#dlg").hidden) return void ($("#dlg").hidden = true);
      if (!pal.hidden) return closeSearch();
      if (modalOpen()) return M.close?.();
      if (!drawer.hidden) return drawerClose?.();
      return;
    }
    if (e.key === "Enter" && e.ctrlKey && !drawer.hidden && drawer.contains(e.target as Node)) {
      e.preventDefault();
      return drawer.requestSubmit();
    }
    if (e.key === "k" && e.ctrlKey && !e.altKey && !inEditor) {
      e.preventDefault();
      return openSearch();
    }
    if ((e.key === "s" || e.key === "S") && e.ctrlKey && S.editing) {
      e.preventDefault();
      return void saveNote();
    }
    if (e.key === "f" && e.ctrlKey && !e.altKey && !inEditor) {
      e.preventDefault();
      // 在设置里：搜设置
      if (S.view === "settings") return focusSettingsSearch();
      S.todoView = false;
      go("notes");
      $<HTMLInputElement>("#nb-search").focus();
    }
  },
  true,
);

// 设置里的弹窗：点外面关掉（恢复码那种必须看完、确认过的不关）
$("#modal").addEventListener("mousedown", (e) => {
  if (e.target === $("#modal") && !$("#modal").querySelector("[data-sticky]")) hideModal();
});

// ---------- 启动 ----------

(async () => {
  let dir: string;
  [S.settings, S.focus, S.sync, dir] = await Promise.all([api.getSettings(), api.getFocus(), api.syncStatus(), api.attachmentDir()]);
  setTags(S.settings.tags);
  setNoteCats(S.settings.note_cats);
  getVersion().then((v) => ($("#brand").title = T.version(v)));
  if (await api.isTestCopy().catch(() => false)) $("#brand").insertAdjacentHTML("beforeend", `<span class="tag" style="--c:var(--danger)">${T.testCopy}</span>`);
  setAttachmentDir(dir);
  setTableGridButton(true);
  setFileSaveButton(true);
  void checkNotice();
  renderTitleSync();
  await load();
  const nav = await api.takeNav();
  if (nav) applyNav(nav);
  // 窗口是藏着建的：第一屏画好了再显示，免得先闪一下白底（见 win.rs）
  requestAnimationFrame(async () => {
    await appWin.show();
    await appWin.setFocus();
    if (!nav) cap.focus();
  });

  listen("data-changed", () => load());
  listen<Checklist>("checklist-changed", (e) => {
    checklist = e.payload;
    renderChecklist();
  });
  api.checklistGet().then((c) => {
    checklist = c;
    renderChecklist();
  }, () => {});
  listen<FocusInfo>("focus-changed", (e) => {
    S.focus = e.payload;
    renderFocus();
    renderToday();
  });
  listen<SyncStatus>("sync-status", (e) => {
    const before = S.sync?.state;
    S.sync = e.payload;
    renderTitleSync();
    renderBanner();
    settingsOnSync(before, e.payload);
  });
  listen<Settings>("settings-changed", (e) => {
    // 在桌面组件上点了「关掉」：设置页上的开关跟着变
    const widgetFlip = S.settings?.widget !== e.payload.widget;
    S.settings = e.payload;
    setTags(e.payload.tags);
    setNoteCats(e.payload.note_cats);
    renderAll();
    settingsOnChange(widgetFlip);
  });
  listen<Nav>("nav", (e) => applyNav(e.payload));
  // 单独的笔记窗口开了、关了：正在这里改同一条的先收起来
  const noteWins = (ids: number[]) => {
    S.noteWins = new Set(ids);
    if (S.view !== "notes" || S.todoView) return;
    if (!S.editing || (edNote?.id && S.noteWins.has(edNote.id))) renderNoteDoc();
  };
  listen<number[]>("note-windows", (e) => noteWins(e.payload));
  api.noteWindows().then(noteWins).catch(() => {});
  listen<UpdateStatus>("update-status", (e) => {
    S.update = e.payload;
    renderUpdate();
  });
  api.updateStatus().then((u) => {
    S.update = u;
    renderUpdate();
  });
  api.updateNotice().then((n) => n && say(n));
  // 协议：检查更新时顺便取的；主窗口没开着时来的，下次打开时显示
  listen<TermsView>("terms-changed", (e) => {
    S.terms = e.payload;
    renderBanner();
  });
  api.termsInfo().then((t) => {
    S.terms = t;
    renderBanner();
  }, () => {});
  // 时间在走，每分钟刷新一次"今天"
  setInterval(() => {
    renderToday();
    renderCounts();
  }, 60000);
})();
