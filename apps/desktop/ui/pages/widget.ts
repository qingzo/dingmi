// 桌面组件（UI设计.md U8、design/ui/u8-widget.html）：贴在桌面上的一张卡片，团子猫从顶边探头；
// 从上到下是日期和件数、下一条、今天、待看、常用、输入框。默认不透明，设置里开了「毛玻璃」才透出壁纸（desk.rs）
import "../styles/ui.css";
import "../styles/widget.css";
import { isDark, mascotOn, onModeChange } from "../lib/look";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, errText, type Device, type Note, type Reminder, type Settings } from "../lib/api";
import { plain } from "@shared/md";
import { cat, type CatMood } from "@shared/mascot";
import { icon, type IconName } from "@shared/icons";
import { editPins, initPins, setPinsConfig } from "../lib/pins";
import { guess, quickSaveItems } from "../lib/quick";
import { T } from "../text/widget";
import { $, addDays, ago, dayStr, esc, fireAt, friendlyDT, isRecurring, noteCatDef, parseDT, setNoteCats, setTags, tagColor, timeOf, weekdayCn } from "@shared/util";

const win = getCurrentWindow();
/** 「今天」最多几行，「待看」最多几条 */
const MAX_TODAY = 6;
const MAX_TOREAD = 3;
const TOREAD = "toread";

const S = {
  reminders: [] as Reminder[],
  notes: [] as Note[],
  settings: null as Settings | null,
  devices: [] as Device[],
  busy: false,
  /** 刚勾掉一条时，等勾选的动画放完再重画 */
  holdUntil: 0,
  /** 团子猫现在的样子；null 是还没画过 */
  mood: null as CatMood | null,
};

// 收起了哪几块，只记在这台电脑的这个组件里
const FOLD_KEY = "widget-fold";
let folded: string[] = [];
try {
  folded = JSON.parse(localStorage.getItem(FOLD_KEY) ?? "[]");
} catch {
  folded = [];
}

const hiddenPart = (p: string) => !!S.settings?.widget_hide.includes(p);
const hour = () => new Date().getHours();
/** 夜里 22:00–6:00 团子猫睡觉 */
const night = () => hour() >= 22 || hour() < 6;

// ---------- 画 ----------

/** 45 分钟后 / 2 小时 10 分钟后 / 明天 09:00 */
function countdown(f: string): string {
  const d = parseDT(f)!;
  const min = Math.max(0, Math.ceil((d.getTime() - Date.now()) / 60000));
  if (dayStr(d) !== dayStr(new Date())) return friendlyDT(f);
  if (min < 1) return T.soon;
  if (min < 60) return `${T.inMin(min)} · ${timeOf(f)}`;
  return `${T.inHour(Math.floor(min / 60), min % 60)} · ${timeOf(f)}`;
}

function todayLists() {
  const tk = dayStr(new Date());
  const pending = S.reminders.filter((r) => !r.done && fireAt(r).slice(0, 10) <= tk).sort((a, b) => fireAt(a).localeCompare(fireAt(b)));
  const done = S.reminders.filter(
    (r) => (r.done && r.completed_at?.startsWith(tk)) || (isRecurring(r.repeat) && r.last_done_at?.startsWith(tk)),
  );
  return { pending, done };
}

/** 明天要做的几件（夜里日期下面那一行用） */
function tomorrowCount() {
  const tm = dayStr(addDays(new Date(), 1));
  return S.reminders.filter((r) => !r.done && fireAt(r).slice(0, 10) === tm).length;
}

/** 团子猫：夜里睡觉；白天全做完了开心；今天没事也眯一会儿；平时睁眼 */
function setMood(pending: number, done: number) {
  const m: CatMood = night() ? "sleep" : pending ? "normal" : done ? "happy" : "sleep";
  if (m === S.mood) return;
  // 睡觉的 zz 一直轻轻飘；开心是刚做完最后一件时蹦一下（刚打开组件时不蹦）；减少动画时都不动
  const still = document.documentElement.dataset.motion === "reduce";
  const peek = $("#peek");
  peek.classList.toggle("play", !still && (m === "sleep" || (m === "happy" && S.mood !== null)));
  peek.innerHTML = cat(m);
  S.mood = m;
}

function renderHead(pending: number, doneN: number) {
  const now = new Date();
  const m = mascotOn();
  $("#w-day").textContent = T.date(now.getMonth() + 1, now.getDate());
  $("#w-wd").textContent = weekdayCn(now);
  const tm = !pending && hour() >= 22 ? tomorrowCount() : 0;
  $("#w-sub").innerHTML = pending ? T.left(pending) : tm ? T.tomorrow(tm) : doneN ? T.allDone(m) : T.free(m);
}

function renderNext() {
  const now = new Date();
  const next = S.reminders
    .filter((r) => !r.done && (parseDT(fireAt(r)) ?? now) > now)
    .sort((a, b) => fireAt(a).localeCompare(fireAt(b)))[0];
  const box = $("#next");
  box.hidden = !next || hiddenPart("next");
  if (!next) return;
  box.dataset.rid = String(next.id);
  box.innerHTML = `${icon("bell", "s16")}<div class="m"><b>${esc(next.title)}</b><small class="num">${esc(countdown(fireAt(next)))}</small></div>`;
}

/** 前几天就该做、还没做的：写"昨天"或者日期 */
function pastDay(f: string) {
  const d = parseDT(f);
  if (!d || dayStr(d) >= dayStr(new Date())) return "";
  return dayStr(d) === dayStr(addDays(new Date(), -1)) ? T.yesterday : `${d.getMonth() + 1}/${d.getDate()}`;
}

function remRow(r: Reminder, done: boolean) {
  const f = fireAt(r);
  const late = !done && (parseDT(f) ?? new Date()) < new Date();
  // 周期提醒做完后已经排到下一次了，没法在这里取消
  const fixed = done && isRecurring(r.repeat);
  const time = done ? timeOf(r.completed_at ?? r.last_done_at ?? f) : pastDay(f) || timeOf(f);
  return `<div class="it${done ? " done" : ""}${fixed ? " fixed" : ""}" data-rid="${r.id}">
    <button class="task" data-chk="${r.id}" aria-label="${done ? T.undo : T.done}">${icon("check")}</button>
    <span class="t">${esc(r.title)}</span><span class="tm num${late ? " late" : ""}">${time}</span></div>`;
}

/** 一块的标题：图标、名字，右边收起的箭头 */
function blockHead(id: string, ico: IconName, name: string) {
  const el = document.querySelector<HTMLElement>(`[data-fold="${id}"]`)!;
  const closed = folded.includes(id);
  el.innerHTML = `${icon(ico)}<span>${esc(name)}</span><span class="n">${icon("chevron-up")}</span>`;
  el.title = closed ? T.unfold : T.fold;
  el.setAttribute("aria-expanded", String(!closed));
}

function renderToday() {
  const { pending, done } = todayLists();
  renderHead(pending.length, done.length);
  setMood(pending.length, done.length);
  // 白天全做完了：「今天」那块换成一条「做完啦」
  const allOk = !night() && !pending.length && done.length > 0 && !hiddenPart("today");
  const ok = $("#allok");
  ok.hidden = !allOk;
  ok.innerHTML = allOk ? `${icon("circle-check", "s16")}<span>${esc(T.allOk(done.length, mascotOn()))}</span>` : "";
  $("#sec-today").hidden = allOk || hiddenPart("today");
  blockHead("today", "sun", T.today);
  const rows = [...pending.map((r) => remRow(r, false)), ...done.map((r) => remRow(r, true))];
  const more = rows.length - MAX_TODAY;
  $("#today").innerHTML = rows.length
    ? rows.slice(0, MAX_TODAY).join("") + (more > 0 ? `<button class="morelink" data-go="today">${T.moreRows(more)}</button>` : "")
    : `<div class="none">${T.noToday}</div>`;
}

/** 待看那条右边的小字：带链接的写网站名，不然写什么时候记的 */
function noteSub(n: Note) {
  const url = `${n.title} ${n.body}`.match(/https?:\/\/[^\s)]+/)?.[0];
  try {
    if (url) return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    // 不是网址就写时间
  }
  return ago(n.created_at).replace(/ \d+:\d+$/, "");
}

function renderToread() {
  const c = noteCatDef(TOREAD);
  $("#sec-toread").hidden = !c || hiddenPart("toread");
  if (!c) return;
  blockHead("toread", "bookmark", c.name);
  const list = S.notes.filter((n) => n.tag === TOREAD).sort((a, b) => b.created_at.localeCompare(a.created_at));
  const more = list.length - MAX_TOREAD;
  $("#toread").innerHTML = list.length
    ? list
        .slice(0, MAX_TOREAD)
        .map(
          (n) =>
            `<div class="rd" data-nid="${n.id}"><span class="sq" style="${tagColor(c.color)}"></span><span class="t">${esc(n.title.trim() || plain(n.body).slice(0, 30) || T.blankNote)}</span><small>${esc(noteSub(n))}</small></div>`,
        )
        .join("") + (more > 0 ? `<button class="morelink" data-go="toread">${T.moreRows(more)}</button>` : "")
    : `<div class="none">${esc(T.noToread(c.name))}</div>`;
}

function render() {
  for (const el of document.querySelectorAll<HTMLElement>("[data-part]")) el.hidden = hiddenPart(el.dataset.part!);
  for (const id of ["today", "toread", "pins"]) $(`#sec-${id}`).classList.toggle("closed", folded.includes(id));
  blockHead("pins", "pin", T.pins);
  $<HTMLElement>('[data-m="pins"]').hidden = hiddenPart("pins");
  renderNext();
  renderToday();
  renderToread();
}

async function load() {
  [S.reminders, S.notes] = await Promise.all([api.listReminders(), api.listNotes()]);
  const wait = S.holdUntil - Date.now();
  if (wait > 0) setTimeout(render, wait);
  else render();
}

async function loadSettings(s?: Settings) {
  S.settings = s ?? (await api.getSettings());
  setTags(S.settings.tags);
  setNoteCats(S.settings.note_cats);
  setPinsConfig(S.settings.launcher);
  S.devices = await api.listDevices().catch(() => []);
}

/** 不会变的那些字和图标 */
function paintStatic() {
  const mb = $("#menu-btn");
  mb.innerHTML = icon("ellipsis", "s16");
  mb.title = T.more;
  mb.setAttribute("aria-label", T.more);
  const mi = (m: string, ico: IconName, label: string) => `<button class="mi" data-m="${m}" role="menuitem">${icon(ico)}${label}</button>`;
  $("#menu").innerHTML =
    mi("open", "app-window", T.menu.open) +
    mi("move", "move", T.menu.move) +
    mi("pins", "pencil", T.menu.pins) +
    mi("parts", "settings-2", T.menu.parts) +
    '<div class="msep"></div>' +
    mi("off", "eye-off", T.menu.off);
  $("#in-ico").outerHTML = icon("sparkles");
  inp.placeholder = T.placeholder;
  inp.setAttribute("aria-label", T.inputLabel);
  $("#movebar").innerHTML = `<span>${T.moveHint}</span><button class="btn sm" id="move-home">${T.moveHome}</button><button class="btn sm pri" id="move-done">${T.moveDone}</button>`;
}

// ---------- 底色：不透明 / 毛玻璃 ----------

/** 毛玻璃上叠的颜色：这种风格的卡片色，半透明；设置里的「深浅」（0–100，50 是默认）调浓淡 */
function tint(): [number, number, number, number] {
  const v = getComputedStyle(document.documentElement).getPropertyValue("--surf").trim();
  const hex = /^#([0-9a-f]{6})$/i.exec(v)?.[1];
  const n = hex ? parseInt(hex, 16) : 0xfffcf8;
  const level = S.settings?.widget_glass_level ?? 50;
  const a = Math.round(Math.min(245, Math.max(64, (isDark() ? 0xc0 : 0xb0) + (level - 50) * 1.6)));
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255, a];
}

/** 按设置换底（Rust 那边跟着改窗口大小，卡片在屏幕上不动），顺便显示出来（不抢焦点） */
async function applyLook() {
  document.body.classList.toggle("glass", !!S.settings?.widget_glass);
  const glass = await api.widgetShow(tint());
  // 系统上不了毛玻璃（Windows 10 等）就是不透明的
  document.body.classList.toggle("glass", glass);
  fit();
}

// ---------- 窗口跟着卡片的高度走 ----------

let lastH = 0;
async function fit() {
  // 菜单打开时窗口至少要能放下菜单
  const menu = $("#menu");
  const h = Math.ceil(Math.max($("#wrap").offsetHeight, menu.hidden ? 0 : menu.getBoundingClientRect().bottom + 12));
  if (h !== lastH) {
    lastH = h;
    await api.fitWindow(h);
  }
}
new ResizeObserver(() => fit()).observe($("#wrap"));

// ---------- 点 ----------

function closeMenu() {
  $("#menu").hidden = true;
  $("#menu-btn").setAttribute("aria-expanded", "false");
  fit();
}

async function menuAction(m: string) {
  closeMenu();
  if (m === "open") await api.openWindow("main");
  else if (m === "parts") await api.openMainNav("settings", undefined, undefined, "widget", "parts");
  else if (m === "off") await api.widgetOff();
  else if (m === "pins") {
    setFold("pins", false);
    editPins();
  } else if (m === "move") {
    document.body.classList.add("moving");
    $("#movebar").hidden = false;
  }
}

function setFold(id: string, closed: boolean) {
  folded = closed ? [...new Set([...folded, id])] : folded.filter((x) => x !== id);
  try {
    localStorage.setItem(FOLD_KEY, JSON.stringify(folded));
  } catch {
    // 存不下也没关系，只是下次不记得
  }
  render();
}

async function check(id: number, btn: HTMLElement) {
  const row = btn.closest<HTMLElement>(".it")!;
  if (row.classList.contains("fixed")) return;
  const undo = row.classList.contains("done");
  row.classList.toggle("done", !undo);
  S.holdUntil = Date.now() + 600;
  try {
    await api.reminderAction(id, undo ? "undo" : "done");
  } catch {
    row.classList.toggle("done", undo);
  }
}

$("#w").addEventListener("click", async (e) => {
  const el = e.target as HTMLElement;
  if (document.body.classList.contains("moving")) return;
  if (el.closest("#menu-btn")) {
    const menu = $("#menu");
    menu.hidden = !menu.hidden;
    $("#menu-btn").setAttribute("aria-expanded", String(!menu.hidden));
    fit();
    return;
  }
  const m = el.closest<HTMLElement>("[data-m]");
  if (m) return menuAction(m.dataset.m!);
  if (!$("#menu").hidden && !el.closest("#menu")) return closeMenu();
  const chk = el.closest<HTMLElement>("[data-chk]");
  if (chk) return check(Number(chk.dataset.chk), chk);
  const fold = el.closest<HTMLElement>("[data-fold]");
  if (fold) return setFold(fold.dataset.fold!, !folded.includes(fold.dataset.fold!));
  const go = el.closest<HTMLElement>("[data-go]");
  if (go) return go.dataset.go === "today" ? api.openMainNav("today") : api.openMainNav("notes", undefined, undefined, TOREAD);
  const rem = el.closest<HTMLElement>("[data-rid]");
  if (rem) return api.openMainNav("reminders", undefined, Number(rem.dataset.rid));
  const note = el.closest<HTMLElement>("[data-nid]");
  if (note) return api.openMainNav("notes", Number(note.dataset.nid));
});

// 移动位置：按住卡片拖（交给系统拖窗口），放好点「好了」记住
$("#wrap").addEventListener("pointerdown", (e) => {
  if (e.button !== 0 || !document.body.classList.contains("moving") || (e.target as HTMLElement).closest(".movebar button")) return;
  win.startDragging();
});
function endMove() {
  document.body.classList.remove("moving");
  $("#movebar").hidden = true;
  api.widgetPlace();
}
$("#movebar").addEventListener("click", (e) => {
  const b = (e.target as HTMLElement).closest("button");
  if (b?.id === "move-done") endMove();
  else if (b?.id === "move-home") api.widgetPlace(true);
});

// ---------- 记一句 ----------

const inp = $<HTMLInputElement>("#inp");
const pre = $("#pre");
let preTimer = 0;

/** 回车：提醒 · 明天 09:00 */
function preview(v: string) {
  if (!v) return "";
  const g = guess(v, S.devices);
  const what = [g.reminder && `${T.asReminder} · ${g.reminder.when}`, g.note && (g.reminder ? T.alsoNote : `${T.asNote} · ${T.toNotebook}`), g.place && T.onlyOn(g.place)]
    .filter(Boolean)
    .join("，");
  // 说得含糊：先定的时间怎么来的，接着补一句时间这里就变
  const vague = g.reminder?.guess ? `<span style="color:var(--warn)"> · ${esc(g.reminder.guess)}</span>` : "";
  return `${T.enter}<b>${esc(what)}</b>${vague}${S.settings?.ai_key ? `<span> · ${T.aiCheck}</span>` : ""}`;
}

inp.addEventListener("input", () => {
  clearTimeout(preTimer);
  pre.className = "pv";
  pre.innerHTML = preview(inp.value.trim());
});

$("#form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const v = inp.value.trim();
  if (!v || S.busy) return;
  const useAI = !!S.settings?.ai_key;
  S.busy = true;
  inp.disabled = true;
  pre.className = "pv";
  pre.innerHTML = `<span class="spin"></span>${useAI ? T.thinking : T.saving}`;
  try {
    const items = await quickSaveItems(v, useAI, S.devices);
    const r = items.find((i) => i.type === "reminder");
    const n = items.find((i) => i.type === "note");
    pre.textContent = r && r.type === "reminder" ? T.savedRem(friendlyDT(r.due), r.title, !!n) : T.savedNote(n?.title || v);
    pre.className = "pv ok";
    inp.value = "";
    preTimer = window.setTimeout(() => {
      if (!inp.value) pre.innerHTML = "";
    }, 3000);
  } catch (err) {
    pre.className = "pv bad";
    pre.textContent = T.saveFail(errText(err));
  } finally {
    S.busy = false;
    inp.disabled = false;
    inp.focus();
  }
});

document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  if (document.body.classList.contains("moving")) return endMove();
  if (!$("#menu").hidden) return closeMenu();
  if (document.activeElement === inp && !S.busy) {
    inp.value = "";
    pre.innerHTML = "";
    inp.blur();
  }
});

// ---------- 启动 ----------

(async () => {
  paintStatic();
  await initPins(() => !hiddenPart("pins"));
  await loadSettings();
  await load();
  await fit();
  // 画好了再摆好、显示
  await applyLook();
  onModeChange(() => applyLook());

  listen("data-changed", () => load());
  listen<Settings>("settings-changed", async (e) => {
    await loadSettings(e.payload);
    // 关掉团子猫、换了风格：猫和带猫的那几句跟着换
    S.mood = null;
    render();
    await applyLook();
  });
  // 倒计时、「今天」、团子猫睡没睡跟着时间走
  setInterval(render, 30000);
})();
