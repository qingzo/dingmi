// 从团子猫的画法（shared/mascot.ts，U3 定稿）生成图标图片。改了猫或者图标后跑一次：node scripts/build-icons.mjs
// - 托盘：程序图标（奶油色方块）的 4 种状态 × 16、20、24、32 像素 → apps/desktop/src-tauri/icons/tray/
//   （方块不怕任务栏深浅，浅色、深色两套是一样的图，Rust 那边的挑法不用改）
// - 程序图标（奶油色方块 + 猫头）：先出 1024 的图，再用 tauri icon 生成安装包、任务栏用的那几个
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { Resvg } from "@resvg/resvg-js";
import { appIcon } from "../shared/mascot.ts";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const icons = join(root, "apps/desktop/src-tauri/icons");
const png = (svg, px) => new Resvg(svg, { fitTo: { mode: "width", value: px } }).render().asPng();

// 托盘图标不留边：方块撑满整个格子，和别的软件的托盘图标一样大（程序图标留边是 Windows 的习惯）
const trayIcon = (px, state) => appIcon(px, state).replace('viewBox="0 0 64 64"', 'viewBox="4 4 56 56"');
mkdirSync(join(icons, "tray"), { recursive: true });
for (const state of ["normal", "remind", "focus", "bad"]) {
  for (const dark of [false, true]) {
    for (const px of [16, 20, 24, 32]) {
      writeFileSync(join(icons, "tray", `${state}-${dark ? "dark" : "light"}-${px}.png`), png(trayIcon(px, state), px));
    }
  }
}

const tmp = mkdtempSync(join(tmpdir(), "dingmi-icon-"));
const big = join(tmp, "app-icon.png");
writeFileSync(big, png(appIcon(64), 1024));
writeFileSync(join(icons, "app-icon.svg"), appIcon(64) + "\n");
execFileSync(process.execPath, [join(root, "node_modules/@tauri-apps/cli/tauri.js"), "icon", big, "-o", join(tmp, "out")], { stdio: "inherit" });
for (const f of ["32x32.png", "64x64.png", "128x128.png", "128x128@2x.png", "icon.icns", "icon.ico", "icon.png",
  "Square30x30Logo.png", "Square44x44Logo.png", "Square71x71Logo.png", "Square89x89Logo.png", "Square107x107Logo.png",
  "Square142x142Logo.png", "Square150x150Logo.png", "Square284x284Logo.png", "Square310x310Logo.png", "StoreLogo.png"]) {
  copyFileSync(join(tmp, "out", f), join(icons, f));
}

// 安装包图标（tauri.conf.json 的 nsis.installerIcon）：程序图标右下角加一个橙色的「往下」角标，装好以后的图标还是上面那个
const badge = `<g><circle cx="52" cy="52" r="10" fill="#ff8a3d" stroke="#3b2418" stroke-width="2"/><path d="M52 46.5 V56 M48 52.2 L52 56.2 L56 52.2" fill="none" stroke="#fff" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"/></g>`;
const setup = join(tmp, "installer.png");
writeFileSync(setup, png(appIcon(64).replace(/<\/svg>\s*$/, `${badge}</svg>`), 1024));
execFileSync(process.execPath, [join(root, "node_modules/@tauri-apps/cli/tauri.js"), "icon", setup, "-o", join(tmp, "setup")], { stdio: "inherit" });
copyFileSync(join(tmp, "setup", "icon.ico"), join(icons, "installer.ico"));
rmSync(tmp, { recursive: true, force: true });
console.log("图标生成好了：", icons);
