// 手机上的外观（需求-0.2 第 29.4 节）：风格在「我的」里单独选，存在这部手机上，不跟电脑同步；
// 深浅色跟着手机系统。根元素上的 data-style / data-mode / data-mascot 和电脑一样，颜色这些在 tokens.css 里按它们换。
// 手机上没有团子猫开关，跟着风格的默认值：简洁不显示
import type { StyleId } from "@shared/text";
import { ls } from "./sync";

const sysDark = matchMedia("(prefers-color-scheme: dark)");
const root = document.documentElement;

export const getStyle = (): StyleId => {
  const s = ls.get("style");
  return s === "simple" || s === "calm" ? s : "warm";
};
export const isDark = () => sysDark.matches;
export const mascotOn = () => root.dataset.mascot !== "off";

function apply() {
  const s = getStyle();
  if (s === "warm") delete root.dataset.style;
  else root.dataset.style = s;
  root.dataset.mode = isDark() ? "dark" : "light";
  // 手机上没有动画的设置：跟着手机系统的「减少动态效果」
  root.dataset.motion = "system";
  root.dataset.mascot = s === "simple" ? "off" : "on";
  // 地址栏、状态栏的颜色跟着底色
  const bg = getComputedStyle(root).getPropertyValue("--bg").trim();
  let meta = document.querySelector<HTMLMetaElement>('meta[name="theme-color"]');
  if (!meta) {
    meta = document.createElement("meta");
    meta.name = "theme-color";
    document.head.append(meta);
  }
  if (bg) meta.content = bg;
}

/** 换风格：马上换上，记在这部手机上 */
export function setStyle(s: StyleId) {
  ls.set("style", s === "warm" ? null : s);
  apply();
  watchers.forEach((cb) => cb());
}

const watchers: (() => void)[] = [];
/** 风格或深浅色变了（团子猫跟着变，界面要重画） */
export const onLookChange = (cb: () => void) => watchers.push(cb);

apply();
sysDark.addEventListener("change", () => {
  apply();
  watchers.forEach((cb) => cb());
});
