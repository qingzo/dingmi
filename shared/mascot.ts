// 团子猫：U3 定稿（design/ui/u3-mascot.html、design/ui/mascot.js）的画法，电脑和手机网页共用。
// 颜色是猫自己的，不跟风格变。关掉团子猫时（根元素 data-mascot="off"）用它的地方自己换成平实的样子

/** 8 种表情：平时、摇铃铛、着急、睡觉、开心、伸懒腰、打哈欠、挥手 */
export type CatMood = "normal" | "ring" | "worry" | "sleep" | "happy" | "stretch" | "yawn" | "wave";
export type HeadState = "normal" | "focus" | "remind" | "bad";

const OUT = "#3b2418", BODY = "#fffaf3", EAR = "#ffab45", CHEEK = "#ff8f9a", BELL = "#ffb14a";
const f = (n: number) => +n.toFixed(1);
const O = "M18 92 C16 64 24 46 33 39 L35 22 Q36 17 41 20 L51 31 Q60 29 69 31 L79 20 Q84 17 85 22 L87 39 C96 46 104 64 102 92 Q102 101 93 101 L27 101 Q18 101 18 92 Z";
const SW = (w: number) => `stroke="${OUT}" stroke-width="${w}" stroke-linecap="round" stroke-linejoin="round" fill="none"`;

/** 眼睛 + 嘴 + 眉毛 + 脸蛋；中心 (60,64) */
function face(e: CatMood): string {
  const x = 60, y = 64, s = .95;
  const X = (d: number) => f(x + d * s), Y = (d: number) => f(y + d * s), k = (v: number) => f(v * s);
  const dot = (r: number, hl: number) => `<circle cx="${X(-12)}" cy="${Y(0)}" r="${k(r)}" fill="${OUT}"/><circle cx="${X(12)}" cy="${Y(0)}" r="${k(r)}" fill="${OUT}"/>` +
    `<circle cx="${X(-12 + r * .35)}" cy="${Y(-r * .4)}" r="${k(hl)}" fill="#fff"/><circle cx="${X(12 + r * .35)}" cy="${Y(-r * .4)}" r="${k(hl)}" fill="#fff"/>`;
  const arcs = (dir: number) => `<path d="M${X(-17)} ${Y(0)} q${k(5)} ${k(dir * 6)} ${k(10)} 0 M${X(7)} ${Y(0)} q${k(5)} ${k(dir * 6)} ${k(10)} 0" ${SW(k(2.6))}/>`;
  const cheeks = `<ellipse cx="${X(-20)}" cy="${Y(8)}" rx="${k(4.8)}" ry="${k(3)}" fill="${CHEEK}" opacity=".6"/><ellipse cx="${X(20)}" cy="${Y(8)}" rx="${k(4.8)}" ry="${k(3)}" fill="${CHEEK}" opacity=".6"/>`;
  const w = `<path d="M${X(-6)} ${Y(7)} q${k(3)} ${k(4)} ${k(6)} 0 q${k(3)} ${k(4)} ${k(6)} 0" ${SW(k(2.6))}/>`;
  switch (e) {
    case "normal": return cheeks + dot(4.3, 1.3) + w;
    case "ring": return cheeks + dot(5.2, 1.8) + `<ellipse cx="${X(0)}" cy="${Y(9)}" rx="${k(3.4)}" ry="${k(4.2)}" fill="${OUT}"/>`;
    case "worry": return cheeks + dot(5, 1.7) +
      `<path d="M${X(-18)} ${Y(-9)} l${k(9)} ${k(-3)} M${X(18)} ${Y(-9)} l${k(-9)} ${k(-3)}" ${SW(k(2.4))}/>` +
      `<path d="M${X(-5)} ${Y(10)} q${k(2.5)} ${k(-3)} ${k(5)} 0 q${k(2.5)} ${k(3)} ${k(5)} 0" ${SW(k(2.4))}/>`;
    case "sleep": return cheeks + arcs(1) + `<path d="M${X(-3)} ${Y(9)} h${k(6)}" ${SW(k(2.4))}/>`;
    case "happy": return cheeks + arcs(-1.2) + `<path d="M${X(-6)} ${Y(6)} q${k(6)} ${k(9)} ${k(12)} 0 Z" fill="${OUT}" stroke="${OUT}" stroke-width="${k(1.5)}" stroke-linejoin="round"/>`;
    case "stretch": return cheeks + `<path d="M${X(-17)} ${Y(-2)} l${k(9)} ${k(3)} l${k(-9)} ${k(3)} M${X(17)} ${Y(-2)} l${k(-9)} ${k(3)} l${k(9)} ${k(3)}" ${SW(k(2.5))}/>` +
      `<path d="M${X(-5)} ${Y(8)} q${k(5)} ${k(7)} ${k(10)} 0 Z" fill="${OUT}" stroke="${OUT}" stroke-width="${k(1.4)}" stroke-linejoin="round"/>`;
    case "yawn": return cheeks + `<path d="M${X(-17)} ${Y(-1)} q${k(5)} ${k(3)} ${k(10)} ${k(1)} M${X(7)} ${Y(0)} q${k(5)} ${k(2)} ${k(10)} ${k(-1)}" ${SW(k(2.5))}/>` +
      `<g class="mouth"><ellipse cx="${X(0)}" cy="${Y(11)}" rx="${k(5)}" ry="${k(6.4)}" fill="${OUT}"/><ellipse cx="${X(0)}" cy="${Y(14.5)}" rx="${k(3)}" ry="${k(2.2)}" fill="#ff8f9a"/></g>` +
      `<circle cx="${X(19)}" cy="${Y(1)}" r="${k(1.8)}" fill="#8fd0ff"/>`;
    case "wave": return cheeks + `<circle cx="${X(-12)}" cy="${Y(0)}" r="${k(4.3)}" fill="${OUT}"/><circle cx="${X(-10.5)}" cy="${Y(-1.7)}" r="${k(1.3)}" fill="#fff"/>` +
      `<path d="M${X(7)} ${Y(1)} q${k(5)} ${k(-6)} ${k(10)} 0" ${SW(k(2.6))}/>` +
      `<path d="M${X(-6)} ${Y(6)} q${k(6)} ${k(8)} ${k(12)} 0 Z" fill="${OUT}" stroke="${OUT}" stroke-width="${k(1.4)}" stroke-linejoin="round"/>`;
  }
}

const star = (cx: number, cy: number, r: number) => `<path d="M${cx} ${cy - r} Q${cx} ${cy} ${cx + r} ${cy} Q${cx} ${cy} ${cx} ${cy + r} Q${cx} ${cy} ${cx - r} ${cy} Q${cx} ${cy} ${cx} ${cy - r} Z" fill="#ffc53d"/>`;
const paw = (cx: number, cy: number, rot = 0) => `<g transform="rotate(${rot} ${cx} ${cy})"><ellipse cx="${cx}" cy="${cy}" rx="7.5" ry="8.5" fill="${BODY}" stroke="${OUT}" stroke-width="2.6"/><path d="M${cx - 3} ${cy - 5} v3 M${cx} ${cy - 6} v3 M${cx + 3} ${cy - 5} v3" ${SW(1.5)}/></g>`;

/** 整只猫（SVG 文本）。动作要外面套一个 class="mascot play" 的元素才播一次，见 mascot.css */
export function cat(e: CatMood): string {
  let pre = "", post = "", bodyExtra = "";
  if (e === "ring") pre = `<g class="waves"><path d="M8 44 q-6 10 0 20 M0 38 q-9 16 0 32 M112 44 q6 10 0 20 M120 38 q9 16 0 32" stroke="${EAR}" stroke-width="3.4" fill="none" stroke-linecap="round"/></g>`;
  if (e === "worry") post = `<path d="M100 30 q6 9 0 13 q-6 -4 0 -13 Z" fill="#8fd0ff" stroke="${OUT}" stroke-width="1.6" stroke-linejoin="round"/><path d="M112 22 v10 M112 37 v1" stroke="#e5484d" stroke-width="3.2" stroke-linecap="round"/>`;
  if (e === "sleep") post = `<g class="zz" fill="#9a8f87" font-family="Segoe UI, sans-serif" font-weight="700"><text x="98" y="30" font-size="17">z</text><text x="111" y="14" font-size="12">z</text></g>`;
  if (e === "happy") pre = `<g class="stars">${star(4, 30, 7)}${star(114, 38, 6)}${star(106, 10, 4)}</g>`;
  if (e === "stretch") bodyExtra = `<g class="arms">${paw(20, 34, -25)}${paw(100, 34, 25)}<path d="M6 18 l4 4 M114 18 l-4 4 M60 4 v5" stroke="${EAR}" stroke-width="2.6" stroke-linecap="round"/></g>`;
  if (e === "yawn") post = `<text x="98" y="30" font-size="14" font-family="Segoe UI" fill="#9a8f87" font-weight="700">~</text>`;
  if (e === "wave") post = `<g class="hand">${paw(104, 54, 20)}</g><path d="M116 36 q5 4 4 10 M122 30 q8 7 6 18" stroke="${EAR}" stroke-width="2.4" fill="none" stroke-linecap="round"/>`;
  const bell = `<g class="bell"><circle cx="60" cy="87" r="5.5" fill="${BELL}" stroke="${OUT}" stroke-width="2.2"/><path d="M55.5 86 h9" stroke="${OUT}" stroke-width="1.6" stroke-linecap="round"/></g>`;
  return `<svg class="${e}" viewBox="-16 -2 152 112" aria-hidden="true">${pre}
    <g class="body">${bodyExtra}
      <path d="${O}" fill="${BODY}"/>
      <path d="M35 22 Q36 17 41 20 L51 31 Q41 33 33 39 Z" fill="${EAR}"/>
      <path d="M37.5 26 L44 31.5 Q39.5 33 36.5 35 Z" fill="#ffd0a0"/><path d="M82.5 26 L76 31.5 Q80.5 33 83.5 35 Z" fill="#ffc8bd"/>
      <path d="${O}" fill="none" stroke="${OUT}" stroke-width="3" stroke-linejoin="round"/>
      ${face(e)}
      <path d="M38 101 q0 -9 9 -9 q9 0 9 9 M64 101 q0 -9 9 -9 q9 0 9 9" fill="${BODY}" stroke="${OUT}" stroke-width="2.6" stroke-linejoin="round"/>
      ${bell}
    </g>${post}</svg>`;
}

const HEAD = "M3 29 C2 19 4.5 13 7.5 10.5 L8 3.6 Q8.4 1.8 10.2 2.8 L14 6.9 Q16 6.5 18 6.9 L21.8 2.8 Q23.6 1.8 24 3.6 L24.5 10.5 C27.5 13 30 19 29 29 Z";
/** 只有脑袋：托盘图标和快捷框左边那个。focus 闭眼、remind 带铃铛、bad 右上角小红点 */
export function head(s: HeadState = "normal", px = 24, dark = false): string {
  const line = dark ? "#2a1a10" : OUT;
  const eyes = s === "focus"
    ? `<path d="M8.4 18.2 q2.5 2.6 5 0 M18.6 18.2 q2.5 2.6 5 0" stroke="${line}" stroke-width="2.2" stroke-linecap="round" fill="none"/>`
    : `<circle cx="11" cy="18.6" r="2.5" fill="${line}"/><circle cx="21" cy="18.6" r="2.5" fill="${line}"/>`;
  const remind = s === "remind" ? `<g transform="translate(19 17)"><path d="M6.5 1 C3.2 1 2 3.4 2 6.2 V9 L0.6 11 H12.4 L11 9 V6.2 C11 3.4 9.8 1 6.5 1 Z" fill="#f07326" stroke="${dark ? "#1f1f22" : "#fff"}" stroke-width="1.6" stroke-linejoin="round"/><circle cx="6.5" cy="12.6" r="1.6" fill="#f07326" stroke="${dark ? "#1f1f22" : "#fff"}" stroke-width="1"/></g>` : "";
  const bad = s === "bad" ? `<circle cx="26" cy="6.5" r="5" fill="#e5484d" stroke="${dark ? "#1f1f22" : "#fff"}" stroke-width="1.8"/>` : "";
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${px}" height="${px}" viewBox="0 0 32 32" aria-hidden="true">
    <path d="${HEAD}" fill="${BODY}"/>
    <path d="M8 3.6 Q8.4 1.8 10.2 2.8 L14 6.9 Q10 7.8 7.5 10.5 Z" fill="${EAR}"/>
    <path d="${HEAD}" fill="none" stroke="${line}" stroke-width="2.2" stroke-linejoin="round"/>
    <ellipse cx="7.6" cy="23" rx="2" ry="1.3" fill="${CHEEK}" opacity=".7"/><ellipse cx="24.4" cy="23" rx="2" ry="1.3" fill="${CHEEK}" opacity=".7"/>
    ${eyes}${remind}${bad}</svg>`;
}

/** 程序图标（U3 的 C：奶油色圆角方块 + 猫头；2026-11-06 试用后从 B 暖橘方块改成 C）。安装包、开始菜单、任务栏、主窗口左上角、手机主屏幕都用它；
 *  托盘也用它，按状态换：focus 闭眼、remind 右下角铃铛、bad 右上角小红点。scripts/build-icons.mjs 拿它生成图片 */
export function appIcon(px = 64, state: HeadState = "normal"): string {
  const g = `tg${px}${state}`;
  const eyes =
    state === "focus"
      ? `<path d="M8.4 18.2 q2.6 2.6 5.2 0 M18.4 18.2 q2.6 2.6 5.2 0" stroke="${OUT}" stroke-width="2" stroke-linecap="round" fill="none"/>`
      : `<circle cx="11" cy="18.6" r="2.4" fill="${OUT}"/><circle cx="21" cy="18.6" r="2.4" fill="${OUT}"/>
    <circle cx="11.8" cy="17.7" r=".8" fill="#fff"/><circle cx="21.8" cy="17.7" r=".8" fill="#fff"/>`;
  const badge =
    state === "remind"
      ? `<circle cx="47" cy="47" r="12" fill="#f07326" stroke="#fff" stroke-width="2.5"/><g transform="translate(39.8 39) scale(1.1)"><path d="M6.5 1 C3.2 1 2 3.4 2 6.2 V9 L0.6 11 H12.4 L11 9 V6.2 C11 3.4 9.8 1 6.5 1 Z" fill="#fff"/><circle cx="6.5" cy="12.6" r="1.6" fill="#fff"/></g>`
      : state === "bad"
        ? `<circle cx="50" cy="14" r="9" fill="#e5484d" stroke="#fff" stroke-width="3"/>`
        : "";
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${px}" height="${px}" viewBox="0 0 64 64" aria-hidden="true"><defs><linearGradient id="${g}" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#fff6ea"/><stop offset="1" stop-color="#ffdcbc"/></linearGradient></defs><rect x="4" y="4" width="56" height="56" rx="15" fill="url(#${g})"/><rect x="4.5" y="4.5" width="55" height="55" rx="14.5" fill="none" stroke="rgba(160,90,40,.25)"/>
  <g transform="translate(9 11) scale(1.44)">
    <path d="${HEAD}" fill="${BODY}"/>
    <path d="M8 3.6 Q8.4 1.8 10.2 2.8 L14 6.9 Q10 7.8 7.5 10.5 Z" fill="${EAR}"/>
    <path d="${HEAD}" fill="none" stroke="${OUT}" stroke-width="2" stroke-linejoin="round"/>
    <ellipse cx="7.6" cy="23" rx="2" ry="1.3" fill="${CHEEK}" opacity=".7"/><ellipse cx="24.4" cy="23" rx="2" ry="1.3" fill="${CHEEK}" opacity=".7"/>
    ${eyes}
    ${state === "focus" ? "" : `<path d="M14 22.5 q1 1.3 2 0 q1 1.3 2 0" stroke="${OUT}" stroke-width="1.2" fill="none" stroke-linecap="round"/>`}</g>${badge}</svg>`;
}
