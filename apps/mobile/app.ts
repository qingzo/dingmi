// 叮咪手机版（UI设计 U11，设计稿 design/ui/u11-mobile.html）：今天 / 提醒 / 笔记本 / 我的。
// 网址是 /m/#t=<手机令牌>.<收件箱公钥>&k=<空间密钥>（以前是 /m/<手机令牌>.<收件箱公钥>，照样认，见 pair.ts），和原来的收件箱网页是同一个。
// 没输同步密码时只能发给电脑（收件箱）；输过之后能看、能记，数据在手机上解密，见同步方案第 14 节。
import "./app.css";
import "./look";
import { icon, type IconName } from "@shared/icons";
import { cat, head, type CatMood } from "@shared/mascot";
import { fileSize, highlight, renderMd, setFileInfo, setTodo, snippet, todos } from "@shared/md";
import { captureDrafts, tableClick, tableInput, tableKey, tableStates, type TableCtx } from "@shared/tables";
import { STYLES, TEXT, type StyleId } from "@shared/text";
import { ago, copyText, esc, noteCatChips, noteCatDef, noteCatList, repeatLabel, setNoteCats, setTags, tagChips, tagColor, tagHtml, weekdayCn } from "@shared/util";
import { readRepeatExtras, REPEAT_KINDS, repeatExtrasHtml, showRepeatExtras } from "@shared/repeat-edit";
import { parseLocal } from "../desktop/ui/lib/parse";
import { fileAad, fromB64, open, seal, sealInbox, unlockBox1 } from "./crypto";
import { hitToReminder, hitWhen, reminderHits, type ParsedReminder } from "./hit";
import {
  blankReminder, cleanFileName, complete, dayStr, DEFAULT_NOTE_CATS, DEFAULT_TAGS, extFor, fireAt, fmtMin, isRecurring, MAX_ATTACHMENT, normalize, overLimit, noteTitle, nowSec, parseDT, toggleTodoLine,
  undoComplete, uuidv7, type Limits, type Payload, type Repeat, type SAttachment, type SNote, type SShared, type SReminder, type TagDef,
} from "./model";
import { getStyle, isDark, mascotOn, onLookChange, setStyle } from "./look";
import { Engine, ls, wipe } from "./sync";
import { browserPlace, forgetMaster, loadMaster, saveMaster } from "./keystore";
import { cleanUrl, parseHash, readPairing } from "./pair";
import { cleanCode, phoneWords, sealKey, withSpaceKey } from "./join";
import { T } from "./text";

type View = "today" | "reminders" | "notes" | "me";
type RepeatArg = Parameters<typeof repeatLabel>[0];

const $ = <T extends HTMLElement = HTMLElement>(s: string, r: ParentNode = document) => r.querySelector(s) as T;
const $$ = <T extends HTMLElement = HTMLElement>(s: string, r: ParentNode = document) => [...r.querySelectorAll(s)] as T[];
const app = $("#app");

// ---------- 网址里的配对信息 ----------

/** 网址 # 后面的参数（浏览器不会发给服务器）：t 是 <手机令牌>.<收件箱公钥>，k 是空间密钥（电脑添加手机时带上的），
 * text 是快捷指令带来的内容 */
const hashArgs = parseHash(location.hash);
// 配对信息：# 里的 t=，或者以前的网址路径里的；都没有（安卓从主屏幕的 /m/ 打开）用上次存下的
const pairing = readPairing(location.pathname, hashArgs, ls.get("pair"));
const seg = pairing.seg;
const dot = seg.lastIndexOf(".");
const token = dot > 0 ? seg.slice(0, dot) : "";
let inboxPk: Uint8Array | null = null;
try {
  if (token) inboxPk = fromB64(seg.slice(dot + 1));
} catch {
  /* 网址不完整，下面会提示 */
}
if (token) ls.set("pair", seg);
/** 记住的主密钥放在哪（加密存放，见 keystore.ts） */
const keyPlace = browserPlace(ls);
let spaceKey: Uint8Array | null = null;
try {
  const k = hashArgs.get("k") || ls.get("space-key");
  if (k && fromB64(k).length === 32) {
    spaceKey = fromB64(k);
    ls.set("space-key", k);
  }
} catch {
  /* 解锁时再提示 */
}
const standalone = (navigator as { standalone?: boolean }).standalone || matchMedia("(display-mode: standalone)").matches;
const ua = navigator.userAgent;
const device: keyof typeof T.devices = /iPhone|iPod/.test(ua)
  ? "iphone"
  : /iPad/.test(ua) || (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1)
    ? "ipad"
    : /Android/.test(ua)
      ? "android"
      : "other";

const savedView = ls.get("view");
const S = {
  view: (savedView === "more" ? "me" : (savedView as View)) || "today",
  q: "",
  /** 提醒按什么看：todo 还没做的 / cycle 周期 / done 做完的 / 分类 id */
  rFilter: "todo",
  /** 笔记按哪个分类看："all" 全部，"none" 没分类的 */
  nCat: "all",
  engine: null as Engine | null,
  key: null as Uint8Array | null,
  /** 发给电脑时选的图片、文件 */
  files: [] as File[],
  /** 长按「发送」换成直接存成笔记 */
  noteMode: false,
  /** 「发到电脑了」显示到什么时候 */
  sentUntil: 0,
  /** 解锁页的提示 */
  lockMsg: "",
  /** 发送框下面的一行提示（重画时保留） */
  capMsg: { text: "", kind: "" },
  /** 输入框里认出的提醒（需求-0.4 第 31 节） */
  hits: [] as ParsedReminder[],
  /** 刚「记成提醒」的：显示「记好了」时用；null 时显示的是「发到电脑了」 */
  saved: null as { what: string; uids: string[] } | null,
  /** 刚记的那几条，在列表里浅橘底闪一下 */
  flash: new Set<string>(),
};
if (!["today", "reminders", "notes", "me"].includes(S.view)) S.view = "today";

// ---------- 时间的写法（U13）：今天 14:00、明天 08:30、这周 周三 15:00、更远 11 月 12 日，跨年才写年份 ----------

const dayDiff = (d: Date) => {
  const t = new Date();
  return Math.round((new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime() - new Date(t.getFullYear(), t.getMonth(), t.getDate()).getTime()) / 86400000);
};
const monthDay = (d: Date) => `${d.getFullYear() !== new Date().getFullYear() ? `${d.getFullYear()} 年 ` : ""}${d.getMonth() + 1} 月 ${d.getDate()} 日`;

function whenText(s: string | null | undefined): string {
  const d = parseDT(s);
  if (!d) return "";
  const hm = s!.slice(11, 16);
  const n = dayDiff(d);
  if (n === 0) return hm;
  if (n === 1) return `${T.tomorrow} ${hm}`;
  if (n === -1) return `昨天 ${hm}`;
  if (n > 1 && n < 7) return `${weekdayCn(d)} ${hm}`;
  return monthDay(d);
}

/** 改提醒时「什么时候」右边那一小段：11/6 14:00，周期的带上怎么重复 */
function whenPreview(due: string, rep: Repeat): string {
  const d = parseDT(due);
  if (!d) return "";
  return `${d.getMonth() + 1}/${d.getDate()} ${due.slice(11, 16)}${isRecurring(rep) ? ` · ${repeatLabel(rep as RepeatArg, due)}` : ""}`;
}

/** 用本地规则理解「什么时候」那一句（手机上不用 AI） */
function parseWhen(s: string): { due: string; repeat: Repeat } | null {
  const it = parseLocal(s).find((x) => x.type === "reminder");
  if (!it || it.type !== "reminder") return null;
  const r = it.repeat;
  return { due: it.due, repeat: { kind: r.kind, days: [...r.days], monthday: r.monthday, every: r.every, month: r.month ?? 0, nth: r.nth ?? 0 } };
}

// ---------- 小工具 ----------

let toastTimer = 0;
/** 底部的小提示；withCat：前面带团子猫的小脑袋（做完一件时） */
function toast(msg: string, action?: { label: string; run: () => void }, withCat = false) {
  document.querySelector(".toast")?.remove();
  const t = document.createElement("div");
  const c = withCat && mascotOn();
  t.className = `toast${c ? "" : " plain"}`;
  t.setAttribute("role", "status");
  t.innerHTML = `${c ? `<span class="cat">${head("normal", 26)}</span>` : ""}<span class="msg">${esc(msg)}</span>${action ? `<button class="undo">${esc(action.label)}</button>` : ""}`;
  if (action)
    t.querySelector("button")!.onclick = () => {
      t.remove();
      action.run();
    };
  document.body.append(t);
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => t.remove(), action ? 4500 : 2600);
}

/** 两下确认：第一下变红写着"确认删除？"，3 秒内再点才算 */
function sure(b: HTMLElement, label: string = T.sure): boolean {
  if (b.classList.contains("sure")) return true;
  const old = b.innerHTML;
  b.classList.add("sure");
  b.textContent = label;
  setTimeout(() => {
    b.classList.remove("sure");
    b.innerHTML = old;
  }, 3000);
  return false;
}

/** 空白页：团子猫 + 一句话；关掉团子猫时只留字 */
const empty = (mood: CatMood, title: string, sub = "") =>
  `<div class="empty"><span class="cat mascot">${cat(mood)}</span><b>${esc(title)}</b>${sub ? `<span>${esc(sub)}</span>` : ""}</div>`;

const ibtn = (ic: IconName, label: string, attrs = "", cls = "") => `<button class="ibtn ${cls}" aria-label="${esc(label)}" ${attrs}>${icon(ic)}</button>`;

// ---------- 弹出的面板和整页（一层一层叠着；安卓的返回键关掉最上面那层） ----------

interface Layer {
  el: HTMLElement;
  onClose?: () => void;
}
const layers: Layer[] = [];

function pushLayer(el: HTMLElement, onClose?: () => void): HTMLElement {
  document.body.append(el);
  layers.push({ el, onClose });
  history.pushState({ layer: layers.length }, "");
  document.body.classList.add("locked");
  return el;
}

const popWaiters: (() => void)[] = [];
/** 关掉最上面那层（走浏览器的返回，这样和安卓的返回键是一回事）；关好了才 resolve */
function popLayer(): Promise<void> {
  if (!layers.length) return Promise.resolve();
  return new Promise((resolve) => {
    popWaiters.push(resolve);
    history.back();
  });
}

/** 关掉全部 */
function closeLayers() {
  if (layers.length) history.go(-layers.length);
}

window.addEventListener("popstate", () => {
  const depth = (history.state as { layer?: number } | null)?.layer ?? 0;
  while (layers.length > depth) {
    const l = layers.pop()!;
    l.el.remove();
    l.onClose?.();
  }
  if (!layers.length) document.body.classList.remove("locked");
  popWaiters.splice(0).forEach((f) => f());
});

/** 底部弹出的面板：点外面或者往下拖关掉 */
function sheet(html: string): HTMLElement {
  const w = document.createElement("div");
  w.className = "scrim sheet-wrap";
  w.innerHTML = `<div class="sheet" role="dialog"><span class="grab" aria-hidden="true"></span>${html}</div>`;
  w.addEventListener("click", (e) => {
    if (e.target === w) void popLayer();
  });
  const s = w.firstElementChild as HTMLElement;
  // 按住顶上那条往下拖
  let y0 = -1;
  s.addEventListener("touchstart", (e) => {
    const t = e.target as HTMLElement;
    y0 = t.closest(".grab, .sheet > h4") ? e.touches[0].clientY : -1;
  });
  s.addEventListener("touchmove", (e) => {
    if (y0 < 0) return;
    const dy = Math.max(0, e.touches[0].clientY - y0);
    s.style.transform = `translateY(${dy}px)`;
  });
  s.addEventListener("touchend", (e) => {
    if (y0 < 0) return;
    const dy = e.changedTouches[0].clientY - y0;
    y0 = -1;
    s.style.transform = "";
    if (dy > 90) void popLayer();
  });
  pushLayer(w);
  return s;
}

/** 盖满屏幕的一页（看笔记、编辑笔记） */
function screen(html: string, cls = "", onClose?: () => void): HTMLElement {
  const w = document.createElement("div");
  w.className = `screen ${cls}`;
  w.setAttribute("role", "dialog");
  w.innerHTML = html;
  return pushLayer(w, onClose);
}

// ---------- 数据 ----------

const E = () => S.engine!;
/** 某一类记录的内容 */
function of(k: "reminder"): SReminder[];
function of(k: "note"): SNote[];
function of(k: "attachment"): SAttachment[];
function of(k: "shared"): SShared[];
function of(k: "device"): { uid: string; name?: string }[];
function of(k: Payload["k"]): unknown[] {
  return [...(S.engine?.items.values() ?? [])].filter((x) => x.p.k === k).map((x) => x.p.d);
}
const reminders = () => of("reminder").filter((r) => !r.deleted_at && !r.purged);
// 置顶的排最前面（后置顶的在上），其余按修改时间
const notes = () =>
  of("note")
    .filter((n) => !n.deleted_at && !n.purged)
    .sort((a, b) => (b.pinned_at ?? "").localeCompare(a.pinned_at ?? "") || b.updated_at.localeCompare(a.updated_at));
const byFire = (a: SReminder, b: SReminder) => fireAt(a).localeCompare(fireAt(b));
const findR = (uid: string) => reminders().find((r) => r.uid === uid);
const findN = (uid: string) => notes().find((n) => n.uid === uid);
const saveR = (d: SReminder) => E().save({ k: "reminder", d: { ...d, updated_at: nowSec() } });
const saveN = (d: SNote) => E().save({ k: "note", d: { ...d, updated_at: nowSec() } });

/** 分类在共用设置里（电脑上「设置 → 分类」改的），没同步过来时用默认的 */
function sharedList(field: "tags" | "note_cats", fallback: TagDef[]): TagDef[] {
  try {
    const s = of("shared").find((x) => x.uid === "settings");
    const t = s ? JSON.parse(s.value)[field] : null;
    return Array.isArray(t) ? (t.length || field === "note_cats" ? t : fallback) : fallback;
  } catch {
    return fallback;
  }
}
const currentTags = () => sharedList("tags", DEFAULT_TAGS);
const currentNoteCats = () => sharedList("note_cats", DEFAULT_NOTE_CATS);

function todayData() {
  const tk = dayStr(new Date());
  const tm = dayStr(new Date(Date.now() + 86400000));
  const all = reminders();
  return {
    pending: all.filter((r) => !r.done && fireAt(r).slice(0, 10) <= tk).sort(byFire),
    doneToday: all.filter((r) => (r.done && r.completed_at?.startsWith(tk)) || (isRecurring(r.repeat) && r.last_done_at?.startsWith(tk))),
    tomorrow: all.filter((r) => !r.done && fireAt(r).slice(0, 10) === tm).sort(byFire),
  };
}

// ---------- 渲染 ----------

let renderLater = false;

function render() {
  if (!S.engine) return;
  setTags(currentTags());
  setNoteCats(currentNoteCats());
  // 正在输入时不整页重画（iPhone 上会收起键盘）：发送框等输完再画，搜索框只换下面的列表
  refreshDoc();
  const typing = document.activeElement?.id;
  if (typing === "cap") {
    renderLater = true;
    return;
  }
  if (typing === "q" && S.view === "notes") {
    $("#nl").innerHTML = noteList();
    return;
  }
  renderLater = false;
  const capText = ($("#cap") as HTMLTextAreaElement | null)?.value ?? "";
  const views: Record<View, () => string> = { today: viewToday, reminders: viewReminders, notes: viewNotes, me: viewMe };
  app.innerHTML = `${views[S.view]()}${tabs()}`;
  const cap = $("#cap") as HTMLTextAreaElement | null;
  if (cap) {
    cap.value = capText;
    renderPics();
  }
}

/** 打开着的笔记跟着刷新 */
function refreshDoc() {
  const doc = $("#doc");
  const n = doc && findN(doc.dataset.uid!);
  if (n) fillDoc(n);
}

const TABS: [View, IconName][] = [
  ["today", "sun"],
  ["reminders", "bell"],
  ["notes", "notebook-pen"],
  ["me", "user-round"],
];

function tabs() {
  const n = todayData().pending.length;
  return `<nav class="tabs" aria-label="页面">${TABS.map(
    ([v, ic]) =>
      `<button data-view="${v}" ${S.view === v ? 'aria-current="page"' : ""}><span class="ico">${icon(ic)}${v === "today" && n ? `<i class="bd">${n}</i>` : ""}</span>${T.tabs[v]}</button>`,
  ).join("")}</nav>`;
}

/** 页面顶上：大标题、小字、右边的按钮 */
const top = (title: string, sub = "", right = "") =>
  `<header class="ph"><h1>${esc(title)}${sub ? `<small>${esc(sub)}</small>` : ""}</h1>${right}</header>`;

function syncBtn() {
  const st = E().status;
  if (st === "offline" || st === "revoked") return ibtn("cloud-off", T.syncNow, 'data-act="sync"', "bad");
  return ibtn("refresh-cw", st === "syncing" ? T.syncing : T.syncNow, 'data-act="sync"', st === "syncing" ? "spinning" : "");
}

/** 出问题时顶上的红色横幅：手机作废了、连不上服务器。能用的部分照常用 */
function banners() {
  const st = S.engine?.status;
  if (st === "revoked") return banner(T.bRevoked, T.bRevokedText);
  if (st === "offline") return banner(T.bOffline, T.bOfflineText, `<button class="btn sm" data-act="sync">${T.bRetry}</button>`);
  return "";
}
const banner = (b: string, text: string, extra = "") => `<div class="banner err" role="alert">${icon("cloud-off")}<div><b>${esc(b)}</b>${esc(text)}</div>${extra}</div>`;

/** 一条提醒。short：分组已经写了哪天，时间只写钟点 */
function remRow(r: SReminder, o: { done?: boolean; short?: boolean } = {}) {
  const f = fireAt(r);
  const late = !o.done && (parseDT(f) ?? new Date()) < new Date();
  const rec = isRecurring(r.repeat);
  const time = o.done ? T.doneAt(whenText(r.completed_at ?? r.last_done_at ?? f)) : o.short ? f.slice(11, 16) : whenText(f);
  const sub = [r.detail, !o.done && r.snooze_count ? T.snoozed(r.snooze_count) : ""].filter(Boolean).join(" · ");
  const tags = [
    r.urgent ? `<span class="tag" style="--c:var(--danger)">${T.urgent}</span>` : "",
    r.vague && !o.done ? `<span class="tag" style="${tagColor("amber")}" title="${T.vagueTip}">${T.vague}</span>` : "",
    rec ? `<span class="tag" style="${tagColor("purple")}">${icon("repeat")}${esc(repeatLabel(r.repeat as RepeatArg, r.due_at))}</span>` : "",
    rec && r.tag === "cycle" ? "" : tagHtml(r.tag),
  ].join("");
  return `<div class="it${o.done ? " done" : ""}${S.flash.has(r.uid) ? " flash" : ""}" data-rem="${r.uid}">
    <button class="task${o.done ? " on" : ""}" data-chk="${r.uid}" data-done="${o.done ? 1 : 0}" aria-label="${o.done ? T.uncomplete : T.complete}">${icon("check")}</button>
    <span class="t"><span class="tt">${esc(r.title)}</span>${tags ? `<span class="tags">${tags}</span>` : ""}${sub ? `<small>${esc(sub)}</small>` : ""}</span>
    <span class="tm${late ? " late" : ""}">${esc(time)}</span>
  </div>`;
}

const group = (title: string, rows: string) => (rows ? `<div class="grp">${esc(title)}</div><div class="card">${rows}</div>` : "");

function viewToday() {
  const now = new Date();
  const { pending, doneToday, tomorrow } = todayData();
  const list = pending.map((r) => remRow(r, { short: dayDiff(parseDT(fireAt(r)) ?? now) === 0 })).join("") + doneToday.map((r) => remRow(r, { done: true })).join("");
  const date = `${now.getMonth() + 1} 月 ${now.getDate()} 日 ${weekdayCn(now)}`;
  return `${top(T.today, T.todaySub(date, pending.length), syncBtn())}
    <main class="pb">${banners()}${sendBox()}${queueBar()}
    ${list ? group(T.grpToday, list) : `<div class="grp">${T.grpToday}</div>${empty("sleep", TEXT.todayEmpty(mascotOn()))}`}
    ${group(T.grpTomorrow, tomorrow.map((r) => remRow(r, { short: true })).join(""))}</main>`;
}

function viewReminders() {
  const active = reminders().filter((r) => !r.done);
  const doneList = reminders().filter((r) => r.done);
  const match = (r: SReminder, t: string) => r.tag === t || (t === "cycle" && isRecurring(r.repeat));
  if (!["todo", "cycle", "done"].includes(S.rFilter) && !currentTags().some((t) => t.id === S.rFilter)) S.rFilter = "todo";
  const chips: [string, string, string][] = [
    ["todo", T.rfTodo, ""],
    ["cycle", T.rfCycle, ""],
    ["done", T.rfDone, ""],
    ...tagsInUse(active).map((t) => [t.id, t.name, tagColor(t.color)] as [string, string, string]),
  ];
  let body = "";
  if (S.rFilter === "done") {
    const list = doneList.sort((a, b) => (b.completed_at ?? "").localeCompare(a.completed_at ?? "")).slice(0, 200);
    body = list.length ? `<div class="card">${list.map((r) => remRow(r, { done: true })).join("")}</div>` : empty("normal", T.doneEmpty);
  } else if (S.rFilter === "cycle") {
    const list = active.filter((r) => isRecurring(r.repeat)).sort(byFire);
    body = group(T.grpCycle, list.map((r) => remRow(r)).join(""));
  } else {
    const list = (S.rFilter === "todo" ? active : active.filter((r) => match(r, S.rFilter))).sort(byFire);
    // 过点了 / 今天 / 明天 / 这周（到周日）/ 以后
    const now = new Date();
    const toSunday = 7 - (now.getDay() || 7);
    const buckets: [string, SReminder[], boolean][] = [
      [T.grpLate, [], false],
      [T.grpToday, [], true],
      [T.grpTomorrow, [], true],
      [T.grpWeek, [], false],
      [T.grpLater, [], false],
    ];
    for (const r of list) {
      const d = parseDT(fireAt(r)) ?? now;
      const n = dayDiff(d);
      buckets[d < now ? 0 : n === 0 ? 1 : n === 1 ? 2 : n <= toSunday ? 3 : 4][1].push(r);
    }
    body = buckets.map(([t, rs, short]) => group(t, rs.map((r) => remRow(r, { short })).join(""))).join("");
  }
  if (!body) body = empty("normal", T.remEmpty, T.remEmptySub);
  return `${top(T.reminders, "", ibtn("plus", T.newReminder, 'data-act="new-rem"'))}
    <main class="pb">${banners()}
    <div class="chips">${chips.map(([k, l, c]) => `<button class="chip${c ? " tagged" : ""}" ${c ? `style="${c}"` : ""} data-rf="${esc(k)}" aria-pressed="${S.rFilter === k}">${esc(l)}</button>`).join("")}</div>
    ${body}</main>`;
}

/** 分类筛选只显示有提醒在用的分类（「周期」前面已经有了） */
function tagsInUse(list: SReminder[]): TagDef[] {
  const used = new Set(list.map((r) => r.tag));
  return currentTags().filter((t) => t.id !== "cycle" && used.has(t.id));
}

function viewNotes() {
  return `${top(T.notes, "", ibtn("plus", T.newNote, 'data-act="new-note"'))}
    <main class="pb">${banners()}
    <label class="srch">${icon("search")}<input id="q" type="search" value="${esc(S.q)}" placeholder="${T.searchPh}" aria-label="${T.searchPh}" autocomplete="off" enterkeyhint="search"><button type="button" class="x" data-act="clear-q" aria-label="${T.clear}" ${S.q ? "" : "hidden"}>${icon("x")}</button></label>
    ${noteCatRow()}
    <div id="nl">${noteList()}</div></main>`;
}

/** 笔记分类：一排，点一下筛，再点一下回到全部 */
function noteCatRow() {
  const all = notes();
  if (S.nCat !== "all" && S.nCat !== "none" && !noteCatDef(S.nCat)) S.nCat = "all";
  const none = all.filter((n) => !noteCatDef(n.tag)).length;
  const items: [string, string, string][] = [
    ["all", T.all, ""],
    ...noteCatList().map((t) => [t.id, t.name, tagColor(t.color)] as [string, string, string]),
    ...(none && none < all.length ? [["none", T.noCat, ""] as [string, string, string]] : []),
  ];
  if (items.length < 2) return "";
  return `<div class="chips">${items
    .map(([id, name, color]) => `<button class="chip${color ? " tagged" : ""}" data-nc="${esc(id)}" ${color ? `style="${color}"` : ""} aria-pressed="${S.nCat === id}">${esc(name)}</button>`)
    .join("")}</div>`;
}

function noteList() {
  const q = S.q.trim();
  const ql = q.toLowerCase();
  const all = notes().filter((n) => S.nCat === "all" || (S.nCat === "none" ? !noteCatDef(n.tag) : n.tag === S.nCat));
  const list = q ? all.filter((n) => n.title.toLowerCase().includes(ql) || n.body.toLowerCase().includes(ql)) : all;
  const rows = list
    .slice(0, 300)
    .map((n) => {
      const inBody = !!q && !n.title.toLowerCase().includes(ql);
      const c = noteCatDef(n.tag);
      return `<button class="nbi" data-note="${n.uid}" ${c ? `style="${tagColor(c.color)}"` : ""}><b>${c ? '<span class="sq"></span>' : ""}<span>${highlight(esc(n.title), q)}</span>${n.pinned_at ? icon("pin", "pin") : ""}</b>
        <small>${inBody ? T.inBody : ""}${snippet(n.body, q, 60) || T.blank}</small><span class="meta">${esc(ago(n.updated_at))}</span></button>`;
    })
    .join("");
  if (rows) return `${q ? `<div class="grp">${T.found(list.length)}</div>` : ""}<div class="card">${rows}</div>`;
  if (q) return empty("normal", TEXT.notFound(mascotOn()));
  if (S.nCat !== "all" && notes().length) return empty("normal", T.catEmpty, T.catEmptySub);
  return empty("normal", T.notesEmpty, T.notesEmptySub);
}

function viewMe() {
  const e = E();
  const st = e.status;
  const syncSub = [st === "ok" && e.lastSync ? T.lastSync(ago(e.lastSync)) : "", e.pending.size ? T.pending(e.pending.size) : "", T.counts(reminders().length, notes().length)].filter(Boolean).join(" · ");
  const styleName = STYLES.find(([id]) => id === getStyle())![1];
  return `${top(T.me)}
    <main class="pb">${banners()}
    <div class="card">
      ${row(T.phone, T.phoneSub(T.devices[device], true))}
      ${row(T.sync, syncSub, T.status[st], 'data-act="sync"', st === "offline" || st === "revoked" ? "bad" : "")}
      ${row(T.way, T.waySub(location.host))}
      ${row(T.addPc, T.addPcSub, "", 'data-act="add-pc"')}
      ${homeRows()}
    </div>
    <div class="card">
      ${row(T.style, T.styleSub, styleName, 'data-act="style"')}
      ${row(T.mode, "", T.modeVal)}
      ${row(T.lock, T.lockSub, "", 'data-act="lock"', "danger")}
    </div>
    <p class="fine">${T.meFoot}</p></main>`;
}

/** 「我的」里的一行；attrs 给了就能点 */
function row(name: string, sub: string, value = "", attrs = "", cls = "") {
  const tag = attrs ? "button" : "div";
  return `<${tag} class="prow ${cls}" ${attrs}><span class="t"><span class="nm">${esc(name)}</span>${sub ? `<small>${esc(sub)}</small>` : ""}</span>${value ? `<span class="v">${esc(value)}</span>` : ""}${attrs ? icon("chevron-right", "chev") : ""}</${tag}>`;
}

/** 加到主屏幕那一行 */
function homeRows() {
  return row(T.home, standalone ? T.homeDone : T.homeSub, standalone ? "" : T.howto, standalone ? "" : 'data-act="home"');
}

// ---------- 「我的」里弹出的几个面板 ----------

/** 风格预览卡片的颜色：底、卡片、主色、字（浅色 / 深色），和 tokens.css 一致 */
const SWATCH: Record<StyleId, [string, string, string, string][]> = {
  warm: [["#f6f1ec", "#fffcf8", "#f07326", "#2a2421"], ["#17130f", "#231e1a", "#ff8a45", "#f4eee9"]],
  simple: [["#f4f4f5", "#ffffff", "#1d1d1f", "#1d1d1f"], ["#111113", "#1b1b1e", "#ededef", "#ededef"]],
  calm: [["#edf1ee", "#f9fbf9", "#3f7d66", "#24302a"], ["#121614", "#1a201d", "#7fbfa2", "#e6ece8"]],
};

function openStyle() {
  const cards = () =>
    STYLES.map(([id, name, sub]) => {
      const [bg, surf, acc, ink] = SWATCH[id][isDark() ? 1 : 0];
      return `<button class="look" data-pick="${id}" aria-pressed="${getStyle() === id}">
        <span class="lpv" style="background:${bg}"><span style="background:${surf}"><i style="background:${acc}"></i><b style="background:${ink}"></b><b style="background:${ink}"></b></span></span>
        <span class="nm">${name}</span><small>${sub}</small></button>`;
    }).join("");
  const s = sheet(`<h4>${T.style}</h4><p class="lead">${T.styleLead}</p><div class="looks">${cards()}</div>`);
  s.addEventListener("click", (e) => {
    const b = (e.target as HTMLElement).closest<HTMLElement>("[data-pick]");
    if (!b) return;
    setStyle(b.dataset.pick as StyleId);
    $(".looks", s).innerHTML = cards();
  });
}

const steps = (list: readonly string[]) => `<ol class="steps">${list.map((x) => `<li>${x}</li>`).join("")}</ol>`;

/** 加一台电脑（需求-1.x 第 34.3 节）：叮咪同步输电脑上的 8 位数字、核对 4 个字再允许；自建服务器生成配对码 */
async function openAddPc() {
  const auth = { Authorization: "Bearer " + token };
  const official = await fetch("/m/api/space", { headers: auth })
    .then((r) => (r.ok ? r.json() : {}))
    .then((v: { official?: boolean }) => v.official !== false)
    .catch(() => true);
  if (!spaceKey) return void sheet(`<h4>${T.addPc}</h4><p class="lead">${T.addPcNoKey}</p>`);
  const key = spaceKey;
  if (!official) {
    const s = sheet(`<h4>${T.addPc}</h4><p class="lead">${T.addPcSelf}</p><div id="ap-out"></div><p class="cap-msg" id="ap-msg" role="status"></p><button class="btn pri full" id="ap-make">${T.addPcMake}</button>`);
    $("#ap-make", s).onclick = async () => {
      const msg = $("#ap-msg", s);
      try {
        const r = await fetch("/m/api/invite", { headers: auth });
        if (!r.ok) throw new Error(await r.text());
        const code = withSpaceKey(((await r.json()) as { code: string }).code, key);
        $("#ap-out", s).innerHTML = `<code class="pair-code">${esc(code)}</code>`;
        $("#ap-make", s).outerHTML = `<button class="btn pri full" id="ap-copy">${T.addPcCopy}</button>`;
        $("#ap-copy", s).onclick = () => void navigator.clipboard.writeText(code).then(() => (msg.textContent = T.addPcCopied));
      } catch (e) {
        msg.className = "cap-msg bad";
        msg.textContent = e instanceof TypeError ? T.netBad : e instanceof Error ? e.message : String(e);
      }
    };
    return;
  }
  const s = sheet(`<h4>${T.addPc}</h4><div id="ap-body"><p class="lead">${T.addPcLead}</p>
    <input class="field code-in" id="ap-code" inputmode="numeric" autocomplete="one-time-code" maxlength="9" placeholder="0000 0000" aria-label="${T.addPc}">
    <p class="cap-msg" id="ap-msg" role="status"></p><button class="btn pri full" id="ap-next" disabled>${T.addPcNext}</button></div>`);
  const inp = $<HTMLInputElement>("#ap-code", s);
  const say = (t: string, bad = true) => {
    const m = $("#ap-msg", s);
    m.className = `cap-msg${bad ? " bad" : ""}`;
    m.textContent = t;
  };
  inp.oninput = () => {
    const c = cleanCode(inp.value);
    $<HTMLButtonElement>("#ap-next", s).disabled = c.length !== 8;
    say("", false);
  };
  setTimeout(() => inp.focus(), 60);
  $("#ap-next", s).onclick = async () => {
    const code = cleanCode(inp.value);
    try {
      const r = await fetch(`/m/api/join/${code}`, { headers: auth });
      if (!r.ok) throw new Error(await r.text());
      const pc = (await r.json()) as { name: string; key: string };
      const pcKey = fromB64(pc.key);
      const words = await phoneWords(pcKey);
      $("#ap-body", s).innerHTML = `<p class="lead"><b>${esc(T.addPcAsk(pc.name))}</b></p><p class="lead">${T.addPcLook}</p>
        <div class="words">${[...words].map((w) => `<span>${esc(w)}</span>`).join("")}</div>
        <p class="cap-msg" id="ap-msg" role="status"></p>
        <button class="btn pri full" id="ap-yes">${T.addPcYes}</button><button class="btn full" id="ap-no">${T.addPcNo}</button>`;
      const answer = async (sealed: string | null) => {
        const r2 = await fetch(`/m/api/join/${code}`, { method: "POST", headers: { ...auth, "Content-Type": "application/json" }, body: JSON.stringify(sealed ? { sealed } : {}) });
        if (!r2.ok) throw new Error(await r2.text());
      };
      $("#ap-yes", s).onclick = () =>
        answer(sealKey(pcKey, key)).then(
          () => ($("#ap-body", s).innerHTML = `<p class="lead">${esc(T.addPcDone(pc.name))}</p>`),
          (e) => say(e instanceof Error ? e.message : String(e)),
        );
      $("#ap-no", s).onclick = () =>
        answer(null).then(
          () => ($("#ap-body", s).innerHTML = `<p class="lead">${T.addPcBlocked}</p>`),
          (e) => say(e instanceof Error ? e.message : String(e)),
        );
    } catch (e) {
      say(e instanceof TypeError ? T.netBad : e instanceof Error ? e.message : String(e));
    }
  };
}

function openHome() {
  sheet(`<h4>${T.homeTitle}</h4>${steps(device === "android" ? T.homeAndroid : T.homeIos)}<p class="lead">${T.homeNote}</p>`);
}

// ---------- 改提醒（底部弹出的面板） ----------

const WEEK = ["一", "二", "三", "四", "五", "六", "日"];

function nextHour() {
  const d = new Date();
  d.setHours(d.getHours() + 1, 0, 0, 0);
  return fmtMin(d);
}

const newReminder = (): SReminder => blankReminder(nextHour());

/** 「重复」那一栏：种类 + 每种要填的 */
function repeatBlock(rep: Repeat, d0: Date) {
  const kind = rep.kind || "none";
  const days = rep.days.length ? rep.days : [d0.getDay() || 7];
  return `<select class="field" id="ed-kind" aria-label="${T.repeat}">${REPEAT_KINDS.map(([k, l]) => `<option value="${k}" ${k === kind ? "selected" : ""}>${l}</option>`).join("")}</select>
    ${repeatExtrasHtml(rep as RepeatArg, kind, d0, "inline")}
    <div class="wd" id="ed-week" ${kind === "weekly" ? "" : "hidden"}>${WEEK.map((w, i) => `<button type="button" class="chip" data-wd="${i + 1}" aria-pressed="${days.includes(i + 1)}">${w}</button>`).join("")}</div>
    <div class="inline" id="ed-month" ${kind === "monthly" ? "" : "hidden"}>${T.everyMonth} <input class="field" type="number" inputmode="numeric" min="1" max="31" id="ed-md" value="${rep.monthday || d0.getDate()}"> ${T.day}</div>
    <div class="inline" id="ed-int" ${kind === "interval" ? "" : "hidden"}>${T.every} <input class="field" type="number" inputmode="numeric" min="1" max="365" id="ed-every" value="${rep.every || 2}"> ${T.days}</div>`;
}

/** 读出面板里的「重复」；每周一天都没选时返回 null */
function readRepeat(s: HTMLElement): Repeat | null {
  const k = $<HTMLSelectElement>("#ed-kind", s).value;
  const picked = $$("#ed-week [data-wd]", s).filter((c) => c.getAttribute("aria-pressed") === "true").map((c) => Number(c.dataset.wd));
  if (k === "weekly" && !picked.length) return null;
  return {
    kind: k as Repeat["kind"],
    days: k === "weekly" ? picked : [],
    monthday: k === "monthly" ? Math.min(31, Math.max(1, Number($<HTMLInputElement>("#ed-md", s).value) || 1)) : 0,
    every: k === "interval" ? Math.max(1, Number($<HTMLInputElement>("#ed-every", s).value) || 1) : 0,
    month: 0,
    nth: 0,
    ...readRepeatExtras(s, k),
  };
}

function openReminder(r: SReminder, isNew = false) {
  const due = r.due_at;
  const d0 = parseDT(due) ?? new Date();
  const s = sheet(`<h4>${isNew ? T.newReminder : T.editReminder}</h4>
    <form class="ed" id="ed" autocomplete="off">
      <label class="fld">${T.what}<input class="field" id="ed-title" value="${esc(r.title)}" placeholder="${T.whatPh}" enterkeyhint="next"></label>
      <label class="fld">${T.when}<span class="field when"><input id="ed-when" placeholder="${T.whenPh}" enterkeyhint="done"><span class="pv" id="ed-pv">${esc(whenPreview(due, r.repeat))}</span></span></label>
      <div class="f2">
        <label class="fld"><span id="ed-date-l">${(r.repeat.kind || "none") === "none" ? T.date : T.startDate}</span><input class="field" type="date" id="ed-date" value="${due.slice(0, 10)}"></label>
        <label class="fld">${T.time}<input class="field" type="time" id="ed-time" value="${due.slice(11, 16)}"></label>
      </div>
      <div class="fld"><span>${T.repeat}</span><div class="rep" id="ed-rep">${repeatBlock(r.repeat, d0)}</div></div>
      ${currentTags().length ? `<div class="fld"><span>${T.category}</span><div class="chips wrap" id="ed-tags">${tagChips(r.tag)}</div></div>` : ""}
      <div class="swrow"><div>${T.urgent}<small>${T.urgentSub}</small></div><button type="button" class="tog" role="switch" id="ed-urgent" aria-checked="${r.urgent}" aria-label="${T.urgent}"></button></div>
      <label class="fld">${T.detail}<textarea class="field" id="ed-detail" rows="2" placeholder="${T.detailPh}">${esc(r.detail)}</textarea></label>
      <p class="ferr" id="ed-err" role="alert"></p>
    </form>
    <div class="acts">${isNew ? "" : `<button type="button" class="btn danger" id="ed-del">${T.del}</button>`}${!isNew && r.done && !isRecurring(r.repeat) ? `<button type="button" class="btn" id="ed-undo">${T.undone}</button>` : ""}<span class="sp"></span><button class="btn pri" form="ed" type="submit">${T.save}</button></div>`);
  const pv = $("#ed-pv", s);
  const dateIn = $<HTMLInputElement>("#ed-date", s);
  const timeIn = $<HTMLInputElement>("#ed-time", s);
  /** 下面的日期、时间、重复改了：右边的小字跟着变 */
  const syncPreview = () => {
    const rep = readRepeat(s);
    pv.classList.remove("miss");
    pv.textContent = rep && dateIn.value && timeIn.value ? whenPreview(`${dateIn.value} ${timeIn.value}`, rep) : "";
  };
  const kindChanged = () => {
    const k = $<HTMLSelectElement>("#ed-kind", s).value;
    $("#ed-week", s).hidden = k !== "weekly";
    $("#ed-month", s).hidden = k !== "monthly";
    $("#ed-int", s).hidden = k !== "interval";
    showRepeatExtras(s, k);
    $("#ed-date-l", s).textContent = k === "none" ? T.date : T.startDate;
  };
  // 「什么时候」打一句话：认出来就填好下面的日期、时间、重复
  let whenTimer = 0;
  s.addEventListener("input", (e) => {
    const el = e.target as HTMLElement;
    if (el.id !== "ed-when") return;
    clearTimeout(whenTimer);
    whenTimer = window.setTimeout(() => {
      const v = (el as HTMLInputElement).value.trim();
      if (!v) return syncPreview();
      const got = parseWhen(v);
      if (!got) {
        pv.textContent = T.whenMiss;
        pv.classList.add("miss");
        return;
      }
      dateIn.value = got.due.slice(0, 10);
      timeIn.value = got.due.slice(11, 16);
      $("#ed-rep", s).innerHTML = repeatBlock(got.repeat, parseDT(got.due) ?? new Date());
      kindChanged();
      syncPreview();
    }, 250);
  });
  s.addEventListener("change", (e) => {
    const id = (e.target as HTMLElement).id;
    if (id === "ed-kind") kindChanged();
    if (id !== "ed-when") syncPreview();
  });
  s.addEventListener("click", async (e) => {
    const el = e.target as HTMLElement;
    const wd = el.closest<HTMLElement>("[data-wd]");
    if (wd) {
      wd.setAttribute("aria-pressed", String(wd.getAttribute("aria-pressed") !== "true"));
      syncPreview();
    }
    const tg = el.closest<HTMLElement>("[data-tag]");
    if (tg) {
      const on = tg.getAttribute("aria-pressed") !== "true";
      $$("#ed-tags .chip", s).forEach((c) => c.setAttribute("aria-pressed", "false"));
      tg.setAttribute("aria-pressed", String(on));
    }
    const sw = el.closest<HTMLElement>("#ed-urgent");
    if (sw) sw.setAttribute("aria-checked", String(sw.getAttribute("aria-checked") !== "true"));
    const del = el.closest<HTMLElement>("#ed-del");
    if (del && sure(del)) {
      await saveR({ ...r, deleted_at: nowSec() });
      popLayer();
      toast(T.deleted);
    }
    if (el.closest("#ed-undo")) {
      await setDone(r, false);
      popLayer();
    }
  });
  $<HTMLFormElement>("#ed", s).onsubmit = async (e) => {
    e.preventDefault();
    const err = $("#ed-err", s);
    const title = $<HTMLInputElement>("#ed-title", s).value.trim();
    if (!title) return void (err.textContent = T.noTitle);
    if (!dateIn.value || !timeIn.value) return void (err.textContent = T.noTime);
    const repeat = readRepeat(s);
    if (!repeat) return void (err.textContent = T.noDay);
    const now = new Date();
    const due = normalize(repeat, parseDT(`${dateIn.value} ${timeIn.value}`)!, now);
    // 和电脑端一样：改了时间就重新等着提醒；一次性的已经完成、时间也没挪到以后的，保持完成
    const keepDone = !isRecurring(repeat) && r.done && due <= now;
    const tagEl = $$("#ed-tags [data-tag]", s).find((c) => c.getAttribute("aria-pressed") === "true");
    await saveR({
      ...r,
      title,
      detail: $<HTMLTextAreaElement>("#ed-detail", s).value.trim(),
      due_at: fmtMin(due),
      repeat,
      snoozed_until: null,
      done: keepDone,
      completed_at: keepDone ? r.completed_at : null,
      tag: tagEl?.dataset.tag ?? null,
      urgent: $("#ed-urgent", s).getAttribute("aria-checked") === "true",
      // 改过时间就不算含糊了
      vague: !!r.vague && fmtMin(due) === r.due_at,
    });
    popLayer();
    toast(T.saved(whenFull(fmtMin(due))));
  };
  if (isNew) setTimeout(() => $<HTMLInputElement>("#ed-title", s).focus(), 300);
}

/** 保存提示里的时间：今天也写上「今天」 */
const whenFull = (s: string) => (dayDiff(parseDT(s) ?? new Date()) === 0 ? `${T.today} ${s.slice(11, 16)}` : whenText(s));

/** 完成 / 取消完成；从笔记待办设的提醒，笔记里那一行跟着勾上 */
async function setDone(r: SReminder, done: boolean) {
  const next = done ? complete(r) : undoComplete(r);
  await saveR(next);
  const n = r.note_uid && r.todo && !isRecurring(r.repeat) ? notes().find((x) => x.uid === r.note_uid) : undefined;
  const body = n && toggleTodoLine(n.body, r.todo!, done);
  if (n && body) await saveN({ ...n, body });
  return { next, note: body ? n : undefined };
}

async function completeWithUndo(r: SReminder) {
  const { next, note } = await setDone(r, true);
  const c = mascotOn();
  const msg = isRecurring(r.repeat) ? T.doneNext(c, whenFull(next.due_at)) : TEXT.done(c);
  toast(
    msg,
    {
      label: T.undo,
      run: async () => {
        await saveR(r);
        if (note) await saveN(note);
      },
    },
    true,
  );
}

// ---------- 看笔记 ----------

function openNote(uid: string) {
  const s = screen(`<header class="sh">${ibtn("chevron-left", T.back, "data-close")}<span class="sp"></span>${ibtn("pencil", T.edit, 'data-act="edit-note"')}</header>
    <div class="doc" id="doc" data-uid="${uid}"></div>`);
  const n = findN(uid);
  if (n) fillDoc(n);
  // 笔记里的表格：切换样子、搜索、分组、展开、改一行
  const tables: TableCtx = {
    uid,
    body: () => findN(uid)?.body ?? "",
    render: () => {
      const c = findN(uid);
      if (c) fillDoc(c);
    },
    save: async (body) => {
      const c = findN(uid);
      if (c) await saveN({ ...c, body });
    },
    toast,
  };
  s.addEventListener("input", (e) => tableInput(e.target as HTMLElement, tables));
  s.addEventListener("keydown", (e) => tableKey(e, tables));
  s.addEventListener("click", async (e) => {
    const el = e.target as HTMLElement;
    if (el.closest("[data-close]")) return void popLayer();
    const cur = findN(uid);
    if (!cur) return;
    if (el.closest('[data-act="edit-note"]')) return editNote(cur);
    const box = el.closest<HTMLElement>("[data-box]");
    if (box) return toggleTodo(cur, Number(box.dataset.box));
    const cp = el.closest<HTMLElement>("[data-copy]");
    if (cp) {
      await copyText(cp.dataset.copy!);
      return toast(T.copied);
    }
    if (await tableClick(el, tables)) return;
    const lnk = el.closest<HTMLElement>("a.lnk");
    if (lnk?.dataset.href) return void window.open(lnk.dataset.href, "_blank", "noopener");
    const fc = el.closest<HTMLElement>("[data-file]");
    if (fc) return void tapFile(fc);
    const img = el.closest<HTMLImageElement>(".att img");
    if (img?.src && !img.parentElement!.classList.contains("wait")) return viewImage(img.src);
    const rem = el.closest<HTMLElement>("[data-open-rem]");
    if (rem) {
      const r = reminders().find((x) => x.uid === rem.dataset.openRem);
      if (r) openReminder(structuredClone(r));
      return;
    }
    const del = el.closest<HTMLElement>("#nd-del");
    if (del && sure(del)) {
      await saveN({ ...cur, deleted_at: nowSec() });
      popLayer();
      toast(T.deleted);
    }
  });
}

function fillDoc(n: SNote) {
  const doc = $("#doc");
  captureDrafts(doc, n.uid);
  const q = S.view === "notes" ? S.q.trim() : "";
  const rem = n.reminder_uid ? findR(n.reminder_uid) : undefined;
  const c = noteCatDef(n.tag);
  doc.innerHTML = `<h2>${highlight(esc(n.title), q)}</h2>
    <div class="meta">${c ? `<span class="tag" style="${tagColor(c.color)}">${esc(c.name)}</span>` : ""}${esc(T.changed(ago(n.updated_at)))}</div>
    <div class="md">${n.body.trim() ? renderMd(n.body, q, undefined, tableStates(n.uid)) : `<p class="blank">${T.blankNote}</p>`}</div>
    ${rem ? `<div class="linked">${icon("bell")}${T.linked}<a class="lnk" data-open-rem="${rem.uid}">${esc(rem.title)}</a></div>` : ""}
    <div class="foot"><button class="btn danger" id="nd-del">${icon("trash-2")}${T.delNote}</button></div>`;
  loadImages(doc);
}

/** 图片点开看大图 */
function viewImage(src: string) {
  const s = screen(`<button class="ibtn" data-close aria-label="${T.close}">${icon("x")}</button><img src="${esc(src)}" alt="">`, "viewer");
  s.addEventListener("click", () => void popLayer());
}

/** 和电脑端一样：待办连着一条一次性提醒、两边状态一致时，勾待办就是完成提醒 */
async function toggleTodo(n: SNote, line: number) {
  const t = todos(n.body).find((x) => x.line === line);
  if (!t) return;
  const r = reminders().find((x) => x.note_uid === n.uid && x.todo === t.text);
  if (r && !isRecurring(r.repeat) && r.done === t.done) {
    await setDone(r, !t.done);
    if (!t.done) toast(T.todoAndRem, undefined, true);
    return;
  }
  await saveN({ ...n, body: setTodo(n.body, line, !t.done) });
}

// ---------- 编辑笔记：源码编辑 + 一排插入按钮（屏幕小，不做实时预览） ----------

const TOOLS: [keyof typeof T.tools, IconName][] = [
  ["heading", "heading"],
  ["todo", "square-check"],
  ["list", "list"],
  ["ordered", "list-ordered"],
  ["bold", "bold"],
  ["quote", "quote"],
  ["link", "link"],
  ["table", "table"],
];
const LINE_PREFIX: Partial<Record<keyof typeof T.tools, string>> = { heading: "# ", todo: "- [ ] ", list: "- ", ordered: "1. ", quote: "> " };

/** 插入按钮：行首的记号加上（已经有就去掉）；粗体、链接包住选中的字；表格另起一行 */
function insertMd(ta: HTMLTextAreaElement, kind: keyof typeof T.tools) {
  const { value } = ta;
  const a = ta.selectionStart ?? value.length;
  const b = ta.selectionEnd ?? a;
  const sel = value.slice(a, b);
  const ls0 = value.lastIndexOf("\n", a - 1) + 1;
  const prefix = LINE_PREFIX[kind];
  ta.focus();
  if (prefix) {
    // 别的行首记号先换掉（列表换成待办这种）；同一种再按一下就去掉
    const old = Object.values(LINE_PREFIX).find((p) => value.startsWith(p!, ls0)) ?? "";
    const put = old === prefix ? "" : prefix;
    ta.setRangeText(put, ls0, ls0 + old.length, "preserve");
    const caret = Math.max(ls0, a - old.length + put.length);
    ta.setSelectionRange(caret, caret);
  } else if (kind === "bold") {
    const inner = sel || T.boldPh;
    ta.setRangeText(`**${inner}**`, a, b, "end");
    if (!sel) ta.setSelectionRange(a + 2, a + 2 + inner.length);
  } else if (kind === "link") {
    if (sel) {
      ta.setRangeText(`[${sel}](https://)`, a, b, "end");
      ta.setSelectionRange(a + sel.length + 3 + 8, a + sel.length + 3 + 8);
    } else {
      ta.setRangeText(T.linkTpl, a, b, "end");
      ta.setSelectionRange(a + 1, T.linkTpl.indexOf("]") + a);
    }
  } else if (kind === "table") {
    const before = value.slice(0, a);
    const nl = before && !before.endsWith("\n") ? "\n" : "";
    ta.setRangeText(nl + T.tableTpl, a, b, "end");
    const at = a + nl.length + T.tableTpl.indexOf("|  |") + 2;
    ta.setSelectionRange(at, at);
  }
  ta.dispatchEvent(new Event("input", { bubbles: true }));
}

function editNote(n: SNote | null) {
  const isNew = !n;
  const now = nowSec();
  const base: SNote = n ?? { uid: uuidv7(), title: "", body: "", tag: S.nCat !== "all" && S.nCat !== "none" ? S.nCat : null, reminder_uid: null, created_at: now, updated_at: now, deleted_at: null, purged: false };
  // 这次编辑里传上去的附件：没保存就关掉的话标成彻底删除，不留在笔记外面
  const uploaded: SAttachment[] = [];
  let saved = false;
  const vv = window.visualViewport;
  /** 键盘弹出来时整页缩到键盘上面，插入按钮贴着键盘 */
  const fit = () => {
    if (!vv) return;
    s.style.height = `${vv.height}px`;
    s.style.top = `${vv.offsetTop}px`;
    s.classList.toggle("kb", vv.height < window.innerHeight * 0.8);
  };
  const s = screen(
    `<header class="sh"><button class="btn ghost" data-close>${T.cancel}</button><b>${isNew ? T.newNote : T.editNote}</b><button class="btn pri" data-act="save">${T.save}</button></header>
    <div class="ne">
      <input class="ne-title" id="ne-title" value="${esc(base.title)}" placeholder="${T.titlePh}" aria-label="${T.titlePh}">
      ${noteCatList().length ? `<div class="chips" id="ne-tags">${noteCatChips(base.tag)}</div>` : ""}
      <textarea id="ne-body" placeholder="${T.bodyPh}" aria-label="${T.bodyPh}">${esc(base.body)}</textarea>
    </div>
    <div class="ne-up" id="ne-up" role="status" hidden></div>
    <div class="ne-tools" id="ne-tools">${TOOLS.map(([k, ic]) => `<button type="button" class="tb" data-ins="${k}" aria-label="${T.tools[k]}">${icon(ic)}</button>`).join("")}<label class="tb" aria-label="${T.tools.attach}">${icon("paperclip")}<input type="file" id="ne-file" multiple hidden></label></div>`,
    "editor",
    () => {
      vv?.removeEventListener("resize", fit);
      vv?.removeEventListener("scroll", fit);
      if (!saved) for (const a of uploaded) void E().save({ k: "attachment", d: { ...a, deleted_at: nowSec(), purged: true } });
    },
  );
  vv?.addEventListener("resize", fit);
  vv?.addEventListener("scroll", fit);
  fit();
  const ta = $<HTMLTextAreaElement>("#ne-body", s);
  // 按插入按钮时不让输入框失去焦点（不然键盘会收起来）
  $("#ne-tools", s).addEventListener("pointerdown", (e) => {
    if ((e.target as HTMLElement).closest("[data-ins]")) e.preventDefault();
  });
  $<HTMLInputElement>("#ne-file", s).addEventListener("change", async (e) => {
    const input = e.target as HTMLInputElement;
    const files = [...(input.files ?? [])];
    input.value = "";
    const say = $("#ne-up", s);
    for (const picked of files) {
      if (picked.size > MAX_ATTACHMENT) {
        toast(T.attTooBig(picked.name));
        continue;
      }
      // 叮咪同步的套餐：图片超了先压缩，文件超了传不了
      if (overLimit(picked.type, picked.size, limits)) {
        say.hidden = false;
        say.textContent = T.shrinking;
      }
      const f = await fitPlan(picked);
      if (typeof f === "string") {
        toast(f);
        continue;
      }
      say.hidden = false;
      say.textContent = T.uploading(f.name);
      try {
        const { att, line } = await uploadAttachment(f, base.uid);
        uploaded.push(att);
        const at = ta.selectionStart ?? ta.value.length;
        const before = ta.value.slice(0, at);
        ta.setRangeText(`${before && !before.endsWith("\n") ? "\n" : ""}${line}\n`, at, ta.selectionEnd ?? at, "end");
      } catch (err) {
        toast(T.upFail(f.name, err instanceof Error ? err.message : String(err)));
      }
    }
    say.hidden = true;
  });
  s.addEventListener("click", async (e) => {
    const el = e.target as HTMLElement;
    if (el.closest("[data-close]")) return void popLayer();
    const ins = el.closest<HTMLElement>("[data-ins]");
    if (ins) return insertMd(ta, ins.dataset.ins as keyof typeof T.tools);
    const tg = el.closest<HTMLElement>("[data-tag]");
    if (tg) {
      const on = tg.getAttribute("aria-pressed") !== "true";
      $$("#ne-tags .chip", s).forEach((c) => c.setAttribute("aria-pressed", "false"));
      tg.setAttribute("aria-pressed", String(on));
    }
    if (el.closest('[data-act="save"]')) {
      const body = ta.value;
      const title = $<HTMLInputElement>("#ne-title", s).value;
      if (!title.trim() && !body.trim()) return toast(T.writeFirst);
      const tagEl = $$("#ne-tags [data-tag]", s).find((c) => c.getAttribute("aria-pressed") === "true");
      saved = true;
      await saveN({ ...base, title: noteTitle(title, body), body, tag: tagEl?.dataset.tag ?? null });
      await popLayer();
      // 新笔记存好后直接打开看
      if (isNew) openNote(base.uid);
      toast(isNew ? T.savedNote : T.noteSaved);
    }
  });
  if (isNew) setTimeout(() => ta.focus(), 300);
}

/** 在手机上加密一个附件、传到服务器，再记一条附件记录（顺序和电脑端一样：先传原文件，再传记录）。返回笔记里要插的那一行 */
async function uploadAttachment(f: File, noteUid: string): Promise<{ att: SAttachment; line: string }> {
  const bytes = new Uint8Array(await f.arrayBuffer());
  const mime = f.type || "application/octet-stream";
  const image = mime.startsWith("image/");
  const name = image ? "" : cleanFileName(f.name) || T.attFile;
  const uid = uuidv7();
  const file = `${uid}.${extFor(mime, name)}`;
  let r: Response;
  try {
    r = await fetch(`/m/api/file/${uid}`, {
      method: "POST",
      headers: { Authorization: "Bearer " + token, "Content-Type": "application/octet-stream" },
      body: seal(S.key!, fileAad(uid), bytes) as BodyInit,
    });
  } catch {
    throw new Error(T.netDown);
  }
  if (!r.ok) throw new Error((await r.text().catch(() => "")) || T.serverSaid(r.status));
  const sha = [...new Uint8Array(await crypto.subtle.digest("SHA-256", bytes))].map((x) => x.toString(16).padStart(2, "0")).join("");
  const att: SAttachment = { uid, file, mime, size: bytes.length, sha256: sha, note_uid: noteUid, created_at: nowSec(), deleted_at: null, purged: false, name };
  await E().save({ k: "attachment", d: att });
  // 自己刚传的，不用再下载
  if (!image) fileCache.set(uid, new File([bytes as BlobPart], name, { type: mime }));
  return { att, line: image ? `![](att:${file})` : `[${name}](att:${file})` };
}

// ---------- 叮咪同步的套餐限单个图片 / 文件多大（需求-1.0 第 32.1 节第 5 条） ----------

/** 服务器说的限额；自建服务器、旧服务器没有这个接口，都是 null（不限） */
let limits: Limits = { max_image: null, max_file: null };
const limitsReady: Promise<void> = token
  ? fetch("/m/api/limits", { headers: { Authorization: "Bearer " + token } })
      .then(async (r) => {
        if (r.ok) limits = await r.json();
      })
      .catch(() => {})
  : Promise.resolve();

/** 要传的图片、文件按套餐看一遍：图片超了在手机上压一份小的（JPEG），压不下来、文件超了回一句给人看的话 */
async function fitPlan(f: File): Promise<File | string> {
  await limitsReady;
  const over = overLimit(f.type, f.size, limits);
  if (!over) return f;
  if (over === "file") return T.overFile(f.name || T.attFile, fileSize(limits.max_file ?? 0));
  const small = await shrinkImage(f, limits.max_image ?? 0).catch(() => null);
  return small ?? T.imgTooBig(f.name || T.pic);
}

/** 浏览器里压图片：先降质量，还不够就按比例缩小，最短边小到 320 还压不下来就算了（动图不压）。照片的方向浏览器会转正 */
async function shrinkImage(f: File, max: number): Promise<File | null> {
  if (!/^image\/(jpeg|png|webp|heic|heif)$/.test(f.type)) return null;
  const bmp = await createImageBitmap(f);
  try {
    const canvas = document.createElement("canvas");
    const encode = (w: number, h: number, q: number) => {
      canvas.width = w;
      canvas.height = h;
      const g = canvas.getContext("2d")!;
      // 透明的地方垫白底（JPEG 没有透明）
      g.fillStyle = "#fff";
      g.fillRect(0, 0, w, h);
      g.drawImage(bmp, 0, 0, w, h);
      return new Promise<Blob | null>((res) => canvas.toBlob(res, "image/jpeg", q));
    };
    const done = (b: Blob) => new File([b], `${f.name.replace(/\.[^.]*$/, "") || T.pic}.jpg`, { type: "image/jpeg" });
    // 手机浏览器的画布最多一千六百万像素左右，太大的先缩到这以内
    let scale = Math.min(1, Math.sqrt(16_000_000 / (bmp.width * bmp.height)));
    let last = f.size;
    for (const q of [0.85, 0.75]) {
      const b = await encode(Math.round(bmp.width * scale), Math.round(bmp.height * scale), q);
      if (b && b.size <= max) return done(b);
      if (b) last = b.size;
    }
    scale *= Math.min(0.9, Math.sqrt(max / last) * 0.95);
    for (let i = 0; i < 10; i++) {
      const w = Math.round(bmp.width * scale);
      const h = Math.round(bmp.height * scale);
      if (Math.min(w, h) < 320) return null;
      const b = await encode(w, h, 0.8);
      if (b && b.size <= max) return done(b);
      scale *= 0.85;
    }
    return null;
  } finally {
    bmp.close();
  }
}

/** 太大、叮咪同步不传的附件：原文件只在哪台设备上（给人看的一句）；照常同步的回 undefined */
function awayOf(a: SAttachment): string | undefined {
  if (!a.only_on) return undefined;
  const pc = of("device").find((d) => d.uid === a.only_on)?.name;
  return T.awayOn(pc || T.someOtherPc);
}

/** 解开过的文件附件（编号 → 文件），这次打开网页期间留着，再点就直接打开 */
const fileCache = new Map<string, File>();

setFileInfo((file) => {
  const a = of("attachment").find((x) => x.file === file);
  return a && { size: a.size, local: fileCache.has(a.uid), away: awayOf(a) };
});

async function tapFile(card: HTMLElement) {
  const a = of("attachment").find((x) => x.file === card.dataset.file && !x.purged);
  if (!a) return toast(T.fileNotSynced);
  const away = awayOf(a);
  if (away) return toast(away);
  const ready = fileCache.get(a.uid);
  if (ready) return shareFile(ready);
  if (card.classList.contains("busy")) return;
  card.classList.add("busy");
  card.querySelector(".fm")!.textContent = T.downloading;
  try {
    const r = await fetch(`/m/api/file/${encodeURIComponent(a.uid)}`, { headers: { Authorization: "Bearer " + token } });
    if (r.status === 404) throw new Error(T.fileNotUploaded);
    if (!r.ok) throw new Error((await r.text().catch(() => "")) || T.serverSaid(r.status));
    const plain = open(S.key!, fileAad(a.uid), new Uint8Array(await r.arrayBuffer()));
    if (!plain) throw new Error(T.fileBad);
    fileCache.set(a.uid, new File([plain as BlobPart], a.name || a.file, { type: a.mime }));
    card.querySelector(".fm")!.textContent = `${fileSize(a.size)} · ${T.downloaded}`;
    toast(T.downloaded);
  } catch (err) {
    card.querySelector(".fm")!.textContent = fileSize(a.size);
    toast(err instanceof TypeError ? T.noServer : err instanceof Error ? err.message : String(err));
  } finally {
    card.classList.remove("busy");
  }
}

/** 用系统的分享菜单打开：能预览、存到「文件」、用别的 App 打开；不支持的浏览器直接打开 */
function shareFile(f: File) {
  if (navigator.canShare?.({ files: [f] })) {
    navigator.share({ files: [f] }).catch(() => {});
    return;
  }
  const url = URL.createObjectURL(f);
  if (!window.open(url, "_blank")) location.href = url;
}

// 笔记里的图片：从服务器下载加密的原图，在手机上解开

const imgCache = new Map<string, Promise<string | null>>();

function loadImages(root: HTMLElement) {
  const atts = of("attachment");
  for (const img of $$<HTMLImageElement>("img[data-att]", root)) {
    const p = img.parentElement!;
    p.classList.add("wait");
    p.dataset.msg = T.imgLoading;
    const a = atts.find((x) => x.file === img.dataset.att && !x.purged);
    if (!a) {
      p.dataset.msg = T.imgGone;
      continue;
    }
    // 太大、叮咪同步不传的：服务器上没有，不去下载
    const away = awayOf(a);
    if (away) {
      p.dataset.msg = away;
      continue;
    }
    if (!imgCache.has(a.uid)) imgCache.set(a.uid, fetchImage(a));
    imgCache.get(a.uid)!.then((url) => {
      if (url) {
        img.src = url;
        p.classList.remove("wait");
      } else {
        p.dataset.msg = T.imgGone;
        imgCache.delete(a.uid);
      }
    });
  }
}

async function fetchImage(a: SAttachment): Promise<string | null> {
  try {
    const r = await fetch(`/m/api/file/${encodeURIComponent(a.uid)}`, { headers: { Authorization: "Bearer " + token } });
    if (!r.ok) return null;
    const plain = open(S.key!, fileAad(a.uid), new Uint8Array(await r.arrayBuffer()));
    return plain ? URL.createObjectURL(new Blob([plain as BlobPart], { type: a.mime || "image/jpeg" })) : null;
  } catch {
    return null;
  }
}

// ---------- 发给电脑（收件箱，和原来的收件箱网页一样） ----------

/** 一次发的（加密后）最多多大：最多 50 MB 的东西，加上说明和加密多出来的一点 */
const MAX_INBOX = MAX_ATTACHMENT + 1024 * 1024;
const QUEUE = "queue";

/** 今天页顶上的「发给电脑」框。刚发出去时换成团子猫「发到电脑了」，刚记成提醒时换成「记好了」 */
function sendBox() {
  if (Date.now() < S.sentUntil) return S.saved ? savedCard(S.saved) : sentCard(T.sent(mascotOn()), T.sentSub);
  const note = S.noteMode && !!S.engine;
  return `<div class="send" id="send">
    <textarea id="cap" rows="2" placeholder="${note ? T.notePh : T.sendPh}" aria-label="${note ? T.saveNote : T.inboxTitle}"></textarea>
    <div class="thumbs" id="pics"></div>
    <div id="hit">${hitHtml()}</div>
    <div class="row">
      <button class="ib" data-act="paste">${icon("clipboard-paste")}${T.paste}</button>
      <label class="ib">${icon("image")}${T.image}<input type="file" id="file" accept="image/*" multiple hidden></label>
      <label class="ib" aria-label="${T.file}">${icon("paperclip")}<input type="file" id="file2" multiple hidden></label>
      <span class="sp"></span>
      <button class="btn pri" data-act="cap-send" id="send-btn">${note ? `${icon("notebook-pen")}${T.saveNote}` : T.send}</button>
    </div>
    <p class="cap-msg ${S.capMsg.kind}" id="cap-msg" role="status">${esc(S.capMsg.text)}</p>
  </div>`;
}

const sentCard = (title: string, sub: string) =>
  `<div class="send" id="send"><div class="sent" data-act="sent-ok"><span class="cat mascot play">${cat("happy")}</span><span class="okmark">${icon("circle-check")}</span><b>${esc(title)}</b><span>${sub}</span></div></div>`;

const savedCard = (s: { what: string; uids: string[] }) =>
  `<div class="send" id="send"><div class="sent" data-act="sent-ok"><span class="cat mascot play">${cat("happy")}</span><span class="okmark">${icon("circle-check")}</span><b>${T.hitSaved}</b><span class="sub">${T.hitSavedSub(esc(s.what))}</span><button type="button" class="hit-edit" data-act="hit-edit">${s.uids.length > 1 ? T.hitGoList : T.hitEdit}</button></div></div>`;

// ---------- 认出时间、直接记成提醒（需求-0.4 第 31 节） ----------

/** 一行里的时间：周期写「每天 08:00」，一次性的写「明天 15:00」「11/10 周一 09:00」，都带钟点 */
function hitTime(p: ParsedReminder) {
  const { due, repeat } = hitWhen(p);
  const hm = due.slice(11, 16);
  if (isRecurring(repeat)) return `${repeatLabel(repeat as RepeatArg, due)} ${hm}`;
  const d = parseDT(due) ?? new Date();
  const n = dayDiff(d);
  return n >= 0 && n < 7 ? whenFull(due) : `${d.getMonth() + 1}/${d.getDate()} ${weekdayCn(d)} ${hm}`;
}

/** 输入框下面那条浅橘的「记成提醒」；不该出时是空的 */
function hitHtml() {
  if (!S.hits.length || S.noteMode || S.files.length) return "";
  const g = S.hits.find((h) => h.guess);
  const rows = S.hits.map((h) => `<div class="ln"><b>${esc(h.title)}</b><span>${esc(hitTime(h))}</span></div>`).join("");
  return `<div class="hit">${icon("alarm-clock")}<div class="ls">${rows}</div>${g ? `<small>${esc(T.hitGuess(S.hits.length > 1 ? g.title : "", g.guess))}</small>` : ""}<button type="button" class="btn sm" data-act="hit-save">${icon("bell-plus")}${T.hitSave(S.hits.length)}</button></div>`;
}

/** 按输入框里现在的字重新认一遍，换掉下面那条 */
function refreshHit() {
  const cap = $<HTMLTextAreaElement>("#cap");
  S.hits = cap ? reminderHits(cap.value, { unlocked: !!S.engine, noteMode: S.noteMode, files: S.files.length }) : [];
  const box = $("#hit");
  if (box) box.innerHTML = hitHtml();
}

/** 点「记成提醒」：一条一条照新建提醒的存法存好（加密、排进待上传），换成「记好了」 */
async function saveHits() {
  refreshHit();
  const list = S.hits.map((h) => hitToReminder(h));
  if (!list.length) return;
  const cap = $<HTMLTextAreaElement>("#cap");
  // 先离开输入框：输入框里有光标时不整页重画，下面的列表就出不来新记的那条
  cap?.blur();
  if (cap) cap.value = "";
  S.hits = [];
  capMsg("");
  S.flash = new Set(list.map((r) => r.uid));
  setTimeout(() => S.flash.clear(), 2500);
  for (const r of list) await saveR(r);
  showSent({ what: list.length > 1 ? T.hitSavedMany(list.length) : `${whenFull(list[0].due_at)} ${list[0].title}`, uids: list.map((r) => r.uid) });
}

/** 发送框换回能输入的样子（发完、换模式时） */
function resetSend() {
  const box = $("#send");
  if (!box) return;
  box.outerHTML = sendBox();
  renderPics();
}

let sentTimer = 0;
/** 换成「发到电脑了」，saved 不是 null 时换成「记好了」；3.5 秒后换回输入框 */
function showSent(saved: { what: string; uids: string[] } | null = null) {
  S.saved = saved;
  S.sentUntil = Date.now() + 3500;
  resetSend();
  clearTimeout(sentTimer);
  sentTimer = window.setTimeout(() => {
    S.sentUntil = 0;
    resetSend();
  }, 3500);
}

/** 没网时先存在手机上的那几条 */
function queueBar() {
  const n = queue().length;
  return `<div class="queue" id="queue" ${n ? "" : "hidden"}>${icon("wifi-off")}<span>${T.queue(n)}</span></div>`;
}
const refreshQueue = () => {
  const q = $("#queue");
  if (q) q.outerHTML = queueBar();
};

function renderPics() {
  const box = $("#pics");
  if (!box) return;
  box.innerHTML = "";
  S.files.forEach((f, i) => {
    const d = document.createElement("div");
    if (f.type.startsWith("image/")) {
      d.className = "pic";
      const img = document.createElement("img");
      img.src = URL.createObjectURL(f);
      img.onload = () => URL.revokeObjectURL(img.src);
      d.append(img);
    } else {
      d.className = "pic file";
      d.innerHTML = `${icon("file-text")}<span class="fn">${esc(f.name)}</span><span class="fm">${fileSize(f.size)}</span>`;
    }
    const x = document.createElement("button");
    x.type = "button";
    x.className = "x";
    x.innerHTML = icon("x");
    x.setAttribute("aria-label", T.removeFile);
    x.onclick = () => {
      S.files.splice(i, 1);
      renderPics();
    };
    d.append(x);
    box.append(d);
  });
  // 带了图片、文件时不出「记成提醒」，拿掉以后再出
  refreshHit();
}

let capTimer = 0;
function capMsg(text: string, kind = "") {
  S.capMsg = { text, kind };
  const m = $("#cap-msg");
  if (m) {
    m.textContent = text;
    m.className = "cap-msg " + kind;
  }
  // 成功的提示过一会儿自己消失，出错的留着
  clearTimeout(capTimer);
  if (kind === "ok") capTimer = window.setTimeout(() => capMsg(""), 5000);
}

// 明文格式：4 字节（大端）说明的长度 + 说明（JSON）+ 每张图片的原始字节，依次接在后面
async function pack(text: string, list: File[]) {
  const imgs = await Promise.all(list.map((f) => f.arrayBuffer().then((b) => new Uint8Array(b))));
  const head = new TextEncoder().encode(
    JSON.stringify({
      v: 1,
      text,
      sent_at: nowSec(),
      images: list.map((f, i) => ({ name: f.name || "image", mime: f.type || "application/octet-stream", size: imgs[i].length })),
    }),
  );
  const out = new Uint8Array(4 + head.length + imgs.reduce((n, b) => n + b.length, 0));
  new DataView(out.buffer).setUint32(0, head.length);
  out.set(head, 4);
  let p = 4 + head.length;
  for (const b of imgs) {
    out.set(b, p);
    p += b.length;
  }
  return out;
}

class Offline extends Error {}

async function postInbox(text: string, list: File[]) {
  const body = sealInbox(inboxPk!, await pack(text, list));
  if (body.length > MAX_INBOX) throw new Error(T.packTooBig);
  let r: Response;
  try {
    r = await fetch("/m/api/inbox", {
      method: "POST",
      headers: { Authorization: "Bearer " + token, "Content-Type": "application/octet-stream" },
      body: body as BodyInit,
    });
  } catch {
    throw new Offline(T.offline);
  }
  if (!r.ok) throw new Error((await r.text().catch(() => "")) || T.serverSaid(r.status));
}

const queue = (): string[] => {
  try {
    return JSON.parse(ls.get(QUEUE) || "[]");
  } catch {
    return [];
  }
};

/** 之前没网时存下的文字：有网了（打开时、切回来时、网络连上时、每 30 秒）补发 */
let flushing = false;
async function flushQueue() {
  const q = queue();
  if (!q.length || flushing) return;
  flushing = true;
  const left: string[] = [];
  for (const text of q) await postInbox(text, []).catch(() => left.push(text));
  // 补发期间又存进来的也留着
  const added = queue().slice(q.length);
  ls.set(QUEUE, JSON.stringify([...left, ...added]));
  flushing = false;
  refreshQueue();
  if (left.length < q.length) toast(T.flushed(q.length - left.length), undefined, true);
}

let sending = false;
/** 发出去返回 "sent"，没网先存在手机上返回 "queued"，出错返回 null（say 写出原因） */
async function sendInbox(text: string, list: File[], say: (text: string, kind?: string) => void): Promise<"sent" | "queued" | null> {
  if (sending) return null;
  sending = true;
  say(list.length ? T.sendingBig : T.sending);
  try {
    await postInbox(text, list);
    say("");
    return "sent";
  } catch (e) {
    // 只有文字的先存在手机上（图片太大，存不下）
    if (e instanceof Offline && !list.length && text) {
      ls.set(QUEUE, JSON.stringify([...queue(), text]));
      say("");
      refreshQueue();
      return "queued";
    }
    say(e instanceof Error ? e.message : String(e), "bad");
    return null;
  } finally {
    sending = false;
  }
}

/** 点「发送」：发给电脑，或者（长按换过模式时）直接存成笔记 */
async function sendCapture() {
  const cap = $<HTMLTextAreaElement>("#cap");
  const text = cap.value.trim();
  if (S.noteMode && S.engine) {
    if (!text) return capMsg(T.noteEmpty, "bad");
    if (S.files.length) return capMsg(T.noteWithFiles, "bad");
    cap.value = "";
    capMsg("");
    await saveN({ uid: uuidv7(), title: noteTitle("", text), body: text, tag: null, reminder_uid: null, created_at: nowSec(), updated_at: nowSec(), deleted_at: null, purged: false });
    return toast(T.savedNote, undefined, true);
  }
  if (!text && !S.files.length) return capMsg(T.sendEmpty, "bad");
  const got = await sendInbox(text, S.files.slice(), capMsg);
  if (!got) return;
  // 发送期间可能同步过、重画过，重新找一次输入框
  const now = $<HTMLTextAreaElement>("#cap");
  if (now) now.value = "";
  S.files = [];
  if (got === "sent") showSent();
  else renderPics();
}

// ---------- 事件 ----------

// 长按「发送」：换成直接存成笔记（再长按换回来）
let pressTimer = 0;
let longPressed = false;
document.addEventListener("pointerdown", (e) => {
  if (!(e.target as HTMLElement).closest("#send-btn") || !S.engine) return;
  longPressed = false;
  pressTimer = window.setTimeout(() => {
    longPressed = true;
    S.noteMode = !S.noteMode;
    const text = ($("#cap") as HTMLTextAreaElement | null)?.value ?? "";
    resetSend();
    const cap = $("#cap") as HTMLTextAreaElement | null;
    if (cap) cap.value = text;
    refreshHit();
    navigator.vibrate?.(15);
    toast(S.noteMode ? T.toNoteMode : T.toSendMode);
  }, 550);
});
for (const ev of ["pointerup", "pointercancel", "pointerleave"]) document.addEventListener(ev, () => clearTimeout(pressTimer));
document.addEventListener("contextmenu", (e) => {
  if ((e.target as HTMLElement).closest("#send-btn")) e.preventDefault();
});

app.addEventListener("click", async (e) => {
  const el = e.target as HTMLElement;
  const v = el.closest<HTMLElement>("[data-view]");
  if (v) {
    S.view = v.dataset.view as View;
    ls.set("view", S.view);
    render();
    scrollTo(0, 0);
    return;
  }
  const chk = el.closest<HTMLElement>("[data-chk]");
  if (chk) {
    const r = findR(chk.dataset.chk!);
    if (!r) return;
    if (chk.dataset.done === "1") {
      if (isRecurring(r.repeat)) return toast(T.recurDone);
      await setDone(r, false);
    } else await completeWithUndo(r);
    return;
  }
  const rowEl = el.closest<HTMLElement>("[data-rem]");
  if (rowEl) {
    const r = findR(rowEl.dataset.rem!);
    if (r) openReminder(structuredClone(r));
    return;
  }
  const nb = el.closest<HTMLElement>("[data-note]");
  if (nb) return openNote(nb.dataset.note!);
  const rf = el.closest<HTMLElement>("[data-rf]");
  if (rf) {
    S.rFilter = rf.dataset.rf!;
    return render();
  }
  const nc = el.closest<HTMLElement>("[data-nc]");
  if (nc) {
    S.nCat = nc.dataset.nc === S.nCat ? "all" : nc.dataset.nc!;
    return render();
  }
  const actEl = el.closest<HTMLElement>("[data-act]");
  const act = actEl?.dataset.act;
  if (act === "sync") return void S.engine?.sync();
  if (act === "paste") return void pasteClip();
  if (act === "new-rem") return openReminder(newReminder(), true);
  if (act === "new-note") return editNote(null);
  if (act === "style") return openStyle();
  if (act === "home") return openHome();
  if (act === "add-pc") return void openAddPc();
  if (act === "hit-save") return saveHits();
  if (act === "hit-edit") {
    const uids = S.saved?.uids ?? [];
    S.sentUntil = 0;
    resetSend();
    const r = uids.length === 1 ? findR(uids[0]) : undefined;
    if (r) return openReminder(structuredClone(r));
    S.view = "reminders";
    ls.set("view", S.view);
    render();
    return scrollTo(0, 0);
  }
  if (act === "sent-ok") {
    S.sentUntil = 0;
    return resetSend();
  }
  if (act === "clear-q") {
    S.q = "";
    const q = $<HTMLInputElement>("#q");
    q.value = "";
    $("#nl").innerHTML = noteList();
    actEl!.hidden = true;
    return q.focus();
  }
  if (act === "lock" && sure($(".nm", actEl!), T.lockSure)) return lock(T.locked);
  if (act === "cap-send") {
    if (longPressed) return void (longPressed = false);
    return sendCapture();
  }
});

app.addEventListener("change", (e) => {
  const f = e.target as HTMLInputElement;
  if ((f.id !== "file" && f.id !== "file2") || !f.files) return;
  void addFiles([...f.files]);
  f.value = "";
});

/** 选好的图片、文件放进待发列表；单个和一次合起来都不能超过 50 MB。叮咪同步的套餐：图片超了先压缩，文件超了直接说 */
async function addFiles(list: File[]) {
  for (const picked of list) {
    if (picked.size > MAX_ATTACHMENT) {
      capMsg(T.tooBig(picked.name), "bad");
      continue;
    }
    if (overLimit(picked.type, picked.size, limits)) capMsg(T.shrinking);
    const f = await fitPlan(picked);
    if (typeof f === "string") {
      capMsg(f, "bad");
      continue;
    }
    if (f !== picked) capMsg("");
    if (S.files.reduce((n, x) => n + x.size, 0) + f.size > MAX_ATTACHMENT) {
      capMsg(T.totalTooBig, "bad");
      break;
    }
    S.files.push(f);
  }
  renderPics();
}

/** 「粘贴」按钮：iPhone 会先弹出系统的「粘贴」小气泡，点了才读得到。截图放进待发图片，文字插在光标处 */
async function pasteClip() {
  const cap = $<HTMLTextAreaElement>("#cap");
  const insert = (text: string) => {
    if (!text) return capMsg(T.clipNoText, "bad");
    const at = cap.selectionStart ?? cap.value.length;
    cap.setRangeText(text, at, cap.selectionEnd ?? at, "end");
    capMsg("");
  };
  try {
    if (navigator.clipboard?.read) {
      for (const it of await navigator.clipboard.read()) {
        const img = it.types.find((t) => t.startsWith("image/"));
        if (img) {
          const blob = await it.getType(img);
          void addFiles([new File([blob], `${T.pastedImage}.${extFor(img, "")}`, { type: img })]);
          return;
        }
        if (it.types.includes("text/plain")) return insert(await (await it.getType("text/plain")).text());
      }
      return capMsg(T.clipEmpty, "bad");
    }
    insert(await navigator.clipboard.readText());
  } catch {
    capMsg(T.clipDenied, "bad");
  }
}

let hitTimer = 0;
app.addEventListener("input", (e) => {
  const el = e.target as HTMLInputElement;
  // 边打字边认：停 0.3 秒再认
  if (el.id === "cap") {
    clearTimeout(hitTimer);
    hitTimer = window.setTimeout(refreshHit, 300);
  }
  if (el.id === "q") {
    S.q = el.value;
    $("#nl").innerHTML = noteList();
    $("[data-act=clear-q]").hidden = !S.q;
  }
});

// 发送框输完（离开输入框）再把攒着的刷新画上
app.addEventListener("focusout", (e) => {
  if ((e.target as HTMLElement).id === "cap" && renderLater) setTimeout(render, 0);
});

// 换了风格、手机换了深浅色：团子猫跟着变，重画一遍
onLookChange(() => {
  if (S.engine) render();
});

// ---------- 第一次打开、退出 ----------

interface SpaceInfo {
  salt: string | null;
  box1: string | null;
}

async function space(): Promise<SpaceInfo> {
  const r = await fetch("/m/api/space", { headers: { Authorization: "Bearer " + token } });
  if (r.status === 401) throw new Error(T.revoked);
  if (!r.ok) throw new Error((await r.text().catch(() => "")) || T.serverSaid(r.status));
  return r.json();
}

async function lock(msg: string) {
  S.engine = null;
  S.key = null;
  S.noteMode = false;
  for (const k of ["since", "last-sync"]) ls.set(k, null);
  await forgetMaster(keyPlace);
  await wipe();
  S.lockMsg = msg;
  showUnlock();
}

function showUnlock() {
  closeLayers();
  app.innerHTML = `<main class="cent">
    <span class="cat mascot play big">${cat("wave")}</span>
    <h1>${T.hi}</h1>
    <p>${T.hiLead}</p>
    ${S.lockMsg ? `<div class="banner info">${icon("info")}<div>${esc(S.lockMsg)}</div></div>` : ""}
    <form id="ul" autocomplete="on">
      <input type="text" name="username" value="叮咪同步" autocomplete="username" hidden>
      <label class="fld">${T.pw}<input class="field" type="password" id="pw" placeholder="${T.pwPh}" autocomplete="current-password"></label>
      <p class="cap-msg" id="ul-msg" role="status"></p>
      <button class="btn pri full" id="ul-go" type="submit">${T.go}</button>
    </form>
    <button class="link" id="ul-inbox">${T.inboxOnly}</button>
    <p class="fine">${T.pwNote}</p>
  </main>`;
  $("#ul-inbox").onclick = showInboxOnly;
  $<HTMLFormElement>("#ul").onsubmit = async (e) => {
    e.preventDefault();
    const pw = $<HTMLInputElement>("#pw").value;
    const msg = $("#ul-msg");
    const btn = $<HTMLButtonElement>("#ul-go");
    if (!pw) return;
    btn.disabled = true;
    msg.className = "cap-msg";
    msg.textContent = T.checking;
    try {
      const sp = await space();
      if (!sp.salt || !sp.box1) throw new Error(T.noSyncYet);
      if (!spaceKey) throw new Error(T.noKey);
      msg.textContent = T.deriving;
      const key = await unlockBox1(pw, sp.salt, sp.box1, spaceKey);
      await saveMaster(keyPlace, key);
      S.lockMsg = "";
      await start(key);
    } catch (err) {
      msg.className = "cap-msg bad";
      msg.textContent = err instanceof TypeError ? T.netBad : err instanceof Error ? err.message : String(err);
      btn.disabled = false;
    }
  };
}

/** 没输同步密码：只往电脑发东西 */
function showInboxOnly() {
  app.innerHTML = `${top(T.inboxTitle, T.inboxSub)}
    <main class="pb">${sendBox()}${queueBar()}
    <p class="fine">${T.inboxNote}${standalone ? "" : `<br>${T.inboxHome}`}</p>
    <button class="link" id="to-unlock">${T.unlock}，${T.unlockSub}</button>
    <div class="card">${row(T.addPc, T.addPcSub, "", 'data-act="add-pc"')}${row(T.style, T.styleSub, STYLES.find(([id]) => id === getStyle())![1], 'data-act="style"')}${homeRows()}</div></main>`;
  $("#to-unlock").onclick = showUnlock;
  renderPics();
}

const SHARE_BOX = "dingmi-share";

/** 安卓分享菜单带过来的（sw.js 接住放在缓存里的）：文字、链接、图片、文件，取出来就删掉 */
async function takeShared(): Promise<{ text: string; files: File[] } | null> {
  if (!("caches" in window)) return null;
  try {
    const box = await caches.open(SHARE_BOX);
    const r = await box.match("/m/share/data");
    if (!r) return null;
    const d = (await r.json()) as { title: string; text: string; url: string; files: { name: string; type: string }[] };
    const files = await Promise.all(
      d.files.map(async (f, i) => {
        const b = await (await box.match(`/m/share/file/${i}`))?.blob();
        return b ? new File([b], f.name || "file", { type: f.type || b.type }) : null;
      }),
    );
    await caches.delete(SHARE_BOX);
    return { text: [...new Set([d.title, d.text, d.url].map((s) => s.trim()).filter(Boolean))].join("\n"), files: files.filter((f): f is File => !!f) };
  } catch {
    return null;
  }
}

/** 安卓从别的 App 分享过来（文字、链接在 ?title=&text=&url= 里；带图片、文件的由 sw.js 放在缓存里）：先看一眼，可以补一句、去掉几张图再发 */
function showShare(content: string, files: File[], then: () => void) {
  app.innerHTML = `${top(T.inboxTitle, "", ibtn("x", T.close, 'id="sh-x"'))}
    <main class="pb"><div class="send" id="send">
      ${content ? `<div class="shared">${esc(content).replace(/(https?:\/\/\S+)/g, '<span class="url">$1</span>').replace(/\n/g, "<br>")}</div>` : ""}
      <div class="thumbs" id="pics"></div>
      <textarea id="sh-add" rows="1" placeholder="${T.shareAdd}" aria-label="${T.shareAdd}"></textarea>
      <div class="row"><span class="sp"></span><button class="btn pri" id="sh-go">${T.send}</button></div>
      <p class="cap-msg" id="sh-msg" role="status"></p>
    </div>${queueBar()}<p class="fine">${T.shareBack}</p></main>`;
  const say = (text: string, kind = "") => {
    const m = $("#sh-msg");
    if (m) (m.textContent = text), (m.className = "cap-msg " + kind);
  };
  // 和网页里选图片一样：单个、一次合起来都不能超过 50 MB，超了的不放进来，直接说；
  // 叮咪同步的套餐：图片超了先压缩，文件超了直接说
  S.files = [];
  void (async () => {
    for (const picked of files) {
      if (picked.size > MAX_ATTACHMENT) {
        say(T.tooBig(picked.name), "bad");
        continue;
      }
      const f = await fitPlan(picked);
      if (typeof f === "string") say(f, "bad");
      else if (S.files.reduce((n, x) => n + x.size, 0) + f.size > MAX_ATTACHMENT) say(T.totalTooBig, "bad");
      else S.files.push(f);
    }
    renderPics();
  })();
  $("#sh-x").onclick = () => {
    S.files = [];
    then();
  };
  $("#sh-go").onclick = async () => {
    const text = [$<HTMLTextAreaElement>("#sh-add").value.trim(), content].filter(Boolean).join("\n");
    if (!text && !S.files.length) return say(T.sendEmpty, "bad");
    const got = await sendInbox(text, S.files.slice(), say);
    if (got) S.files = [];
    if (got === "sent") $("#send").outerHTML = sentCard(T.sent(mascotOn()), T.shareBack);
    else if (got === "queued") $("#send").remove();
  };
}

async function start(key: Uint8Array) {
  S.key = key;
  S.engine = new Engine(token, key, () => {
    render();
    // 同步好了说明有网，顺手把没发出去的补发了
    if (S.engine?.status === "ok") void flushQueue();
  });
  await S.engine.load();
  render();
  await S.engine.sync();
}

// ---------- 启动 ----------

(async () => {
  if (!token || !inboxPk || inboxPk.length !== 32) {
    app.innerHTML = `<main class="cent"><span class="cat mascot">${cat("worry")}</span><h1>${T.hi}</h1><p>${T.badLink}</p></main>`;
    return;
  }
  // 安卓分享菜单要有 Service Worker（sw.js）；注册不了（证书不受信任、浏览器不支持）就算了，照旧用「粘贴」
  if ("serviceWorker" in navigator) navigator.serviceWorker.register("/m/sw.js", { scope: "/m/" }).catch(() => {});
  // 快捷指令打开时网址后面带着 #text=<内容>，看到就直接发给电脑（说明已经不给了，以前做过的照样能用）。
  // 安卓分享过来的在 ?title=&text=&url= 里（带图片、文件的是 ?shared=1，内容在 sw.js 放的缓存里），先给看一眼再发。
  // 去掉这些，留着 k：加到主屏幕时存的是当前网址，主屏幕上的网页和 Safari 的存储是分开的，要靠网址带过去
  const shortcut = (hashArgs.get("text") || "").trim();
  const qs = new URLSearchParams(location.search);
  let shared = [...new Set(["title", "text", "url"].map((k) => (qs.get(k) || "").trim()).filter(Boolean))].join("\n");
  let sharedFiles: File[] = [];
  const box = qs.get("shared") === "1" ? await takeShared() : null;
  if (box) ({ text: shared, files: sharedFiles } = box);
  // 网址换成 /m/#t=…&k=…：以前的网址（令牌在路径里）也换掉，之后再加到主屏幕、分享出去的都是新的写法
  const k = hashArgs.get("k");
  history.replaceState(null, "", cleanUrl(seg, k));
  // 以前的网址给的清单带着令牌：换成同一份的（打开 /m/，用存下的配对信息）。已经装好的照样能用
  if (pairing.legacy) document.querySelector('link[rel="manifest"]')?.setAttribute("href", "/m/app.webmanifest");

  // 以前明文存的主密钥在这里换成加密的
  const saved = await loadMaster(keyPlace);
  const home = async () => {
    if (saved) await start(saved);
    else showInboxOnlyOrUnlock();
  };
  if (shared || sharedFiles.length) showShare(shared, sharedFiles, () => void home());
  else await home();

  await flushQueue();
  if (shortcut) {
    const cap = $<HTMLTextAreaElement>("#cap");
    if (cap) cap.value = shortcut;
    const got = await sendInbox(shortcut, [], capMsg);
    if (got) {
      if (cap) cap.value = "";
      if (got === "sent") {
        showSent();
        toast(T.sentBack, undefined, true);
      }
    }
  }

  // 在前台时每 30 秒同步一次（有没发出去的也补发）；切回来、网络连上时马上来一次
  setInterval(() => {
    if (document.visibilityState !== "visible") return;
    if (S.engine && !layers.length) void S.engine.sync();
    else if (!S.engine) void flushQueue();
  }, 30000);
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState !== "visible") return;
    void S.engine?.sync();
    void flushQueue();
  });
  window.addEventListener("online", () => {
    void flushQueue();
    void S.engine?.sync();
  });
})();

/** 没输过同步密码：先看看电脑上开没开同步，没开就直接是只往电脑发东西 */
function showInboxOnlyOrUnlock() {
  showUnlock();
  space()
    .then((sp) => {
      if (!sp.salt && !S.engine) showInboxOnly();
    })
    .catch(() => {});
}
