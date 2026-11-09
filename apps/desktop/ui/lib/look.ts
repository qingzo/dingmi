// 外观：风格、深浅色、字号、减少动画、团子猫（需求-0.2 第 29 节）。每个窗口一打开就 import 它。
// 根元素上设 data-style / data-mode / data-motion / data-mascot，颜色这些都在 styles/tokens.css 里按它们换。
// 上次的外观记在 localStorage（几个窗口共用），打开窗口时先按它画，免得先闪一下默认的样子
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { api, type LookStyle, type Settings } from "./api";

type Look = Pick<Settings, "style" | "theme" | "font_size" | "mascot" | "motion">;

const KEY = "dingmi-look";
const DEFAULT: Look = { style: "warm", theme: "system", font_size: "m", mascot: null, motion: "on" };
/** 字号：只缩放主窗口和笔记窗口，快捷框、提醒卡片这些窗口大小是定好的 */
const SCALE = { s: 0.92, m: 1, l: 1.1 };

const sysDark = matchMedia("(prefers-color-scheme: dark)");
let cur: Look = DEFAULT;
let scaled = false;
const modeWatchers: ((dark: boolean) => void)[] = [];

/** 这种风格下团子猫默认开不开：简洁默认关 */
export const mascotDefault = (style: LookStyle) => style !== "simple";
/** 团子猫开着没有：手动改过就听用户的，没改过跟着风格 */
export const mascotOn = (l: Pick<Look, "style" | "mascot"> = cur) => l.mascot ?? mascotDefault(l.style);
/** 现在是不是深色 */
export const isDark = () => (cur.theme === "system" ? sysDark.matches : cur.theme === "dark");

/** 深浅色变了（换了设置，或者跟随系统时系统换了） */
export function onModeChange(cb: (dark: boolean) => void) {
  modeWatchers.push(cb);
}

function apply(l: Look) {
  const wasDark = isDark();
  cur = { ...DEFAULT, ...l };
  const d = document.documentElement;
  if (cur.style === "warm") delete d.dataset.style;
  else d.dataset.style = cur.style;
  d.dataset.mode = isDark() ? "dark" : "light";
  d.dataset.motion = cur.motion;
  d.dataset.mascot = mascotOn() ? "on" : "off";
  if (scaled) getCurrentWebview().setZoom(SCALE[cur.font_size] ?? 1).catch(() => {});
  if (wasDark !== isDark()) modeWatchers.forEach((cb) => cb(isDark()));
}

function remember(l: Look) {
  try {
    localStorage.setItem(KEY, JSON.stringify(l));
  } catch {
    // 存不了就每次按默认的先画
  }
}

function pick(s: Settings): Look {
  return { style: s.style, theme: s.theme, font_size: s.font_size, mascot: s.mascot, motion: s.motion };
}

/** 设置里改了外观：马上换上（不用等存好），也记下来给之后打开的窗口 */
export function previewLook(s: Settings) {
  const l = pick(s);
  remember(l);
  apply(l);
}

/** 主窗口、笔记窗口调：字号跟着设置缩放 */
export function scaleWithFontSize() {
  scaled = true;
  apply(cur);
}

try {
  apply({ ...DEFAULT, ...JSON.parse(localStorage.getItem(KEY) ?? "{}") });
} catch {
  apply(DEFAULT);
}
sysDark.addEventListener("change", () => {
  if (cur.theme !== "system") return;
  document.documentElement.dataset.mode = sysDark.matches ? "dark" : "light";
  modeWatchers.forEach((cb) => cb(sysDark.matches));
});
api.getSettings().then(previewLook, () => {});
listen<Settings>("settings-changed", (e) => previewLook(e.payload));
