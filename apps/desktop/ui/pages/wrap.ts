import "../styles/ui.css";
import "../styles/cards.css";
import { mascotOn } from "../lib/look";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, errText, type Reminder } from "../lib/api";
import { icon } from "@shared/icons";
import { cat } from "@shared/mascot";
import { $, dayStr, esc, fireAt, fmtDT, isRecurring, parseDT, timeOf } from "@shared/util";
import { dateLine, remindText as R, whenPast, wrapText as T } from "../text/cards";

// 每天收尾（U5）：今天做完了什么、还剩什么，剩下的挪到明天或者顺手做掉。只看今天，不做总结。团子猫打哈欠。
// 在一台电脑上收尾后，别的电脑当天不再弹；正开着的卡片收到同步过来的"已收尾"也会自己关掉。

const win = getCurrentWindow();
const stage = $("#stage");
const card = $("#card");
/** 做完的最多列几个 */
const MAX_DONE = 12;
let closing = false;
let shown = false;
/** 打开时今天是不是已经收尾过（手动打开的）；那样的话不因为"已收尾"自己关掉 */
let wrappedAtOpen = true;

const byFire = (a: Reminder, b: Reminder) => fireAt(a).localeCompare(fireAt(b));

function lists(all: Reminder[]) {
  const tk = dayStr(new Date());
  const left = all.filter((r) => !r.done && !r.deleted_at && fireAt(r).slice(0, 10) <= tk).sort(byFire);
  const done = all.filter((r) => !r.deleted_at && ((r.done && r.completed_at?.startsWith(tk)) || (isRecurring(r.repeat) && r.last_done_at?.startsWith(tk))));
  return { left, done };
}

async function render() {
  const { left, done } = lists(await api.listReminders());
  const now = new Date();
  const tk = dayStr(now);
  const catOn = mascotOn();
  const allDone = !left.length;
  const groups: string[] = [];
  if (done.length) {
    const chips = `<div class="donechips">${done
      .slice(0, MAX_DONE)
      .map((r) => `<span>${esc(r.title)}</span>`)
      .join("")}${done.length > MAX_DONE ? `<span class="rest">${T.doneMore(done.length - MAX_DONE)}</span>` : ""}</div>`;
    // 全做完时只剩一排绿色小标签，不要标题
    groups.push(allDone ? `<div class="grp">${chips}</div>` : `<div class="grp"><h5>${icon("circle-check")}${T.done} <span class="n">${T.count(done.length)}</span></h5>${chips}</div>`);
  }
  if (left.length) {
    groups.push(`<div class="grp"><h5>${icon("circle-dashed")}${T.left} <span class="n">${T.count(left.length)}</span></h5><div class="mini scroll">${left
      .map((r) => {
        const f = fireAt(r);
        const late = f.slice(0, 10) < tk || (parseDT(f)?.getTime() ?? 0) < now.getTime();
        return `<div class="it"><button class="task" data-done="${r.id}" aria-label="${R.done}">${icon("check")}</button>
          <span class="t">${esc(r.title)}</span><span class="tm${late ? " late" : ""}">${f.slice(0, 10) < tk ? whenPast(parseDT(f) ?? now) : timeOf(f)}</span>
          <span class="acts2"><button class="btn sm" data-tmr="${r.id}">${T.moveTmr}</button></span></div>`;
      })
      .join("")}</div></div>`);
  }
  card.innerHTML = `
    <div class="sky"><span class="cat mascot${shown ? "" : " play"}">${cat(allDone && done.length ? "happy" : "yawn")}</span>
      <button class="x" data-finish aria-label="${T.finish}" title="${T.finish}">${icon("x")}</button>
      <div class="date">${dateLine(now)} · ${timeOf(fmtDT(now))}</div>
      <h4>${T.hello(catOn, allDone && done.length > 0)}</h4>
      <p>${T.say(catOn, done.length, left.length)}</p></div>
    ${groups.length ? `<div class="body">${groups.join("")}</div>` : ""}
    <p class="msg" id="msg"></p>
    <div class="foot">${left.length ? `<span class="sp">${T.tmrHint}</span>` : ""}${
      left.length > 1 ? `<button class="btn" data-all>${T.allTmr}</button>` : ""
    }<button class="btn pri" data-finish>${T.finish}</button></div>`;
  await document.fonts.ready;
  await api.fitWindow(stage.offsetHeight);
  if (!shown) {
    shown = true;
    await api.showQuiet();
  }
}

/** 收尾完了：记下今天已收尾（别的电脑就不再弹），关掉 */
async function finish() {
  if (closing) return;
  closing = true;
  await api.wrapDone().catch(() => {});
  await win.close();
}

card.addEventListener("click", async (e) => {
  const el = e.target as HTMLElement;
  if (el.closest("[data-finish]")) return finish();
  const done = el.closest<HTMLElement>("[data-done]");
  const tmr = el.closest<HTMLElement>("[data-tmr]");
  try {
    if (done) await api.reminderAction(Number(done.dataset.done), "done");
    else if (tmr) await api.reminderAction(Number(tmr.dataset.tmr), "tomorrow");
    else if (el.closest("[data-all]")) {
      const { left } = lists(await api.listReminders());
      for (const r of left) await api.reminderAction(r.id, "tomorrow");
      return finish();
    } else return;
  } catch (err) {
    $("#msg").textContent = T.failed(errText(err));
    await api.fitWindow(stage.offsetHeight);
    return;
  }
  await render();
});

// 按住卡片空白处能拖走（按钮、输入框、链接照常点）
$("#card").addEventListener("mousedown", (e) => {
  if (e.button !== 0 || (e.target as HTMLElement).closest("button, a, input, textarea, select, label, [data-act]")) return;
  win.startDragging();
});

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") finish();
  if (e.key === "Enter" && !(e.target as HTMLElement).closest?.("button")) finish();
});

// 别的电脑上已经收尾了：这张卡片也收起来；否则数据变了就刷新一下
listen("data-changed", async () => {
  if (closing) return;
  if (!wrappedAtOpen && (await api.wrapStatus())) {
    closing = true;
    await win.close();
    return;
  }
  await render();
});

(async () => {
  wrappedAtOpen = await api.wrapStatus().catch(() => false);
  await render();
})();
