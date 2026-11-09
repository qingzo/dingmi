// 电脑端每个窗口能调哪些命令（Tauri 的 capabilities，安全审查第 4 条）：
// 从每个窗口的网页入口（apps/desktop/ui/pages/*.ts）顺着 import 找到它用到的所有代码，看里面调了 api.xxx 的哪几个，
// 对到 api.ts 里的命令名，生成 apps/desktop/src-tauri/capabilities/app-<窗口>.json：每个窗口只给它自己用到的命令。
// 网页万一被塞进了别的脚本，小窗口（快捷框、提醒卡片、桌面组件……）也调不了导出、恢复、同步这些敏感命令。
//
//   node scripts/caps.mjs          看看每个窗口用到哪些命令，和现在的 capabilities 对不对得上
//   node scripts/caps.mjs --write  重新生成（加了命令、页面里多调了一个命令以后跑一下；apps/desktop/ui/lib/caps.test.ts 会提醒）
//
// 命令都要写进 src-tauri/build.rs 的 COMMANDS（Tauri 按它生成 allow-<命令> 权限；有了这份清单，没给权限的窗口就调不了）。
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const ui = path.join(root, "apps/desktop/ui");
const tauriDir = path.join(root, "apps/desktop/src-tauri");
const capDir = path.join(tauriDir, "capabilities");

/** 窗口（label，可以带 *）→ 网页入口。和 win.rs 里建窗口时用的网页一致 */
export const WINDOWS = {
  main: "pages/main.ts",
  flash: "pages/flash.ts",
  qnote: "pages/qnote.ts",
  remind: "pages/remind.ts",
  morning: "pages/morning.ts",
  wrap: "pages/wrap.ts",
  widget: "pages/widget.ts",
  "note-*": "pages/note.ts",
  onboard: "pages/onboard.ts",
};

const read = (f) => fs.readFileSync(f, "utf8");

/** api.ts 里每个方法调的命令：方法名 → [命令] */
export function apiCommands() {
  const src = read(path.join(ui, "lib/api.ts"));
  const body = src.slice(src.indexOf("export const api = {"));
  const out = {};
  const parts = body.split(/^ {2}(\w+)(?=: |\()/m);
  for (let i = 1; i < parts.length; i += 2) {
    out[parts[i]] = [...parts[i + 1].matchAll(/invoke(?:<[^(]*?>)?\(\s*"(\w+)"/g)].map((m) => m[1]);
  }
  return out;
}

/** 一个文件 import 的本仓库里的文件（相对路径和 @shared/） */
function imports(file) {
  const src = read(file);
  const out = [];
  for (const m of src.matchAll(/(?:^|\n)\s*(?:import|export)\s[^;]*?from\s*"([^"]+)"|(?:^|\n)\s*import\s*"([^"]+)"/g)) {
    const spec = m[1] ?? m[2];
    // 只拿类型的（import type / export type）不会打包进去
    if (/^\s*(?:import|export)\s+type\s/.test(m[0])) continue;
    let base;
    if (spec.startsWith("@shared/")) base = path.join(root, "shared", spec.slice(8));
    else if (spec.startsWith(".")) base = path.resolve(path.dirname(file), spec);
    else continue;
    const hit = [base, `${base}.ts`, path.join(base, "index.ts")].find((f) => fs.existsSync(f) && fs.statSync(f).isFile() && f.endsWith(".ts"));
    if (hit) out.push(hit);
  }
  return out;
}

/** 从入口顺着 import 找到的所有 .ts 文件 */
function closure(entry) {
  const seen = new Set();
  const todo = [entry];
  while (todo.length) {
    const f = todo.pop();
    if (seen.has(f)) continue;
    seen.add(f);
    todo.push(...imports(f));
  }
  return seen;
}

/** 每个窗口用到的命令（排好序） */
export function usage() {
  const api = apiCommands();
  const apiFile = path.join(ui, "lib/api.ts");
  const out = {};
  for (const [win, entry] of Object.entries(WINDOWS)) {
    const cmds = new Set();
    for (const f of closure(path.join(ui, entry))) {
      if (f === apiFile) continue;
      // 去掉注释（注释里会提到 api.ts 之类）
      const src = read(f).replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|\s)\/\/.*$/gm, "$1");
      for (const m of src.matchAll(/\bapi\.(\w+)/g)) {
        if (!api[m[1]]) throw new Error(`${path.relative(root, f)} 用了 api.${m[1]}，api.ts 里没有`);
        api[m[1]].forEach((c) => cmds.add(c));
      }
      // 不经过 api.ts 直接 invoke 的
      for (const m of src.matchAll(/\binvoke(?:<[^(]*?>)?\(\s*"(\w+)"/g)) cmds.add(m[1]);
    }
    out[win] = [...cmds].sort();
  }
  return out;
}

/** lib.rs 里 generate_handler! 登记的命令名（去掉模块前缀） */
export function handlerCommands() {
  const src = read(path.join(tauriDir, "src/lib.rs"));
  const block = src.slice(src.indexOf("generate_handler!["), src.indexOf("])", src.indexOf("generate_handler![")));
  return [...block.matchAll(/([\w:]+),/g)].map((m) => m[1].split("::").pop()).filter((c) => c !== "generate_handler").sort();
}

/** build.rs 里 COMMANDS 清单 */
export function manifestCommands() {
  const src = read(path.join(tauriDir, "build.rs"));
  const block = src.slice(src.indexOf("COMMANDS"), src.indexOf("];", src.indexOf("COMMANDS")));
  return [...block.matchAll(/"(\w+)"/g)].map((m) => m[1]).sort();
}

const fileOf = (win) => path.join(capDir, `app-${win.replace("-*", "")}.json`);

export function capability(win, cmds) {
  return `${JSON.stringify(
    {
      $schema: "../gen/schemas/desktop-schema.json",
      identifier: `app-${win.replace("-*", "")}`,
      description: `${win} 窗口能调的叮咪命令（node scripts/caps.mjs --write 按页面代码生成，别手改）`,
      windows: [win],
      permissions: cmds.map((c) => `allow-${c.replace(/_/g, "-")}`),
    },
    null,
    2,
  )}\n`;
}

/** 现在的 capabilities 文件和代码对不对得上：回问题列表 */
export function check() {
  const problems = [];
  const use = usage();
  const handlers = handlerCommands();
  const manifest = manifestCommands();
  for (const c of handlers) if (!manifest.includes(c)) problems.push(`命令 ${c} 没写进 build.rs 的 COMMANDS`);
  for (const c of manifest) if (!handlers.includes(c)) problems.push(`build.rs 的 COMMANDS 里有 ${c}，lib.rs 没登记这个命令`);
  for (const [win, cmds] of Object.entries(use)) {
    for (const c of cmds) if (!handlers.includes(c)) problems.push(`${win} 调了 ${c}，lib.rs 没登记这个命令`);
    const f = fileOf(win);
    // Windows 上检出来可能是 CRLF，比的时候不管换行
    if (!fs.existsSync(f) || read(f).replace(/\r\n/g, "\n") !== capability(win, cmds)) problems.push(`${path.relative(root, f)} 和页面代码对不上，跑一下 node scripts/caps.mjs --write`);
  }
  return problems;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const use = usage();
  if (process.argv.includes("--write")) {
    for (const [win, cmds] of Object.entries(use)) fs.writeFileSync(fileOf(win), capability(win, cmds));
    console.log("写好了：", Object.keys(use).map((w) => path.relative(root, fileOf(w))).join("、"));
  }
  for (const [win, cmds] of Object.entries(use)) console.log(`${win}（${cmds.length}）：${cmds.join(" ")}`);
  const p = check();
  console.log(p.length ? `\n对不上：\n  ${p.join("\n  ")}` : "\n都对得上");
  process.exitCode = p.length ? 1 : 0;
}
