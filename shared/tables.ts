// 笔记里表格的样子和操作（主窗口和手机版共用）：列表 / 卡片 / 表格、搜索、分组、收起时显示哪几列、
// 展开哪几行、改一行写回正文。画表格在 md.ts 的 renderTable。
// 样子、分组、摘要列记在这台设备上（localStorage，每条笔记一份）；搜索、展开、正在改的只在这次打开时有效
import { defaultSum, findTables, MAX_SUM } from "./md";

export type TableView = "list" | "cards" | "table";

export interface TableState {
  view?: TableView;
  /** 按第几列分组，null 不分组 */
  group?: number | null;
  /** 收起时显示哪几列 */
  sum?: number[];
  /** 这张表里搜的字 */
  q?: string;
  /** 展开着的行 */
  open?: number[];
  /** 收起来的组 */
  closed?: string[];
  /** 正在改的行；等于行数时是在加一行 */
  editing?: number | null;
  /** 改到一半的内容（重画时不丢） */
  draft?: string[];
  /** 表格视图里点了「展开」的长格子（"行:列"） */
  more?: string[];
  menu?: "group" | "sum" | null;
  /** 「删掉这行」点过一次了 */
  delArm?: boolean;
}

type Saved = Pick<TableState, "view" | "group" | "sum">;

const key = (uid: string) => `tview:${uid}`;

function loadSaved(uid: string): Record<string, Saved> {
  try {
    const v = JSON.parse(localStorage.getItem(key(uid)) ?? "{}");
    // 0.3.3 存的是 "table" / "cards"
    for (const k of Object.keys(v)) if (typeof v[k] === "string") v[k] = { view: v[k] };
    return v;
  } catch {
    return {};
  }
}

const temp = new Map<string, TableState>();

/** 每张表格现在的样子和状态，给 renderMd */
export function tableStates(uid: string): (n: number) => TableState {
  const saved = loadSaved(uid);
  return (n) => ({ ...saved[n], ...temp.get(`${uid}:${n}`) });
}

function update(uid: string, n: number, patch: TableState) {
  const keep: (keyof Saved)[] = ["view", "group", "sum"];
  const toSave = Object.fromEntries(Object.entries(patch).filter(([k]) => keep.includes(k as keyof Saved)));
  if (Object.keys(toSave).length) {
    const saved = loadSaved(uid);
    saved[n] = { ...saved[n], ...toSave };
    try {
      localStorage.setItem(key(uid), JSON.stringify(saved));
    } catch {
      // 存不下就只这次有效
    }
  }
  const k = `${uid}:${n}`;
  temp.set(k, { ...temp.get(k), ...Object.fromEntries(Object.entries(patch).filter(([k]) => !keep.includes(k as keyof Saved))) });
}

const get = (uid: string, n: number) => tableStates(uid)(n);

/** 重画之前，把表单里改到一半的内容记下来 */
export function captureDrafts(root: ParentNode, uid: string) {
  for (const f of root.querySelectorAll<HTMLElement>("[data-tform]")) {
    const vals = [...f.querySelectorAll<HTMLTextAreaElement>("[data-tf]")].map((t) => t.value);
    update(uid, Number(f.dataset.tform), { draft: vals });
  }
}

/** 一行格子写回 Markdown：| 写成 \|，换行写成 <br> */
const rowLine = (cells: string[]) => `| ${cells.map((c) => c.trim().replace(/\r?\n/g, "<br>").replace(/\|/g, "\\|")).join(" | ")} |`;

/** 改第 n 张表格的第 i 行：cells 为 null 是删掉；i 等于行数是加在最后；after 是在第 i 行后面插一行 */
export function editRow(src: string, n: number, i: number, cells: string[] | null, after = false): string {
  const t = findTables(src)[n];
  if (!t) return src;
  const lines = src.split("\n");
  const at = t.start + 2 + i;
  const full = cells && t.titles.map((_, k) => cells[k] ?? "");
  if (!full) lines.splice(at, 1);
  else if (after) lines.splice(at + 1, 0, rowLine(full));
  else if (i >= t.rows.length) lines.splice(t.start + t.lines, 0, rowLine(full));
  else lines[at] = rowLine(full);
  return lines.join("\n");
}

export interface TableCtx {
  uid: string;
  body: () => string;
  /** 重画这条笔记 */
  render: () => void;
  /** 保存改过的正文 */
  save: (body: string) => Promise<void>;
  toast: (msg: string) => void;
}

/** 表格上的点击；处理了回 true */
export async function tableClick(el: HTMLElement, ctx: TableCtx): Promise<boolean> {
  const { uid } = ctx;
  const n = (attr: string) => Number(el.closest<HTMLElement>(`[${attr}]`)!.getAttribute(attr));
  const btn = (attr: string) => el.closest<HTMLElement>(`[${attr}]`);
  // 开着的菜单：点别处就关上
  const openMenu = [...document.querySelectorAll<HTMLElement>("[data-tmenu][aria-expanded=true]")][0];
  if (openMenu && !el.closest(".tmenu") && !el.closest("[data-tmenu]")) {
    update(uid, Number(openMenu.dataset.tmenu), { menu: null });
    ctx.render();
    return true;
  }
  let b: HTMLElement | null;
  if ((b = btn("data-tview"))) update(uid, n("data-tview"), { view: b.dataset.v as TableView, menu: null });
  else if ((b = btn("data-tmenu"))) {
    const m = b.dataset.m as "group" | "sum";
    update(uid, n("data-tmenu"), { menu: get(uid, n("data-tmenu")).menu === m ? null : m });
  } else if ((b = btn("data-tgroup"))) update(uid, n("data-tgroup"), { group: b.dataset.k === "" ? null : Number(b.dataset.k), menu: null, closed: [] });
  else if ((b = btn("data-tsum"))) {
    const t = n("data-tsum");
    const k = Number(b.dataset.k);
    const titles = findTables(ctx.body())[t]?.titles ?? [];
    const cur = get(uid, t).sum ?? defaultSum(titles);
    if (!cur.includes(k) && cur.length >= MAX_SUM) {
      ctx.toast(`最多 ${MAX_SUM} 列`);
      return true;
    }
    const next = cur.includes(k) ? cur.filter((x) => x !== k) : [...cur, k].sort((a, c) => a - c);
    update(uid, t, { sum: next.length ? next : [0] });
  } else if ((b = btn("data-trow"))) {
    const t = n("data-trow");
    const i = Number(b.dataset.i);
    const s = get(uid, t);
    if (s.editing === i) update(uid, t, { editing: null, draft: undefined, delArm: false });
    const open = s.open ?? [];
    update(uid, t, { open: open.includes(i) ? open.filter((x) => x !== i) : [...open, i] });
  } else if ((b = btn("data-tgh"))) {
    const t = n("data-tgh");
    const g = b.dataset.g!;
    const closed = get(uid, t).closed ?? [];
    update(uid, t, { closed: closed.includes(g) ? closed.filter((x) => x !== g) : [...closed, g] });
  } else if ((b = btn("data-tmore"))) {
    const t = n("data-tmore");
    update(uid, t, { more: [...(get(uid, t).more ?? []), b.dataset.key!] });
  } else if ((b = btn("data-tact"))) {
    await tableAct(b, ctx);
    return true;
  } else return false;
  ctx.render();
  return true;
}

async function tableAct(b: HTMLElement, ctx: TableCtx) {
  const { uid } = ctx;
  const t = Number(b.dataset.t);
  const i = Number(b.dataset.i);
  const act = b.dataset.tact;
  const rows = findTables(ctx.body())[t]?.rows ?? [];
  const done = (patch: TableState) => update(uid, t, { editing: null, draft: undefined, delArm: false, ...patch });
  if (act === "edit") update(uid, t, { editing: i, draft: undefined, delArm: false });
  else if (act === "cancel") done({});
  else if (act === "add") update(uid, t, { editing: rows.length, draft: undefined, delArm: false });
  else if (act === "save") {
    const form = b.closest<HTMLElement>("[data-tform]")!;
    const vals = [...form.querySelectorAll<HTMLTextAreaElement>("[data-tf]")].map((x) => x.value);
    if (!vals.some((v) => v.trim())) return ctx.toast("还什么都没填");
    done({ open: [...(get(uid, t).open ?? []), i] });
    await ctx.save(editRow(ctx.body(), t, i, vals));
    ctx.toast(i >= rows.length ? "加好了" : "已保存");
  } else if (act === "dup") {
    const copy = [...(rows[i] ?? [])];
    copy[0] = `${copy[0] ?? ""}（副本）`;
    update(uid, t, { open: [i + 1], editing: i + 1, draft: copy, delArm: false });
    await ctx.save(editRow(ctx.body(), t, i, copy, true));
  } else if (act === "del") {
    if (!get(uid, t).delArm) {
      update(uid, t, { delArm: true });
      ctx.render();
      return;
    }
    done({ open: [] });
    await ctx.save(editRow(ctx.body(), t, i, null));
    ctx.toast("删掉了这一行");
  }
  ctx.render();
}

/** 表格里的搜索框：每打一个字重画，光标留在原处 */
export function tableInput(el: HTMLElement, ctx: TableCtx): boolean {
  const inp = el.closest<HTMLInputElement>("[data-tq]");
  if (!inp) return false;
  const n = Number(inp.dataset.tq);
  const pos = inp.selectionStart ?? inp.value.length;
  update(ctx.uid, n, { q: inp.value });
  ctx.render();
  const again = document.querySelector<HTMLInputElement>(`[data-tq="${n}"]`);
  again?.focus();
  again?.setSelectionRange(pos, pos);
  return true;
}

/** 改一行时：Ctrl + Enter 保存，Esc 取消 */
export function tableKey(e: KeyboardEvent, ctx: TableCtx): boolean {
  const form = (e.target as HTMLElement).closest<HTMLElement>("[data-tform]");
  if (!form) return false;
  const pick = (act: string) => form.querySelector<HTMLElement>(`[data-tact="${act}"]`);
  if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
    e.preventDefault();
    void tableAct(pick("save")!, ctx);
    return true;
  }
  if (e.key === "Escape") {
    e.preventDefault();
    e.stopPropagation();
    void tableAct(pick("cancel")!, ctx);
    return true;
  }
  return false;
}
