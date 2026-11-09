// 设置里的搜索（需求-0.2 第 6.2 节）：搜每一行的名字、小字说明和常见叫法。
// 只管算，不碰界面；行的名字和叫法在 text/settings.ts

export interface SetRow {
  /** 在哪一页 */
  page: string;
  /** 那一行的 id（页面上 data-row） */
  id: string;
  name: string;
  sub?: string;
  /** 常见叫法：打这些也能找到 */
  also?: string[];
}

export interface Found {
  row: SetRow;
  /** 名字里要标出来的字：[从, 到) */
  marks: [number, number][];
  score: number;
}

/** 比较时不管大小写、空格 */
const norm = (s: string) => s.toLowerCase().replace(/\s+/g, "");

/** 名字一个字一个字地拆开（去掉空格、变小写），带上它在原文里是第几个字 */
const letters = (s: string): [string, number][] =>
  [...s].map((c, i) => [c.toLowerCase(), i] as [string, number]).filter(([c]) => !/\s/.test(c));

/** q 整段在 s 里：回每个字的下标 */
function together(s: string[], q: string[]): number[] | null {
  for (let i = 0; i + q.length <= s.length; i++) if (q.every((c, k) => s[i + k] === c)) return q.map((_, k) => i + k);
  return null;
}

/** q 的字按顺序都在 s 里（中间可以隔着别的字）：回每个字的下标 */
function inOrder(s: string[], q: string[]): number[] | null {
  const at: number[] = [];
  let from = 0;
  for (const c of q) {
    const i = s.indexOf(c, from);
    if (i < 0) return null;
    at.push(i);
    from = i + 1;
  }
  return at;
}

/** 连着的位置并成一段 */
function ranges(at: number[]): [number, number][] {
  const out: [number, number][] = [];
  for (const i of at) {
    const last = out[out.length - 1];
    if (last && last[1] === i) last[1] = i + 1;
    else out.push([i, i + 1]);
  }
  return out;
}

/** 一行和打的字对不对得上，对不上回 null。名字里整段有 > 常见叫法 > 名字里按顺序有这几个字 > 小字说明里有 */
export function matchRow(row: SetRow, query: string): Found | null {
  const q = [...norm(query)];
  if (!q.length) return null;
  const ls = letters(row.name);
  const s = ls.map(([c]) => c);
  const toMarks = (idx: number[]) => ranges(idx.map((i) => ls[i][1]));
  const whole = together(s, q);
  if (whole) return { row, marks: toMarks(whole), score: 100 - Math.min(whole[0], 20) };
  const seq = inOrder(s, q);
  const qq = q.join("");
  const also = (row.also ?? []).map(norm);
  if (also.some((a) => a.includes(qq) || (a.length >= 2 && qq.includes(a)))) return { row, marks: seq ? toMarks(seq) : [], score: 80 };
  // 字隔得越开越不像，隔得太开就不算
  const gap = seq ? seq[seq.length - 1] - seq[0] - (seq.length - 1) : 99;
  if (seq && gap <= 6) return { row, marks: toMarks(seq), score: 60 - gap };
  if (row.sub && norm(row.sub).includes(qq)) return { row, marks: [], score: 30 };
  return null;
}

/** 搜设置：按对得上的程度排，一样的名字短的在前（「同步方式」比「同步 AI Key」更像），再一样的按原来的顺序 */
export function findRows(query: string, rows: SetRow[], limit = 12): Found[] {
  return rows
    .map((r, i) => [matchRow(r, query), i] as const)
    .filter((x): x is readonly [Found, number] => !!x[0])
    .sort((a, b) => b[0].score - a[0].score || [...a[0].row.name].length - [...b[0].row.name].length || a[1] - b[1])
    .slice(0, limit)
    .map(([f]) => f);
}

/** 名字套上 <mark>，其余的字转义 */
export function markName(name: string, marks: [number, number][], esc: (s: string) => string): string {
  const cs = [...name];
  let out = "";
  let i = 0;
  for (const [a, b] of marks) {
    out += esc(cs.slice(i, a).join("")) + `<mark>${esc(cs.slice(a, b).join(""))}</mark>`;
    i = b;
  }
  return out + esc(cs.slice(i).join(""));
}

/** text/settings.ts 的 ROWS（页 → 行 id → 名字、说明、叫法）摊成一张表 */
export function rowsOf(table: Record<string, Record<string, Omit<SetRow, "page" | "id">>>): SetRow[] {
  return Object.entries(table).flatMap(([page, rows]) => Object.entries(rows).map(([id, r]) => ({ page, id, ...r })));
}
