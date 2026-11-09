// 设置（UI设计.md U9、design/ui/u9-settings.html；需求-0.2 第 6 节）：主窗口内容区里，左边一栏「设置」标题、搜索（Ctrl + F）和 9 页导航，右边是这一页。
// 同步那页在 settings-sync.ts，几块共用的小工具在 settings-ui.ts，字在 text/settings.ts，样子在 styles/settings.css
import "../styles/settings.css";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { api, errText, type DiagPart, type WebKey, type Checklist, type Exported, type ExportProgress, type LaunchConfig, type LaunchItem, type LookStyle, type Restored, type RestorePeek, type Settings, type SyncStatus, type TagDef, type TermsDoc, type TermsView, type UpdateStatus } from "../lib/api";
import { mascotDefault, mascotOn, previewLook } from "../lib/look";
import { iconHtml, visible } from "../lib/launch";
import { checkKey } from "../lib/webkey";
import { findRows, markName, rowsOf, type Found } from "../lib/setfind";
import { icon, type IconName } from "@shared/icons";
import { appIcon, cat } from "@shared/mascot";
import { $, $$, esc, hotkeyFromEvent, hotkeyText, noteCatList, setNoteCats, setTags, tagColor, tagList, TAG_COLORS, toast } from "@shared/util";
import { armed, hideModal, hooks, S } from "./main-state";
import { busy, dialog, h5, head, hkMsg, lockDialog, note, row, rows, saveSettings, select, tog } from "./settings-ui";
import { backupTime, renderSync, settingsSyncChanged, syncBad, syncClick, syncInput, syncPageHtml } from "./settings-sync";
import { PAGES, ROWS, T, TD, type PageId } from "../text/settings";

const SEARCH_ROWS = rowsOf(ROWS);
const pageName = (p: string) => PAGES.find(([k]) => k === p)?.[2] ?? "";
const pageIcon = (p: string) => (PAGES.find(([k]) => k === p)?.[1] ?? "settings") as IconName;
const isPage = (p: string): p is PageId => PAGES.some(([k]) => k === p);

// ---------- 框：左边标题、搜索、9 页；右边一页 ----------

$('section[data-view="settings"]').innerHTML = `
  <div class="set">
    <nav class="snav" aria-label="${T.title}">
      <h3>${T.title}</h3>
      <label class="field ssearch" title="${T.searchTip}">${icon("search")}<input id="set-q" placeholder="${T.search}" autocomplete="off" spellcheck="false" aria-label="${T.search}"><kbd>Ctrl F</kbd></label>
      ${PAGES.map(([k, ic, n]) => `<button class="sn" data-sec="${k}">${icon(ic as IconName, "s16")}<span>${n}</span>${k === "sync" ? `<i class="dot" title="${T.syncBad}" hidden></i>` : ""}</button>`).join("")}
    </nav>
    <div class="sbody" id="set"></div>
  </div>`;

const body = $("#set");
const qIn = $<HTMLInputElement>("#set-q");

// 打开设置时停在上次看的那页（只记在这台电脑上：存在这个窗口自己的 localStorage 里）
const LAST = "dingmi.settings.page";
try {
  const last = localStorage.getItem(LAST);
  if (last && isPage(last)) S.setSec = last;
} catch {
  /* 存不了就算了 */
}
function rememberPage() {
  try {
    localStorage.setItem(LAST, S.setSec);
  } catch {
    /* 存不了就算了 */
  }
}

/** 0.1 时的旧页名（托盘、桌面组件、横幅里还可能传过来）→ 现在的页和行 */
const OLD: Record<string, [PageId, string]> = {
  look: ["general", "style"],
  keys: ["general", "hk_flash"],
  tags: ["remind", "tags"],
  launcher: ["quick", ""],
  phone: ["sync", "phone"],
};

/** 跳到设置的某一页某一行（托盘、上手清单、引导、横幅里的「去设置」都走这里） */
export function openSettings(page?: string | null, rowId?: string | null) {
  const [p, r] = page && OLD[page] ? OLD[page] : [page, rowId];
  clearSearch();
  if (p && isPage(p)) S.setSec = p;
  S.setRow = rowId || r || "";
  if (S.view === "settings") void renderSettings();
  else hooks.go("settings");
}

/** Ctrl + F：在设置里聚焦搜索框 */
export function focusSettingsSearch() {
  qIn.focus();
  qIn.select();
}

function renderNav() {
  $$(".snav .sn").forEach((b) => b.setAttribute("aria-current", String(!query && b.dataset.sec === S.setSec)));
  const dot = document.querySelector<HTMLElement>(".snav .dot");
  if (dot) dot.hidden = !syncBad(S.sync);
}

$(".snav").addEventListener("click", (e) => {
  const b = (e.target as HTMLElement).closest<HTMLElement>("[data-sec]");
  if (!b) return;
  stopRecording();
  clearSearch();
  S.setSec = b.dataset.sec!;
  S.setRow = "";
  void renderSettings();
});

// ---------- 搜索（需求-0.2 第 6.2 节）：结果列成「页 › 那一行」，点了跳过去、那一行闪一下 ----------

let query = "";
let hits: Found[] = [];
let hitSel = 0;

function clearSearch() {
  query = "";
  qIn.value = "";
}

function renderHits() {
  hits = findRows(query, SEARCH_ROWS);
  hitSel = Math.min(hitSel, Math.max(0, hits.length - 1));
  renderNav();
  body.scrollTop = 0;
  body.innerHTML = hits.length
    ? `<div class="spage"><div class="sres" role="listbox" aria-label="${T.search}">${hits
        .map(
          (f, i) =>
            `<button class="sr${i === hitSel ? " sel" : ""}" role="option" aria-selected="${i === hitSel}" data-hit="${i}" aria-label="${esc(T.hitLabel(pageName(f.row.page), f.row.name))}">${icon(pageIcon(f.row.page), "s16")}<span class="where">${esc(pageName(f.row.page))}</span>${icon("chevron-right", "s16")}<b>${markName(f.row.name, f.marks, esc)}</b>${i === hitSel ? `<kbd>Enter</kbd>` : ""}</button>`,
        )
        .join("")}</div></div>`
    : `<div class="spage"><div class="empty"><span class="cat">${cat("normal")}</span><b>${T.noHit(mascotOn())}</b></div></div>`;
}

function goHit(i: number) {
  const f = hits[i];
  if (!f) return;
  clearSearch();
  openSettings(f.row.page, f.row.id);
}

// 用输入法打字时等字上屏了再搜
qIn.addEventListener("compositionend", () => qIn.dispatchEvent(new Event("input")));
qIn.addEventListener("input", (e) => {
  if ((e as InputEvent).isComposing) return;
  query = qIn.value.trim();
  hitSel = 0;
  if (query) renderHits();
  else void renderSettings();
});
qIn.addEventListener("keydown", (e) => {
  if (e.isComposing) return;
  if (e.key === "Escape" && qIn.value) {
    e.stopPropagation();
    clearSearch();
    void renderSettings();
  } else if (query && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
    e.preventDefault();
    hitSel = (hitSel + (e.key === "ArrowDown" ? 1 : -1) + hits.length) % Math.max(1, hits.length);
    renderHits();
  } else if (query && e.key === "Enter") {
    e.preventDefault();
    goHit(hitSel);
  }
});

/** 跳过来的那一行：滚到能看见，浅橘底闪一下。有几块（备份、手机）是后画的，没找到时等一下再找 */
function flashRow(id: string, tries = 6) {
  const el = body.querySelector<HTMLElement>(`[data-row="${CSS.escape(id)}"]`);
  if (!el) return void (tries && setTimeout(() => flashRow(id, tries - 1), 150));
  el.scrollIntoView({ block: "center" });
  el.classList.add("flash");
  setTimeout(() => el.classList.remove("flash"), 1400);
}

// ---------- 画一页 ----------

let rendering = 0;

async function renderSettings() {
  const s = S.settings;
  if (!s) return;
  if (query) return renderHits();
  const page = (isPage(S.setSec) ? S.setSec : "general") as PageId;
  S.setSec = page;
  rememberPage();
  renderNav();
  const n = ++rendering;
  const html = await PAGE[page](s);
  // 画的时候又换了一页：这次的不要了
  if (n !== rendering || query) return;
  // 同一页重画（存了设置、点了「管理」）时不跳回顶上
  const keep = body.querySelector(`.spage[data-page="${page}"]`) ? body.scrollTop : 0;
  body.innerHTML = `<section class="spage" data-page="${page}">${html}</section>`;
  body.scrollTop = keep;
  if (page === "sync") renderSync();
  if (page === "remind" || page === "notes") renderCats();
  const r = S.setRow;
  S.setRow = "";
  if (r) requestAnimationFrame(() => flashRow(r));
}

const PAGE: Record<PageId, (s: Settings) => string | Promise<string>> = {
  general: generalHtml,
  remind: remindHtml,
  notes: () => `${head(pageName("notes"), T.hint.notes as string)}${h5(ROWS.notes.note_cats.name, { id: "note_cats" })}<div class="srows" id="ncat-rows"></div><p class="fhelp err" id="ncat-msg"></p>
    <div data-row="obsidian">${note("soft", "info", "", esc(T.obsidian))}</div>`,
  quick: quickHtml,
  widget: widgetHtml,
  ai: aiHtml,
  sync: () => syncPageHtml(),
  data: dataHtml,
  about: aboutHtml,
};

// ---------- 通用：开机、快捷键、外观 ----------

const HK: [keyof Settings, keyof typeof ROWS.general][] = [
  ["hk_flash", "hk_flash"],
  ["hk_qnote", "hk_qnote"],
  ["hk_focus", "hk_focus"],
  ["hk_main", "hk_main"],
];
const hkName = (k: keyof Settings) => ROWS.general[k as keyof typeof ROWS.general]?.name ?? "";

function keyBtn(k: keyof Settings) {
  const v = (S.settings?.[k] as string) ?? "";
  const rec = S.recording === k;
  const inner = rec ? T.hkRec : v ? hotkeyText(v).split(" + ").map((x) => `<kbd>${esc(x)}</kbd>`).join("<i>+</i>") : T.hkNone;
  return `<span class="hkw"><button class="key${rec ? " rec" : ""}${v ? "" : " none"}" data-hk-set="${k}" title="${T.hkTip}" aria-label="${esc(hkName(k))}">${inner}</button><button class="btn sm ghost icon" data-hk-clear="${k}" title="${esc(T.hkClear(hkName(k)))}" aria-label="${esc(T.hkClear(hkName(k)))}" ${v && !rec ? "" : "hidden"}>${icon("x")}</button></span>`;
}
const hkRows = () => HK.map(([k, id]) => row(id, ROWS.general[id], keyBtn(k), { local: true })).join("");

/** 风格预览卡片的颜色：底、卡片、主色、字（浅色 / 深色）。要同时画出三种风格，只能照抄 tokens.css 里的值 */
const STYLE_SWATCH: Record<LookStyle, [string, string, string, string][]> = {
  warm: [["#f6f1ec", "#fffcf8", "#f07326", "#2a2421"], ["#17130f", "#231e1a", "#ff8a45", "#f4eee9"]],
  simple: [["#f4f4f5", "#ffffff", "#1d1d1f", "#1d1d1f"], ["#111113", "#1b1b1e", "#ededef", "#ededef"]],
  calm: [["#edf1ee", "#f9fbf9", "#3f7d66", "#24302a"], ["#121614", "#1a201d", "#7fbfa2", "#e6ece8"]],
};

function lookHtml(s: Settings): string {
  const G = ROWS.general;
  const seg = (k: "theme" | "font_size" | "motion", opts: [string, string][]) =>
    `<div class="seg">${opts.map(([v, l]) => `<button data-look="${k}" data-v="${v}" aria-pressed="${s[k] === v}">${l}</button>`).join("")}</div>`;
  const dark = document.documentElement.dataset.mode === "dark" ? 1 : 0;
  const cards = T.styles
    .map(([id, name, sub]) => {
      const [bg, surf, acc, ink] = STYLE_SWATCH[id as LookStyle][dark];
      return `<button class="look-card" data-look="style" data-v="${id}" aria-pressed="${s.style === id}">
      <span class="look-pv" style="background:${bg}"><span style="background:${surf};color:${ink}"><i style="background:${acc}"></i><b style="background:${ink}"></b><b style="background:${ink}"></b></span></span>
      <span class="look-name">${name}</span><small>${sub}</small></button>`;
    })
    .join("");
  const styleName = T.styles.find(([id]) => id === s.style)?.[1] ?? "";
  const catSub = esc(G.mascot.sub!) + (s.mascot === null ? T.mascotFollow(styleName, mascotDefault(s.style)) : T.mascotMine);
  return rows(
    row("style", G.style, "", { local: true, cls: "look-row" }) + `<div class="look-cards">${cards}</div>`,
    row("theme", G.theme, seg("theme", T.themes), { local: true }),
    row("font_size", G.font_size, seg("font_size", T.sizes), { local: true }),
    row("mascot", G.mascot, `<button class="tog" role="switch" data-look="mascot" aria-checked="${mascotOn(s)}" aria-label="${G.mascot.name}"></button>`, { local: true, sub: catSub }),
    row("motion", G.motion, seg("motion", T.motions), { local: true }),
  );
}

async function generalHtml(s: Settings) {
  const G = ROWS.general;
  const notice = await api.hotkeyNotice().catch(() => null);
  return `${head(pageName("general"), T.hint.general as string)}
    ${rows(
      row("autostart", G.autostart, tog("autostart", G.autostart.name), { local: true }),
      row("sound", G.sound, tog("sound", G.sound.name)),
      row("flash_selection", G.flash_selection, tog("flash_selection", G.flash_selection.name), { local: true }),
    )}
    ${h5(T.hkSec)}<div class="srows" id="hk-rows">${hkRows()}</div><p class="fhelp${notice ? " err" : ""}" id="hk-msg">${notice ? esc(notice) : ""}</p>
    ${h5(T.lookSec)}<div id="look-box">${lookHtml(s)}</div>
    ${h5(T.winSec)}${rows(
      row("main_win", G.main_win, `<button class="btn sm" id="main-reset">${T.mainReset}</button>`, { local: true }),
      row("flash_pos", G.flash_pos, `<button class="btn sm" id="q-pos-reset">${T.posReset}</button>`, { local: true }),
    )}`;
}

/** 外观的点击：先换上，再存 */
async function lookClick(b: HTMLElement) {
  const s = S.settings!;
  const k = b.dataset.look!;
  if (k === "style") s.style = b.dataset.v as LookStyle;
  else if (k === "theme") s.theme = b.dataset.v as Settings["theme"];
  else if (k === "font_size") s.font_size = b.dataset.v as Settings["font_size"];
  else if (k === "mascot") s.mascot = !mascotOn(s);
  else if (k === "motion") s.motion = b.dataset.v as Settings["motion"];
  previewLook(s);
  const box = document.querySelector("#look-box");
  // 深浅色换了，预览卡片也要按新的画；等根元素上的 data-mode 换好
  if (box) requestAnimationFrame(() => (box.innerHTML = lookHtml(s)));
  await saveSettings();
}

// ---------- 快捷键：点一下进入录制，按下组合键就换上；Esc 取消，Backspace 清掉 ----------

function redrawKeys() {
  const box = document.querySelector("#hk-rows");
  if (box) box.innerHTML = hkRows();
}

export function stopRecording() {
  if (!S.recording) return;
  S.recording = "";
  redrawKeys();
}

/** 正在录的快捷键录到了一个组合键（键盘事件，或者网页里收不到、窗口转过来的 Alt + 空格） */
async function recordHotkey(combo: string) {
  const key = S.recording;
  if (!key) return;
  const clash = HK.find(([k]) => k !== key && S.settings?.[k] === combo);
  if (clash) {
    const b = document.querySelector(`[data-hk-set="${key}"]`);
    if (b) b.textContent = T.hkClash(hkName(clash[0]));
    return;
  }
  (S.settings as unknown as Record<string, string>)[key] = combo;
  stopRecording();
  hkMsg(T.hkSet(hotkeyText(combo)));
  await saveSettings();
}

/** 正在录快捷键时的按键：录了返回 true（main.ts 的键盘处理先问这里） */
export async function recordKey(e: KeyboardEvent): Promise<boolean> {
  if (!S.recording) return false;
  e.preventDefault();
  e.stopPropagation();
  const key = S.recording;
  if (e.key === "Escape") {
    stopRecording();
    return true;
  }
  if (e.key === "Backspace" || e.key === "Delete") {
    (S.settings as unknown as Record<string, string>)[key] = "";
    stopRecording();
    hkMsg(T.hkCleared);
    await saveSettings();
    return true;
  }
  const hk = hotkeyFromEvent(e);
  if (!hk) return true;
  if (!hk.hasMod) {
    const b = document.querySelector(`[data-hk-set="${key}"]`);
    if (b) b.textContent = T.hkNeedMod;
    return true;
  }
  await recordHotkey(hk.combo);
  return true;
}

// ---------- 提醒 ----------

const WRAP_TIMES = ["16:00", "16:30", "17:00", "17:30", "18:00", "18:30", "19:00", "19:30", "20:00", "20:30", "21:00", "21:30", "22:00"];

function remindHtml() {
  const R = ROWS.remind;
  const look = (id: string) => `<button class="btn sm ghost" id="${id}">${T.look}</button>`;
  return `${head(pageName("remind"), T.hint.remind as string)}
    ${rows(
      row("snooze_min", R.snooze_min, select("snooze_min", R.snooze_min.name, [5, 10, 15, 30, 60].map((m) => [m, T.snoozeOpt(m)]))),
      row("ring_every_min", R.ring_every_min, select("ring_every_min", R.ring_every_min.name, T.ringOpts)),
      row("auto_focus", R.auto_focus, tog("auto_focus", R.auto_focus.name), { local: true }),
    )}
    ${h5(T.cardsSec)}
    ${rows(
      row("morning", R.morning, look("s-morning") + tog("morning", R.morning.name)),
      row("wrap", R.wrap, look("s-wrapnow") + select("wrap_time", T.wrapAt, WRAP_TIMES.map((t) => [t, t])) + tog("wrap", R.wrap.name)),
    )}
    ${h5(R.tags.name, { id: "tags" })}<div class="srows" id="tag-rows"></div><p class="fhelp err" id="tag-msg"></p>`;
}

// ---------- 提醒分类、笔记分类：颜色、名字和条数、换颜色 / 改名 / 删掉；最下面「加一个」 ----------
// 两种管法一样，只是存在设置里不同的地方、用在不同的东西上

const MAX_CATS = 12;
type CatKindKey = "tag" | "ncat";

interface CatKind {
  field: "tags" | "note_cats";
  what: string;
  list: () => TagDef[];
  apply: (list: TagDef[]) => void;
  used: (id: string) => number;
}

const CAT_KINDS: Record<CatKindKey, CatKind> = {
  tag: { field: "tags", what: "提醒", list: tagList, apply: setTags, used: (id) => S.reminders.filter((r) => r.tag === id).length },
  ncat: { field: "note_cats", what: "笔记", list: noteCatList, apply: setNoteCats, used: (id) => S.notes.filter((n) => n.tag === id).length },
};

/** 正在改名字的那个（"tag:id"）、正在加新分类的那一种 */
let catEdit = "";
let catAdding: CatKindKey | "" = "";

function renderCats() {
  (Object.keys(CAT_KINDS) as CatKindKey[]).forEach(renderCatRows);
}

function renderCatRows(k: CatKindKey) {
  const box = document.querySelector<HTMLElement>(`#${k}-rows`);
  if (!box) return;
  const kind = CAT_KINDS[k];
  const list = kind.list();
  const ib = (attr: string, ic: IconName, label: string, cls = "") => `<button class="btn sm ghost icon${cls}" ${attr} title="${label}" aria-label="${label}">${icon(ic)}</button>`;
  box.innerHTML =
    list
      .map((t) => {
        const key = `${k}:${t.id}`;
        const picking = S.tagPick === key;
        const name =
          catEdit === key
            ? `<input class="field" data-tname value="${esc(t.name)}" maxlength="8" aria-label="${T.catName}">`
            : `<span class="nm">${esc(t.name)} <small>${T.catCount(kind.used(t.id))}</small></span>`;
        return `<div class="catrow" data-tid="${esc(t.id)}" data-kind="${k}" style="${tagColor(t.color)}">
          <span class="sw"></span>${name}
          ${ib(`data-tcolor aria-expanded="${picking}"`, "palette", T.catColor)}${ib("data-tren", "pencil", T.catRename)}${ib("data-tdel", "trash-2", T.catDel, " danger")}
        </div>${picking ? `<div class="swatches" data-kind="${k}">${TAG_COLORS.map((c) => `<button data-tpick="${c}" style="${tagColor(c)}" aria-pressed="${c === t.color}" aria-label="${c}"></button>`).join("")}</div>` : ""}`;
      })
      .join("") +
    (list.length >= MAX_CATS
      ? `<div class="catrow"><small>${T.catFull}</small></div>`
      : catAdding === k
        ? `<div class="catrow" data-kind="${k}"><span class="sw add">${icon("plus")}</span><input class="field" id="${k}-new" data-tnew maxlength="8" placeholder="${T.catNew(kind.what)}" aria-label="${T.catNew(kind.what)}"><button class="btn sm pri" data-tadd>${T.catAddOk}</button></div>`
        : `<div class="catrow" data-kind="${k}"><button class="btn sm ghost add" data-tnewopen>${icon("plus")}${T.catAdd}</button></div>`);
  box.querySelector<HTMLInputElement>("[data-tname], [data-tnew]")?.focus();
}

function catMsg(k: CatKindKey, text: string) {
  const m = document.querySelector(`#${k}-msg`);
  if (m) m.textContent = text;
}

/** 分类改了：存设置（几台电脑共用），各处跟着刷新 */
async function saveCats(k: CatKindKey, list: TagDef[]) {
  const kind = CAT_KINDS[k];
  S.settings![kind.field] = list;
  kind.apply(list);
  await saveSettings();
  hooks.renderAll();
  renderCats();
}

const catNameTaken = (k: CatKindKey, name: string, except = "") => CAT_KINDS[k].list().some((t) => t.name === name && t.id !== except);

async function addCat(k: CatKindKey) {
  const inp = $<HTMLInputElement>(`#${k}-new`);
  const name = inp.value.trim();
  if (!name) return inp.focus();
  if (catNameTaken(k, name)) return catMsg(k, T.catTaken(name));
  const list = CAT_KINDS[k].list();
  const usedColors = new Set(list.map((t) => t.color));
  const color = TAG_COLORS.find((c) => !usedColors.has(c)) ?? "gray";
  const id = "t" + Date.now().toString(36) + Math.random().toString(36).slice(2, 5);
  catMsg(k, "");
  await saveCats(k, [...list, { id, name, color }]);
  toast(T.catAdded(name));
}

async function renameCat(inp: HTMLInputElement) {
  const r = inp.closest<HTMLElement>("[data-tid]")!;
  const k = r.dataset.kind as CatKindKey;
  const id = r.dataset.tid!;
  const old = CAT_KINDS[k].list().find((t) => t.id === id)!;
  const name = inp.value.trim();
  catEdit = "";
  if (!name || name === old.name) return renderCats();
  if (catNameTaken(k, name, id)) {
    catMsg(k, T.catTaken(name));
    return renderCats();
  }
  catMsg(k, "");
  await saveCats(k, CAT_KINDS[k].list().map((t) => (t.id === id ? { ...t, name } : t)));
  toast(T.catRenamed);
}

/** 分类那几行里的按钮，处理了返回 true */
async function catClick(el: HTMLElement): Promise<boolean> {
  const k = el.closest<HTMLElement>("[data-kind]")?.dataset.kind as CatKindKey | undefined;
  if (!k) return false;
  const kind = CAT_KINDS[k];
  if (el.closest("[data-tnewopen]")) {
    catAdding = k;
    renderCatRows(k);
    return true;
  }
  if (el.closest("[data-tadd]")) {
    await addCat(k);
    return true;
  }
  const pick = el.closest<HTMLElement>("[data-tpick]");
  if (pick) {
    const id = S.tagPick.slice(k.length + 1);
    S.tagPick = "";
    await saveCats(k, kind.list().map((t) => (t.id === id ? { ...t, color: pick.dataset.tpick! } : t)));
    return true;
  }
  const r = el.closest<HTMLElement>("[data-tid]");
  if (!r) return true;
  const id = r.dataset.tid!;
  const key = `${k}:${id}`;
  if (el.closest("[data-tcolor]")) {
    S.tagPick = S.tagPick === key ? "" : key;
    renderCats();
    return true;
  }
  if (el.closest("[data-tren]")) {
    catEdit = key;
    renderCats();
    return true;
  }
  const del = el.closest<HTMLElement>("[data-tdel]");
  if (!del) return true;
  const t = kind.list().find((x) => x.id === id)!;
  if (!armed(del, T.catDelSure)) {
    catMsg(k, T.catDelAsk(t.name, kind.used(id), kind.what));
    return true;
  }
  catMsg(k, "");
  await saveCats(k, kind.list().filter((x) => x.id !== id));
  await api.clearTag(id, k === "ncat").catch((e) => catMsg(k, T.catClearFail(kind.what, errText(e))));
  toast(T.catDeleted(t.name));
  return true;
}

// ---------- 快捷框（原来叫「启动器」）：能找到哪些、自己加的、别名和不显示的 ----------

const customId = (target: string) => `${/^https?:\/\//i.test(target) ? "url" : "path"}:${target}`;
/** 「管理」点开的那一块 */
let quickOpen: "" | "custom" | "aliases" = "";

async function quickHtml(s: Settings) {
  const Q = ROWS.quick;
  const items = await api.launcherList().catch((): LaunchItem[] => []);
  const c = s.launcher;
  const byId = new Map(items.map((i) => [i.id, i]));
  const del = (attr: string, i: number, label = T.remove, danger = true) => `<button class="btn sm ghost${danger ? " danger" : ""}" ${attr}="${i}">${label}</button>`;
  const none = `<div class="srow sub"><div><small>${T.none}</small></div></div>`;
  const manage = (k: string) => `<button class="btn sm" data-qopen="${k}" aria-expanded="${quickOpen === k}">${quickOpen === k ? T.fold : T.manage}</button>`;
  const custom =
    quickOpen !== "custom"
      ? ""
      : c.custom
          .map((x, i) => {
            const it = byId.get(customId(x.target));
            const ic = iconHtml(it ?? { icon: "", kind: /^https?:\/\//i.test(x.target) ? "link" : "file", name: x.name });
            return `<div class="srow sub lrow">${ic}<div>${esc(x.name)}<small class="path">${esc(x.target)}${it?.missing ? T.gone : ""}</small></div>${del("data-lcdel", i)}</div>`;
          })
          .join("") +
        `<div class="srow sub ldrop" id="l-drop">${icon("download")}<div>${T.dropHere}<small>${T.dropSub}</small></div></div>
        <div class="srow sub"><div></div><button class="btn sm" data-ladd="folder">${icon("folder")}${T.addFolder}</button><button class="btn sm" data-ladd="file">${icon("file-text")}${T.addFile}</button><button class="btn sm" data-ladd="link">${icon("link")}${T.addLink}</button></div>
        <div class="srow sub" id="l-url" hidden><input class="field" id="l-url-u" placeholder="${T.urlPh}" spellcheck="false" aria-label="${T.addLink}"><input class="field" id="l-url-n" placeholder="${T.urlName}" aria-label="${T.urlName}"><button class="btn sm pri" id="l-url-ok">${T.urlOk}</button></div>`;
  const aliases =
    quickOpen !== "aliases"
      ? ""
      : `<div class="srow sub head"><div><b>${T.aliasesH}</b></div></div>${c.aliases.map((a, i) => `<div class="srow sub"><div><kbd>${esc(a.alias)}</kbd> → ${esc(byId.get(a.id)?.name ?? T.noLonger)}</div>${del("data-ladel", i)}</div>`).join("") || none}
        <div class="srow sub head"><div><b>${T.hiddenH}</b></div></div>${c.hidden.map((id, i) => `<div class="srow sub"><div>${esc(byId.get(id)?.name ?? id.replace(/^\w+:/, ""))}</div>${del("data-lhdel", i, T.showAgain, false)}</div>`).join("") || none}`;
  return `${head(pageName("quick"), (T.hint.quick as (hk: string) => string)(hotkeyText(s.hk_flash)))}
    ${rows(
      row("sources", Q.sources, `<button class="btn sm" id="l-rescan">${T.rescan}</button>`, { local: true, sub: esc(T.found(visible(items, c).length)) }),
      ...T.srcs.map(([k, l, sub]) => `<div class="srow sub" data-row="src_${k}"><div>${l}${sub ? `<small>${sub}</small>` : ""}</div><button class="tog" role="switch" data-lsrc="${k}" aria-checked="${!c.off.includes(k)}" aria-label="${l}"></button></div>`),
    )}
    ${rows(
      row("pin_rows", Q.pin_rows, select("pin_rows", Q.pin_rows.name, T.pinRows), { local: true }),
      row("flash_space", Q.flash_space, tog("flash_space", Q.flash_space.name), { local: true, exp: true }),
    )}
    ${rows(
      row("custom", Q.custom, manage("custom"), { local: true, sub: esc(T.customSub(c.custom.length)) }) + custom,
      row("aliases", Q.aliases, manage("aliases"), { local: true, sub: esc(T.aliasSub(c.aliases.length, c.hidden.length)) }) + aliases,
    )}
    ${rows(row("calc", Q.calc, tog("calc", Q.calc.name)))}
    ${h5(Q.web_keys.name, { id: "web_keys" })}<p class="hint">${esc(T.wkHint)}</p>
    <div class="srows" id="wk-rows">${webKeysHtml(s.web_keys)}</div><p class="fhelp err" id="wk-msg"></p>`;
}

// ---------- 网页关键字（需求-0.3 第 10.2 节）：每行 关键字、名字和网址、改；点「改」原地变成三个输入框 ----------

/** 正在改第几条；-1 是正在加新的；null 是没在改 */
let wkEdit: number | null = null;

function webKeysHtml(list: WebKey[]): string {
  const form = (k: WebKey, i: number) => `<div class="srow wkform" data-wkform="${i}">
      <input class="field" id="wk-name" value="${esc(k.name)}" placeholder="${T.wkName}" aria-label="${T.wkName}" maxlength="20">
      <input class="field mono" id="wk-key" value="${esc(k.key)}" placeholder="${T.wkKey}" aria-label="${T.wkKey}" maxlength="16" spellcheck="false">
      <input class="field mono" id="wk-url" value="${esc(k.url)}" placeholder="${T.wkUrl}" aria-label="${T.wkUrl}" spellcheck="false">
      <span class="wkbtns">${i >= 0 ? `<button class="btn sm ghost danger" data-wkdel>${T.remove}</button>` : ""}<button class="btn sm ghost" data-wkcancel>${T.wkCancel}</button><button class="btn sm pri" data-wkok>${T.wkOk}</button></span></div>`;
  return (
    list
      .map((k, i) =>
        wkEdit === i
          ? form(k, i)
          : `<div class="srow cmdrow"><span class="k">${esc(k.key)}</span><div>${esc(k.name)}<small>${esc(k.url)}</small></div><button class="btn sm ghost icon" data-wkedit="${i}" title="${T.wkEdit}" aria-label="${T.wkEdit} ${esc(k.name)}">${icon("pencil")}</button></div>`,
      )
      .join("") +
    (wkEdit === -1 ? form({ name: "", key: "", url: "" }, -1) : `<div class="srow cmdrow"><span></span><div><button class="btn sm ghost" data-wknew>${icon("plus")}${T.wkAdd}</button></div></div>`)
  );
}

function renderWebKeys(focus = true) {
  const box = document.querySelector<HTMLElement>("#wk-rows");
  if (!box || !S.settings) return;
  box.innerHTML = webKeysHtml(S.settings.web_keys);
  if (focus) box.querySelector<HTMLInputElement>(wkEdit === -1 ? "#wk-name" : "#wk-key")?.focus();
}

const wkMsg = (t: string) => {
  const m = document.querySelector<HTMLElement>("#wk-msg");
  if (m) m.textContent = t;
};

async function saveWebKey() {
  const s = S.settings;
  if (!s || wkEdit === null) return;
  const k: WebKey = { name: $<HTMLInputElement>("#wk-name").value.trim(), key: $<HTMLInputElement>("#wk-key").value.trim(), url: $<HTMLInputElement>("#wk-url").value.trim() };
  const bad = checkKey(k, s.web_keys.filter((_, i) => i !== wkEdit));
  if (bad) return wkMsg(T.wkBad[bad]);
  wkMsg("");
  s.web_keys = wkEdit === -1 ? [...s.web_keys, k] : s.web_keys.map((x, i) => (i === wkEdit ? k : x));
  wkEdit = null;
  if (await saveSettings()) toast(T.wkSaved(k.name));
  renderWebKeys(false);
}

/** 网页关键字那几行的按钮，处理了返回 true */
async function webKeyClick(el: HTMLElement): Promise<boolean> {
  const s = S.settings;
  if (!s || !el.closest("#wk-rows")) return false;
  const ed = el.closest<HTMLElement>("[data-wkedit]");
  if (ed || el.closest("[data-wknew]")) {
    wkEdit = ed ? Number(ed.dataset.wkedit) : -1;
    wkMsg("");
    renderWebKeys();
  } else if (el.closest("[data-wkok]")) await saveWebKey();
  else if (el.closest("[data-wkcancel]")) {
    wkEdit = null;
    wkMsg("");
    renderWebKeys(false);
  } else {
    const del = el.closest<HTMLElement>("[data-wkdel]");
    if (del && wkEdit !== null && wkEdit >= 0 && armed(del, T.wkDelSure)) {
      const gone = s.web_keys[wkEdit];
      s.web_keys = s.web_keys.filter((_, i) => i !== wkEdit);
      wkEdit = null;
      if (await saveSettings()) toast(T.wkDeleted(gone.name));
      renderWebKeys(false);
    }
  }
  return true;
}

async function saveLauncher(next: LaunchConfig, done = T.saved) {
  try {
    await api.launcherSave(next);
    S.settings!.launcher = next;
    await renderSettings();
    toast(done);
  } catch (e) {
    toast(T.saveFail(errText(e)));
  }
}

// ---------- 自己加的：从资源管理器、桌面拖进来（快捷方式解开成真实程序，见 launcher.rs resolve_drops） ----------

/** 拖到「自己加的」那一块（展开的拖放区，或者收着时那一行）上 */
const dropZone = (el: EventTarget | null) => (el as HTMLElement | null)?.closest?.('#l-drop, [data-page="quick"] [data-row="custom"]') ?? null;

body.addEventListener("dragover", (e) => {
  if (!dropZone(e.target) || !e.dataTransfer?.types.includes("Files")) return;
  e.preventDefault();
  e.dataTransfer.dropEffect = "copy";
  document.querySelector("#l-drop")?.classList.add("over");
});
body.addEventListener("dragleave", (e) => {
  if (dropZone(e.target) && !dropZone(e.relatedTarget)) document.querySelector("#l-drop")?.classList.remove("over");
});
body.addEventListener("drop", (e) => {
  if (!dropZone(e.target)) return;
  e.preventDefault();
  document.querySelector("#l-drop")?.classList.remove("over");
  const files = e.dataTransfer?.files;
  if (!files?.length) return;
  // 网页拿不到文件的位置：交给 WebView2，程序那边拿到路径再发回来（files-dropped）
  const wv = (window as unknown as { chrome?: { webview?: { postMessageWithAdditionalObjects?: (m: string, o: FileList) => void } } }).chrome?.webview;
  if (!wv?.postMessageWithAdditionalObjects) return toast(T.dropNoPath);
  wv.postMessageWithAdditionalObjects("dingmi-drop:custom", files);
});
listen<{ tag: string; paths: string[] }>("files-dropped", async ({ payload }) => {
  if (payload.tag !== "custom" || !payload.paths.length || !S.settings) return;
  const got = await api.launcherResolve(payload.paths).catch(() => []);
  const c = S.settings.launcher;
  const fresh = got.filter((x, i) => !c.custom.some((y) => y.target === x.target) && got.findIndex((y) => y.target === x.target) === i);
  quickOpen = "custom";
  if (!fresh.length) {
    await renderSettings();
    return toast(T.dropAdded(0, got.length));
  }
  await saveLauncher({ ...c, custom: [...c.custom, ...fresh] }, T.dropAdded(fresh.length, got.length - fresh.length));
});

function addCustom(target: string, name: string) {
  const c = S.settings!.launcher;
  if (c.custom.some((x) => x.target === target)) return toast(T.addedAlready);
  return saveLauncher({ ...c, custom: [...c.custom, { name, target }] }, T.added(name));
}

/** 快捷框那页的按钮，处理了返回 true */
async function quickClick(el: HTMLElement): Promise<boolean> {
  const c = S.settings?.launcher;
  if (!c || !el.closest('[data-page="quick"]')) return false;
  const pick = (attr: string) => el.closest<HTMLElement>(`[${attr}]`)?.getAttribute(attr) ?? null;
  const open = pick("data-qopen");
  if (open) {
    quickOpen = quickOpen === open ? "" : (open as typeof quickOpen);
    await renderSettings();
    return true;
  }
  const src = pick("data-lsrc");
  if (src !== null) {
    const off = c.off.includes(src) ? c.off.filter((x) => x !== src) : [...c.off, src];
    await saveLauncher({ ...c, off });
    return true;
  }
  if (el.closest("#l-rescan")) {
    await api.launcherRescan();
    toast(T.rescanning);
    return true;
  }
  const cdel = pick("data-lcdel");
  if (cdel !== null) {
    const gone = c.custom[Number(cdel)];
    const id = customId(gone.target);
    await saveLauncher({ ...c, custom: c.custom.filter((_, i) => i !== Number(cdel)), pins: c.pins.filter((p) => p !== id) }, T.removed(gone.name));
    return true;
  }
  const adel = pick("data-ladel");
  if (adel !== null) {
    await saveLauncher({ ...c, aliases: c.aliases.filter((_, i) => i !== Number(adel)) });
    return true;
  }
  const hdel = pick("data-lhdel");
  if (hdel !== null) {
    await saveLauncher({ ...c, hidden: c.hidden.filter((_, i) => i !== Number(hdel)) }, T.shownAgain);
    return true;
  }
  const add = pick("data-ladd");
  if (add === "link") {
    $("#l-url").hidden = false;
    $("#l-url-u").focus();
    return true;
  }
  if (add) {
    const p = await api.pickPath(add === "folder");
    if (p) await addCustom(p, p.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || p);
    return true;
  }
  if (el.closest("#l-url-ok")) {
    let u = $<HTMLInputElement>("#l-url-u").value.trim();
    if (!u) return true;
    if (!/^https?:\/\//i.test(u)) u = "https://" + u;
    let name = $<HTMLInputElement>("#l-url-n").value.trim();
    try {
      name ||= new URL(u).hostname.replace(/^www\./, "");
    } catch {
      toast(T.urlBad);
      return true;
    }
    await addCustom(u, name);
    return true;
  }
  return false;
}

// ---------- 桌面组件 ----------

/** 这台电脑能不能开毛玻璃（Windows 11 才有）；还没问过是 null */
let glassOk: boolean | null = null;

async function widgetHtml(s: Settings) {
  const W = ROWS.widget;
  glassOk ??= await api.widgetGlassOk().catch(() => false);
  return `${head(pageName("widget"), T.hint.widget as string)}
    ${rows(
      row("widget", W.widget, tog("widget", W.widget.name), { local: true }),
      glassOk ? row("widget_glass", W.widget_glass, tog("widget_glass", W.widget_glass.name), { local: true }) : "",
      glassOk && s.widget_glass
        ? row("widget_glass_level", W.widget_glass_level, `<input type="range" class="range" min="0" max="100" step="5" data-set="widget_glass_level" data-num value="${s.widget_glass_level}" aria-label="${W.widget_glass_level.name}">`, { local: true })
        : "",
      row("place", W.place, `<button class="btn sm" id="s-whome">${T.placeBtn}</button>`, { local: true }),
    )}
    ${h5(W.parts.name, { local: true, id: "parts" })}
    ${rows(
      ...T.partsList.map(([p, l, sub]) => `<div class="srow" data-row="part_${p}"><div>${l}<small>${sub}</small></div><button class="tog" role="switch" data-wpart="${p}" aria-checked="${!s.widget_hide.includes(p)}" aria-label="${l}"></button></div>`),
    )}`;
}

// ---------- AI ----------

/** 上次测试连接的结果，在下面用绿色 / 红色小条说清楚 */
let aiResult = "";

function aiHtml(s: Settings) {
  const A = ROWS.ai;
  return `${head(pageName("ai"), T.hint.ai as string)}
    ${rows(
      row("ai_key", A.ai_key, `<span class="keyw"><input class="field mono" type="password" id="s-key" data-set="ai_key" value="${esc(s.ai_key)}" placeholder="${T.keyPh}" spellcheck="false" aria-label="${A.ai_key.name}"><button class="btn sm ghost" id="s-eye">${T.show}</button></span>`, {
        sub: esc(A.ai_key.sub!).replace("platform.deepseek.com", `<button class="lnk" id="s-ds">platform.deepseek.com</button>`),
      }),
      row("ai_base", A.ai_base, `<input class="field mono" id="s-base" data-set="ai_base" value="${esc(s.ai_base)}" spellcheck="false" aria-label="${A.ai_base.name}">`),
      row("ai_model", A.ai_model, `<input class="field mono short" id="s-model" data-set="ai_model" value="${esc(s.ai_model)}" spellcheck="false" aria-label="${A.ai_model.name}">`),
      row("ai_test", A.ai_test, `<button class="btn sm" id="s-test">${T.testBtn}</button>`),
      row("sync_ai_key", A.sync_ai_key, tog("sync_ai_key", A.sync_ai_key.name)),
    )}
    <div id="ai-msg">${aiResult}</div>`;
}

async function testAi(b: HTMLButtonElement) {
  const box = $("#ai-msg");
  const s = S.settings!;
  const t0 = performance.now();
  let reply = "";
  const err = await busy(b, T.testing, async () => {
    reply = await api.aiTest(s);
  });
  aiResult = err
    ? note("err", "circle-alert", T.testBad, esc(err))
    : note("ok", "circle-check", T.testOk, esc(T.testOkSub(s.ai_model, ((performance.now() - t0) / 1000).toFixed(1), reply.slice(0, 40))));
  box.innerHTML = aiResult;
}

// ---------- 数据：回收站、导出、恢复、本机自动备份、数据文件 ----------

async function dataHtml() {
  const D = ROWS.data;
  const [path, local] = await Promise.all([api.dataPath().catch(() => ""), api.localBackups().catch(() => null)]);
  const trashN = S.trash.reminders.length + S.trash.notes.length;
  return `${head(pageName("data"), T.hint.data as string)}
    ${rows(
      row("trash", D.trash, `<button class="btn sm" id="s-trash">${T.open}</button>`, { sub: esc(T.trashSub(trashN)) }),
      row("export", D.export, `<button class="btn sm" id="s-export">${T.exportBtn}</button>`),
      row("restore", D.restore, `<button class="btn sm" id="s-restore">${T.restoreBtn}</button>`),
      row("autobackup", D.autobackup, local ? `<button class="btn sm ghost" id="s-bkdir" data-dir="${esc(local.dir)}">${icon("folder-open")}${T.openDir}</button>` : "", {
        sub: esc(local?.last ? T.autoSub(backupTime(local.last)) : T.autoNone),
      }),
      row("path", D.path, `<button class="btn sm ghost" id="s-path" data-path="${esc(path)}">${T.open}</button>`, { sub: `<span class="path">${esc(path)}</span>` }),
    )}`;
}

/** 导出全部：先说清楚里面是明文、存到哪；导出时显示进度，导完可以打开所在文件夹 */
async function showExport() {
  let path = await api.exportDefault().catch(() => "");
  dialog(`<h4>${TD.exTitle}</h4><p>${TD.exWhy}</p>${note("warn", "triangle-alert", "", esc(TD.exWarn))}
    <div class="fl"><span>${TD.exTo}</span><div class="ex-path"><span class="path" id="ex-path">${esc(path)}</span><button type="button" class="btn sm ghost" id="ex-pick">${TD.exPick}</button></div></div>
    <div class="fl" id="ex-prog" hidden><i class="prog"><i id="ex-bar" style="width:0"></i></i><small class="fhelp" id="ex-step"></small></div>
    <p class="fhelp err" id="ex-err"></p>
    <div class="acts"><button type="button" class="btn" id="ex-cancel">${T.cancel}</button><button type="button" class="btn pri" id="ex-go">${TD.exGo}</button></div>`);
  const btns = () => $$<HTMLButtonElement>("#ex-pick, #ex-cancel, #ex-go");
  $("#ex-cancel").onclick = hideModal;
  $("#ex-pick").onclick = async () => {
    const p = await api.exportPick(path).catch(() => null);
    if (p) $("#ex-path").textContent = path = p;
  };
  $("#ex-go").onclick = async () => {
    lockDialog(true);
    btns().forEach((b) => (b.disabled = true));
    $("#ex-err").textContent = "";
    $("#ex-prog").hidden = false;
    $("#ex-step").textContent = TD.exPrep;
    const off = await listen<ExportProgress>("export-progress", ({ payload: p }) => {
      const bar = document.querySelector<HTMLElement>("#ex-bar");
      const step = document.querySelector<HTMLElement>("#ex-step");
      if (!bar || !step) return;
      bar.style.width = `${p.total ? Math.round((p.done / p.total) * 100) : 0}%`;
      step.textContent = p.stage === "download" ? TD.exDl(p.done + 1, p.total) : TD.exPack(p.done, p.total);
    });
    try {
      showExported(await api.exportAll(path));
    } catch (e) {
      lockDialog(false);
      $("#ex-prog").hidden = true;
      $("#ex-err").textContent = TD.exFail(errText(e));
      btns().forEach((b) => (b.disabled = false));
    } finally {
      off();
    }
  };
}

function showExported(r: Exported) {
  const n = r.missing.length;
  const some = r.missing.slice(0, 3).join("、") + (n > 3 ? " 等" : "");
  dialog(`<h4>${TD.exDone}</h4><p>${TD.exCount(r.reminders, r.notes, r.attachments)}</p><p class="path">${esc(r.path)}</p>
    ${n ? note("warn", "triangle-alert", "", esc(TD.exMissing(n, some))) : ""}
    <div class="acts"><button type="button" class="btn" id="ex-open">${icon("folder-open")}${TD.exOpen}</button><button type="button" class="btn pri" data-close>${T.done}</button></div>`);
  $("#ex-open").onclick = () => api.revealPath(r.path).catch((e) => toast(errText(e)));
}

/** 从导出包恢复：选包 → 看看里面有什么、选合并还是替换 → 恢复 */
async function pickRestore() {
  const path = await api.restorePick().catch(() => null);
  if (!path) return;
  let p: RestorePeek;
  try {
    p = await api.restorePeek(path);
  } catch (e) {
    dialog(`<h4>${TD.rsTitle}</h4>${note("err", "circle-alert", "", esc(errText(e)))}<p class="path">${esc(path)}</p>
      <div class="acts"><button type="button" class="btn" id="rs-again">${TD.rsAgain}</button><button type="button" class="btn pri" data-close>${T.done}</button></div>`);
    $("#rs-again").onclick = () => void pickRestore();
    return;
  }
  const opt = (mode: "merge" | "replace", on: boolean) => {
    const [name, , sub] = mode === "merge" ? TD.rsMerge : TD.rsReplace;
    return `<button type="button" class="choice" data-rmode="${mode}" aria-pressed="${on}" ${mode === "replace" && p.sync_on ? "disabled" : ""}><span class="radio${on ? " on" : ""}"></span><div>${name}<small>${sub}</small></div></button>`;
  };
  dialog(
    `<h4>${TD.rsTitle}</h4><p><b>${esc(TD.rsAt(p.exported_at.slice(0, 10), p.reminders, p.notes, p.attachments))}</b></p><p class="path">${esc(path)}</p>
    <div class="fl"><span>${TD.rsHow}</span><div class="srows" id="rs-mode">${opt("merge", true)}${opt("replace", false)}</div></div>
    ${p.sync_on ? note("warn", "triangle-alert", "", esc(TD.rsSyncOn)) : ""}
    <p class="fhelp">${TD.rsMdNote}</p><p class="fhelp err" id="rs-err"></p>
    <div class="acts"><button type="button" class="btn" id="rs-cancel">${T.cancel}</button><button type="button" class="btn pri" id="rs-go">${TD.rsMerge[1]}</button></div>`,
    { wide: true },
  );
  let replace = false;
  $("#rs-cancel").onclick = hideModal;
  $("#rs-mode").onclick = (e) => {
    const c = (e.target as HTMLElement).closest<HTMLButtonElement>("[data-rmode]");
    if (!c || c.disabled) return;
    replace = c.dataset.rmode === "replace";
    $$("#rs-mode [data-rmode]").forEach((x) => {
      x.setAttribute("aria-pressed", String(x === c));
      x.querySelector(".radio")?.classList.toggle("on", x === c);
    });
    $("#rs-go").textContent = (replace ? TD.rsReplace : TD.rsMerge)[1];
  };
  $("#rs-go").onclick = async () => {
    const go = $<HTMLButtonElement>("#rs-go");
    const label = go.textContent;
    const ctl = () => $$<HTMLButtonElement>("#rs-cancel, #rs-go, #rs-mode [data-rmode]");
    lockDialog(true);
    ctl().forEach((b) => (b.disabled = true));
    go.textContent = TD.rsBusy;
    $("#rs-err").textContent = "";
    try {
      showRestored(await api.restoreApply(path, replace));
    } catch (e) {
      lockDialog(false);
      $("#rs-err").textContent = errText(e);
      go.textContent = label;
      ctl().forEach((b) => (b.disabled = b.dataset.rmode === "replace" && p.sync_on));
    }
  };
}

function showRestored(r: Restored) {
  const n = r.reminders + r.notes + r.attachments;
  dialog(`<h4>${TD.rsDone}</h4><p>${n ? TD.rsGot(!!r.backup, r.reminders, r.notes, r.attachments) : TD.rsNothing}</p>
    ${r.backup ? `<p>${TD.rsBackup}</p><p class="path">${esc(r.backup)}</p>` : ""}
    <div class="acts"><button type="button" class="btn pri" data-close>${T.done}</button></div>`);
}

// ---------- 关于 ----------

export const TOO_OLD = T.tooOld;
const mb = (n: number) => `${(n / 1048576).toFixed(1)} MB`;
export const pct = (u: UpdateStatus) => (u.size ? Math.floor((u.got / u.size) * 100) : 0);

/** 检查更新那一行：右边的按钮跟着情况变 */
function updateRows() {
  const u = S.update;
  const A = ROWS.about;
  if (!u) return row("update", A.update);
  const v = u.version ?? "";
  const line =
    u.checking ? T.checking
    : u.state === "downloading" ? T.downloading(v, pct(u))
    : u.state === "ready" ? T.ready(v)
    : u.error ? u.error
    : u.state === "available" ? T.available(v, u.size ? mb(u.size) : "")
    : u.checked_at ? T.latest(u.checked_at.slice(5, 16))
    : T.neverChecked;
  const btn =
    u.state === "ready" ? `<button class="btn sm pri" data-upd="install">${T.restartBtn}</button>`
    : u.state === "available" ? `<button class="btn sm pri" data-upd="download">${T.updateBtn}</button>`
    : `<button class="btn sm" data-upd="check" ${u.checking || u.state === "downloading" ? "disabled" : ""}>${T.checkBtn}</button>`;
  return (
    row("update", A.update, (u.version && u.notes ? `<button class="btn sm ghost" data-upd="notes">${T.whatsNew}</button>` : "") + btn, {
      sub: esc(line) + (u.too_old ? `<span class="bad">${esc(T.tooOld)}</span>` : ""),
    }) + row("auto_update", A.auto_update, tog("auto_update", A.auto_update.name), { local: true })
  );
}

/** 「关于」那页的检查更新跟着变 */
export function renderAboutRows() {
  const box = document.querySelector("#about-upd");
  if (box) box.innerHTML = updateRows();
}

let checklistHidden: boolean | null = null;

/** 协议那一行：30 天内更新过的，小字前面写「M 月 D 日更新」（橘色，需求-1.x 第 36.6 节） */
function termsSub(t: { sub?: string }, d: TermsDoc | undefined): string | undefined {
  if (!d?.recent) return undefined;
  return `<span class="upd">${esc(T.termsUpdated(d.recent))}</span>${t.sub ? ` · ${esc(t.sub)}` : ""}`;
}

async function aboutHtml() {
  const A = ROWS.about;
  const [v, cl, tv] = await Promise.all([
    getVersion().catch(() => S.update?.current ?? ""),
    api.checklistGet().catch((): Checklist | null => null),
    api.termsInfo().catch((): TermsView | null => null),
  ]);
  checklistHidden = cl?.hidden ?? null;
  return `<div class="about-top">${appIcon(56)}<div><h4>${esc(T.version(v))}</h4><p class="hint">${T.slogan}</p></div></div>
    <div class="srows" id="about-upd">${updateRows()}</div>
    ${h5(T.helpSec)}
    ${rows(
      row("onboard", A.onboard, `<button class="btn sm ghost" id="s-onboard">${T.look}</button>`),
      row("docs", A.docs, `<button class="btn sm ghost" data-site="/docs/">${T.open}</button>`),
      row("faq", A.faq, `<button class="btn sm ghost" data-site="/faq/">${T.open}</button>`),
      row("checklist", A.checklist, checklistHidden === false ? "" : `<button class="btn sm ghost" id="s-checklist">${T.findBack}</button>`, {
        local: true,
        sub: esc(checklistHidden === false ? T.checklistOn : T.checklistOff),
      }),
    )}
    ${h5(T.moreSec)}
    ${rows(
      row("terms", A.terms, `<button class="btn sm ghost" data-site="/terms/">${T.open}</button>`, { sub: termsSub(A.terms, tv?.terms) }),
      row("sync_terms", A.sync_terms, `<button class="btn sm ghost" data-site="/sync/terms/">${T.open}</button>`, { sub: termsSub(A.sync_terms, tv?.sync_terms) }),
      row("privacy", A.privacy, `<button class="btn sm ghost" data-site="/privacy/">${T.open}</button>`, { sub: termsSub(A.privacy, tv?.privacy) }),
      row("contact", A.contact, `<button class="btn sm ghost" data-site="/contact/">${T.open}</button>`),
      row("diag", A.diag, `<button class="btn sm" id="s-diag">${T.exportBtn}</button>`),
      row("licenses", A.licenses, `<button class="btn sm ghost" id="s-lic">${T.look}</button>`),
    )}`;
}

async function installUpdate() {
  try {
    await api.updateInstall();
  } catch (e) {
    toast(errText(e));
  }
}

/** 点「更新」：下载好了就重启更新，不然开始下载（进度在顶上那一条里） */
export function startUpdate() {
  if (S.update?.state === "ready") return installUpdate();
  api.updateDownload().catch((e) => toast(errText(e)));
}

/** 看看更新了什么 */
export function showUpdateNotes() {
  const u = S.update;
  if (!u?.version) return;
  const can = u.state === "available" || u.state === "ready";
  dialog(
    `<h4>${esc(T.notesTitle(u.version))}${u.date ? ` <small class="fhelp">${esc(u.date)}</small>` : ""}</h4>
    <p class="notes">${esc(u.notes || T.noNotes)}</p>
    <div class="acts">${u.size ? `<span class="fhelp sp">${T.pkgSize(mb(u.size))}</span>` : ""}<button type="button" class="btn" data-close>${T.later}</button>${can ? `<button type="button" class="btn pri" id="un-go">${u.state === "ready" ? T.restartBtn : T.updateBtn}</button>` : ""}</div>`,
    { wide: true },
  );
  const go = document.querySelector<HTMLElement>("#un-go");
  if (go)
    go.onclick = () => {
      hideModal();
      startUpdate();
    };
}

/** 导出诊断信息：列出会放进去的几样（可以一个个打开看看），确认了再导出到桌面 */
async function showDiag(b: HTMLButtonElement) {
  let parts: DiagPart[] = [];
  const bad = await busy(b, TD.dgPrep, async () => (parts = await api.diagPrepare()));
  if (bad) return toast(TD.dgFail(bad));
  dialog(
    `<h4>${TD.dgTitle}</h4><p>${TD.dgWhy}</p>
    <div class="srows">${parts.map((p, i) => `<div class="srow"><div>${esc(p.name)}<small>${esc(p.desc)}</small></div><button type="button" class="btn sm ghost" data-dglook="${i}">${TD.dgLook}</button></div>`).join("")}</div>
    <p class="fhelp err" id="dg-err"></p>
    <div class="acts"><button type="button" class="btn" data-close>${T.cancel}</button><button type="button" class="btn pri" id="dg-go">${TD.dgGo}</button></div>`,
    { wide: true },
  );
  // 打不开时写在弹窗里（小提示会被弹窗挡住）
  $$<HTMLButtonElement>("[data-dglook]").forEach(
    (x) => (x.onclick = () => api.openPath(parts[Number(x.dataset.dglook)].path).catch((e) => ($("#dg-err").textContent = errText(e)))),
  );
  $<HTMLButtonElement>("#dg-go").onclick = async () => {
    let path = "";
    const err = await busy($<HTMLButtonElement>("#dg-go"), TD.dgPrep, async () => (path = await api.diagExport()));
    if (err) return void ($("#dg-err").textContent = TD.dgFail(err));
    // 反馈渠道建好后（需求-0.4 第 16.4 节）在这里写上发到哪里
    dialog(`<h4>${TD.dgDone}</h4><p class="path">${esc(path)}</p>
      <div class="acts"><button type="button" class="btn" id="dg-open">${icon("folder-open")}${TD.exOpen}</button><button type="button" class="btn pri" data-close>${T.done}</button></div>`);
    $("#dg-open").onclick = () => api.revealPath(path).catch((e) => toast(errText(e)));
  };
}

function showLicenses() {
  dialog(`<h4>${T.licTitle}</h4><p>${T.licIntro}</p>
    <div class="srows">${T.lics.map(([n, l]) => `<div class="srow"><div>${esc(n)}</div><span class="fhelp">${esc(l)}</span></div>`).join("")}</div>
    <div class="acts"><button type="button" class="btn pri" data-close>${T.close}</button></div>`);
}

// ---------- 点了什么 ----------

body.addEventListener("click", async (e) => {
  const el = e.target as HTMLElement;
  const s = S.settings;
  if (!s) return;
  const hit = el.closest<HTMLElement>("[data-hit]");
  if (hit) return goHit(Number(hit.dataset.hit));
  // 通用 → 窗口：主窗口恢复默认大小、快捷框放回原位
  if (el.closest("#main-reset")) {
    await api.mainReset();
    return toast(T.mainResetDone);
  }
  if (el.closest("#q-pos-reset")) {
    await api.flashPosReset();
    return toast(T.posResetDone);
  }
  if (el.closest("[data-godiag]")) return openSettings("about", "diag");
  if (await syncClick(el)) return;
  if (await webKeyClick(el)) return;
  if (await quickClick(el)) return;
  if (await catClick(el)) return;
  const look = el.closest<HTMLElement>("[data-look]");
  if (look) return lookClick(look);
  const sw = el.closest<HTMLElement>("button[data-set]");
  if (sw) {
    const k = sw.dataset.set as keyof Settings;
    const v = sw.getAttribute("aria-checked") !== "true";
    sw.setAttribute("aria-checked", String(v));
    (s as unknown as Record<string, unknown>)[k] = v;
    if (await saveSettings()) toast(T.saved);
    // 开了毛玻璃才有「深浅」那一行
    if (k === "widget_glass") await renderSettings();
    return;
  }
  const part = el.closest<HTMLElement>("button[data-wpart]");
  if (part) {
    const p = part.dataset.wpart!;
    const on = part.getAttribute("aria-checked") !== "true";
    part.setAttribute("aria-checked", String(on));
    s.widget_hide = on ? s.widget_hide.filter((x) => x !== p) : [...s.widget_hide, p];
    if (await saveSettings()) toast(T.saved);
    return;
  }
  const upd = el.closest<HTMLElement>("[data-upd]")?.dataset.upd;
  if (upd === "install" || upd === "download") return startUpdate();
  if (upd === "notes") return showUpdateNotes();
  if (upd === "check") {
    try {
      S.update = await api.updateCheck();
      hooks.renderUpdate();
      if (S.update.state === "idle") toast(T.isLatest);
      else if (S.update.state === "available") toast(T.hasNew(S.update.version ?? ""));
    } catch (err) {
      toast(errText(err));
    }
    return;
  }
  const clear = el.closest<HTMLElement>("[data-hk-clear]");
  if (clear) {
    stopRecording();
    (s as unknown as Record<string, string>)[clear.dataset.hkClear!] = "";
    redrawKeys();
    hkMsg(T.hkCleared);
    await saveSettings();
    return;
  }
  const hk = el.closest<HTMLElement>("[data-hk-set]");
  if (hk) {
    const k = hk.dataset.hkSet as keyof Settings;
    const again = S.recording === k;
    stopRecording();
    if (!again) {
      S.recording = k;
      redrawKeys();
      document.querySelector<HTMLElement>(`[data-hk-set="${k}"]`)?.focus();
    }
    return;
  }
  const b = el.closest<HTMLButtonElement>("button");
  // 关于里的官网链接：使用说明、常见问题、隐私说明、用户协议、叮咪同步服务协议、联系我们（网址照 UI设计.md U16、需求-1.0 第 32.1 节）
  const site = b?.closest<HTMLElement>("[data-site]")?.dataset.site;
  if (site) return void api.openUrl(`https://dingmi.app${site}`).catch((err) => toast(errText(err)));
  switch (b?.id) {
    case "s-whome":
      await api.widgetPlace(true);
      return toast(T.placed(s.widget));
    case "s-eye": {
      const inp = $<HTMLInputElement>("#s-key");
      inp.type = inp.type === "password" ? "text" : "password";
      b.textContent = inp.type === "password" ? T.show : T.hide;
      return;
    }
    case "s-ds":
      return void api.openUrl("https://platform.deepseek.com").catch((err) => toast(errText(err)));
    case "s-test":
      return testAi(b);
    case "s-morning":
      return void api.openWindow("morning");
    case "s-wrapnow":
      return void api.openWindow("wrap");
    case "s-trash":
      return hooks.go("trash");
    case "s-export":
      return void showExport();
    case "s-restore":
      return void pickRestore();
    case "s-bkdir":
      return void api.openPath(b.dataset.dir ?? "").catch((err) => toast(errText(err)));
    case "s-path":
      return void api.revealPath(b.dataset.path ?? "").catch((err) => toast(errText(err)));
    case "s-onboard":
      return void api.onboardOpen().catch((err) => toast(errText(err)));
    case "s-lic":
      return showLicenses();
    case "s-diag":
      return void showDiag(el.closest<HTMLButtonElement>("#s-diag")!);
    case "s-checklist": {
      const c = await api.checklistHide(false).catch(() => null);
      toast(!c ? T.checklistFail : c.hidden ? T.checklistDone : T.checklistBack);
      if (c && !c.hidden) await renderSettings();
      return;
    }
  }
});

body.addEventListener("input", (e) => syncInput(e.target as HTMLElement));

body.addEventListener("change", async (e) => {
  const el = e.target as HTMLInputElement | HTMLSelectElement;
  const k = el.dataset.set as keyof Settings | undefined;
  if (!k || !S.settings) return;
  (S.settings as unknown as Record<string, unknown>)[k] = "num" in el.dataset ? Number(el.value) : el.value.trim();
  if (await saveSettings()) toast(T.saved);
});

body.addEventListener("keydown", (e) => {
  const el = e.target as HTMLElement;
  if (e.isComposing) return;
  if (el.matches("[data-tname]")) {
    if (e.key === "Enter") void renameCat(el as HTMLInputElement);
    else if (e.key === "Escape") {
      e.stopPropagation();
      catEdit = "";
      renderCats();
    }
  } else if (el.closest("[data-wkform]")) {
    if (e.key === "Enter") void saveWebKey();
    else if (e.key === "Escape") {
      e.stopPropagation();
      wkEdit = null;
      wkMsg("");
      renderWebKeys(false);
    }
  } else if (el.matches("[data-tnew]")) {
    const k = el.closest<HTMLElement>("[data-kind]")!.dataset.kind as CatKindKey;
    if (e.key === "Enter") void addCat(k);
    else if (e.key === "Escape") {
      e.stopPropagation();
      catAdding = "";
      renderCatRows(k);
    }
  }
});

// 改名字时点到别处：照样存
body.addEventListener("focusout", (e) => {
  const el = e.target as HTMLElement;
  if (el.matches("[data-tname]") && el.isConnected) void renameCat(el as HTMLInputElement);
});

// ---------- 别处改了 ----------

/** 同步状态变了：导航上的小红点；同步页开着时重画 */
export function settingsOnSync(before: SyncStatus["state"] | undefined, now: SyncStatus) {
  renderNav();
  if (S.view === "settings" && S.setSec === "sync" && !query) settingsSyncChanged(before, now);
}

/** 设置在别处改了（比如桌面组件上点了「关掉」、另一台电脑改了分类） */
export function settingsOnChange(widgetFlip: boolean) {
  if (S.view !== "settings" || query) return;
  if (widgetFlip && S.setSec === "widget") return void renderSettings();
  // 正在改分类名字时不重画，免得打断输入
  if (!catEdit && !catAdding) renderCats();
}

// Alt + 空格网页里收不到，窗口转过来（sys.rs no_alt_menu）
listen<string>("alt-space", (e) => void recordHotkey(e.payload));
// 快捷框能找到的东西变了（装了新程序、自己加的找好了图标）
listen("launcher-changed", () => {
  if (S.view === "settings" && S.setSec === "quick" && !query) void renderSettings();
});

export { renderSettings };
