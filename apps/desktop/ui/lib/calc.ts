// 随手算（需求-0.3 第 10.1 节）：快捷框里打的是一个算式时，结果排第一行，回车复制。
// 只认数字和 + - * / × ÷ ^ % ( )，全角的数字和符号也认，末尾可以多一个 =；至少有一个运算符才算。
// 日期写法（2026-03-05、3/5）交给时间识别，这里不管：调用的地方先看本地规则认没认出时间

export interface Calc {
  /** 复制用的结果（不带千分位）；算不了是 null */
  value: string | null;
  /** 显示用的结果（大数加千分位） */
  shown: string;
  /** 算式，× ÷ 换成好看的写法 */
  expr: string;
}

const FULL: Record<string, string> = { "＋": "+", "－": "-", "＊": "*", "／": "/", "（": "(", "）": ")", "％": "%", "＾": "^", "＝": "=", "．": ".", "。": "." };

type Tok = { n: number } | { op: string };

function tokens(s: string): Tok[] | null {
  const out: Tok[] = [];
  let i = 0;
  while (i < s.length) {
    const c = s[i];
    if (c === " ") {
      i++;
      continue;
    }
    const num = /^(?:\d+(?:\.\d*)?|\.\d+)/.exec(s.slice(i));
    if (num) {
      out.push({ n: Number(num[0]) });
      i += num[0].length;
      continue;
    }
    const op = c === "×" || c === "x" || c === "X" ? "*" : c === "÷" ? "/" : c;
    if (!"+-*/^%()".includes(op)) return null;
    out.push({ op });
    i++;
  }
  return out;
}

/** 递归下降：加减 < 乘除 < 负号 < 乘方（右结合） < 百分号 < 数字、括号 */
function evaluate(toks: Tok[]): number | null {
  let p = 0;
  const peek = () => toks[p] as { op?: string; n?: number } | undefined;
  const isOp = (o: string) => peek()?.op === o;
  function sum(): number {
    let v = product();
    while (isOp("+") || isOp("-")) {
      const o = (toks[p++] as { op: string }).op;
      const r = product();
      v = o === "+" ? v + r : v - r;
    }
    return v;
  }
  function product(): number {
    let v = unary();
    while (isOp("*") || isOp("/")) {
      const o = (toks[p++] as { op: string }).op;
      const r = unary();
      v = o === "*" ? v * r : v / r;
    }
    return v;
  }
  function unary(): number {
    if (isOp("-")) {
      p++;
      return -unary();
    }
    if (isOp("+")) {
      p++;
      return unary();
    }
    return power();
  }
  function power(): number {
    const b = percent();
    if (isOp("^")) {
      p++;
      return b ** unary();
    }
    return b;
  }
  function percent(): number {
    let v = atom();
    while (isOp("%")) {
      p++;
      v /= 100;
    }
    return v;
  }
  function atom(): number {
    const t = peek();
    if (t?.n !== undefined) {
      p++;
      return t.n;
    }
    if (t?.op === "(") {
      p++;
      const v = sum();
      if (!isOp(")")) throw new Error("括号没配对");
      p++;
      return v;
    }
    throw new Error("算式不完整");
  }
  try {
    const v = sum();
    return p === toks.length ? v : null;
  } catch {
    return null;
  }
}

/** 最多 10 位有效数字，去掉末尾的 0 */
function plain(v: number): string {
  if (v === 0) return "0";
  return String(Number(v.toPrecision(10)));
}

/** 整数部分每三位加一个逗号（科学计数法的不加） */
function grouped(s: string): string {
  if (/e/i.test(s)) return s;
  const [int, frac] = s.split(".");
  const g = int.replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  return frac ? `${g}.${frac}` : g;
}

/** 打的是算式就算出来，不是算式回 null */
export function calc(input: string): Calc | null {
  let s = input
    .trim()
    .replace(/[＋－＊／（）％＾＝．。]/g, (c) => FULL[c])
    .replace(/[０-９]/g, (c) => String(c.charCodeAt(0) - 0xff10));
  s = s.replace(/\s*=\s*$/, "");
  // 只有数字和运算符号；至少一个运算符（开头的负号不算）；得有数字
  if (!s || !/^[\d\s.+\-*/×÷xX^%()]+$/.test(s) || !/\d/.test(s)) return null;
  if (!/(?:\d|\)|%)\s*[+\-*/×÷xX^]|%/.test(s)) return null;
  const toks = tokens(s);
  if (!toks) return null;
  const v = evaluate(toks);
  if (v === null) return null;
  const expr = s
    .replace(/\s+/g, "")
    .replace(/[*xX×]/g, " × ")
    .replace(/[/÷]/g, " ÷ ")
    .replace(/[+^]/g, " $& ")
    // 减号两边空一格，负号不空
    .replace(/(\d|\)|%)-/g, "$1 - ")
    .replace(/\s+/g, " ")
    .trim();
  if (!Number.isFinite(v)) return { value: null, shown: "", expr };
  const value = plain(v);
  return { value, shown: grouped(value), expr };
}
