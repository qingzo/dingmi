// 笔记的「格式化」和「粘贴带格式的内容」：只调整空格和记号，不改字。
// 规则跟着 md.ts 的 renderMd 走：它认的才整理，免得一格式化显示效果就变了。

/** 一个字在等宽中文字体（黑体）里占几格：中文、中文标点、大多数符号两格，英文数字一格 */
function charWidth(cp: number): number {
  if (cp < 0x80) return 1;
  if (cp === 0x200b || cp === 0x200c || cp === 0x200d || cp === 0xfe0f) return 0;
  if (cp >= 0x2000) return cp >= 0xff61 && cp <= 0xffdc ? 1 : 2; // 半角片假名、半角韩文一格
  if ([0xa7, 0xa8, 0xb0, 0xb1, 0xb7, 0xd7, 0xf7].includes(cp)) return 2;
  if ((cp >= 0x391 && cp <= 0x3c9) || (cp >= 0x401 && cp <= 0x451)) return 2; // 希腊、俄文字母在黑体里是全角
  return 1;
}

export function dispWidth(s: string): number {
  let w = 0;
  for (const ch of s) w += charWidth(ch.codePointAt(0)!);
  return w;
}

/** 表格的分隔行：| --- | :---: | ---: |（和 md.ts 一样） */
const TABLE_SEP = /^\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?$/;

/** 一行拆成格子，格子里的 \| 原样留着（写回去时不用再转义） */
export const splitRow = (line: string) =>
  line
    .trim()
    .replace(/^\|/, "")
    .replace(/(?<!\\)\|$/, "")
    .split(/(?<!\\)\|/)
    .map((c) => c.trim());

/** 第 i 行开始是不是一张表格，是的话返回占几行（表头 + 分隔行 + 后面连着的带 | 的行） */
export function tableLen(lines: string[], i: number): number {
  const head = lines[i]?.trim() ?? "";
  const sep = lines[i + 1]?.trim() ?? "";
  if (!head.includes("|") || !sep.includes("|") || !TABLE_SEP.test(sep)) return 0;
  let n = 2;
  while (i + n < lines.length && lines[i + n].trim().includes("|")) n++;
  return n;
}

/** 把一张表格（几行源码）排整齐：列对齐、缺的格补上、分隔行补齐，保留每列的对齐方式 */
export function formatTable(rows: string[]): string[] {
  const head = splitRow(rows[0]);
  const sep = splitRow(rows[1]);
  const body = rows.slice(2).map(splitRow);
  const n = Math.max(head.length, sep.length, ...body.map((r) => r.length));
  const pad = (r: string[]) => [...r, ...Array(n - r.length).fill("")];
  const align = pad(sep).map((c) => (c.startsWith(":") && c.endsWith(":") && c.length > 1 ? "c" : c.endsWith(":") ? "r" : c.startsWith(":") ? "l" : ""));
  const all = [pad(head), ...body.map(pad)];
  const width = Array.from({ length: n }, (_, j) => Math.max(3, ...all.map((r) => dispWidth(r[j]))));
  const cell = (s: string, j: number) => {
    const space = width[j] - dispWidth(s);
    if (align[j] === "r") return " ".repeat(space) + s;
    if (align[j] === "c") return " ".repeat(Math.floor(space / 2)) + s + " ".repeat(Math.ceil(space / 2));
    return s + " ".repeat(space);
  };
  const line = (r: string[]) => `| ${r.map(cell).join(" | ")} |`;
  const dash = (j: number) => {
    const a = align[j];
    const w = width[j];
    return a === "c" ? `:${"-".repeat(w - 2)}:` : a === "r" ? `${"-".repeat(w - 1)}:` : a === "l" ? `:${"-".repeat(w - 1)}` : "-".repeat(w);
  };
  return [line(all[0]), `| ${width.map((_, j) => dash(j)).join(" | ")} |`, ...all.slice(1).map(line)];
}

const HEADING = /^#{1,3}\s+/;
const BULLET = /^(\s*)[-*+]\s+(.*)$/;
const TODO = /^(\s*)[-*+]\s+\[( |x|X)\]\s*(.*)$/;
/** 编号列表：1. / 1) 后面要有空格，1、可以不要；"3.14 是圆周率"这种不算 */
const ORDERED = /^(\s*)(\d+)(?:([.)])\s+|(、)\s*)(.*)$/;

/** 整篇笔记格式化：表格对齐，列表记号统一成 -，编号重排，标题前空一行，连续空行并成一行，去掉行尾空格 */
export function formatMarkdown(src: string): string {
  const lines = src.replace(/\r\n?/g, "\n").split("\n");
  const out: string[] = [];
  let fence = false;
  /** 正在排的编号列表：下一个该是几（0 表示不在编号列表里） */
  let next = 0;
  const blank = () => {
    if (out.length && out[out.length - 1] !== "") out.push("");
  };
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i].replace(/\s+$/, "");
    // ``` 包起来的一段原样留着
    if (/^\s*```/.test(raw)) {
      fence = !fence;
      out.push(raw);
      next = 0;
      continue;
    }
    if (fence) {
      out.push(lines[i]);
      continue;
    }
    const tl = tableLen(lines, i);
    if (tl) {
      blank();
      out.push(...formatTable(lines.slice(i, i + tl)));
      out.push("");
      i += tl - 1;
      next = 0;
      continue;
    }
    if (!raw.trim()) {
      blank();
      next = 0;
      continue;
    }
    const indent = (s: string) => s.replace(/\t/g, "  ");
    let m: RegExpMatchArray | null;
    if ((m = raw.match(TODO))) {
      out.push(`${indent(m[1])}- [${m[2] === " " ? " " : "x"}] ${m[3]}`.replace(/\s+$/, ""));
      next = 0;
    } else if ((m = raw.match(BULLET))) {
      out.push(`${indent(m[1])}- ${m[2]}`);
      next = 0;
    } else if ((m = raw.match(ORDERED))) {
      const num = next || Number(m[2]);
      out.push(m[4] ? `${indent(m[1])}${num}、${m[5]}` : `${indent(m[1])}${num}${m[3]} ${m[5]}`);
      next = num + 1;
    } else if (HEADING.test(raw.trim())) {
      blank();
      out.push(raw.trim().replace(/^(#{1,3})\s+/, "$1 "));
      next = 0;
    } else if (/^\s*>/.test(raw)) {
      out.push(raw.trim().replace(/^>\s*/, "> ").replace(/\s+$/, ""));
      next = 0;
    } else {
      out.push(raw);
      next = 0;
    }
  }
  while (out.length && out[out.length - 1] === "") out.pop();
  while (out.length && out[0] === "") out.shift();
  return out.join("\n");
}

// ---------- 粘贴网页、Word、Excel 里带格式的内容 ----------

const BLOCK = new Set(["P", "DIV", "SECTION", "ARTICLE", "HEADER", "FOOTER", "MAIN", "ASIDE", "NAV", "FIGURE", "FIGCAPTION", "DL", "DT", "DD", "ADDRESS"]);

/** 网页、Word、Excel 复制来的 HTML 转成 Markdown；里面没有标题、列表、表格、链接、粗体这些，就返回 null（按纯文本粘） */
export function htmlToMd(html: string): string | null {
  const doc = new DOMParser().parseFromString(html, "text/html");
  doc.querySelectorAll("script, style, meta, link, title, head, noscript, template").forEach((e) => e.remove());
  if (!doc.body.querySelector("h1, h2, h3, h4, h5, h6, ul, ol, table, a[href^='http'], strong, b, blockquote, del, s, strike, hr")) return null;
  const clean = (s: string) => s.replace(/[\s ]+/g, " ");

  /** 一段行内内容转成一行 Markdown */
  const inline = (node: Node): string => {
    if (node.nodeType === Node.TEXT_NODE) return clean(node.textContent ?? "");
    if (!(node instanceof HTMLElement)) return "";
    const kids = () => [...node.childNodes].map(inline).join("");
    const tag = node.tagName;
    if (tag === "BR") return "\n";
    if (tag === "IMG") {
      const src = node.getAttribute("src") ?? "";
      return /^https?:/.test(src) ? `[${node.getAttribute("alt") || "图片"}](${src})` : "";
    }
    // Word 粘过来的粗体常是 <b style="font-weight:normal">，按样式算
    const weight = node.style.fontWeight;
    const bold = tag === "STRONG" || (tag === "B" && weight !== "normal" && weight !== "400") || Number(weight) >= 600 || weight === "bold";
    const wrap = (mark: string) => {
      const t = kids();
      const m = t.match(/^(\s*)(.*?)(\s*)$/s)!;
      return m[2] ? `${m[1]}${mark}${m[2]}${mark}${m[3]}` : t;
    };
    if (bold) return wrap("**");
    if (tag === "DEL" || tag === "S" || tag === "STRIKE") return wrap("~~");
    if (tag === "CODE" || tag === "KBD") return `\`${node.textContent ?? ""}\``;
    if (tag === "A") {
      const href = node.getAttribute("href") ?? "";
      const t = kids().trim();
      if (!/^https?:/.test(href)) return t;
      return !t || t === href ? href : `[${t.replace(/[[\]]/g, "")}](${href})`;
    }
    return kids();
  };

  const lines: string[] = [];
  const para = (s: string) => {
    for (const l of s.split("\n")) lines.push(l.trim());
  };
  const gap = () => {
    if (lines.length && lines[lines.length - 1] !== "") lines.push("");
  };

  const block = (node: Node, depth = 0): void => {
    if (node.nodeType === Node.TEXT_NODE) {
      const t = clean(node.textContent ?? "");
      if (t.trim()) para(t);
      return;
    }
    if (!(node instanceof HTMLElement)) return;
    const tag = node.tagName;
    const hm = tag.match(/^H([1-6])$/);
    if (hm) {
      gap();
      lines.push(`${"#".repeat(Math.min(3, Number(hm[1])))} ${inline(node).replace(/\n/g, " ").trim()}`);
      gap();
    } else if (tag === "UL" || tag === "OL") {
      gap();
      let k = Number(node.getAttribute("start")) || 1;
      for (const li of node.children) {
        if (li.tagName !== "LI") continue;
        const own = [...li.childNodes].filter((c) => !(c instanceof HTMLElement && (c.tagName === "UL" || c.tagName === "OL")));
        const text = own.map(inline).join("").replace(/\s*\n\s*/g, " ").trim();
        const box = li.querySelector(":scope > input[type=checkbox]") as HTMLInputElement | null;
        const mark = box ? `- [${box.checked ? "x" : " "}] ` : tag === "OL" ? `${k++}. ` : "- ";
        lines.push(`${"  ".repeat(depth)}${mark}${text}`);
        li.querySelectorAll(":scope > ul, :scope > ol").forEach((sub) => blockList(sub as HTMLElement, depth + 1));
      }
      gap();
    } else if (tag === "TABLE") {
      gap();
      const rows = [...node.querySelectorAll("tr")].map((tr) =>
        [...tr.querySelectorAll("th, td")].map((c) =>
          inline(c)
            .trim()
            .replace(/\s*\n\s*/g, "<br>")
            .replace(/\|/g, "\\|"),
        ),
      );
      if (rows.length) {
        const n = Math.max(...rows.map((r) => r.length));
        const row = (r: string[]) => `| ${[...r, ...Array(n - r.length).fill("")].join(" | ")} |`;
        lines.push(...formatTable([row(rows[0]), `| ${Array(n).fill("---").join(" | ")} |`, ...rows.slice(1).map(row)]));
      }
      gap();
    } else if (tag === "BLOCKQUOTE") {
      gap();
      const inner = htmlToMdLines(node);
      for (const l of inner) if (l) lines.push(`> ${l}`);
      gap();
    } else if (tag === "PRE") {
      gap();
      for (const l of (node.textContent ?? "").replace(/\n$/, "").split("\n")) lines.push(l);
      gap();
    } else if (tag === "HR") {
      gap();
      lines.push("---");
      gap();
    } else if (BLOCK.has(tag) || tag === "BODY" || tag === "LI" || tag === "TBODY") {
      // 块里混着行内的字和别的块：行内的攒成一段，碰到块就先放下
      let buf = "";
      const flush = () => {
        if (buf.trim()) para(buf);
        buf = "";
      };
      for (const c of node.childNodes) {
        if (c instanceof HTMLElement && (BLOCK.has(c.tagName) || /^(H[1-6]|UL|OL|TABLE|BLOCKQUOTE|PRE|HR)$/.test(c.tagName))) {
          flush();
          block(c, depth);
        } else buf += inline(c);
      }
      flush();
      if (BLOCK.has(tag) && tag !== "DIV") gap();
    } else {
      const t = inline(node);
      if (t.trim()) para(t);
    }
  };
  const blockList = (el: HTMLElement, depth: number) => {
    const at = lines.length;
    block(el, depth);
    // 嵌套的列表紧跟在上一项下面，不要空行
    if (lines[at] === "") lines.splice(at, 1);
    if (lines[lines.length - 1] === "") lines.pop();
  };
  /** 引用块里面的内容单独转一遍，再在每行前面加 > */
  const htmlToMdLines = (el: HTMLElement) => {
    const saved = lines.splice(0);
    const box = document.createElement("div");
    box.append(...[...el.childNodes].map((c) => c.cloneNode(true)));
    block(box);
    const got = lines.splice(0);
    lines.push(...saved);
    return got;
  };

  block(doc.body);
  const text = lines
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
  return text || null;
}
