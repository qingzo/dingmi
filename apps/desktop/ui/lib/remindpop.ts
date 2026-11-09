// 设提醒的小框（U7「待办设提醒」「选中文字设成提醒」）：打一句话说什么时候（和快捷框同一套理解），
// 实时显示理解成几点；下面几个常用时间。设过的能改、能「不提醒了」
import "../styles/editor.css";
import { esc, friendlyDT } from "@shared/util";
import { parseLocal } from "./parse";
import { T } from "../text/notes";

export interface RemindAsk {
  anchor: HTMLElement;
  /** 显示在最上面的一行（选中文字设成提醒时：要提醒的事） */
  title?: string;
  /** 输入框里先放的字 */
  phrase?: string;
  /** 已经设过的时间："YYYY-MM-DD HH:MM" */
  due?: string;
  /** 设过的：多一个「不提醒了」 */
  canDelete?: boolean;
}

/** 一句话里说的时间；没说时间回 null */
export function whenOf(text: string): string | null {
  const r = parseLocal(text).find((i) => i.type === "reminder");
  return r && r.type === "reminder" ? r.due : null;
}

let closeOpen: (() => void) | null = null;

/** 回选好的时间；"delete" 是点了「不提醒了」；null 是取消了 */
export function askRemind(o: RemindAsk): Promise<{ due: string } | "delete" | null> {
  closeOpen?.();
  const box = document.createElement("form");
  box.className = "rpop";
  box.innerHTML = `${o.title ? `<div class="rt" title="${esc(o.title)}">${esc(T.rpTitle(o.title))}</div>` : ""}
    <label class="rf"><input placeholder="${T.rpWhen}" maxlength="200" autocomplete="off" aria-label="${T.rpWhen}"><span class="pv"></span></label>
    <div class="ed-chips">${T.rpChips.map((c) => `<button type="button" class="ed-chip" data-when="${esc(c)}">${esc(c)}</button>`).join("")}</div>
    <div class="acts">${o.canDelete ? `<button type="button" class="btn sm ghost danger" data-r="del">${T.rpDelete}</button><span class="sp"></span>` : ""}<button type="button" class="btn sm ghost" data-r="cancel">${T.rpCancel}</button><button class="btn sm pri" data-r="ok">${T.rpDone}</button></div>`;
  document.body.append(box);
  const input = box.querySelector("input")!;
  const pv = box.querySelector<HTMLElement>(".pv")!;
  const ok = box.querySelector<HTMLButtonElement>('[data-r="ok"]')!;
  input.value = o.phrase ?? "";
  let due: string | null = null;
  const preview = () => {
    const v = input.value.trim();
    due = v ? whenOf(v) : (o.due ?? null);
    pv.textContent = due ? `→ ${friendlyDT(due)}` : v ? T.rpNone : "";
    pv.classList.toggle("bad", !due);
    ok.disabled = !due;
    box.querySelectorAll<HTMLElement>("[data-when]").forEach((c) => c.classList.toggle("on", c.dataset.when === v));
  };
  preview();
  // 放在按钮下面，放不下就放上面；左右不出窗口
  const r = o.anchor.getBoundingClientRect();
  const w = box.offsetWidth;
  const h = box.offsetHeight;
  box.style.left = `${Math.max(8, Math.min(r.left, innerWidth - w - 8))}px`;
  box.style.top = `${r.bottom + 6 + h <= innerHeight ? r.bottom + 6 : Math.max(8, r.top - h - 6)}px`;
  input.focus();
  input.select();

  return new Promise((resolve) => {
    const done = (v: { due: string } | "delete" | null) => {
      closeOpen = null;
      document.removeEventListener("pointerdown", outside, true);
      document.removeEventListener("keydown", key, true);
      box.remove();
      resolve(v);
    };
    const outside = (e: Event) => {
      if (!box.contains(e.target as Node) && !o.anchor.contains(e.target as Node)) done(null);
    };
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      done(null);
    };
    closeOpen = () => done(null);
    document.addEventListener("pointerdown", outside, true);
    document.addEventListener("keydown", key, true);
    input.addEventListener("input", preview);
    box.addEventListener("submit", (e) => {
      e.preventDefault();
      if (due) done({ due });
    });
    box.addEventListener("click", (e) => {
      const el = e.target as HTMLElement;
      const chip = el.closest<HTMLElement>("[data-when]");
      if (chip) {
        input.value = chip.dataset.when!;
        preview();
        input.focus();
        return;
      }
      const b = el.closest<HTMLElement>("[data-r]")?.dataset.r;
      if (b === "cancel") done(null);
      else if (b === "del") done("delete");
    });
  });
}
