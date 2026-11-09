// 桌面组件上的「常用」：放程序、文件夹、文件、网址，点一下就打开；文件夹点小箭头看里面最新的几个文件。
// 放什么记在设置里（Settings.launcher.pins，只属于这台电脑），快捷框空着时那一排也先显示这些；和快捷框共用一份列表（launch.ts）。
// 页面上要有 #sec-pins，里面放 #pins、#pins-edit、#pin-msg、#pin-pop、#drophint（见 widget.html）
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { api, errText, type LaunchConfig, type LaunchItem, type LaunchStats } from "./api";
import { EMPTY_STATS, iconHtml, kindText, markName, rank, shortName, visible } from "./launch";
import { P } from "../text/widget";
import { icon, type IconName } from "@shared/icons";
import { $, ago, dayStr, esc } from "@shared/util";

/** 一行 6 个，最多两行（U8） */
const MAX_PINS = 12;

let items: LaunchItem[] = [];
let cfg: LaunchConfig = { pins: [], aliases: [], custom: [], hidden: [], off: [] };
let stats: LaunchStats = EMPTY_STATS;
let editing = false;
/** 下面展开的是什么：某个文件夹里的文件、添加、加网址 */
let pop: { kind: "stack"; id: string } | { kind: "add" } | { kind: "url" } | null = null;

const pinsEl = () => $("#pins");
const byId = (id: string) => items.find((i) => i.id === id);

export function setPinsConfig(c: LaunchConfig) {
  cfg = c;
  render();
}

/** 开始编辑（⋯ 菜单里的「编辑常用」）：能拿掉、拖动换顺序、添加 */
export function editPins() {
  editing = true;
  closePop();
  render();
}

// ---------- 画 ----------

function render() {
  const list = cfg.pins.map(byId).filter((x): x is LaunchItem => !!x);
  // 平时只摆放上来的；编辑时（或者一个都还没有时）最后一格是「添加」
  const edit = $("#pins-edit");
  edit.textContent = P.edit;
  edit.hidden = !editing;
  pinsEl().classList.toggle("editing", editing);
  pinsEl().innerHTML =
    list
      .map(
        (it) => `<button class="pin${it.missing ? " gone" : ""}" data-pin="${esc(it.id)}" title="${esc(it.name)}${it.path ? "\n" + esc(it.path) : ""}"><span class="a">${iconHtml(it)}${
          it.kind === "folder" ? `<span class="fo" data-stack="${esc(it.id)}" title="${P.seeFiles}">${icon("chevron-down")}</span>` : ""
        }<span class="x" data-unpin="${esc(it.id)}" title="${P.unpin}">${icon("x")}</span></span><span class="nm">${esc(shortName(it.name))}</span></button>`,
      )
      .join("") +
    ((editing || !list.length) && cfg.pins.length < MAX_PINS
      ? `<button class="pin add" data-add title="${P.addTitle}"><span class="a"><span class="lic">${icon("plus")}</span></span><span class="nm">${P.add}</span></button>`
      : "");
  if (pop?.kind === "stack" && !byId(pop.id)) closePop();
}

function msg(text: string, bad = false) {
  const m = $("#pin-msg");
  m.className = `pin-msg${bad ? " bad" : ""}`;
  m.textContent = text;
  m.hidden = !text;
  clearTimeout(msg.t);
  if (text) msg.t = window.setTimeout(() => (m.hidden = true), 3500);
}
msg.t = 0;

function closePop() {
  pop = null;
  $("#pin-pop").hidden = true;
  $("#pin-pop").innerHTML = "";
  for (const b of pinsEl().querySelectorAll(".open")) b.classList.remove("open");
}

/** 修改时间：今天 14:02 / 昨天 / 10月28日 */
function when(ms: number) {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  return ago(`${dayStr(d)} ${p(d.getHours())}:${p(d.getMinutes())}`);
}

/** 文件夹里的文件：按扩展名挑个图标 */
function fileIcon(name: string, dir: boolean): IconName {
  if (dir) return "folder";
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  if (["png", "jpg", "jpeg", "gif", "webp", "bmp", "heic", "svg"].includes(ext)) return "image";
  if (["xls", "xlsx", "csv", "et"].includes(ext)) return "sheet";
  return "file-text";
}

async function showStack(id: string, btn: HTMLElement) {
  const it = byId(id);
  if (!it) return;
  if (pop?.kind === "stack" && pop.id === id) return closePop();
  closePop();
  pop = { kind: "stack", id };
  btn.classList.add("open");
  const box = $("#pin-pop");
  try {
    const p = await api.folderPeek(it.path);
    if (pop?.kind !== "stack" || pop.id !== id) return;
    const more = p.total - p.files.length;
    box.innerHTML = `${p.files.map((f) => `<button class="fr" data-file="${esc(f.path)}" title="${esc(f.name)}">${icon(fileIcon(f.name, f.dir))}<span class="fn">${esc(f.name)}</span><small class="num">${when(f.modified)}</small></button>`).join("")}
      ${!p.total ? `<div class="pp-more">${P.emptyDir}</div>` : more > 0 ? `<div class="pp-more">${P.moreFiles(more)}</div>` : ""}
      <button class="fr" data-opendir="${esc(id)}">${icon("folder-open")}<span class="fn">${esc(P.openDir(shortName(it.name)))}</span></button>`;
  } catch (err) {
    box.innerHTML = `<div class="pp-more bad">${esc(errText(err))}</div>`;
  }
  box.hidden = false;
}

async function showAdd() {
  if (pop?.kind === "add") return closePop();
  closePop();
  pop = { kind: "add" };
  stats = await api.launcherStats().catch(() => EMPTY_STATS);
  const box = $("#pin-pop");
  box.innerHTML = `<label class="field pp-q">${icon("search")}<input id="pp-q" placeholder="${P.search}" autocomplete="off" spellcheck="false" aria-label="${P.searchLabel}"></label>
    <div class="pp-list" id="pp-list"></div>
    <div class="pp-acts"><button class="btn sm" data-addk="folder">${P.pickFolder}</button><button class="btn sm" data-addk="file">${P.pickFile}</button><button class="btn sm" data-addk="link">${P.addUrl}</button></div>
    <div class="pp-more">${P.dragTip}</div>`;
  box.hidden = false;
  fillPicks();
  $("#pp-q").focus();
}

function fillPicks() {
  const q = $<HTMLInputElement>("#pp-q").value.trim();
  const pool = visible(items, cfg).filter((i) => !cfg.pins.includes(i.id));
  const list = q
    ? rank(q, pool, cfg, stats, 6).map((h) => h.it)
    : [...pool].sort((a, b) => (stats.n[b.id] ?? 0) + b.runs / 4 - (stats.n[a.id] ?? 0) - a.runs / 4 || a.name.localeCompare(b.name, "zh")).slice(0, 6);
  $("#pp-list").innerHTML = list.length
    ? list.map((it, i) => `<button class="fr${q && i === 0 ? " on" : ""}" data-pick="${esc(it.id)}">${iconHtml(it)}<span class="fn">${markName(it.name, q)}</span><small>${kindText(it)}</small></button>`).join("")
    : `<div class="pp-more">${P.noMatch}</div>`;
}

function showUrl() {
  pop = { kind: "url" };
  $("#pin-pop").innerHTML = `<form class="pp-url" id="pp-url">
    <input class="field" name="u" placeholder="${P.urlPh}" autocomplete="off" spellcheck="false" aria-label="${P.urlLabel}">
    <input class="field" name="n" placeholder="${P.namePh}" autocomplete="off" aria-label="${P.nameLabel}">
    <div class="pp-acts"><button class="btn sm pri">${P.urlAdd}</button><button type="button" class="btn sm" data-cancel>${P.cancel}</button></div></form>`;
  $<HTMLInputElement>("#pp-url input[name=u]").focus();
}

// ---------- 改 ----------

async function save(next: LaunchConfig, done = "") {
  try {
    await api.launcherSave(next);
    cfg = next;
    render();
    if (done) msg(done);
  } catch (err) {
    msg(P.saveFail(errText(err)), true);
  }
}

function addPin(id: string, name: string) {
  closePop();
  if (cfg.pins.includes(id)) return msg(P.already(name));
  if (cfg.pins.length >= MAX_PINS) return msg(P.full(MAX_PINS), true);
  return save({ ...cfg, pins: [...cfg.pins, id] }, P.added(name));
}

/** 自己加的文件、文件夹、网址：先加进快捷框能搜到的（找好图标后列表会更新），再放上来 */
function addTargets(list: { target: string; name: string }[]) {
  closePop();
  let custom = cfg.custom;
  const pins = [...cfg.pins];
  for (const { target, name } of list) {
    if (pins.length >= MAX_PINS) break;
    if (!custom.some((c) => c.target === target)) custom = [...custom, { name, target }];
    const id = `${/^https?:\/\//.test(target) ? "url" : "path"}:${target}`;
    if (!pins.includes(id)) pins.push(id);
  }
  const names = list.map((x) => x.name).join("、");
  return save({ ...cfg, custom, pins }, pins.length >= MAX_PINS && list.length > 1 ? P.fullNow(MAX_PINS) : P.added(names));
}

const baseName = (p: string) => p.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || p;

async function pick(folder: boolean) {
  const p = await api.pickPath(folder);
  if (p) addTargets([{ target: p, name: baseName(p) }]);
}

// ---------- 点、拖 ----------

async function onClick(e: MouseEvent) {
  const el = e.target as HTMLElement;
  if (el.closest("#pins-edit")) {
    editing = false;
    closePop();
    render();
    return;
  }
  const un = el.closest<HTMLElement>("[data-unpin]");
  if (un) return save({ ...cfg, pins: cfg.pins.filter((p) => p !== un.dataset.unpin) });
  const stk = el.closest<HTMLElement>("[data-stack]");
  if (stk && !editing) return showStack(stk.dataset.stack!, stk.closest<HTMLElement>(".pin")!);
  if (el.closest("[data-add]")) return showAdd();
  const pin = el.closest<HTMLElement>("[data-pin]");
  if (pin && !editing) {
    try {
      await api.launcherOpen(pin.dataset.pin!);
    } catch (err) {
      msg(errText(err), true);
    }
    return;
  }
  const file = el.closest<HTMLElement>("[data-file]");
  if (file) {
    closePop();
    return api.openPath(file.dataset.file!).catch((err) => msg(errText(err), true));
  }
  const dir = el.closest<HTMLElement>("[data-opendir]");
  if (dir) {
    closePop();
    return api.launcherOpen(dir.dataset.opendir!).catch((err) => msg(errText(err), true));
  }
  const pk = el.closest<HTMLElement>("[data-pick]");
  if (pk) return addPin(pk.dataset.pick!, byId(pk.dataset.pick!)?.name ?? "");
  const k = el.closest<HTMLElement>("[data-addk]")?.dataset.addk;
  if (k === "folder" || k === "file") return pick(k === "folder");
  if (k === "link") return showUrl();
  if (el.closest("[data-cancel]")) return closePop();
}

// 编辑时按住拖动换顺序（组件要收资源管理器拖进来的文件，网页自己的拖放用不了，这里自己跟着鼠标走）
let drag: string | null = null;
function dragTarget(e: PointerEvent) {
  return document.elementFromPoint(e.clientX, e.clientY)?.closest<HTMLElement>("[data-pin], [data-add]") ?? null;
}
function onPointerDown(e: PointerEvent) {
  const b = (e.target as HTMLElement).closest<HTMLElement>("[data-pin]");
  if (!editing || e.button !== 0 || !b || (e.target as HTMLElement).closest("[data-unpin]")) return;
  drag = b.dataset.pin!;
  b.classList.add("dragging");
  pinsEl().setPointerCapture(e.pointerId);
}
function onPointerMove(e: PointerEvent) {
  if (!drag) return;
  const t = dragTarget(e);
  for (const x of pinsEl().querySelectorAll(".before")) if (x !== t) x.classList.remove("before");
  if (t && t.dataset.pin !== drag) t.classList.add("before");
}
function onPointerUp(e: PointerEvent) {
  if (!drag) return;
  const t = dragTarget(e);
  const id = drag;
  drag = null;
  if (!t || t.dataset.pin === id) return render();
  const pins = cfg.pins.filter((p) => p !== id);
  const at = t.dataset.pin ? pins.indexOf(t.dataset.pin) : pins.length;
  pins.splice(at < 0 ? pins.length : at, 0, id);
  save({ ...cfg, pins });
}

export async function initPins(enabled: () => boolean) {
  items = await api.launcherList().catch(() => []);
  render();
  listen("launcher-changed", async () => {
    items = await api.launcherList();
    render();
  });
  const sec = $("#sec-pins");
  sec.addEventListener("click", onClick);
  pinsEl().addEventListener("pointerdown", onPointerDown);
  pinsEl().addEventListener("pointermove", onPointerMove);
  pinsEl().addEventListener("pointerup", onPointerUp);
  sec.addEventListener("input", (e) => {
    if ((e.target as HTMLElement).id === "pp-q") fillPicks();
  });
  sec.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && (pop || editing)) {
      e.stopPropagation();
      if (pop) closePop();
      else {
        editing = false;
        render();
      }
    }
    if (e.key === "Enter" && (e.target as HTMLElement).id === "pp-q") sec.querySelector<HTMLElement>("#pp-list [data-pick]")?.click();
  });
  sec.addEventListener("submit", (e) => {
    e.preventDefault();
    const f = e.target as HTMLFormElement;
    let u = (f.elements.namedItem("u") as HTMLInputElement).value.trim();
    if (!u) return;
    if (!/^https?:\/\//i.test(u)) u = "https://" + u;
    let name = (f.elements.namedItem("n") as HTMLInputElement).value.trim();
    try {
      name ||= new URL(u).hostname.replace(/^www\./, "");
    } catch {
      return msg(P.badUrl, true);
    }
    addTargets([{ target: u, name }]);
  });
  // 从资源管理器把文件、文件夹拖到组件上
  const hint = $("#drophint");
  hint.textContent = P.drop;
  await getCurrentWebview().onDragDropEvent((e) => {
    const p = e.payload;
    if (!enabled()) return;
    if (p.type === "enter" || p.type === "over") hint.hidden = false;
    else hint.hidden = true;
    if (p.type === "drop" && p.paths.length) addTargets(p.paths.map((target) => ({ target, name: baseName(target) })));
  });
}
