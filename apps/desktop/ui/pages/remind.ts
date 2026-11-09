import "../styles/ui.css";
import "../styles/cards.css";
import "../lib/look";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, errText, type Device, type Note, type Reminder, type ReminderAction } from "../lib/api";
import { parseLocal } from "../lib/parse";
import { icon } from "@shared/icons";
import { cat, type CatMood } from "@shared/mascot";
import { $, addDays, chime, esc, fireAt, fmtDT, isRecurring, parseDT, repeatLabel, setTags, tagHtml } from "@shared/util";
import { remindText as T, whenFull, whenPast, whenShort } from "../text/cards";

// 提醒卡片（U5）：右下角一张，几条一起到点也只有一张，依次处理。出现时不抢键盘，点一下才接收按键：
// 回车 = 完成，Esc = 稍后。右上角 ✕ 也是稍后，不是关掉。专注结束后一起放出来的几条列成一张；
// 别的电脑处理了的，变淡写一句在哪台处理的，2 秒后滑走

const win = getCurrentWindow();
const stage = $("#stage");
const card = $("#card");
let queue: Reminder[] = [];
/** 专注结束时一起放出来的（还没处理的）id */
let batch: number[] = [];
let devices: Device[] = [];
let notes: Note[] = [];
let snoozeMin = 10;
let sound = true;
let shown = false;
/** card 一条 / pick 推迟到… / held 专注时攒的几条 / gone 别的电脑处理了 */
let mode: "card" | "pick" | "held" | "gone" = "card";
/** 卡片上正显示的那条（card、pick 时） */
let cur: Reminder | null = null;
/** 卡片上正显示的提醒 */
let showing: Reminder[] = [];
/** 在这张卡片上自己处理过的：从队列里没了是因为自己点了，不是别的电脑 */
const mine = new Set<number>();

const lateMin = (r: Reminder) => Math.floor((Date.now() - (parseDT(fireAt(r))?.getTime() ?? Date.now())) / 60000);
/** 过点 5 分钟以上（或者开机补弹的）算过点了 */
const isLate = (r: Reminder) => lateMin(r) >= 5;
const timeText = (r: Reminder) => whenPast(parseDT(fireAt(r)) ?? new Date());

/** 左上角：团子猫，关掉团子猫时是铃铛（cards.css 按 data-mascot 二选一） */
const corner = (mood: CatMood, late = false) =>
  `<span class="cat mascot play">${cat(mood)}</span><span class="bellic${late ? " late" : ""}">${icon("bell-ring", "s20")}</span>`;

/** 这几条现在专注攒着的：放出来以后还在队列里的 */
const heldList = () => queue.filter((r) => batch.includes(r.id));
/** 先处理别的（之前就在弹的、紧急的），再一起处理专注时攒的 */
const others = () => queue.filter((r) => !batch.includes(r.id));

function linkedNote(r: Reminder): Note | undefined {
  return notes.find((n) => !n.deleted_at && (n.reminder_id === r.id || (!!r.note_uid && n.uid === r.note_uid)));
}

function onlyOn(r: Reminder): string {
  const names = r.only_on.map((id) => devices.find((d) => d.uid === id)?.name).filter((n): n is string => !!n);
  return names.length ? `<span class="plain">${icon("monitor")}${esc(T.onlyOn(names))}</span>` : "";
}

function render() {
  card.classList.remove("gone", "leave");
  const rest = others();
  if (!rest.length && heldList().length >= 2) return renderHeld(heldList());
  renderCard(rest[0] ?? queue[0]);
}

function renderCard(r: Reminder) {
  mode = "card";
  cur = r;
  showing = [r];
  const late = isLate(r);
  const rec = isRecurring(r.repeat);
  const min = lateMin(r);
  const when = late
    ? `<div class="when late">${T.late(timeText(r))}<small>${T.lateAgo(min)}</small></div>`
    : `<div class="when">${T.ring(timeText(r))}</div>`;
  const meta = [
    r.urgent ? `<span class="tag" style="--c: var(--danger)">${icon("zap")}${T.urgent}</span>` : "",
    rec ? `<span class="tag" style="--c: var(--tc-purple)">${icon("repeat")}${esc(repeatLabel(r.repeat, r.due_at))}</span>` : "",
    r.tag && !(rec && r.tag === "cycle") ? tagHtml(r.tag) : "",
    onlyOn(r),
    r.snooze_count ? `<span class="plain">${icon("alarm-clock")}${T.snoozed(r.snooze_count)}</span>` : "",
  ].join("");
  const note = linkedNote(r);
  const next = queue.filter((x) => x.id !== r.id);
  card.classList.toggle("stack", next.length > 0);
  card.innerHTML = `
    <button class="x" data-act="snooze" title="${esc(T.close(snoozeMin))}" aria-label="${esc(T.snooze(snoozeMin))}">${icon("x")}</button>
    <div class="hd">${corner(late ? "worry" : "ring", late)}<div class="m">${when}
      <div class="title">${esc(r.title)}</div>${r.detail ? `<div class="detail">${esc(r.detail)}</div>` : ""}</div>
      ${next.length ? `<span class="more">${T.more(next.length)}</span>` : ""}</div>
    ${meta ? `<div class="meta">${meta}</div>` : ""}
    ${note ? `<button class="link" data-note="${note.id}">${icon("notebook-pen")}<span>${esc(note.title || "")}</span><small>${T.openNote}</small></button>` : ""}
    <div class="acts">
      <button class="btn pri" data-act="done">${icon("check")}${rec ? T.doneOnce : T.done}</button>
      <button class="btn" data-act="snooze">${T.snooze(snoozeMin)}</button>
      <button class="btn" data-act="tomorrow">${T.tomorrow}</button>
      <button class="btn ghost" data-pick>${T.pick}</button>
    </div>
    ${
      next.length
        ? `<div class="meta next"><span class="plain">${esc(T.next(next.slice(0, 2).map((x) => x.title)))}${next.length > 2 ? " …" : ""}</span>
            <span class="sp"></span><button class="btn sm ghost" data-all="done">${T.allDone}</button></div>`
        : ""
    }
    <p class="msg" id="msg"></p>`;
  fit();
}

function renderHeld(list: Reminder[]) {
  mode = "held";
  cur = null;
  showing = list;
  card.classList.remove("stack");
  card.innerHTML = `
    <button class="x" data-all="snooze" title="${esc(T.allSnooze(snoozeMin))}" aria-label="${esc(T.allSnooze(snoozeMin))}">${icon("x")}</button>
    <div class="hd">${corner("normal")}<div class="m"><div class="when">${T.heldWhen(list.length)}</div><div class="title">${T.heldTitle}</div></div></div>
    <div class="mini scroll">${list
      .map(
        (r) => `<div class="it"><button class="task" data-done="${r.id}" aria-label="${T.done}">${icon("check")}</button>
          <span class="t">${esc(r.title)}</span><span class="tm${isLate(r) ? " late" : ""}">${timeText(r)}</span>
          <button class="btn sm" data-later="${r.id}">${T.later}</button></div>`,
      )
      .join("")}</div>
    <div class="acts"><button class="btn pri" data-all="done">${icon("check")}${T.allDone}</button><button class="btn" data-all="snooze">${T.allSnooze(snoozeMin)}</button></div>
    <p class="msg" id="msg"></p>`;
  fit();
}

/** 「推迟到…」：几个常用的时间一点就走，或者打一句话（和快捷框同一套时间识别） */
function renderPick() {
  if (!cur) return;
  mode = "pick";
  const now = new Date();
  const at = (d: Date, h: number, m = 0) => new Date(d.getFullYear(), d.getMonth(), d.getDate(), h, m);
  const tomorrow = addDays(now, 1);
  const monday = addDays(now, ((8 - now.getDay()) % 7) || 7);
  const opts: [string, Date][] = [
    [T.quick.half, new Date(now.getTime() + 30 * 60000)],
    [T.quick.hour, new Date(now.getTime() + 60 * 60000)],
    now.getHours() < 19 ? [T.quick.tonight, at(now, 20)] : [T.quick.three, new Date(now.getTime() + 180 * 60000)],
    [T.quick.tmrMorning, at(tomorrow, 9)],
    [T.quick.tmrAfternoon, at(tomorrow, 14)],
    [T.quick.monday, at(monday, 9)],
  ];
  card.querySelector(".acts")?.remove();
  card.querySelector(".meta.next")?.remove();
  card.querySelector(".link")?.remove();
  const box = document.createElement("div");
  box.className = "pick";
  box.innerHTML = `<div class="lab">${T.pickLabel}</div>
    <div class="chips">${opts.map(([l, d]) => `<button class="chip" data-until="${fmtDT(d)}">${l}</button>`).join("")}</div>
    <label class="field pk"><input id="pk-in" autocomplete="off" spellcheck="false" placeholder="${esc(T.pickHint)}"><span class="pv" id="pk-pv"></span></label>
    <div class="acts"><button class="btn pri" id="pk-go" disabled>${T.pickGo}</button><button class="btn ghost" data-back>${T.back}</button></div>`;
  card.insertBefore(box, $("#msg"));
  const inp = $<HTMLInputElement>("#pk-in");
  inp.addEventListener("input", showGuess);
  inp.focus();
  fit();
}

/** 打的那句话理解成几点：没打是 null */
function guess(v: string): Date | "none" | "past" | null {
  if (!v.trim()) return null;
  const it = parseLocal(v, new Date(), devices).find((x) => x.type === "reminder");
  const d = it?.type === "reminder" ? parseDT(it.due) : null;
  if (!d) return "none";
  return d <= new Date() ? "past" : d;
}

function showGuess() {
  const g = guess($<HTMLInputElement>("#pk-in").value);
  const pv = $("#pk-pv");
  const go = $<HTMLButtonElement>("#pk-go");
  pv.classList.toggle("bad", !(g instanceof Date));
  pv.textContent = g instanceof Date ? `→ ${whenFull(g)}` : g === "none" ? T.pickNone : g === "past" ? T.pickPast : "";
  go.disabled = !(g instanceof Date);
  go.textContent = g instanceof Date ? T.pickTo(whenShort(g)) : T.pickGo;
}

function pickGo() {
  const g = guess($<HTMLInputElement>("#pk-in").value);
  if (g instanceof Date) act("snooze_to", fmtDT(g));
}

async function fit() {
  await document.fonts.ready;
  await api.fitWindow(stage.offsetHeight);
}

function showErr(err: unknown) {
  card.querySelectorAll("button").forEach((x) => (x.disabled = false));
  $("#msg").textContent = T.failed(errText(err));
  fit();
}

async function act(action: ReminderAction, until?: string, ids = cur ? [cur.id] : []) {
  if (!ids.length) return;
  card.querySelectorAll("button").forEach((x) => (x.disabled = true));
  ids.forEach((id) => mine.add(id));
  try {
    for (const id of ids) await api.reminderAction(id, action, until);
  } catch (err) {
    return showErr(err);
  }
  await reload();
}

/** 卡片上那条从队列里没了、又不是自己点的：别的电脑处理了的话回一句话 */
async function goneText(r: Reminder): Promise<string | null> {
  const where = await api.reminderChangedOn(r.id).catch(() => null);
  if (where === null) return null;
  const now = (await api.listReminders().catch(() => [])).find((x) => x.id === r.id);
  const what = !now || now.deleted_at ? "deleted" : now.done || now.last_done_at !== r.last_done_at ? "done" : "moved";
  return T.elsewhere(where, what);
}

function showGone(text: string) {
  mode = "gone";
  card.classList.add("gone");
  card.querySelector(".x")?.remove();
  card.querySelectorAll(".acts, .pick, .meta.next, .link, .msg").forEach((x) => x.remove());
  card.insertAdjacentHTML("beforeend", `<div class="gonenote">${icon("circle-check")}${esc(text)}</div>`);
  fit();
  setTimeout(() => {
    card.classList.add("leave");
    // 减少动画时动画时长是 0，不一定有 animationend，按时长兜底
    const next = () => {
      if (mode !== "gone") return;
      mode = "card";
      if (!queue.length) return void win.close();
      enter();
      render();
    };
    card.addEventListener("animationend", next, { once: true });
    setTimeout(next, 600);
  }, 2000);
}

/** 换下一张时也滑进来 */
function enter() {
  card.classList.remove("in", "leave", "gone");
  void card.offsetWidth;
  card.classList.add("in");
}

let chain = Promise.resolve();
/** 重读队列；排着队一个一个来，免得两次一起画乱了 */
const reload = () => (chain = chain.then(doReload, doReload));

async function doReload() {
  const [q, b, n] = await Promise.all([api.getQueue(), api.focusBatch().catch(() => []), api.listNotes().catch(() => notes)]);
  const lost = mode === "card" || mode === "pick" ? showing.filter((r) => !mine.has(r.id) && !q.some((x) => x.id === r.id)) : [];
  const was = cur?.id;
  queue = q;
  batch = b;
  notes = n;
  if (mode === "gone") return;
  if (shown && lost.length) {
    const text = await goneText(lost[0]);
    if (text) return showGone(text);
  }
  if (!queue.length) return void win.close();
  // 正在选推迟到几点、那条还在：不打断
  if (mode === "pick" && queue.some((r) => r.id === was)) return;
  const first = !shown;
  const before = showing.map((r) => r.id).join();
  render();
  if (!first && showing.map((r) => r.id).join() !== before && mode === "card") enter();
  if (first) {
    shown = true;
    await fit();
    // 不抢焦点：正在别处打字不会被打断，点一下卡片才接收按键
    await api.showQuiet();
    if (sound) chime(queue.some((r) => r.urgent));
  }
}

card.addEventListener("click", async (e) => {
  const el = e.target as HTMLElement;
  if (mode === "gone") return;
  if (el.closest("[data-pick]")) return renderPick();
  if (el.closest("[data-back]")) return render();
  if (el.closest("#pk-go")) return pickGo();
  const quick = el.closest<HTMLElement>("[data-until]");
  if (quick) return act("snooze_to", quick.dataset.until);
  const note = el.closest<HTMLElement>("[data-note]");
  if (note) return api.openNoteWindow(Number(note.dataset.note));
  const all = el.closest<HTMLElement>("[data-all]");
  if (all) return act(all.dataset.all as ReminderAction, undefined, (mode === "held" ? showing : queue).map((r) => r.id));
  const one = el.closest<HTMLElement>("[data-done], [data-later]");
  if (one) return act(one.dataset.done ? "done" : "snooze", undefined, [Number(one.dataset.done ?? one.dataset.later)]);
  const b = el.closest<HTMLElement>("[data-act]");
  if (b) return act(b.dataset.act as ReminderAction);
  // 点了卡片空白处：之后的回车、Esc 都给卡片
  card.focus();
});

// 点过卡片后才收得到按键（卡片出现时不抢焦点）
document.addEventListener("keydown", (e) => {
  if (e.isComposing || mode === "gone") return;
  const onButton = !!(e.target as HTMLElement).closest?.("button");
  if (e.key === "Escape") {
    e.preventDefault();
    if (mode === "pick") return render();
    if (mode === "held") return void act("snooze", undefined, showing.map((r) => r.id));
    return void act("snooze");
  }
  if (e.key === "Enter" && !onButton) {
    e.preventDefault();
    if (mode === "pick") return pickGo();
    // 几条攒在一起时回车不做什么，免得一下全完成
    if (mode === "card") act("done");
  }
});

listen("queue-changed", () => reload());
// 又到点了一条，或者没处理隔几分钟再响：团子猫再摇一次铃铛
listen("ring", () => {
  if (sound) chime(queue.some((r) => r.urgent));
  const m = card.querySelector(".mascot");
  m?.classList.remove("play");
  void card.offsetWidth;
  m?.classList.add("play");
});

(async () => {
  const [s, d] = await Promise.all([api.getSettings().catch(() => null), api.listDevices().catch(() => [])]);
  snoozeMin = s?.snooze_min ?? 10;
  sound = s?.sound ?? true;
  setTags(s?.tags);
  devices = d;
  await reload();
})();
