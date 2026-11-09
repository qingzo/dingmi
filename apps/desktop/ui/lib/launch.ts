// 找程序（名字、拼音、首字母、别名）、排顺序、画图标。快捷框、桌面组件的「常用」、设置页共用。
// 列表和图标来自 launcher.rs
import { type LaunchConfig, type LaunchItem, type LaunchStats } from "./api";
import { icon } from "@shared/icons";
import { esc } from "@shared/util";

export const EMPTY_STATS: LaunchStats = { n: {}, last: {}, q: {} };

const SRC: Partial<Record<LaunchItem["kind"], string>> = { app: "start", store: "store", desk: "desktop" };

/** 快捷框里能看到的：去掉卸载、帮助这类，去掉自己不想看的、关掉的来源 */
export function visible(items: LaunchItem[], cfg: LaunchConfig): LaunchItem[] {
  const hidden = new Set(cfg.hidden);
  return items.filter((i) => !i.junk && !hidden.has(i.id) && !cfg.off.includes(SRC[i.kind] ?? ""));
}

// ---------- 匹配 ----------

/**
 * 从第 start 段开始，q 能不能一段一段对上：每段可以打全、只打第一个字母，最后一段可以只打开头几个字母。
 * 「wx」「weixin」「weix」「wxin」都能对上「微信」；多音字哪个读音都行
 */
function pyFrom(q: string, pys: string[][], start: number): boolean {
  const go = (i: number, j: number): boolean => {
    if (j === q.length) return true;
    if (i === pys.length) return false;
    const rest = q.slice(j);
    return pys[i].some(
      (r) => r.startsWith(rest) || (r.length > 1 && rest.startsWith(r) && go(i + 1, j + r.length)) || (r[0] === q[j] && go(i + 1, j + 1)),
    );
  };
  return go(start, 0);
}

/** 正好是每段的第一个字母：「wx」→ 微信，「vsc」→ Visual Studio Code */
const isInitials = (q: string, pys: string[][]) => q.length > 1 && q.length === pys.length && pys.every((r, i) => r.some((x) => x[0] === q[i]));

export interface Hit {
  it: LaunchItem;
  score: number;
  /** 为什么对上的：别名 / 拼音首字母 / 首字母 / 拼音 / 上次打开的 */
  why: string;
}

/**
 * 分数：100 名字或别名完全一样，96 正好是首字母，88 名字开头，85 别名开头，82 拼音或单词从头对上，68 从中间某个字（词）开始对上，
 * 62 名字里有这几个字。再加上用得多的、上次打这几个字开的就是它
 */
export function score(q: string, it: LaunchItem, aliases: string[], stats: LaunchStats): Hit | null {
  const name = it.name.toLowerCase();
  const han = /[㐀-鿿]/.test(it.name);
  const flat = q.replace(/\s+/g, "");
  let s = 0;
  let why = "";
  const hit = (v: number, label = "") => {
    if (v > s) [s, why] = [v, label];
  };
  if (name === q) hit(100);
  if (aliases.includes(q)) hit(100, "别名");
  if (aliases.some((a) => a.startsWith(q))) hit(85, "别名");
  if (flat && isInitials(flat, it.pys)) hit(96, han ? "拼音首字母" : "首字母");
  if (name.startsWith(q)) hit(88);
  if (flat && pyFrom(flat, it.pys, 0)) hit(82, han && !name.startsWith(q) ? "拼音" : "");
  if (name.includes(q)) hit(62);
  if (flat && s < 68) for (let i = 1; i < it.pys.length; i++) if (pyFrom(flat, it.pys, i)) hit(68, han && !name.includes(q) ? "拼音" : "");
  if (!s) return null;
  s += Math.min(14, Math.log2(used(it, stats) + 1) * 2);
  if (stats.q[q]?.[0] === it.id) {
    s += 40;
    why = why ? `${why} · 上次打开的` : "上次打开的";
  }
  return { it, score: s, why };
}

/** 用得多不多：在快捷框里打开过几次，加上 Windows 记的次数（打个折，免得盖过在这里的习惯） */
const used = (it: LaunchItem, stats: LaunchStats) => (stats.n[it.id] ?? 0) + Math.min(it.runs, 60) / 4;

/** 按分数排好，最多 limit 个 */
export function rank(q: string, items: LaunchItem[], cfg: LaunchConfig, stats: LaunchStats, limit = 6): Hit[] {
  const ql = q.trim().toLowerCase();
  if (!ql) return [];
  const al = new Map<string, string[]>();
  for (const a of cfg.aliases) al.set(a.id, [...(al.get(a.id) ?? []), a.alias.toLowerCase()]);
  return items
    .map((it) => score(ql, it, al.get(it.id) ?? [], stats))
    .filter((h): h is Hit => !!h)
    .sort((a, b) => b.score - a.score || a.it.name.length - b.it.name.length)
    .slice(0, limit);
}

/** 最近用过的几个（没用过的不算） */
export function recent(items: LaunchItem[], stats: LaunchStats, n = 8): LaunchItem[] {
  return items
    .filter((i) => stats.last[i.id])
    .sort((a, b) => stats.last[b.id] - stats.last[a.id])
    .slice(0, n);
}

/**
 * 输入框空着时显示的那一行：放在「常用」里的在前（和桌面组件共用），再是最近用过的；
 * 不够的按用得多少补满（刚开始用时就是 Windows 记的最常用的）；一样多的桌面上的、自己加的在前，Windows 文件夹里的系统工具最后，叮咪自己不算
 */
export function firstRow(items: LaunchItem[], cfg: LaunchConfig, stats: LaunchStats, n = 8): LaunchItem[] {
  const byId = new Map(items.map((i) => [i.id, i]));
  const out: LaunchItem[] = [];
  const add = (it?: LaunchItem) => {
    if (it && !it.missing && out.length < n && !out.includes(it)) out.push(it);
  };
  cfg.pins.forEach((id) => add(byId.get(id)));
  recent(items, stats, n).forEach(add);
  const order = (i: LaunchItem) =>
    i.id.startsWith("desk:") ? 0 : /^(path|url):/.test(i.id) ? 1 : i.kind === "store" || !/^(c:\\windows|::)/i.test(i.path) ? 2 : 3;
  items
    .filter((i) => i.name !== "叮咪")
    .sort((a, b) => used(b, stats) - used(a, stats) || order(a) - order(b) || a.name.localeCompare(b.name, "zh"))
    .forEach(add);
  return out;
}

/** 名字里的这几个字标出来 */
export function markName(name: string, q: string): string {
  const i = q ? name.toLowerCase().indexOf(q.toLowerCase()) : -1;
  if (i < 0) return esc(name);
  return esc(name.slice(0, i)) + "<mark>" + esc(name.slice(i, i + q.length)) + "</mark>" + esc(name.slice(i + q.length));
}

/** 和 pyFrom 一样一段一段对，回每段用掉了几个字母（对不上是 null） */
function pyPath(q: string, pys: string[][], start: number): number[] | null {
  const go = (i: number, j: number): number[] | null => {
    if (j === q.length) return [];
    if (i === pys.length) return null;
    const rest = q.slice(j);
    for (const r of pys[i]) {
      if (r.startsWith(rest)) return [rest.length];
      const whole = r.length > 1 && rest.startsWith(r) ? go(i + 1, j + r.length) : null;
      if (whole) return [r.length, ...whole];
      const first = r[0] === q[j] ? go(i + 1, j + 1) : null;
      if (first) return [1, ...first];
    }
    return null;
  };
  return go(start, 0);
}

/** 名字里每一段在哪（和 launcher.rs pinyin() 一样分：连着的英文数字一段，小写后面跟大写分开，一个汉字一段，别的不算） */
function segments(name: string): [number, number][] {
  const out: [number, number][] = [];
  let i = 0;
  for (const ch of name) {
    const end = i + ch.length;
    const last = out[out.length - 1];
    const prev = name[i - 1] ?? "";
    if (/[a-z0-9]/i.test(ch)) {
      if (last && last[1] === i && /[a-z0-9]/i.test(prev) && !(/[a-z]/.test(prev) && /[A-Z]/.test(ch))) last[1] = end;
      else out.push([i, end]);
    } else if (/\p{Script=Han}/u.test(ch)) out.push([i, end]);
    i = end;
  }
  return out;
}

/** 标出对上的字：名字里有这几个字就标这几个字，拼音、首字母对上的标那几个字（「wx」→ 微信两个字都标） */
export function markHit(it: Pick<LaunchItem, "name" | "pys">, q: string): string {
  const { name, pys } = it;
  const flat = q.trim().toLowerCase().replace(/\s+/g, "");
  if (!flat || name.toLowerCase().includes(q.trim().toLowerCase())) return markName(name, q.trim());
  const segs = segments(name);
  if (segs.length !== pys.length) return esc(name);
  for (let s = 0; s < pys.length; s++) {
    const path = pyPath(flat, pys, s);
    if (!path) continue;
    const on = new Array<boolean>(name.length).fill(false);
    path.forEach((n, k) => {
      const [a, b] = segs[s + k];
      // 汉字整个字标出来；英文单词标打了的那几个字母
      const end = /[a-z0-9]/i.test(name[a]) ? Math.min(b, a + n) : b;
      for (let x = a; x < end; x++) on[x] = true;
    });
    let html = "";
    let x = 0;
    while (x < name.length) {
      let y = x;
      while (y < name.length && on[y] === on[x]) y++;
      html += on[x] ? `<mark>${esc(name.slice(x, y))}</mark>` : esc(name.slice(x, y));
      x = y;
    }
    return html;
  }
  return esc(name);
}

// ---------- 画 ----------

const SVG = { folder: "folder", link: "globe", file: "file-text" } as const;
export const svgIcon = (k: string) => icon(SVG[k as keyof typeof SVG] ?? SVG.file);

/** 图标：系统给的图片；网址、拿不到图标的画一个 */
export function iconHtml(it: Pick<LaunchItem, "icon" | "kind" | "name">, cls = "lic"): string {
  if (it.icon) return `<img class="${cls}" src="${it.icon}" alt="" draggable="false">`;
  const k = it.kind === "link" || it.kind === "folder" ? it.kind : it.kind === "file" ? "file" : "";
  return k ? `<span class="${cls} ${k}">${svgIcon(k)}</span>` : `<span class="${cls} txt">${esc(it.name.trim().slice(0, 1).toUpperCase())}</span>`;
}

/** 下面那行小字：在哪 */
export function whereText(it: LaunchItem): string {
  if (it.missing) return `已经不在了：${it.path}`;
  if (it.kind === "store") return "微软商店应用";
  if (it.kind === "link") return it.path.replace(/^https?:\/\//, "");
  if (it.kind === "desk") return "桌面上的快捷方式";
  return it.path.startsWith("::") || !it.path ? "开始菜单里的程序" : it.path;
}

/** 放在格子里的短名字 */
export const shortName = (n: string) => n.replace(/^(Microsoft|Google) /, "").replace(/ \d+$/, "");

export const kindText = (it: LaunchItem) => ({ folder: "文件夹", file: "文件", link: "网址" })[it.kind as string] ?? "程序";
