// 写东西时顺手加一个分类（改提醒的抽屉、笔记的分类菜单），不用先去「设置 → 分类」：存进设置（lib.rs add_category），几台电脑共用
import { api, errText, type TagDef } from "./api";
import { icon } from "@shared/icons";
import { esc } from "@shared/util";
import { T } from "../text/notes";

export type CatField = "tags" | "note_cats";

/** 一排小胶囊、分类菜单最后的「＋ 新分类」 */
export const newCatButton = (cls: string) => `<button type="button" class="${cls} newcat" data-newcat>${icon("plus")}${esc(T.newCat)}</button>`;

/** 点了「＋ 新分类」：换成输入框，回车加好（done 拿到新分类），Esc、空着点别处就不加了 */
export function openCatInput(btn: HTMLElement, field: CatField, done: (t: TagDef) => void, fail: (msg: string) => void) {
  const inp = document.createElement("input");
  inp.className = "field catin";
  inp.maxLength = 8;
  inp.placeholder = T.newCatPh;
  inp.setAttribute("aria-label", T.newCat);
  btn.replaceWith(inp);
  inp.focus();
  let busy = false;
  const back = () => inp.isConnected && inp.replaceWith(btn);
  inp.addEventListener("keydown", async (e) => {
    // 回车、Esc 只管这个输入框：不提交外面的表单、不关抽屉
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      return back();
    }
    if (e.key !== "Enter" || e.isComposing || busy) return;
    e.preventDefault();
    e.stopPropagation();
    const name = inp.value.trim();
    if (!name) return back();
    busy = true;
    try {
      const t = await api.addCategory(field, name);
      back();
      done(t);
    } catch (err) {
      busy = false;
      fail(errText(err));
    }
  });
  inp.addEventListener("blur", () => {
    if (!inp.value.trim() && !busy) back();
  });
}
