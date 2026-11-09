// 格子编辑器：像电子表格一样改一张 Markdown 表格。打开时把表格拆成格子，点「完成」再排好写回去。
// 能增删、移动行和列，按某列排序，设每列对齐；格子里的字自动折行，Shift + 回车在格子里换行（写回时变成 <br>）；
// 从 Excel、网页复制的一片格子可以直接贴进来，不够的行列自动加上；撤销 / 重做。
import "../styles/grid.css";
import { formatTable, splitRow } from "./mdformat";
import { esc } from "@shared/util";

type Align = "" | "l" | "c" | "r";

interface Model {
  /** 第 0 行是表头 */
  cells: string[][];
  align: Align[];
}

interface Options {
  /** 表格的几行源码（表头、分隔行、各行） */
  lines: string[];
  /** 点了完成、而且改过了：排好的几行源码 */
  onDone: (lines: string[]) => void;
}

// ---------- 源码和格子互转 ----------

const fromMd = (c: string) => c.replace(/\\\|/g, "|").replace(/<br\s*\/?>/gi, "\n");
const toMd = (c: string) =>
  c
    .replace(/\r/g, "")
    .split("\n")
    .map((l) => l.trim())
    .join("<br>")
    .replace(/^(<br>)+|(<br>)+$/g, "")
    .replace(/\|/g, "\\|");

function parse(lines: string[]): Model {
  const head = splitRow(lines[0]).map(fromMd);
  const sep = splitRow(lines[1] ?? "");
  const rows = lines.slice(2).map((l) => splitRow(l).map(fromMd));
  const n = Math.max(head.length, ...rows.map((r) => r.length));
  const pad = (r: string[]) => [...r, ...Array(n - r.length).fill("")];
  const align = Array.from({ length: n }, (_, j): Align => {
    const s = sep[j] ?? "";
    return s.startsWith(":") && s.endsWith(":") && s.length > 1 ? "c" : s.endsWith(":") ? "r" : s.startsWith(":") ? "l" : "";
  });
  return { cells: [pad(head), ...rows.map(pad)], align };
}

function serialize(m: Model): string[] {
  const row = (r: string[]) => `| ${r.map(toMd).join(" | ")} |`;
  const dash = (a: Align) => (a === "c" ? ":-:" : a === "r" ? "-:" : a === "l" ? ":-" : "---");
  return formatTable([row(m.cells[0]), `| ${m.align.map(dash).join(" | ")} |`, ...m.cells.slice(1).map(row)]);
}

/** 粘贴进来的一片：优先用 HTML 里的表格（Excel、网页），其次按制表符分开的文本（带引号的格子里可以有换行） */
function pastedCells(e: ClipboardEvent): string[][] | null {
  const html = e.clipboardData?.getData("text/html") ?? "";
  if (/<table/i.test(html)) {
    const doc = new DOMParser().parseFromString(html, "text/html");
    const rows = [...doc.querySelectorAll("tr")].map((tr) => [...tr.querySelectorAll("th, td")].map((td) => (td as HTMLElement).innerText ?? td.textContent ?? ""));
    if (rows.length && (rows.length > 1 || rows[0].length > 1)) return rows.map((r) => r.map((c) => c.replace(/ /g, " ").trim()));
  }
  const text = (e.clipboardData?.getData("text/plain") ?? "").replace(/\r\n?/g, "\n").replace(/\n$/, "");
  if (!text.includes("\t")) return null;
  const rows: string[][] = [];
  let row: string[] = [];
  let cell = "";
  let quoted = false;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (quoted) {
      if (ch === '"' && text[i + 1] === '"') (cell += '"'), i++;
      else if (ch === '"') quoted = false;
      else cell += ch;
    } else if (ch === '"' && cell === "") quoted = true;
    else if (ch === "\t") row.push(cell), (cell = "");
    else if (ch === "\n") row.push(cell), rows.push(row), (row = []), (cell = "");
    else cell += ch;
  }
  row.push(cell);
  rows.push(row);
  return rows;
}

const collator = new Intl.Collator("zh-CN", { numeric: true, sensitivity: "base" });

// ---------- 编辑器 ----------

export function openTableGrid(opts: Options) {
  let m = parse(opts.lines);
  const original = JSON.stringify(m);
  let undo: string[] = [];
  let redo: string[] = [];
  /** 正在改的格子开始改之前的样子：离开格子时改过了就记进撤销 */
  let editStart = "";
  let cur = { r: 1 < m.cells.length ? 1 : 0, c: 0 };
  let escArmed = false;

  const wrap = document.createElement("div");
  wrap.className = "tg-wrap";
  wrap.innerHTML = `
    <div class="tg" role="dialog" aria-label="编辑表格">
      <div class="tg-h"><b>编辑表格</b><span class="tg-size" id="tg-size"></span><span class="sp"></span>
        <button class="btn sm ghost" data-g="undo" title="撤销（Ctrl + Z）">撤销</button><button class="btn sm ghost" data-g="redo" title="重做（Ctrl + Y）">重做</button>
        <button class="btn sm ghost" data-g="cancel" title="Esc">取消</button><button class="btn sm pri" data-g="done" title="Ctrl + Enter">完成</button></div>
      <div class="tg-tools">
        <span class="grp"><i>行</i><button class="btn sm ghost" data-g="row-above">上面插一行</button><button class="btn sm ghost" data-g="row-below">下面插一行</button><button class="btn sm ghost" data-g="row-up" title="Alt + ↑">上移</button><button class="btn sm ghost" data-g="row-down" title="Alt + ↓">下移</button><button class="btn sm ghost danger" data-g="row-del">删除</button></span>
        <span class="grp"><i>列</i><button class="btn sm ghost" data-g="col-left">左边插一列</button><button class="btn sm ghost" data-g="col-right">右边插一列</button><button class="btn sm ghost" data-g="col-l" title="Alt + ←">左移</button><button class="btn sm ghost" data-g="col-r" title="Alt + →">右移</button><button class="btn sm ghost danger" data-g="col-del">删除</button></span>
        <span class="grp"><i>按这列</i><button class="btn sm ghost" data-g="sort-asc">升序</button><button class="btn sm ghost" data-g="sort-desc">降序</button></span>
        <span class="grp"><i>对齐</i><button class="btn sm ghost" data-g="al-l" data-al="l">左</button><button class="btn sm ghost" data-g="al-c" data-al="c">中</button><button class="btn sm ghost" data-g="al-r" data-al="r">右</button></span>
      </div>
      <div class="tg-scroll" id="tg-scroll"></div>
      <div class="tg-f"><span id="tg-msg">Tab 下一格 · 回车到下一行 · Shift + 回车在格子里换行 · 可以从 Excel 粘贴一片格子 · Ctrl + Enter 完成 · Esc 取消</span></div>
    </div>`;
  document.body.append(wrap);
  const scroll = wrap.querySelector<HTMLElement>("#tg-scroll")!;
  const msg = wrap.querySelector<HTMLElement>("#tg-msg")!;
  const hint = msg.textContent!;

  const snap = () => JSON.stringify(m);
  const cellEl = (r: number, c: number) => scroll.querySelector<HTMLElement>(`[data-r="${r}"][data-c="${c}"]`);
  const rows = () => m.cells.length;
  const cols = () => m.align.length;

  function render(focus = true) {
    const al = (j: number) => (m.align[j] === "c" ? "center" : m.align[j] === "r" ? "right" : "left");
    scroll.innerHTML = `<table class="tg-table"><tbody>${m.cells
      .map(
        (r, i) =>
          `<tr class="${i === 0 ? "hd" : ""}"><th class="rn">${i === 0 ? "表头" : i}</th>${r
            .map((c, j) => `<td><div class="gc" contenteditable="plaintext-only" spellcheck="false" data-r="${i}" data-c="${j}" style="text-align:${al(j)}">${esc(c)}</div></td>`)
            .join("")}</tr>`,
      )
      .join("")}</tbody></table>`;
    wrap.querySelector("#tg-size")!.textContent = `${cols()} 列 · ${rows() - 1} 行`;
    wrap.querySelectorAll<HTMLButtonElement>("[data-al]").forEach((b) => b.setAttribute("aria-pressed", String(m.align[cur.c] === b.dataset.al)));
    if (focus) place(cur.r, cur.c);
  }

  /** 光标放进某个格子（放在字的最后） */
  function place(r: number, c: number, atStart = false) {
    cur = { r: Math.max(0, Math.min(r, rows() - 1)), c: Math.max(0, Math.min(c, cols() - 1)) };
    const el = cellEl(cur.r, cur.c);
    if (!el) return;
    el.focus();
    const sel = getSelection();
    const range = document.createRange();
    range.selectNodeContents(el);
    range.collapse(atStart);
    sel?.removeAllRanges();
    sel?.addRange(range);
    el.scrollIntoView({ block: "nearest", inline: "nearest" });
    wrap.querySelectorAll<HTMLButtonElement>("[data-al]").forEach((b) => b.setAttribute("aria-pressed", String(m.align[cur.c] === b.dataset.al)));
  }

  /** 当前格子以光标实际所在的为准（不全靠 focusin：窗口在后台时浏览器不发焦点事件） */
  function syncCur() {
    const a = document.activeElement as HTMLElement | null;
    if (a?.classList.contains("gc") && wrap.contains(a)) cur = { r: Number(a.dataset.r), c: Number(a.dataset.c) };
  }

  /** 离开格子、做别的操作之前：格子里改过了就记一笔撤销 */
  function commit() {
    syncCur();
    const now = snap();
    if (editStart && editStart !== now) {
      undo.push(editStart);
      redo = [];
    }
    editStart = now;
  }

  /** 改结构（插行、删列、排序……）：先记撤销，改完重画 */
  function change(fn: () => void) {
    commit();
    const before = snap();
    fn();
    if (snap() !== before) {
      undo.push(before);
      redo = [];
    }
    editStart = snap();
    render();
  }

  const blankRow = () => Array(cols()).fill("");
  const ops: Record<string, () => void> = {
    "row-above": () => change(() => (m.cells.splice(Math.max(1, cur.r), 0, blankRow()), (cur.r = Math.max(1, cur.r)))),
    "row-below": () => change(() => (m.cells.splice(cur.r + 1, 0, blankRow()), cur.r++)),
    "row-up": () => change(() => cur.r > 1 && ([m.cells[cur.r - 1], m.cells[cur.r]] = [m.cells[cur.r], m.cells[cur.r - 1]]) && cur.r--),
    "row-down": () => change(() => cur.r >= 1 && cur.r < rows() - 1 && ([m.cells[cur.r + 1], m.cells[cur.r]] = [m.cells[cur.r], m.cells[cur.r + 1]]) && cur.r++),
    "row-del": () => {
      if (cur.r === 0) return say("表头那一行不能删，可以直接改字");
      change(() => (m.cells.splice(cur.r, 1), (cur.r = Math.min(cur.r, rows() - 1))));
    },
    "col-left": () => change(() => (m.cells.forEach((r) => r.splice(cur.c, 0, "")), m.align.splice(cur.c, 0, ""))),
    "col-right": () => change(() => (m.cells.forEach((r) => r.splice(cur.c + 1, 0, "")), m.align.splice(cur.c + 1, 0, ""), cur.c++)),
    "col-l": () =>
      change(() => {
        if (cur.c === 0) return;
        const k = cur.c;
        m.cells.forEach((r) => ([r[k - 1], r[k]] = [r[k], r[k - 1]]));
        [m.align[k - 1], m.align[k]] = [m.align[k], m.align[k - 1]];
        cur.c--;
      }),
    "col-r": () =>
      change(() => {
        if (cur.c >= cols() - 1) return;
        const k = cur.c;
        m.cells.forEach((r) => ([r[k + 1], r[k]] = [r[k], r[k + 1]]));
        [m.align[k + 1], m.align[k]] = [m.align[k], m.align[k + 1]];
        cur.c++;
      }),
    "col-del": () => {
      if (cols() <= 1) return say("至少要留一列");
      change(() => (m.cells.forEach((r) => r.splice(cur.c, 1)), m.align.splice(cur.c, 1), (cur.c = Math.min(cur.c, cols() - 1))));
    },
    "sort-asc": () => sortBy(1),
    "sort-desc": () => sortBy(-1),
    "al-l": () => setAlign("l"),
    "al-c": () => setAlign("c"),
    "al-r": () => setAlign("r"),
    undo: () => history(undo, redo),
    redo: () => history(redo, undo),
    done: () => finish(true),
    cancel: () => finish(false),
  };

  function sortBy(dir: 1 | -1) {
    syncCur();
    const k = cur.c;
    change(() => {
      const body = m.cells.slice(1);
      // 空格子总在最后
      body.sort((a, b) => (!a[k] && b[k] ? 1 : a[k] && !b[k] ? -1 : dir * collator.compare(a[k], b[k])));
      m.cells = [m.cells[0], ...body];
    });
    say(`已按「${m.cells[0][k] || `第 ${k + 1} 列`}」${dir > 0 ? "升序" : "降序"}排好`);
  }

  function setAlign(a: Align) {
    change(() => (m.align[cur.c] = m.align[cur.c] === a ? "" : a));
  }

  function history(from: string[], to: string[]) {
    commit();
    const s = from.pop();
    if (!s) return;
    to.push(snap());
    m = JSON.parse(s);
    editStart = snap();
    render();
  }

  let sayTimer = 0;
  function say(text: string) {
    msg.textContent = text;
    clearTimeout(sayTimer);
    sayTimer = window.setTimeout(() => (msg.textContent = hint), 2600);
  }

  function finish(ok: boolean) {
    commit();
    const changed = snap() !== original;
    if (!ok && changed && !escArmed) {
      escArmed = true;
      say("改过的内容会丢掉，再按一次 Esc 或「取消」确认");
      setTimeout(() => (escArmed = false), 3000);
      return;
    }
    wrap.remove();
    document.removeEventListener("keydown", onKey, true);
    if (!ok || !changed) return;
    // 末尾整行都空的（回车走到最后一行下面时加的）不写回去
    while (m.cells.length > 1 && m.cells[m.cells.length - 1].every((c) => !c.trim())) m.cells.pop();
    opts.onDone(serialize(m));
  }

  /** 光标在格子里字的开头 / 结尾 */
  function caretAt(el: HTMLElement, end: boolean): boolean {
    const sel = getSelection();
    if (!sel?.rangeCount || !sel.isCollapsed) return false;
    const r = document.createRange();
    r.selectNodeContents(el);
    if (end) r.setStart(sel.getRangeAt(0).endContainer, sel.getRangeAt(0).endOffset);
    else r.setEnd(sel.getRangeAt(0).startContainer, sel.getRangeAt(0).startOffset);
    return r.toString().length === 0;
  }

  function move(dr: number, dc: number) {
    commit();
    let { r, c } = cur;
    c += dc;
    if (c >= cols()) (c = 0), r++;
    if (c < 0) (c = cols() - 1), r--;
    r += dr;
    if (r < 0) return;
    if (r >= rows()) {
      // 最后一行再往下：加一行
      change(() => m.cells.push(blankRow()));
      r = rows() - 1;
    }
    place(r, c, dc > 0);
  }

  function onKey(e: KeyboardEvent) {
    // 开着格子编辑器时，按键都归它（不让窗口的快捷键，比如 Ctrl + W 关窗口、Ctrl + F 搜索，在底下动）
    e.stopPropagation();
    syncCur();
    const inCell = (e.target as HTMLElement).classList?.contains("gc");
    const k = e.key;
    if (k === "Escape") return e.preventDefault(), finish(false);
    if (k === "Enter" && e.ctrlKey) return e.preventDefault(), finish(true);
    if (e.ctrlKey && !e.shiftKey && (k === "z" || k === "Z")) return e.preventDefault(), ops.undo();
    if (e.ctrlKey && (k === "y" || k === "Y" || (e.shiftKey && (k === "z" || k === "Z")))) return e.preventDefault(), ops.redo();
    if (!inCell) return;
    const el = e.target as HTMLElement;
    if (e.altKey && k.startsWith("Arrow")) {
      e.preventDefault();
      return ops[{ ArrowUp: "row-up", ArrowDown: "row-down", ArrowLeft: "col-l", ArrowRight: "col-r" }[k] as string]?.();
    }
    if (k === "Tab") return e.preventDefault(), move(0, e.shiftKey ? -1 : 1);
    if (k === "Enter" && !e.shiftKey) return e.preventDefault(), move(1, 0);
    const oneLine = !el.innerText.includes("\n");
    if (k === "ArrowUp" && oneLine) return e.preventDefault(), move(-1, 0);
    if (k === "ArrowDown" && oneLine) return e.preventDefault(), move(1, 0);
    if (k === "ArrowLeft" && caretAt(el, false) && (cur.c > 0 || cur.r > 0)) return e.preventDefault(), move(0, -1);
    if (k === "ArrowRight" && caretAt(el, true)) return e.preventDefault(), move(0, 1);
  }
  document.addEventListener("keydown", onKey, true);

  scroll.addEventListener("focusin", (e) => {
    const el = e.target as HTMLElement;
    if (!el.classList.contains("gc")) return;
    cur = { r: Number(el.dataset.r), c: Number(el.dataset.c) };
    wrap.querySelectorAll<HTMLButtonElement>("[data-al]").forEach((b) => b.setAttribute("aria-pressed", String(m.align[cur.c] === b.dataset.al)));
  });
  scroll.addEventListener("focusout", () => commit());
  scroll.addEventListener("input", (e) => {
    const el = e.target as HTMLElement;
    if (!el.classList.contains("gc")) return;
    m.cells[Number(el.dataset.r)][Number(el.dataset.c)] = el.innerText.replace(/\n$/, "");
  });
  scroll.addEventListener("paste", (e) => {
    const el = e.target as HTMLElement;
    if (!el.classList?.contains("gc")) return;
    const block = pastedCells(e);
    if (!block) return;
    e.preventDefault();
    const r = Number(el.dataset.r);
    const c = Number(el.dataset.c);
    change(() => {
      const w = Math.max(...block.map((x) => x.length));
      while (cols() < c + w) (m.cells.forEach((x) => x.push("")), m.align.push(""));
      while (rows() < r + block.length) m.cells.push(blankRow());
      block.forEach((br, i) => br.forEach((v, j) => (m.cells[r + i][c + j] = v)));
    });
    say(`贴进了 ${block.length} 行 × ${Math.max(...block.map((x) => x.length))} 列`);
  });
  wrap.addEventListener("click", (e) => {
    const b = (e.target as HTMLElement).closest<HTMLElement>("[data-g]");
    if (b) {
      syncCur();
      ops[b.dataset.g!]?.();
    }
    else if (e.target === wrap) finish(false);
  });
  // 点工具条上的按钮时不要让格子失去光标
  wrap.querySelector(".tg-tools")!.addEventListener("mousedown", (e) => e.preventDefault());

  editStart = snap();
  render();
}

/** 拆格子、写回源码（单独导出给测试用） */
export { parse as parseGrid, serialize as serializeGrid };

/** 一张空表格的源码（编辑器里没有表格时，Ctrl + Alt + T 插一张再打开格子编辑） */
export const blankTable = () => serialize({ cells: [["列 1", "列 2", "列 3"], ["", "", ""]], align: ["", "", ""] });
