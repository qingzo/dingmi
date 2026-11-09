import "../styles/ui.css";
import "../styles/cards.css";
import { mascotOn } from "../lib/look";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, errText, type Note, type Reminder, type Settings } from "../lib/api";
import { icon } from "@shared/icons";
import { cat } from "@shared/mascot";
import { plain } from "@shared/md";
import { $, addDays, dayStr, esc, fireAt, fmtDT, hotkeyText, isRecurring, parseDT, timeOf } from "@shared/util";
import { dateLine, morningText as T, remindText as R, whenPast } from "../text/cards";

// 每日开场白（U5）：每天第一次开电脑时弹，屏幕中间偏下。团子猫伸懒腰。
// 上面一段话说今天几件事、最要紧的是哪件、有没有剩下的；有 AI Key 时这段让 AI 写。不显示天气（叮咪不取天气）

const win = getCurrentWindow();
const stage = $("#stage");
const card = $("#card");
/** 今天的、之前没做完的各列几条 */
const MAX = 3;
let settings: Settings | null = null;
/** AI 写好的那段话：做了操作重画时留着 */
let aiSay: string | null = null;
let shown = false;

/** 最要紧的一件：紧急的优先，其次是接下来最早的 */
function pickTop(list: Reminder[]): Reminder | undefined {
  const now = new Date();
  return list.find((r) => r.urgent) ?? list.find((r) => (parseDT(fireAt(r)) ?? now) >= now) ?? list[0];
}

function split(reminders: Reminder[]) {
  const tk = dayStr(new Date());
  const pending = reminders.filter((r) => !r.done && !r.deleted_at).sort((a, b) => fireAt(a).localeCompare(fireAt(b)));
  return {
    today: pending.filter((r) => fireAt(r).slice(0, 10) === tk),
    late: pending.filter((r) => fireAt(r).slice(0, 10) < tk),
  };
}

/** 昨天和今天记的笔记，新的在前 */
function recentNotes(notes: Note[]) {
  const since = dayStr(addDays(new Date(), -1));
  return notes.filter((n) => !n.deleted_at && n.created_at.slice(0, 10) >= since).sort((a, b) => b.created_at.localeCompare(a.created_at));
}

const row = (r: Reminder, extra = "", cls = "", label = "") =>
  `<div class="it ${cls}"><button class="task" data-done="${r.id}" aria-label="${R.done}">${icon("check")}</button>
    <span class="t">${label}${esc(r.title)}</span>${extra}</div>`;

function render(reminders: Reminder[], notes: Note[]) {
  const now = new Date();
  const catOn = mascotOn();
  const { today, late } = split(reminders);
  const top = pickTop(today);
  const recent = recentNotes(notes);
  const empty = !today.length && !late.length;
  const yesterday = dayStr(addDays(now, -1));
  const say = empty
    ? T.free(catOn, esc(hotkeyText(settings?.hk_flash ?? "Alt+Space")))
    : T.say(today.length, top ? `${timeOf(fireAt(top))} ${esc(top.title)}` : "", late.length, late.every((r) => fireAt(r).slice(0, 10) === yesterday));
  const todayRows = top ? [top, ...today.filter((r) => r !== top)] : today;

  const groups: string[] = [];
  if (today.length) {
    const shownRows = todayRows.slice(0, MAX);
    groups.push(`<div class="grp"><h5>${icon("sun")}${T.today} <span class="n">${T.count(today.length)}</span></h5><div class="mini">
      ${shownRows.map((r) => row(r, `<span class="tm">${timeOf(fireAt(r))}</span>`, r === top ? "top1" : "", r === top ? `<b>${T.top}</b>` : "")).join("")}
      ${today.length > MAX ? `<div class="it rest"><span class="t">${T.restMore(today.length - MAX)}</span></div>` : ""}</div></div>`);
  }
  if (late.length) {
    groups.push(`<div class="grp"><h5>${icon("history")}${T.late} <span class="n">${T.count(late.length)}</span></h5><div class="mini">
      ${late
        .slice(0, MAX)
        .map((r) => row(r, `<span class="tm late">${whenPast(parseDT(fireAt(r)) ?? now)}</span><span class="acts2"><button class="btn sm" data-today="${r.id}">${T.moveToday}</button></span>`))
        .join("")}
      ${late.length > MAX ? `<div class="it rest"><span class="t">${T.restMore(late.length - MAX)}</span></div>` : ""}</div></div>`);
  }
  if (recent.length) {
    const onlyYesterday = recent.every((n) => n.created_at.slice(0, 10) === yesterday);
    groups.push(`<div class="grp"><h5>${icon("notebook-pen")}${T.notes(onlyYesterday)} <span class="n">${T.noteCount(recent.length)}</span></h5><div class="mini">
      ${recent
        .slice(0, MAX)
        .map((n) => `<div class="it"><span class="t">${esc(n.title || plain(n.body).slice(0, 30))}</span><button class="btn sm ghost look" data-note="${n.id}">${T.look}</button></div>`)
        .join("")}</div></div>`);
  }

  card.innerHTML = `
    <div class="sky"><span class="cat mascot${shown ? "" : " play"}">${cat(empty ? "happy" : "stretch")}</span>
      <button class="x" data-close aria-label="${T.close}" title="${T.close}">${icon("x")}</button>
      <div class="date">${dateLine(now)}</div><h4>${T.hello(catOn, now.getHours())}</h4>
      <p id="say">${aiSay !== null ? esc(aiSay) : say}</p>
      <div class="ai" id="ai" hidden><span class="spin"></span>${T.aiWriting}</div></div>
    ${groups.length ? `<div class="body">${groups.join("")}</div>` : ""}
    <p class="msg" id="msg"></p>
    <div class="foot">${
      empty
        ? `<button class="btn pri" data-close>${T.ok}</button>`
        : `<span class="sp">${T.off}</span><button class="btn" data-open>${T.openToday}</button><button class="btn pri" data-close>${T.start}</button>`
    }</div>`;
  return { today, late, empty };
}

async function fit() {
  await document.fonts.ready;
  await api.fitWindow(stage.offsetHeight);
}

async function refresh() {
  const [reminders, notes] = await Promise.all([api.listReminders(), api.listNotes()]);
  render(reminders, notes);
  await fit();
}

/** 之前没做完的挪到今天：还是原来那个钟点；那个钟点今天已经过了，就半小时后 */
function todayAt(r: Reminder): string {
  const now = new Date();
  const f = parseDT(fireAt(r)) ?? now;
  let at = new Date(now.getFullYear(), now.getMonth(), now.getDate(), f.getHours(), f.getMinutes());
  if (at <= now) {
    at = new Date(now.getTime() + 30 * 60000);
    at.setMinutes(Math.ceil(at.getMinutes() / 5) * 5, 0, 0);
  }
  return fmtDT(at);
}

card.addEventListener("click", async (e) => {
  const el = e.target as HTMLElement;
  if (el.closest("[data-close]")) return win.close();
  if (el.closest("[data-open]")) {
    await api.openMainNav("today");
    return win.close();
  }
  const n = el.closest<HTMLElement>("[data-note]");
  if (n) {
    await api.openMainNav("notes", Number(n.dataset.note));
    return win.close();
  }
  const done = el.closest<HTMLElement>("[data-done]");
  const move = el.closest<HTMLElement>("[data-today]");
  if (!done && !move) return;
  try {
    if (done) await api.reminderAction(Number(done.dataset.done), "done");
    else if (move) {
      const r = (await api.listReminders()).find((x) => x.id === Number(move.dataset.today));
      if (r) await api.reminderAction(r.id, "snooze_to", todayAt(r));
    }
  } catch (err) {
    $("#msg").textContent = errText(err);
    return fit();
  }
  await refresh();
});

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") win.close();
  if (e.key === "Enter" && !(e.target as HTMLElement).closest?.("button")) win.close();
});

// 按住卡片空白处能拖走（按钮、输入框、链接照常点）
$("#card").addEventListener("mousedown", (e) => {
  if (e.button !== 0 || (e.target as HTMLElement).closest("button, a, input, textarea, select, label, [data-act]")) return;
  win.startDragging();
});

(async () => {
  const [reminders, notes, s] = await Promise.all([api.listReminders(), api.listNotes(), api.getSettings()]);
  settings = s;
  const { today, late, empty } = render(reminders, notes);
  await fit();
  await api.showQuiet();
  shown = true;

  // 有 AI Key 就让 AI 写一段更像人说的话
  if (!s.ai_key || empty) return;
  $("#ai").hidden = false;
  await fit();
  const lines = [
    "今天的日程：",
    ...today.map((r) => `- ${timeOf(fireAt(r))} ${r.title}${r.detail ? `（${r.detail}）` : ""}${r.urgent ? " [紧急]" : ""}${isRecurring(r.repeat) ? " [周期]" : ""}`),
    ...(late.length ? ["之前没处理完的：", ...late.map((r) => `- ${fireAt(r)} ${r.title}`)] : []),
    "最近的笔记：",
    ...notes.slice(0, 5).map((n) => `- ${n.title}：${plain(n.body).slice(0, 60)}`),
  ];
  try {
    const text = await api.aiMorning(lines.join("\n"));
    const say = $("#say");
    say.classList.add("fade");
    setTimeout(async () => {
      aiSay = text;
      say.textContent = text;
      say.classList.remove("fade");
      await fit();
    }, 250);
  } catch {
    /* AI 不可用时保留本地写的那段 */
  }
  $("#ai").hidden = true;
  await fit();
})();
