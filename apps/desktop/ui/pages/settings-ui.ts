// 设置页几块共用的小工具：画一行、开关、下拉，存设置，弹窗。settings.ts 和 settings-sync.ts 都用
import { api, errText, type Settings } from "../lib/api";
import { icon, type IconName } from "@shared/icons";
import { $, esc, toast } from "@shared/util";
import { hideModal, hooks, M, S, showModal } from "./main-state";
import { T, type RowText } from "../text/settings";

/** 「只在这台电脑」小标 */
export const LOCAL = `<span class="local">${T.local}</span>`;
/** 「实验」小标：还在试的功能 */
export const EXP = `<span class="exp">${T.exp}</span>`;

export interface RowOpt {
  /** 只属于这台电脑 */
  local?: boolean;
  /** 还在试的功能（名字后面「实验」小标） */
  exp?: boolean;
  /** 换掉 ROWS 里的小字说明（HTML，调用的地方自己转义）；null 是不要说明 */
  sub?: string | null;
  cls?: string;
}

/** 设置的一行：名字（+ 小标）、小字说明，右边是控件。data-row 给搜索、「去设置」跳过来用 */
export function row(id: string, t: RowText, ctl = "", o: RowOpt = {}): string {
  const sub = o.sub === undefined ? (t.sub ? esc(t.sub) : "") : o.sub;
  return `<div class="srow${o.cls ? ` ${o.cls}` : ""}" data-row="${id}"><div>${esc(t.name)}${o.exp ? EXP : ""}${o.local ? LOCAL : ""}${sub ? `<small>${sub}</small>` : ""}</div>${ctl}</div>`;
}

/** 一组行 */
export const rows = (...r: string[]) => `<div class="srows">${r.join("")}</div>`;

/** 小节标题 */
export const h5 = (text: string, o: { local?: boolean; id?: string } = {}) =>
  `<h5${o.id ? ` data-row="${o.id}"` : ""}>${esc(text)}${o.local ? LOCAL : ""}</h5>`;

/** 页顶上：标题和一句说明 */
export const head = (title: string, hint = "") => `<h4>${esc(title)}</h4>${hint ? `<p class="hint">${esc(hint)}</p>` : ""}`;

/** 开关：点了直接改 S.settings[k] 再存 */
export const tog = (k: keyof Settings, label: string) =>
  `<button class="tog" role="switch" data-set="${k}" aria-checked="${!!S.settings?.[k]}" aria-label="${esc(label)}"></button>`;

/** 下拉：改了直接存（data-num 存成数字） */
export function select(k: keyof Settings, label: string, opts: [string | number, string][]) {
  const v = String(S.settings?.[k] ?? "");
  return `<select class="field" data-set="${k}" ${typeof opts[0]?.[0] === "number" ? "data-num" : ""} aria-label="${esc(label)}">${opts
    .map(([o, l]) => `<option value="${o}" ${String(o) === v ? "selected" : ""}>${esc(l)}</option>`)
    .join("")}</select>`;
}

/** 横幅：ok 绿 / warn 黄 / err 红 / info 蓝 / soft 灰 */
export const note = (kind: "ok" | "warn" | "err" | "info" | "soft", ic: IconName, title: string, text = "", extra = "") =>
  `<div class="banner ${kind}">${icon(ic)}<div>${title ? `<b>${title}</b>` : ""}${text}</div>${extra}</div>`;

/** 快捷键、存设置时的提示放在通用页「快捷键」下面那一行；不在那一页时用小提示 */
export function hkMsg(text: string, bad = false) {
  const m = document.querySelector<HTMLElement>("#hk-msg");
  if (!m) return void (text && bad && toast(text));
  m.className = `fhelp${bad ? " err" : " ok"}`;
  m.textContent = text;
}

/** 存设置（几台电脑共用的会同步过去）；存好了回 true */
export async function saveSettings(): Promise<boolean> {
  if (!S.settings) return false;
  let ok = true;
  try {
    await api.saveSettings(S.settings);
    hkMsg("");
  } catch (e) {
    // 设置已经存好了，只是有快捷键注册不上：这类提示放在「快捷键」下面
    const t = errText(e);
    if (t === (await api.hotkeyNotice().catch(() => null))) hkMsg(t, true);
    else {
      ok = false;
      toast(T.saveFail(t));
    }
  }
  hooks.afterSave();
  return ok;
}

/** 弹窗：按钮写动作，不写「确定」。sticky 时点外面、按 Esc 都不关 */
export function dialog(html: string, o: { sticky?: boolean; wide?: boolean; form?: string } = {}) {
  const tag = o.form ? "form" : "div";
  showModal(`<${tag} class="dialog${o.wide ? " wide" : ""}"${o.form ? ` id="${o.form}" autocomplete="off"` : ""} role="dialog" ${o.sticky ? "data-sticky" : ""}>${html}</${tag}>`);
  $("#modal").querySelectorAll<HTMLElement>("[data-close]").forEach((b) => (b.onclick = hideModal));
  M.close = o.sticky ? null : hideModal;
}

/** 正在导出、恢复时，弹窗点外面、按 Esc 都不关 */
export function lockDialog(on: boolean) {
  $("#modal").querySelector(".dialog")?.toggleAttribute("data-sticky", on);
  M.close = on ? null : hideModal;
}

/** 按钮上先换成「正在…」，做完换回来；出错时回错误的字 */
export async function busy(b: HTMLButtonElement, text: string, run: () => Promise<unknown>): Promise<string | null> {
  const label = b.innerHTML;
  b.disabled = true;
  b.textContent = text;
  try {
    await run();
    return null;
  } catch (e) {
    return errText(e);
  } finally {
    if (b.isConnected) {
      b.disabled = false;
      b.innerHTML = label;
    }
  }
}
