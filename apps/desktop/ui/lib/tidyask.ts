// 「让 AI 整理」前先问一句这次的要求（主窗口、笔记窗口、快速记笔记共用，U7）：一个贴着按钮的小框——
// 常用要求、自己写一句、最近用过的 3 句；「直接整理」不提要求。最近用过的几句记在这台电脑上
import "../styles/editor.css";
import { icon } from "@shared/icons";
import { esc } from "@shared/util";
import { T } from "../text/notes";

const RECENT_KEY = "tidy-asks";
const PRESETS = T.aiPresets;
const MAX_RECENT = 3;

function recent(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
    return Array.isArray(v) ? v.filter((x) => typeof x === "string").slice(0, MAX_RECENT) : [];
  } catch {
    return [];
  }
}

function remember(ask: string) {
  if (!ask || PRESETS.includes(ask)) return;
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify([ask, ...recent().filter((x) => x !== ask)].slice(0, MAX_RECENT)));
  } catch {
    // 存不下就不记了
  }
}

/** 开着的那个小框怎么关 */
let closeOpen: ((v: string | null) => void) | null = null;

/** 弹出小框，回用户的要求：空字符串是"不提要求"，null 是取消了。已经开着时再点按钮就是关掉 */
export function askTidy(anchor: HTMLElement): Promise<string | null> {
  if (closeOpen) {
    closeOpen(null);
    return Promise.resolve(null);
  }
  const past = recent().filter((x) => !PRESETS.includes(x));
  const box = document.createElement("form");
  box.className = "aiask";
  box.innerHTML = `<b>${T.aiAsk}</b>
    <div class="ed-chips">${PRESETS.map((c, i) => `<button type="button" class="ed-chip${i === 0 ? " on" : ""}" data-ask="${esc(c)}">${esc(c)}</button>`).join("")}</div>
    <input class="in" placeholder="${esc(T.aiInput)}" maxlength="200" autocomplete="off" aria-label="${esc(T.aiInput)}">
    ${past.length ? `<div class="recent">${T.aiRecent}${past.map((c) => `<button type="button" data-past="${esc(c)}">${esc(c)}</button>`).join(" · ")}</div>` : ""}
    <div class="acts"><button type="button" class="btn sm ghost" data-a="cancel">${T.aiCancel}</button><button type="button" class="btn sm" data-a="plain">${T.aiPlain}</button><button class="btn sm pri" data-a="go">${icon("sparkles")}<span></span></button></div>`;
  document.body.append(box);
  const input = box.querySelector("input")!;
  const goText = box.querySelector<HTMLElement>('[data-a="go"] span')!;
  /** 现在按哪个要求整理：选中的常用要求，或者自己写的这句 */
  let ask: string = PRESETS[0];
  const pick = (v: string, typed = false) => {
    ask = v;
    box.querySelectorAll<HTMLElement>("[data-ask]").forEach((c) => c.classList.toggle("on", !typed && c.dataset.ask === v));
    goText.textContent = T.aiGo(v);
  };
  pick(ask);
  // 放在按钮下方，放不下就放上方；左右不出窗口
  const r = anchor.getBoundingClientRect();
  const w = box.offsetWidth;
  const h = box.offsetHeight;
  box.style.left = `${Math.max(8, Math.min(r.right - w, innerWidth - w - 8))}px`;
  box.style.top = `${r.bottom + 6 + h <= innerHeight ? r.bottom + 6 : Math.max(4, r.top - h - 6)}px`;
  input.focus();

  return new Promise((resolve) => {
    const done = (v: string | null) => {
      closeOpen = null;
      document.removeEventListener("pointerdown", outside, true);
      document.removeEventListener("keydown", key, true);
      box.remove();
      if (v) remember(v);
      resolve(v);
    };
    const outside = (e: Event) => {
      if (!box.contains(e.target as Node) && !anchor.contains(e.target as Node)) done(null);
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        done(null);
      }
    };
    closeOpen = done;
    document.addEventListener("pointerdown", outside, true);
    document.addEventListener("keydown", key, true);
    input.addEventListener("input", () => pick(input.value.trim() || PRESETS[0], !!input.value.trim()));
    box.addEventListener("submit", (e) => {
      e.preventDefault();
      done(ask);
    });
    box.addEventListener("click", (e) => {
      const el = e.target as HTMLElement;
      const chip = el.closest<HTMLElement>("[data-ask], [data-past]");
      if (chip) {
        input.value = chip.dataset.past ?? "";
        pick(chip.dataset.ask ?? chip.dataset.past!, !!chip.dataset.past);
        return;
      }
      const a = el.closest<HTMLElement>("[data-a]")?.dataset.a;
      if (a === "cancel") done(null);
      else if (a === "plain") done("");
    });
  });
}
