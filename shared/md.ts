import { convertFileSrc } from "@tauri-apps/api/core";
import type { TableState, TableView } from "./tables";
import { esc } from "./util";

const escRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

// 笔记里的图片读不到（别的电脑传来的还在后台下载）：先显示「图片下载中」，下载完界面会刷新。
// 不写成 img 标签上的 onerror 属性：页面的 CSP 不许内联的事件处理（见 tauri.conf.json、服务器给手机网页的 CSP 头）
if (typeof document !== "undefined") {
  document.addEventListener(
    "error",
    (e) => {
      const img = e.target;
      if (img instanceof HTMLImageElement && img.matches("p.att > img[data-att]")) img.parentElement?.classList.add("wait");
    },
    true,
  );
}

/** 图片文件所在的目录，主窗口启动时设好 */
let attDir = "";
export const setAttachmentDir = (dir: string) => (attDir = dir);
export const attachmentSrc = (file: string) => (attDir ? convertFileSrc(`${attDir}\\${file}`) : "");

/** [标题](网址) 或者光秃秃的网址；点了用浏览器打开（data-href），不在窗口里跳转 */
const LINK = /\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)|(https?:\/\/[^\s<>"，。！？、；：」』）】)]+)/g;
const IMAGE = /^!\[([^\]]*)\]\(att:([\w.-]+)\)$/;
/** 单独一行的 [原文件名](att:文件名)：文件附件 */
const FILE = /^\[([^\]]+)\]\(att:([\w.-]+)\)$/;

/** 表格右上角要不要放「格子编辑」按钮（只有主窗口要，手机版、预览不要） */
let gridButton = false;
export const setTableGridButton = (on: boolean) => (gridButton = on);
const gridBtn = (n: number) => (gridButton ? `<button type="button" class="tgrid" data-tgrid="${n}" title="像电子表格一样改这张表格：增删行列、排序、从 Excel 粘贴">格子编辑</button>` : "");

/** 文件卡片上显示的大小、是否已下载；主窗口和手机版各自提供 */
export interface FileInfo {
  size: number;
  /** 原文件在不在本机（手机版：解开过没有） */
  local: boolean;
  /** 原文件只在某一台设备上（叮咪同步的套餐不传太大的）：给人看的一句，比如「只在「公司电脑」上」；没有是 undefined */
  away?: string;
}
let fileInfo: (file: string) => FileInfo | undefined = () => undefined;
export const setFileInfo = (f: typeof fileInfo) => (fileInfo = f);
/** 附件的大小、在不在本机（电脑端的笔记编辑器画文件卡片用） */
export const fileInfoOf = (file: string) => fileInfo(file);

/** 只在别的设备上的图片：读不到时显示这句，不显示「正在下载」 */
const awayAttr = (file: string) => {
  const away = fileInfo(file)?.away;
  return away ? ` data-away="${esc(away)}"` : "";
};

export function fileSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  return `${(n / 1024 / 1024).toFixed(n < 10 * 1024 * 1024 ? 1 : 0)} MB`;
}

/** 文件卡片：扩展名小方块、文件名、大小。点了打开（没下载的先下载），data-file 是存在本机的文件名 */
/** 文件卡片旁边要不要放「另存为…」（只有电脑上要，手机网页存不了） */
let fileSave = false;
export const setFileSaveButton = (on: boolean) => (fileSave = on);

export function fileCard(name: string, file: string): string {
  const ext = (name.match(/\.([a-z0-9]{1,5})$/i)?.[1] ?? "file").toUpperCase();
  const info = fileInfo(file);
  const meta = info ? `${fileSize(info.size)}${info.away ? ` · ${info.away}` : info.local ? "" : " · 点开时下载"}` : "";
  const tip = fileSave ? "点一下打开。打开的是一份副本，改了不会存回笔记；要留着改，用旁边的「另存为…」" : "点一下打开";
  const card = `<button type="button" class="file-card" data-file="${esc(file)}" title="${tip}"><span class="fx">${esc(ext)}</span><span class="fn">${esc(name)}</span><span class="fm">${meta}</span></button>`;
  return fileSave ? `${card}<button type="button" class="file-save" data-saveas="${esc(file)}" title="用原来的名字存一份到你选的地方">另存为…</button>` : card;
}

/** 在 HTML 片段里给搜索词加 <mark>，跳过标签本身 */
export function highlight(html: string, q: string): string {
  if (!q) return html;
  const re = new RegExp(escRe(esc(q)), "gi");
  return html
    .split(/(<[^>]+>)/)
    .map((part) => (part.startsWith("<") ? part : part.replace(re, (m) => `<mark>${m}</mark>`)))
    .join("");
}

const ICON_COPY = '<svg viewBox="0 0 24 24"><rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15V5a2 2 0 0 1 2-2h10"/></svg>';
const ICON_OUT = '<svg viewBox="0 0 24 24"><path d="M7 17 17 7M9 7h8v8"/></svg>';

/** 复制按钮（鼠标移上去才出现）；text 是要复制的原文 */
const copyBtn = (text: string) => `<button type="button" class="cp" data-copy="${esc(text)}" title="复制" aria-label="复制">${ICON_COPY}</button>`;

/** 链接：光秃秃的网址只显示「主机:端口 ↗」，鼠标停上去看完整的；后面跟复制按钮（复制完整网址）。
 *  url 是已经转义过的文字 */
function link(url: string, text: string | undefined, copy: boolean): string {
  const raw = url.replace(/&amp;/g, "&");
  let label = text;
  if (!label) {
    try {
      label = esc(new URL(raw).host) + ICON_OUT;
    } catch {
      label = url;
    }
  }
  const a = `<a class="lnk" data-href="${url}" title="${url}">${label}</a>`;
  return copy ? `<span class="lnkw">${a}${copyBtn(raw)}</span>` : a;
}

/** 行内的写法：链接、行内代码、粗体、删除线。copy：链接后面要不要复制按钮（表格格子里整格有一个，不重复） */
export function inline(s: string, copy = true): string {
  return esc(s)
    .replace(LINK, (_, text, url, bare) => link(url ?? bare, text, copy))
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<b>$1</b>")
    .replace(/~~([^~]+)~~/g, "<s>$1</s>");
}

export interface Todo {
  /** 在正文里是第几行（从 0 开始） */
  line: number;
  text: string;
  done: boolean;
}

const TODO = /^[-*]\s+\[( |x|X)\]\s*(.*)$/;

/** 正文里所有的待办 */
export const todos = (src: string): Todo[] =>
  src.split("\n").flatMap((raw, line) => {
    const m = raw.trim().match(TODO);
    return m ? [{ line, text: m[2].trim(), done: m[1] !== " " }] : [];
  });

/** 把第 line 行的待办勾上 / 取消，返回新的正文；那一行不是待办就原样返回 */
export function setTodo(src: string, line: number, done: boolean): string {
  const lines = src.split("\n");
  if (lines[line] !== undefined) lines[line] = lines[line].replace(/^(\s*[-*]\s+\[)[ xX](\])/, `$1${done ? "x" : " "}$2`);
  return lines.join("\n");
}

/** 表格的分隔行：| --- | :---: | ---: | */
const TABLE_SEP = /^\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?$/;

/** 表格的一行拆成格子：去掉两头的 |，\| 是格子里的竖线 */
const cells = (line: string) =>
  line
    .replace(/^\|/, "")
    .replace(/(?<!\\)\|$/, "")
    .split(/(?<!\\)\|/)
    .map((c) => c.trim().replace(/\\\|/g, "|"));

/** 正文里的一张表格：从第几行开始、一共占几行、表头、对齐、每一行的格子 */
export interface TableBlock {
  start: number;
  lines: number;
  titles: string[];
  align: string[];
  rows: string[][];
}

/** 第 i 行开始是不是一张表格（表头 + 分隔行） */
function tableAt(lines: string[], i: number): TableBlock | null {
  const head = lines[i].trim();
  const sep = lines[i + 1]?.trim() ?? "";
  if (!head.includes("|") || !sep.includes("|") || !TABLE_SEP.test(sep)) return null;
  const rows: string[][] = [];
  let used = 2;
  for (; i + used < lines.length && lines[i + used].trim().includes("|"); used++) rows.push(cells(lines[i + used].trim()));
  const align = cells(sep).map((c) => (c.startsWith(":") && c.endsWith(":") ? "center" : c.endsWith(":") ? "right" : ""));
  return { start: i, lines: used, titles: cells(head), align, rows };
}

/** 正文里所有的表格（和 renderMd 认的一样），改表格里的一行时用 */
export function findTables(src: string): TableBlock[] {
  const lines = src.split("\n");
  const out: TableBlock[] = [];
  for (let i = 0; i < lines.length; i++) {
    const t = tableAt(lines, i);
    if (t) {
      out.push(t);
      i += t.lines - 1;
    }
  }
  return out;
}

/** 格子里的内容：<br> 换行；链接不单独带复制按钮 */
const cellHtml = (c: string) => inline(c, false).replace(/&lt;br\s*\/?&gt;/gi, "<br>");

/** 格子的纯文字（复制、搜索用）：换行还原成真的换行，链接只留网址，去掉粗体、代码记号 */
export const cellText = (c: string) =>
  c
    .replace(/<br\s*\/?>/gi, "\n")
    .replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g, "$2")
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/`([^`]+)`/g, "$1")
    .trim();

const ICON_CHEV = '<svg viewBox="0 0 24 24"><path d="m9 6 6 6-6 6"/></svg>';
const ICON_FIND = '<svg viewBox="0 0 24 24"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>';
/** 表格视图里超过这么多字的格子先显示两行 */
const LONG_CELL = 40;
/** 超过这么多行的表格上面有搜索框 */
const SEARCH_ROWS = 8;
/** 收起时最多显示几列 */
export const MAX_SUM = 3;

/** 收起时默认显示的列：前两列，再加一列像 IP、电话、日期这样一眼要看的 */
export function defaultSum(titles: string[]): number[] {
  const k = titles.findIndex((t, i) => i >= 2 && /ip|电话|手机|邮箱|账号|日期|时间|到期|价格|金额/i.test(t));
  return [0, 1, k].filter((x) => x >= 0 && x < titles.length);
}

/** 一张表格用哪种样子：超过 4 列时，不超过 4 行是卡片，更多行是列表；4 列以内是表格 */
export function defaultView(t: TableBlock): TableView {
  if (t.titles.length <= 4) return "table";
  return t.rows.length > 4 ? "list" : "cards";
}

/** 画一张表格。n 是第几张，s 是它现在的样子和状态（tables.ts）。电脑端的笔记编辑器单独画一张表格时也用 */
export function renderTable(t: TableBlock, n: number, s: TableState): string {
  const { titles, align } = t;
  const q = (s.q ?? "").trim();
  const ql = q.toLowerCase();
  const hl = (html: string) => (q ? highlight(html, q) : html);
  const withCopy = (c: string) => (c.trim() ? `<span class="cv">${hl(cellHtml(c))}</span>${copyBtn(cellText(c))}` : "");
  const items = t.rows.map((r, i) => ({ r, i })).filter(({ r }) => !ql || r.some((c) => cellText(c).toLowerCase().includes(ql)));

  // 两列的小表格：只有表格，不要工具条
  if (titles.length < 3) {
    const cell = (tag: string, c: string, k: number) =>
      `<${tag}${align[k] ? ` style="text-align:${align[k]}"` : ""}>${tag === "th" ? cellHtml(c) : withCopy(c)}</${tag}>`;
    return `<div class="tblock">${gridBtn(n)}<div class="tbl"><table><thead><tr>${titles.map((c, k) => cell("th", c, k)).join("")}</tr></thead><tbody>${t.rows
      .map((r) => `<tr>${titles.map((_, k) => cell("td", r[k] ?? "", k)).join("")}</tr>`)
      .join("")}</tbody></table></div></div>`;
  }

  const view = s.view ?? defaultView(t);
  const sum = (s.sum ?? defaultSum(titles)).filter((k) => k < titles.length);
  const group = s.group != null && s.group < titles.length ? s.group : null;
  const kv = (k: number, c: string) => `<div class="kv"><span>${cellHtml(titles[k])}</span><span>${withCopy(c)}</span></div>`;

  // 展开的一行：其余字段按原来的顺序；正在改的是一个表单
  const detail = (r: string[], i: number) => {
    if (s.editing === i) {
      const vals = s.draft ?? titles.map((_, k) => r[k] ?? "");
      return `<div class="tform" data-tform="${n}" data-i="${i}">${titles
        .map((h, k) => `<label><span>${cellHtml(h)}</span><textarea data-tf="${k}" rows="1">${esc((vals[k] ?? "").replace(/<br\s*\/?>/gi, "\n"))}</textarea></label>`)
        .join("")}<div class="tacts"><button type="button" class="btn sm pri" data-tact="save" data-t="${n}" data-i="${i}">保存</button><button type="button" class="btn sm ghost" data-tact="cancel" data-t="${n}">取消</button><span class="sp"></span>${
        i < t.rows.length ? `<button type="button" class="btn sm ghost danger${s.delArm ? " sure" : ""}" data-tact="del" data-t="${n}" data-i="${i}">${s.delArm ? "再点一次删掉" : "删掉这行"}</button>` : ""
      }</div></div>`;
    }
    return `<div class="tdetail">${titles.map((_, k) => (r[k]?.trim() && k !== sum[0] && k !== sum[1] ? kv(k, r[k]) : "")).join("")}<div class="tacts"><button type="button" class="btn sm" data-tact="edit" data-t="${n}" data-i="${i}">改</button><button type="button" class="btn sm ghost" data-tact="dup" data-t="${n}" data-i="${i}">复制一行</button></div></div>`;
  };

  const cols = `14px ${sum.map((_, j) => (j === 0 ? "minmax(80px, 1.1fr)" : "minmax(0, 1.4fr)")).join(" ")}`;
  const rowHtml = (r: string[], i: number) => {
    const open = s.editing === i || (s.open ?? []).includes(i) || !!q;
    const summary = sum.map((k, j) => (j === 0 ? `<b>${hl(cellHtml(r[k] ?? ""))}</b>` : `<span>${hl(cellHtml((r[k] ?? "").replace(/<br\s*\/?>/gi, " ")))}</span>`)).join("");
    return `<div class="mrow${open ? " open" : ""}"><button type="button" class="mrh" data-trow="${n}" data-i="${i}" style="grid-template-columns:${cols}">${ICON_CHEV}${summary}</button>${open ? detail(r, i) : ""}</div>`;
  };

  const listView = (list: typeof items) => `<div class="mrows">${list.map(({ r, i }) => rowHtml(r, i)).join("")}</div>`;
  const cardsView = (list: typeof items) =>
    `<div class="cards">${list
      .map(({ r }) => `<div class="tcard"><div class="ct"><b>${hl(cellHtml(r[0] ?? ""))}</b>${r[1]?.trim() ? `<small>${hl(cellHtml(r[1]))}</small>` : ""}</div>${titles.map((_, k) => (k >= 2 && r[k]?.trim() ? kv(k, r[k]) : "")).join("")}</div>`)
      .join("")}</div>`;
  const tableView = (list: typeof items) => {
    const td = (c: string, i: number, k: number) => {
      const key = `${i}:${k}`;
      const long = cellText(c).length > LONG_CELL && !(s.more ?? []).includes(key);
      const inner = c.trim() ? `<div${long ? ' class="clamp"' : ""}>${withCopy(c)}</div>${long ? `<button type="button" class="tmore" data-tmore="${n}" data-key="${key}">展开</button>` : ""}` : "";
      return `<td${align[k] ? ` style="text-align:${align[k]}"` : ""}>${inner}</td>`;
    };
    return `<div class="tbl tbl-big"><table><thead><tr>${titles.map((c, k) => `<th${align[k] ? ` style="text-align:${align[k]}"` : ""}>${cellHtml(c)}</th>`).join("")}</tr></thead><tbody>${list
      .map(({ r, i }) => `<tr>${titles.map((_, k) => td(r[k] ?? "", i, k)).join("")}</tr>`)
      .join("")}</tbody></table></div>`;
  };
  const draw = { list: listView, cards: cardsView, table: tableView }[view];

  // 按某一列分组，每组可以收起
  let body: string;
  if (!items.length) body = `<div class="tempty">没有含「${esc(q)}」的行</div>`;
  else if (group === null) body = draw(items);
  else {
    const groups = new Map<string, typeof items>();
    for (const it of items) {
      const g = cellText(it.r[group] ?? "") || "（空）";
      groups.set(g, [...(groups.get(g) ?? []), it]);
    }
    body = [...groups]
      .map(([g, list]) => {
        const closed = (s.closed ?? []).includes(g);
        return `<button type="button" class="tgh${closed ? " closed" : ""}" data-tgh="${n}" data-g="${esc(g)}">${ICON_CHEV}${esc(g)}<small>${list.length}</small></button>${closed ? "" : draw(list)}`;
      })
      .join("");
  }
  // 加一行：列表里、没在搜索时
  const adding = s.editing === t.rows.length;
  if (view === "list" && !q) {
    body += adding
      ? `<div class="mrows tnew"><div class="mrow open">${detail([], t.rows.length)}</div></div>`
      : `<button type="button" class="btn sm ghost tadd" data-tact="add" data-t="${n}">＋ 加一行</button>`;
  }

  const menu =
    s.menu === "group"
      ? `<div class="tmenu"><div class="h">按哪一列分组</div><button type="button" data-tgroup="${n}" data-k="" aria-checked="${group === null}">不分组</button>${titles
          .map((h, k) => `<button type="button" data-tgroup="${n}" data-k="${k}" aria-checked="${group === k}">${cellHtml(h)}</button>`)
          .join("")}</div>`
      : s.menu === "sum"
        ? `<div class="tmenu"><div class="h">收起时显示哪几列（最多 ${MAX_SUM} 列）</div>${titles
            .map((h, k) => `<button type="button" data-tsum="${n}" data-k="${k}" aria-checked="${sum.includes(k)}">${cellHtml(h)}</button>`)
            .join("")}</div>`
        : "";
  const pick = (m: "group" | "sum", label: string) =>
    `<span class="tpick"><button type="button" class="tpickbtn" data-tmenu="${n}" data-m="${m}" aria-expanded="${s.menu === m}">${label} ▾</button>${s.menu === m ? menu : ""}</span>`;
  const bar = `<div class="tbar">${
    t.rows.length > SEARCH_ROWS ? `<label class="tfind">${ICON_FIND}<input data-tq="${n}" placeholder="搜这张表" value="${esc(q)}" autocomplete="off"></label>` : ""
  }<span class="tcount">${q ? `${items.length} / ${t.rows.length} 行` : `${t.rows.length} 行`}</span><span class="sp"></span>${pick("group", group === null ? "分组" : `分组：${esc(cellText(titles[group]))}`)}${
    view === "list" ? pick("sum", "摘要") : ""
  }<span class="seg">${(["list", "cards", "table"] as const)
    .map((v) => `<button type="button" data-tview="${n}" data-v="${v}" aria-pressed="${view === v}">${{ list: "列表", cards: "卡片", table: "表格" }[v]}</button>`)
    .join("")}</span></div>`;
  // 「格子编辑」放在表格工具条的最右边
  return `<div class="tblock" data-tblock="${n}">${bar.replace(/<\/div>$/, gridBtn(n) + "</div>")}${body}</div>`;
}

/**
 * 一个够用的 Markdown 子集：标题、列表、待办、引用、表格（格子里 <br> 换行）、粗体、行内代码、链接、
 * 图片（单独一行的 ![](att:文件名)）、文件（单独一行的 [原文件名](att:文件名)）。
 * todoExtra：每条待办后面再加点东西（笔记本里的「设提醒」）；待办的勾选框带 data-box="行号"。
 * 每一块（段落、列表项、标题、表格……）带 data-ln="原文第几行"，双击进编辑时把光标放过去
 * tables：每张表格现在的样子和状态（tables.ts 的 tableStates），表格上的按钮都带 data-t* 和第几张
 */
export function renderMd(src: string, q = "", todoExtra?: (t: Todo) => string, tables?: (n: number) => TableState): string {
  let out = "";
  let list: "ul" | "ol" | null = null;
  const close = () => {
    if (list) out += `</${list}>`;
    list = null;
  };
  const lines = src.split("\n");
  let nth = 0;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    let m: RegExpMatchArray | null;
    const tbl = tableAt(lines, i);
    if (tbl) {
      close();
      out += renderTable(tbl, nth, tables?.(nth) ?? {}).replace(/^<(\w+)/, `<$1 data-ln="${i}"`);
      nth++;
      i += tbl.lines - 1;
    } else if ((m = line.match(TODO))) {
      if (list !== "ul") (close(), (out += '<ul class="todo">'), (list = "ul"));
      const t = { line: i, text: m[2].trim(), done: m[1] !== " " };
      out += `<li class="${t.done ? "checked" : ""}" data-ln="${i}"><span class="box" data-box="${i}" title="${t.done ? "取消勾选" : "勾上"}"></span><span class="tt">${inline(m[2])}</span>${todoExtra?.(t) ?? ""}</li>`;
    } else if ((m = line.match(/^[-*]\s+(.*)$/))) {
      if (list !== "ul") (close(), (out += "<ul>"), (list = "ul"));
      out += `<li data-ln="${i}">${inline(m[1])}</li>`;
    } else if ((m = line.match(/^\d+[.、)]\s*(.*)$/))) {
      if (list !== "ol") (close(), (out += "<ol>"), (list = "ol"));
      out += `<li data-ln="${i}">${inline(m[1])}</li>`;
    } else {
      close();
      // 别的电脑传来的图片可能还在后台下载，读不到时先显示"图片下载中"，下载完界面会刷新
      if ((m = line.match(FILE))) out += `<p class="att-file" data-ln="${i}">${fileCard(m[1], m[2])}</p>`;
      else if ((m = line.match(IMAGE)))
        out += `<p class="att" data-ln="${i}"${awayAttr(m[2])}><img data-att="${esc(m[2])}" src="${esc(attachmentSrc(m[2]))}" alt="${esc(m[1])}" loading="lazy" title="点一下看原图"></p>`;
      else if ((m = line.match(/^(#{1,3})\s+(.*)$/))) out += `<h5 data-ln="${i}">${inline(m[2])}</h5>`;
      else if ((m = line.match(/^>\s?(.*)$/))) out += `<blockquote data-ln="${i}">${inline(m[1])}</blockquote>`;
      else if (line) out += `<p data-ln="${i}">${inline(line)}</p>`;
    }
  }
  close();
  return highlight(out, q);
}

/** 去掉 Markdown 记号，得到一行纯文本 */
export const plain = (s: string) =>
  s
    // 表格：去掉分隔行，竖线、格子里的换行换成空格
    .replace(/<br\s*\/?>/gi, " ")
    .replace(/^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)+\|?\s*$/gm, "")
    .replace(/\s*\|\s*/g, " ")
    .replace(/!\[[^\]]*\]\(att:[\w.-]+\)/g, "[图片]")
    .replace(/\[([^\]]+)\]\(att:[\w.-]+\)/g, "[文件] $1")
    .replace(/\[([^\]]+)\]\(https?:\/\/[^\s)]+\)/g, "$1")
    .replace(/^\s*(#{1,3}|>|[-*](\s+\[[ xX]\])?|\d+[.、)])\s*/gm, "")
    .replace(/[`*~]/g, "")
    .replace(/\s*\n\s*/g, " ")
    .trim();

/** AI 整理后的正文是不是比原来少了一大截（去掉格式和空白后不到六成），可能漏了内容 */
export const shrunkMuch = (before: string, after: string) => {
  const len = (s: string) => plain(s).replace(/\s/g, "").length;
  return len(after) < len(before) * 0.6;
};

/** 搜索结果里的一小段上下文 */
export function snippet(body: string, q: string, len = 40): string {
  const text = plain(body);
  if (!q) return esc(text.slice(0, len));
  const i = text.toLowerCase().indexOf(q.toLowerCase());
  if (i < 0) return esc(text.slice(0, len));
  const start = Math.max(0, i - 12);
  return (start ? "…" : "") + highlight(esc(text.slice(start, i + q.length + 30)), q);
}
