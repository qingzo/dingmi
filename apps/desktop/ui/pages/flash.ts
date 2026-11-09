import "../styles/ui.css";
import "../styles/quickbox.css";
import { mascotOn } from "../lib/look";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, errText, type Device, type LaunchConfig, type LaunchItem, type LaunchStats, type Note, type Reminder, type Settings } from "../lib/api";
import { EMPTY_STATS, firstRow, iconHtml, kindText, markHit, rank, shortName, visible, whereText, type Hit as AppHit } from "../lib/launch";
import { parseLocal, type ParsedItem, type ParsedReminder } from "../lib/parse";
import { calc, type Calc } from "../lib/calc";
import { webHit, type WebHit } from "../lib/webkey";
import { spaceOpens } from "../lib/flashspace";
import { quickSaveAll, undoSaved, type Saved } from "../lib/quick";
import { highlight, snippet } from "@shared/md";
import { icon, type IconName } from "@shared/icons";
import { cat, head } from "@shared/mascot";
import { $, ago, copyText, esc, fireAt, isRecurring, noteCatDef, parseDT, repeatLabel, setNoteCats, setTags, tagColor, tagDef, timeOf } from "@shared/util";
import { T } from "../text/quickbox";
import { whenPast, whenShort } from "../text/cards";

// 快捷框（U4）：打程序名打开程序，写一句话记成提醒或笔记，? 开头搜笔记和提醒；打算式出结果，「bd 天气」用网页搜（需求-0.3 第 10 节）。
// 空着时一排 8 个常用（Alt + 数字，或方向键选、回车 / 空格打开）和下一条提醒；打字后结果最多露 6 行，多了往下翻；回车执行选中那一行。
// 点到别处收起时，没写完的那句话留着，下次打开放回去；Esc 关掉的不留。
// 设置里开了「空格启动（实验）」时，打了字以后空格也能打开选中的程序（需求-1.x 第 37 节，规则见 lib/flashspace.ts）

const win = getCurrentWindow();
const stage = $("#stage");
const card = $("#card");
const inp = $<HTMLInputElement>("#inp");
const right = $("#right");
const msg = $("#msg");
const body = $("#body");
const ft = $("#ft");
const savedEl = $("#saved");
const menu = $("#menu");
let busy = false;
let useAI = false;
let devices: Device[] = [];
let calcOn = true;
/** 空格启动（实验）：设置里的开关 */
let spaceOn = false;
let webKeys: Settings["web_keys"] = [];

// 能打开的东西：启动时读一次，变了（launcher-changed）再读；用过几次每次打开时读
let items: LaunchItem[] = [];
let cfg: LaunchConfig = { pins: [], aliases: [], custom: [], hidden: [], off: [] };
let stats: LaunchStats = EMPTY_STATS;
/** 提醒：每次打开时读（空着时要显示下一条）；笔记：这次打开后第一次打字时才读 */
let rems: Reminder[] = [];
let notes: Note[] | null = null;
let loadingNotes: Promise<void> | null = null;

/** 常用里最多放几个（和桌面组件里一样） */
const MAX_PINS = 12;
/** 空着时那一排放几个（Alt + 1~8） */
/** 常用一排几个；设置里可以开两排 */
const FIRST_ROW = 8;
let pinRows = 1;
/** 输入框里是带进来的字（按快捷键前选中的、上次没写完的），第一次 Esc 先清空它 */
let picked = false;
/** 点到别处收起时没写完的那句话：下次打开放回去。Esc 关掉、记好了、打开了东西都不留 */
let draft = "";
const MAX_APPS = 6;
const MAX_SEARCH = 20;
/** 记好了那一行停多久 */
const SAVED_MS = 1500;

type Row =
  | { kind: "app"; h: AppHit }
  | { kind: "note"; n: Note }
  | { kind: "rem"; r: Reminder }
  | { kind: "rec" }
  | { kind: "only" }
  | { kind: "calc"; c: Calc }
  | { kind: "web"; w: WebHit };
let rows: Row[] = [];
let sel = 0;
/** 空着时那一排：常用的、最近用过的（见 launch.ts firstRow）；pinSel 是方向键选中的那个 */
let recents: LaunchItem[] = [];
let pinSel = -1;
/** 选中的那行是用方向键选的（空格启动用）；打字、鼠标挪过去都不算 */
let arrowPicked = false;
/** 第一行的程序是「像程序名」排上来的（空格启动用） */
let appFirst = false;
/** 这句话说得含糊（先定了时间），底下提示换成「先这样记下」 */
let vague = false;
/** 记好了：框缩成一行，消失前能撤销 */
let saved: { s: Saved; text: string } | null = null;
let savedTimer = 0;
/** 每清空一次加一：撤销到一半窗口关掉了，回来就不再动界面 */
let gen = 0;

const searchText = (v: string) => (/^[?？]/.test(v) ? v.slice(1).trim() : null);
const shown = () => visible(items, cfg);
const isRem = (x: ParsedItem): x is ParsedReminder => x.type === "reminder";

function loadNotes() {
  loadingNotes ??= api.listNotes().then(
    (list) => {
      notes = list;
      render();
    },
    () => void (notes = []),
  );
}

// ---------- 画 ----------

const kbd = (k: string) => `<kbd>${esc(k)}</kbd>`;
const ico = (cls: string, name: IconName) => `<span class="ic ${cls}">${icon(name)}</span>`;

/** 一行结果。右边：选中时 ↵，不然 Alt + 第几行（「只记成笔记」写 Tab） */
function line(i: number, ic: string, t: string, s: string, extra = "", cls = "", alt = i < 9 ? T.alt(i + 1) : "") {
  return `<div class="r${cls}" role="option" id="r${i}" data-i="${i}">${ic}<div class="main"><div class="t">${t}</div>${s ? `<div class="s">${s}</div>` : ""}${extra}</div><span class="k"><kbd class="go">↵</kbd>${alt ? `<kbd class="alt">${esc(alt)}</kbd>` : ""}</span></div>`;
}

/** 提醒什么时候：周期的写「每周五 17:00」 */
function remWhen(r: Reminder): string {
  if (r.done) return T.remDone;
  if (isRecurring(r.repeat)) return `${repeatLabel(r.repeat, r.due_at)} ${timeOf(fireAt(r))}`;
  const d = parseDT(fireAt(r));
  return d ? (d < new Date() ? whenPast(d) : whenShort(d)) : "";
}

function appSub({ it, why }: AppHit): string {
  const where = it.missing || ["folder", "file", "link"].includes(it.kind) ? whereText(it) : "";
  return [kindText(it), why, where].filter(Boolean).join(" · ");
}

/** 「记下来」那一行：预览把理解出的时间加粗写出来；说得含糊时下面一条黄色小条 */
function recHtml(i: number, v: string, parsed: ParsedItem[]): string {
  const rs = parsed.filter(isRem);
  const r = rs[0];
  if (!r) return line(i, ico("rem", "bell"), esc(T.rec(v)), esc(T.noTime) + (useAI ? ` · ${esc(T.aiCheck)}` : ""));
  const tag = tagDef(r.tag);
  const place = r.only_on.length ? devices.find((d) => d.uid === r.only_on[0])?.name : "";
  const parts = [
    esc(T.reminder),
    `<b>${esc(whenShort(parseDT(r.due) ?? new Date()))}</b>`,
    r.detail && esc(r.detail),
    isRecurring(r.repeat) && esc(repeatLabel(r.repeat, r.due)),
    r.urgent && esc(T.urgent),
    tag && esc(tag.name),
    place && esc(T.onlyOn(place)),
    rs.length > 1 && esc(T.count(rs.length)),
    parsed.some((x) => x.type === "note") && esc(T.alsoNote),
    useAI && esc(T.aiCheck),
  ].filter(Boolean);
  const hint = r.guess ? `<span class="hint">${icon("circle-help")}${esc(r.guess + T.guessTail)}</span>` : "";
  return line(i, ico("rem", isRecurring(r.repeat) ? "repeat" : "bell"), esc(T.rec(r.title)), parts.join(" · "), hint, hint ? " tall" : "");
}

function rowHtml(row: Row, i: number, v: string, parsed: ParsedItem[]): string {
  const q = searchText(v);
  const hl = q ?? v;
  if (row.kind === "app") return line(i, iconHtml(row.h.it, "ic"), markHit(row.h.it, hl), esc(appSub(row.h)), "", row.h.it.missing ? " gone" : "");
  if (row.kind === "note") {
    const n = row.n;
    const c = noteCatDef(n.tag);
    const tag = c ? `<span class="tagmini" style="${tagColor(c.color)}">${esc(c.name)}</span>` : "";
    const sub = q !== null ? snippet(n.body, q) : esc(ago(n.updated_at));
    return line(i, ico("note", "file-text"), highlight(esc(n.title || T.blankNote), hl) + tag, `${esc(T.note)}${sub ? ` · ${sub}` : ""}`);
  }
  if (row.kind === "rem") {
    const r = row.r;
    return line(i, ico("rem", isRecurring(r.repeat) ? "repeat" : "bell"), highlight(esc(r.title), hl), esc(`${T.reminder} · ${remWhen(r)}`), "", r.done ? " gone" : "");
  }
  if (row.kind === "only") return line(i, ico("note", "notebook-pen"), esc(T.noteOnly), esc(T.noteOnlySub), "", "", "Tab");
  if (row.kind === "calc") return line(i, ico("calc", "calculator"), esc(row.c.value === null ? T.calcBad : `= ${row.c.shown}`), esc(row.c.expr), "", " big");
  if (row.kind === "web") return line(i, ico("web", "search"), esc(T.webSearch(row.w.k.name, row.w.q)), esc(T.webSub(row.w.q)));
  return recHtml(i, v, parsed);
}

/** 笔记和提醒里找：? 开头时正文也找；平时只看标题，免得太多 */
function findNotes(q: string, deep: boolean, max: number): Row[] {
  if (!q) return [];
  const ql = q.toLowerCase();
  const has = (...s: string[]) => s.some((x) => x.toLowerCase().includes(ql));
  const rs = rems.filter((r) => (deep ? has(r.title, r.detail) : has(r.title)));
  // 没做完的提醒排在前面，再是笔记，做完的提醒最后
  return [
    ...rs.filter((r) => !r.done).map((r): Row => ({ kind: "rem", r })),
    ...(notes ?? []).filter((n) => (deep ? has(n.title, n.body) : has(n.title))).map((n): Row => ({ kind: "note", n })),
    ...(deep ? rs.filter((r) => r.done).map((r): Row => ({ kind: "rem", r })) : []),
  ].slice(0, max);
}

function render() {
  if (busy || saved) return;
  closeMenu();
  const v = inp.value.trim();
  const q = searchText(v);
  // 「关键字 + 空格」：带着空格看，只打关键字时不算
  const web = q === null ? webHit(inp.value, webKeys) : null;
  const mode = q !== null ? `<span class="mode">${icon("search")}${esc(T.search)}</span>` : web ? `<span class="mode">${icon("globe")}${esc(web.k.name)}</span>` : "";
  right.innerHTML = `${mode}<button class="qset" id="qset" tabindex="-1" title="${esc(T.settings)}" aria-label="${esc(T.settings)}">${icon("settings")}</button>`;
  appFirst = false;
  if (!v) return renderEmpty();
  if (!notes) loadNotes();
  pinSel = -1;
  let html: string;
  if (q !== null) {
    rows = findNotes(q, true, MAX_SEARCH);
    html = rows.length
      ? listHtml(v, [])
      : !notes
        ? `<div class="qempty plain">${esc(T.searching)}</div>`
        : !q
          ? `<div class="qempty plain">${esc(T.searchHint)}</div>`
          : `<div class="qempty"><span class="cat">${cat("normal")}</span>${esc(T.notFound(mascotOn()))}</div>`;
  } else if (web) {
    // 网页关键字：只有这一行和「记下来」（万一真想记这句话）
    vague = false;
    rows = [{ kind: "web", w: web }, { kind: "rec" }];
    html = listHtml(v, parseLocal(v, new Date(), devices));
  } else {
    const parsed = parseLocal(v, new Date(), devices);
    const hasTime = parsed.some(isRem);
    vague = !!parsed.find(isRem)?.guess;
    const apps = rank(v, shown(), cfg, stats, MAX_APPS).map((h): Row => ({ kind: "app", h }));
    const found = v.length >= 2 ? findNotes(v, false, 3) : [];
    const strong = apps[0]?.kind === "app" && apps[0].h.score >= 80;
    const sentence = hasTime || v.length >= 12 || (/\s/.test(v) && !strong);
    const rec: Row = { kind: "rec" };
    // 「只记成笔记」：认出了时间才有（没时间的本来就存成笔记）；开着 AI 时像一句话也给，AI 可能会设成提醒
    const only: Row[] = hasTime || (useAI && sentence) ? [{ kind: "only" }] : [];
    // 像程序名：程序在前；短短几个字、对上了笔记标题：笔记在前；像一句话：记下来在前，「只记成笔记」排第二
    rows =
      strong && !sentence
        ? [...apps, ...found, rec, ...only]
        : !sentence && found.length
          ? [...found, ...apps, rec, ...only]
          : [rec, ...only, ...found, ...apps.slice(0, 2)];
    // 算式：结果排第一行。认出了时间的（2026-03-05、3/5）是日期，不当算式
    const c = calcOn && !hasTime ? calc(v) : null;
    if (c) rows = [{ kind: "calc", c }, ...rows.filter((r) => r.kind !== "only")];
    appFirst = strong && !sentence && rows[0]?.kind === "app";
    html = listHtml(v, parsed);
  }
  sel = Math.min(sel, Math.max(0, rows.length - 1));
  body.innerHTML = `<div class="sep"></div>${html}`;
  paint();
}

const listHtml = (v: string, parsed: ParsedItem[]) =>
  `<div class="res" id="res" role="listbox">${rows.map((r, i) => rowHtml(r, i, v, parsed)).join("")}</div>`;

/** 下一条提醒：今天的；今天没有了就是明天的第一条；只在别的电脑上提醒的不算 */
function nextRem(): { r: Reminder; d: Date } | null {
  const now = new Date();
  const end = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 2);
  const me = devices.find((d) => d.me)?.uid;
  let best: { r: Reminder; d: Date } | null = null;
  for (const r of rems) {
    if (r.done || r.deleted_at || (r.only_on.length && !(me && r.only_on.includes(me)))) continue;
    const d = parseDT(fireAt(r));
    if (d && d >= now && d < end && (!best || d < best.d)) best = { r, d };
  }
  return best;
}

function nextHtml(): string {
  const nx = nextRem();
  if (!nx) return "";
  const now = new Date();
  const today = nx.d.getDate() === now.getDate();
  const min = Math.round((nx.d.getTime() - now.getTime()) / 60000);
  const left = !today ? "" : min < 1 ? T.soon : min < 60 ? T.inMin(min) : T.inHour(Math.floor(min / 60), min % 60);
  const when = today ? whenPast(nx.d) : whenShort(nx.d);
  return `<button class="next" id="next" tabindex="-1">${icon("bell")}${esc(T.next)}<b>${esc(`${when} ${nx.r.title}`)}</b>${left ? `<span class="in">${esc(left)}</span>` : ""}</button>`;
}

function renderEmpty() {
  rows = [];
  recents = firstRow(shown(), cfg, stats, FIRST_ROW * (pinRows === 2 ? 2 : 1));
  pinSel = Math.min(pinSel, recents.length - 1);
  const pins = recents.length
    ? `<div class="pins" role="listbox">${recents
        .map((it, i) => `<button class="pin" data-r="${i}" title="${esc(it.name)}" tabindex="-1">${iconHtml(it, "ic")}<span class="nm">${esc(shortName(it.name))}</span></button>`)
        .join("")}</div>`
    : "";
  const next = nextHtml();
  body.innerHTML = pins || next ? `<div class="sep"></div>${pins}${next}` : "";
  paintPins();
}

function paint() {
  for (const el of body.querySelectorAll<HTMLElement>(".r")) el.setAttribute("aria-selected", String(Number(el.dataset.i) === sel));
  body.querySelector(`.r[data-i="${sel}"]`)?.scrollIntoView({ block: "nearest" });
  if (rows.length) inp.setAttribute("aria-activedescendant", `r${sel}`);
  else inp.removeAttribute("aria-activedescendant");
  footer();
}

function paintPins() {
  for (const el of body.querySelectorAll<HTMLElement>(".pin")) el.setAttribute("aria-selected", String(Number(el.dataset.r) === pinSel));
  footer();
}

/** 现在按空格算不算打开选中的程序（空格启动，lib/flashspace.ts） */
function spaceNow(composing = false): boolean {
  return spaceOpens({
    on: spaceOn,
    composing,
    value: inp.value,
    app: rows[sel]?.kind === "app",
    picked: arrowPicked,
    top: appFirst && sel === 0,
    webKey: !!webHit(`${inp.value} `, webKeys),
  });
}

/** 底下一行：只列这一刻有用的几个按键 */
function footer() {
  const k = (keys: string[], t: string) => `<span>${keys.map(kbd).join("")}${esc(t)}</span>`;
  const K = T.key;
  const v = inp.value.trim();
  const row = rows[sel];
  const move = rows.length > 1 ? k(["↑", "↓"], K.pick) : "";
  if (!v && pinSel >= 0) {
    ft.innerHTML = k(["↵", K.space], K.open) + k(recents.length > FIRST_ROW ? ["←", "→", "↑", "↓"] : ["←", "→"], K.pick) + k(["Esc"], K.close);
  } else if (!v) {
    ft.innerHTML = (recents.length ? k(["Alt", `1–${Math.min(recents.length, 9)}`], K.pins) + k(["←", "→"], K.pick) : "") + k(["?"], K.searchPrefix) + k(["Esc"], K.close);
  } else if (searchText(v) !== null) {
    ft.innerHTML = (rows.length ? k(["↵"], K.openMain) + move : "") + k(["Esc"], K.close);
  } else if (row?.kind === "app") {
    const it = row.h.it;
    // 开着空格启动、这会儿按空格能打开：回车旁边多一个「空格」（橘色）
    const open = spaceNow() ? `<span>${kbd("↵")}<kbd class="new">${esc(K.space)}</kbd>${esc(K.open)}</span>` : k(["↵"], K.open);
    ft.innerHTML = open + (canAdmin(it) ? k(["Ctrl", "↵"], K.admin) : "") + (canDir(it) ? k(["Shift", "↵"], K.dir) : "") + k([K.rightClick], K.more);
  } else if (row?.kind === "rec") {
    const hasOnly = rows.some((r) => r.kind === "only");
    ft.innerHTML = vague
      ? k(["↵"], K.saveVague) + `<span>${esc(K.vagueTip)}</span>`
      : k(["↵"], K.save) + (hasOnly ? k(["Tab"], K.noteOnly) : "") + move;
  } else if (row?.kind === "only") {
    ft.innerHTML = k(["↵"], K.noteOnly) + k(["Tab"], K.save) + move;
  } else if (row?.kind === "calc") {
    ft.innerHTML = (row.c.value !== null ? k(["↵"], K.copy) : "") + move + k(["Esc"], K.close);
  } else if (row?.kind === "web") {
    ft.innerHTML = k(["↵"], row.w.q ? K.webGo : K.webHome) + `<span>${esc(K.webWhere)}</span>`;
  } else ft.innerHTML = k(["↵"], K.openMain) + move + k(["Esc"], K.close);
}

function say(text: string, kind: "ok" | "err" = "ok") {
  msg.hidden = !text;
  msg.innerHTML = text ? `<div class="banner ${kind}">${icon(kind === "ok" ? "circle-check" : "circle-alert")}<div>${esc(text)}</div></div>` : "";
}

/** 重新播一次进场动画 */
function replay(el: HTMLElement) {
  el.classList.remove("enter");
  void el.offsetWidth;
  el.classList.add("enter");
}

// ---------- 做 ----------

const canAdmin = (it: LaunchItem) => !["store", "link", "folder"].includes(it.kind) && (it.kind === "desk" || !!it.path);
const canDir = (it: LaunchItem) => !["store", "link"].includes(it.kind);

async function launch(it: LaunchItem, how: "open" | "admin" | "dir" = "open") {
  try {
    await api.launcherOpen(it.id, how, inp.value.trim() || undefined);
    close();
  } catch (err) {
    say(T.launchFail(it.name, errText(err)), "err");
  }
}

async function openHit(row: Row) {
  if (row.kind === "note") await api.openMainNav("notes", row.n.id);
  else if (row.kind === "rem") await api.openMainNav("reminders", undefined, row.r.id);
  close();
}

/** 回车复制算出来的结果（不带千分位），框缩成一行「已复制 448」 */
async function copyCalc(c: Calc) {
  if (c.value === null) return;
  try {
    await copyText(c.value);
  } catch (err) {
    return say(errText(err), "err");
  }
  showDone(T.copied(c.value), c.expr);
}

async function openWeb(w: WebHit) {
  try {
    await api.openUrl(w.url);
    close();
  } catch (err) {
    say(T.webFail(errText(err)), "err");
  }
}

/** 记下来；asNote 时只记成笔记 */
async function save(asNote = false) {
  const v = inp.value.trim();
  if (!v || busy) return;
  busy = true;
  inp.disabled = true;
  closeMenu();
  say("");
  body.innerHTML = `<div class="sep"></div><div class="qempty plain"><span class="spin"></span>${esc(useAI && !asNote ? T.thinking : T.saving)}</div>`;
  ft.innerHTML = "";
  try {
    const s = await quickSaveAll(v, useAI, devices, asNote);
    busy = false;
    showSaved(s, v);
  } catch (err) {
    busy = false;
    inp.disabled = false;
    render();
    say(T.saveFail(errText(err)), "err");
    inp.focus();
  }
}

/** 记好了：框缩成一行，团子猫蹦一下（关掉团子猫时是绿色的勾），过一会儿自己消失；鼠标停在上面时不消失 */
function showSaved(s: Saved, text: string) {
  saved = { s, text };
  const on = mascotOn();
  const r = s.items.find(isRem);
  const title = r ? T.savedRem(on, whenShort(parseDT(r.due) ?? new Date())) : T.savedNote(on);
  const sub = r
    ? [r.title, r.detail, s.reminders.length > 1 && T.count(s.reminders.length), r.guess && T.savedGuess, s.notes.length > 0 && T.savedAlsoNote]
    : [s.notes[0]?.title || text];
  if (s.aiFailed) sub.push(T.aiFailed);
  savedEl.innerHTML = `<span class="cat mascot play">${cat("happy")}</span><span class="nocat">${icon("check")}</span><div class="m"><b>${esc(title)}</b><span>${esc(sub.filter(Boolean).join(" · "))}</span></div><button class="undo" id="undo">${esc(T.undo)}</button>`;
  card.hidden = true;
  savedEl.hidden = false;
  replay(savedEl);
  armSaved(s.aiFailed ? SAVED_MS * 2 : SAVED_MS);
}

/** 做好了一件不用撤销的事（复制了结果）：框缩成一行，过一会儿自己消失 */
function showDone(title: string, sub: string) {
  savedEl.innerHTML = `<span class="cat mascot play">${cat("happy")}</span><span class="nocat">${icon("check")}</span><div class="m"><b>${esc(title)}</b><span>${esc(sub)}</span></div>`;
  card.hidden = true;
  savedEl.hidden = false;
  replay(savedEl);
  armSaved(SAVED_MS);
}

function armSaved(ms: number) {
  clearTimeout(savedTimer);
  savedTimer = window.setTimeout(() => close(), ms);
}

/** 撤销：刚记的删掉（不进回收站），回到输入框，这句话还在 */
async function undo() {
  if (!saved) return;
  const { s, text } = saved;
  const g = gen;
  clearTimeout(savedTimer);
  $<HTMLButtonElement>("#undo").disabled = true;
  try {
    await undoSaved(s);
  } catch (err) {
    if (g !== gen) return;
    $("#saved .m span").textContent = T.undoFail(errText(err));
    return armSaved(SAVED_MS * 3);
  }
  if (g !== gen) return;
  saved = null;
  savedEl.hidden = true;
  card.hidden = false;
  inp.disabled = false;
  inp.value = text;
  render();
  say(T.undone);
  inp.focus();
}

function run(i: number, how: "open" | "admin" | "dir" = "open") {
  const row = rows[i];
  if (!row || busy) return;
  if (row.kind === "app") return launch(row.h.it, how);
  if (row.kind === "rec") return save();
  if (row.kind === "only") return save(true);
  if (row.kind === "calc") return copyCalc(row.c);
  if (row.kind === "web") return openWeb(row.w);
  return openHit(row);
}

// ---------- 右键菜单：放到常用里（这里空着时那一排和桌面组件上都有）、起别名、不显示 ----------

let menuFor: LaunchItem | null = null;

function closeMenu() {
  if (menu.hidden) return;
  menu.hidden = true;
  menuFor = null;
  fit();
}

function openMenu(it: LaunchItem, x: number, y: number) {
  menuFor = it;
  const M = T.menu;
  const al = cfg.aliases.filter((a) => a.id === it.id).map((a) => a.alias);
  const mi = (m: string, ic: IconName, t: string, tail = "", cls = "") => `<button class="mi${cls}" data-m="${m}" role="menuitem">${icon(ic)}${esc(t)}${tail}</button>`;
  menu.innerHTML =
    mi("open", "external-link", M.open, kbd("↵")) +
    (canAdmin(it) ? mi("admin", "shield", M.admin, kbd("Ctrl ↵")) : "") +
    (canDir(it) ? mi("dir", "folder-open", M.dir, kbd("Shift ↵")) : "") +
    `<div class="msep"></div>` +
    mi("pin", "pin", cfg.pins.includes(it.id) ? M.unpin : M.pin) +
    mi("alias", "pencil", M.alias, al.length ? `<small>${esc(al.join("、"))}</small>` : "") +
    mi("hide", "eye-off", M.hide, "", " danger");
  menu.hidden = false;
  menu.style.left = `${Math.max(8, Math.min(x, innerWidth - menu.offsetWidth - 24))}px`;
  menu.style.top = `${y}px`;
  fit();
}

async function saveCfg(next: LaunchConfig, done: string) {
  try {
    await api.launcherSave(next);
    cfg = next;
    render();
    say(done);
  } catch (err) {
    say(T.cfgFail(errText(err)), "err");
  }
}

async function menuAction(m: string) {
  const it = menuFor;
  if (!it) return;
  if (m === "alias") return aliasForm(it);
  closeMenu();
  if (m === "open" || m === "admin" || m === "dir") return launch(it, m);
  if (m === "pin") {
    const on = cfg.pins.includes(it.id);
    if (!on && cfg.pins.length >= MAX_PINS) return say(T.pinsFull(MAX_PINS), "err");
    const pins = on ? cfg.pins.filter((p) => p !== it.id) : [...cfg.pins, it.id];
    return saveCfg({ ...cfg, pins }, on ? T.unpinned(it.name) : T.pinned(it.name));
  }
  if (m === "hide") return saveCfg({ ...cfg, hidden: [...cfg.hidden, it.id] }, T.hidden(it.name));
}

function aliasForm(it: LaunchItem) {
  menu.innerHTML = `<form class="alias" id="alias-form"><label for="alias-in">${esc(T.aliasLabel)}</label><input class="field" id="alias-in" placeholder="${esc(T.aliasEg(/[㐀-鿿]/.test(it.name) ? "wx" : "ps"))}" maxlength="20" autocomplete="off" spellcheck="false"><button class="btn pri sm">${esc(T.aliasOk)}</button></form>`;
  fit();
  const ai = $<HTMLInputElement>("#alias-in");
  ai.focus();
  $("#alias-form").addEventListener("submit", (e) => {
    e.preventDefault();
    const alias = ai.value.trim().toLowerCase();
    closeMenu();
    inp.focus();
    if (!alias) return;
    const aliases = [...cfg.aliases.filter((a) => a.alias !== alias), { alias, id: it.id }];
    saveCfg({ ...cfg, aliases }, T.aliased(alias, it.name));
  });
}

menu.addEventListener("click", (e) => {
  const b = (e.target as HTMLElement).closest<HTMLElement>("[data-m]");
  if (b) menuAction(b.dataset.m!);
});
menu.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    e.stopPropagation();
    closeMenu();
    inp.focus();
  }
});

// ---------- 鼠标、键盘 ----------

body.addEventListener("mousemove", (e) => {
  const el = (e.target as HTMLElement).closest<HTMLElement>(".r");
  if (!el || Number(el.dataset.i) === sel) return;
  sel = Number(el.dataset.i);
  arrowPicked = false;
  paint();
});
body.addEventListener("click", (e) => {
  const t = e.target as HTMLElement;
  const p = t.closest<HTMLElement>("[data-r]");
  if (p) return void launch(recents[Number(p.dataset.r)]);
  if (t.closest("#next")) {
    const nx = nextRem();
    if (nx) api.openMainNav("reminders", undefined, nx.r.id).then(() => close(), () => close());
    return;
  }
  const el = t.closest<HTMLElement>(".r");
  if (el) run(Number(el.dataset.i));
});
body.addEventListener("contextmenu", (e) => {
  e.preventDefault();
  const t = e.target as HTMLElement;
  const p = t.closest<HTMLElement>("[data-r]");
  const el = t.closest<HTMLElement>(".r");
  const row = el ? rows[Number(el.dataset.i)] : null;
  const it = p ? recents[Number(p.dataset.r)] : row?.kind === "app" ? row.h.it : null;
  if (it) openMenu(it, e.clientX, e.clientY);
});
// 输入框里留着系统的右键菜单（粘贴），别处不要
document.addEventListener("contextmenu", (e) => {
  if (!(e.target as HTMLElement).closest("input")) e.preventDefault();
});
// 点框里别处时焦点留在输入框里，接着打字
card.addEventListener("mousedown", (e) => {
  if (!(e.target as HTMLElement).closest("input")) e.preventDefault();
});
document.addEventListener("mousedown", (e) => {
  if (!menu.hidden && !(e.target as HTMLElement).closest("#menu")) closeMenu();
});

// 右边的齿轮：到设置的「快捷框」那一页
right.addEventListener("click", (e) => {
  if (!(e.target as HTMLElement).closest("#qset")) return;
  const done = () => close(true);
  api.openMainNav("settings", undefined, undefined, "quick").then(done, done);
});

savedEl.addEventListener("click", (e) => {
  if ((e.target as HTMLElement).closest("#undo")) undo();
});
savedEl.addEventListener("mouseenter", () => clearTimeout(savedTimer));
savedEl.addEventListener("mouseleave", () => {
  if (!savedEl.hidden) armSaved(SAVED_MS);
});

inp.addEventListener("input", () => {
  sel = 0;
  arrowPicked = false;
  if (!busy) say("");
  render();
});

/** 空着时方向键在常用里选：左右挪一个，两排时上下换一排，从第一排再往上（或第一个再往左）回到输入框；回车、空格打开 */
function emptyKey(e: KeyboardEvent) {
  const n = recents.length;
  if (/^Arrow(Right|Down|Left|Up)$/.test(e.key) && n) {
    e.preventDefault();
    if (pinSel < 0) pinSel = e.key === "ArrowRight" || e.key === "ArrowDown" ? 0 : -1;
    else if (e.key === "ArrowRight") pinSel = Math.min(pinSel + 1, n - 1);
    else if (e.key === "ArrowLeft") pinSel -= 1;
    else if (e.key === "ArrowDown") pinSel = pinSel < FIRST_ROW && n > FIRST_ROW ? Math.min(pinSel + FIRST_ROW, n - 1) : pinSel;
    else pinSel = pinSel >= FIRST_ROW ? pinSel - FIRST_ROW : -1;
    paintPins();
  } else if (e.key === "Enter" || (e.key === " " && pinSel >= 0)) {
    // 空格只在选中了图标时算打开，不然就是打了个空格
    e.preventDefault();
    if (pinSel >= 0 && recents[pinSel]) launch(recents[pinSel]);
    else close();
  }
}

inp.addEventListener("keydown", (e) => {
  // 输入法还在选字时，回车、方向键是给输入法的
  if (e.isComposing || e.keyCode === 229) return;
  if (e.key === "Alt") return e.preventDefault();
  // 空格启动：用方向键选中的程序、或者「像程序名」排第一（还没打空格）时，空格打开它；别的时候照常打空格
  if (e.key === " " && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey && spaceNow(e.isComposing)) {
    e.preventDefault();
    return void run(sel, "open");
  }
  // Alt + 数字：空着时打开常用里的第几个，打了字时执行第几行
  if (e.altKey && /^Digit[1-9]$/.test(e.code)) {
    e.preventDefault();
    const n = Number(e.code.slice(5)) - 1;
    if (!inp.value.trim()) {
      if (recents[n]) launch(recents[n]);
    } else run(n);
    return;
  }
  if (!inp.value.trim()) return emptyKey(e);
  if ((e.key === "ArrowDown" || e.key === "ArrowUp") && rows.length) {
    e.preventDefault();
    sel = (sel + (e.key === "ArrowDown" ? 1 : rows.length - 1)) % rows.length;
    arrowPicked = true;
    paint();
  } else if (e.key === "Tab") {
    // Tab：跳到「只记成笔记」，再按一次回到「记下来」
    e.preventDefault();
    const only = rows.findIndex((r) => r.kind === "only");
    const rec = rows.findIndex((r) => r.kind === "rec");
    const to = only >= 0 && sel !== only ? only : rec;
    if (to >= 0) {
      sel = to;
      arrowPicked = false;
      paint();
    }
  } else if (e.key === "Enter") {
    e.preventDefault();
    run(sel, e.ctrlKey ? "admin" : e.shiftKey ? "dir" : "open");
  }
});

document.addEventListener("keydown", (e) => {
  if (saved && (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z") {
    e.preventDefault();
    undo();
  } else if (e.key === "Escape") {
    if (!menu.hidden) closeMenu();
    // 带进来的字（选中的、上次没写完的）：第一次 Esc 先清空（可能是不小心选中的，其实想打开程序），再按才关掉
    else if (picked && inp.value) {
      picked = false;
      inp.value = "";
      sel = 0;
      render();
    } else hide();
  }
});

// 窗口跟着内容高度走（菜单打开时要放得下菜单），透明的空白处不挡住后面的程序
let lastH = 0;
function fit() {
  requestAnimationFrame(() => {
    const h = Math.ceil(Math.max(stage.offsetHeight, menu.hidden ? 0 : menu.offsetTop + menu.offsetHeight + 16));
    if (h !== lastH) {
      lastH = h;
      api.fitWindow(h);
    }
  });
}
new ResizeObserver(fit).observe(stage);

// ---------- 打开、关掉 ----------

/** 多行合成一行，太长的截掉 */
const oneLine = (s: string) => s.replace(/\s*\n\s*/g, " ").slice(0, 500);

/** 清空输入框和结果 */
function clear() {
  gen++;
  busy = false;
  saved = null;
  clearTimeout(savedTimer);
  savedEl.hidden = true;
  card.hidden = false;
  inp.disabled = false;
  inp.value = "";
  picked = false;
  say("");
  closeMenu();
  rows = [];
  sel = 0;
  arrowPicked = false;
  appFirst = false;
  pinSel = -1;
  notes = null;
  loadingNotes = null;
}

function applySettings(s: Settings | null) {
  useAI = !!s?.ai_key;
  setTags(s?.tags);
  setNoteCats(s?.note_cats);
  if (s?.launcher) cfg = s.launcher;
  pinRows = s?.pin_rows ?? 1;
  calcOn = s?.calc ?? true;
  spaceOn = s?.flash_space ?? false;
  webKeys = s?.web_keys ?? [];
}

async function reset() {
  const kept = draft;
  draft = "";
  clear();
  stage.classList.remove("gone");
  replay(card);
  const [s, devs, pickedText, st, rs] = await Promise.all([
    api.getSettings().catch(() => null),
    api.listDevices().catch(() => []),
    api.takeFlashText().catch(() => null),
    api.launcherStats().catch(() => EMPTY_STATS),
    api.listReminders().catch(() => []),
  ]);
  applySettings(s);
  devices = devs;
  stats = st;
  rems = rs;
  // 上次没写完的放回去；没有的话，按快捷键前在别的程序里选中的文字带进来（多行合成一行）。主窗口点「搜索」带来的 ? 优先
  const text = pickedText?.startsWith("?") ? pickedText : kept || pickedText;
  if (text && !inp.value) {
    inp.value = oneLine(text);
    picked = true;
  }
  render();
  inp.focus();
  inp.setSelectionRange(inp.value.length, inp.value.length);
}

/** 先清空再藏起来：下次显示时不会先闪一下上次的内容；消失不留动画。keep：没写完的字留着下次放回去 */
function close(keep = false) {
  draft = keep && !saved && inp.value.trim() ? inp.value : "";
  dragUntil = 0;
  stage.classList.add("gone");
  clear();
  renderEmpty();
  requestAnimationFrame(() => win.hide());
}

const hide = (keep = false) => {
  if (!busy) close(keep);
};

win.onFocusChanged(({ payload: focused }) => {
  // 拖动时焦点会交给窗口边框，不算点了别处；点了别处（或者再按一次快捷键）收起时，没写完的字留着
  if (!focused && !dragging) hide(true);
});

// 按住空白处（左边的猫、底下那行字、边上）能把快捷框拖到别处；拖完记下位置（win.rs flash_moved）
let dragging = false;
let moved = false;
let movedTimer = 0;
let clickTimer = 0;
/** 按下去以后这么久之内挪动都算拖的（中途停一下再接着拖也记得住）；显示快捷框时 Rust 摆位置不算 */
let dragUntil = 0;
/** 拖完了（或者只是点了一下没拖）：拖过的记下位置；焦点拿回来，之后点别处照样收起 */
function dragDone() {
  clearTimeout(movedTimer);
  clearTimeout(clickTimer);
  if (!dragging) return;
  dragging = false;
  if (moved) void api.flashMoved();
  void win.setFocus().then(() => inp.focus());
}
stage.addEventListener("mousedown", (e) => {
  if (e.button !== 0 || (e.target as HTMLElement).closest("input, textarea, button, a, .r, .pin, .menu, .saved")) return;
  e.preventDefault();
  dragging = true;
  moved = false;
  dragUntil = Date.now() + 10000;
  void win.startDragging();
  // 一直没挪：当成只是点了一下
  clickTimer = window.setTimeout(() => {
    if (!moved) dragDone();
  }, 800);
});
win.onMoved(() => {
  if (Date.now() > dragUntil) return;
  moved = true;
  clearTimeout(clickTimer);
  clearTimeout(movedTimer);
  movedTimer = window.setTimeout(() => {
    if (dragging) dragDone();
    else void api.flashMoved();
  }, 400);
});

$("#who").innerHTML = `<span class="cat">${head("normal", 26)}</span><span class="nocat">${icon("bell")}</span>`;
inp.placeholder = T.placeholder;
inp.setAttribute("aria-label", T.inputLabel);

listen("flash-open", () => reset());
// 按快捷键前在别的程序里选中的文字，复制得慢的随后才到（见 lib.rs open_flash）；已经开始打字了就不管
listen<string>("flash-text", ({ payload }) => {
  if (inp.value || busy || saved) return;
  inp.value = oneLine(payload);
  picked = true;
  render();
  inp.setSelectionRange(inp.value.length, inp.value.length);
});
listen("launcher-changed", async () => {
  items = await api.launcherList();
  render();
});
listen<Settings>("settings-changed", (e) => {
  applySettings(e.payload);
  render();
});

// 窗口一般是启动时就藏着建好的（见 win.rs warm_flash），按快捷键才显示；
// 还没建好时按了快捷键，是现建的（#open），准备好了自己显示
(async () => {
  items = await api.launcherList().catch(() => []);
  await reset();
  if (location.hash !== "#open") return;
  await win.show();
  await win.setFocus();
  inp.focus();
})();
