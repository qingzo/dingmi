// 生成第三方许可证清单 THIRD_PARTY_LICENSES.md（需求-0.4 第 13.2 节）。改了依赖以后跑一次：
//   node scripts/licenses.mjs            重新生成 THIRD_PARTY_LICENSES.md
//   node scripts/licenses.mjs --check    只检查，不写文件；有看不出许可证或者可能和 AGPL-3.0 合不到一起的，退出码 1
// 要装好 Rust（用 cargo metadata 取依赖，按 Cargo.lock 来，不联网也行，前提是依赖下载过）；npm 部分只读 package-lock.json。
// 算哪些：
// - Rust：公开的几个包（电脑端 dingmi、服务器 dingmi-server、同步库 dingmi-sync）编进程序的依赖，
//   电脑端按 Windows 算，服务器按 Linux（musl）和 Windows 都算；只在测试里用的不算；只有官方版（不公开）用的不算。
// - npm：会被打进电脑端界面、手机网页的包：package.json 的 dependencies，加上 apps/desktop/ui、apps/mobile、shared
//   里 import 到的包（有几个写在 devDependencies 里，比如 @noble/ciphers、hash-wasm、lucide），以及它们的依赖。
//   只用来打包、测试的（vite、esbuild、vitest、typescript、tauri 命令行……）不算。官方版的后台网页不公开，不算。
// - 字体、图标、内嵌的别人的代码：写在下面的 MANUAL 里，有新的要手动加。
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const outFile = path.join(root, "THIRD_PARTY_LICENSES.md");
const checkOnly = process.argv.includes("--check");

/** 公开的 Rust 包，按什么平台算依赖 */
const RUST_ROOTS = [
  { name: "dingmi", label: "电脑端", platforms: ["x86_64-pc-windows-msvc"] },
  { name: "dingmi-server", label: "服务器", platforms: ["x86_64-unknown-linux-musl", "x86_64-pc-windows-msvc"] },
  { name: "dingmi-sync", label: "同步库", platforms: ["x86_64-unknown-linux-musl", "x86_64-pc-windows-msvc"] },
];

/** 打进产物的前端代码在这几个目录（测试文件除外） */
const FRONT_DIRS = ["apps/desktop/ui", "apps/mobile", "shared"];
/** 这几个目录里只在打包时用的脚本，不算 */
const FRONT_SKIP = [/\.test\.ts$/, /[\\/]build\.mjs$/, /[\\/]tauri-stub\.ts$/];

/** 不是从 crates.io / npm 来的：字体、图标、内嵌的代码。每条：名字、用在哪、许可证、说明（ok：人工确认过不用标出来） */
const MANUAL = [
  {
    name: "文渊圆体（WenYuan Rounded SC）1.110",
    where: "电脑端界面的字体：apps/desktop/ui/fonts/dingmi-rounded.woff2（裁剪成常用字，改名 Dingmi Rounded，做法见 scripts/build-font.py）",
    license: "OFL-1.1",
    note: "© 2024-2026 Takushun Wu（保留字体名 WenYuan Rounded、文渊圆体），字形来自思源黑体（© Adobe，保留字体名 Source）、SN Pro 等，各方版权见许可证全文：apps/desktop/ui/fonts/OFL.txt。https://github.com/takushun-wu/WenYuanFonts",
  },
  {
    name: "Lucide 图标",
    where: "电脑端和手机网页里的图标：shared/icons.ts 从 npm 包 lucide 里取图标数据",
    license: "ISC",
    note: "© Lucide Contributors；一部分图标来自 Feather（© Cole Bemis，MIT）。https://lucide.dev/license",
  },
  {
    name: "TweetNaCl.js（nacl-fast.min.js）",
    where: "手机网页的加密：apps/server/web/nacl-fast.min.js，服务器发手机网页时内嵌进去",
    license: "Unlicense",
    note: "公有领域。https://github.com/dchest/tweetnacl-js",
  },
  {
    name: "Tauri 安装包中文文字",
    where: "apps/desktop/src-tauri/windows/SimpChinese.nsh（从 tauri-cli 2.12.4 自带的那份拷来改的）",
    license: "Apache-2.0 OR MIT",
    note: "https://github.com/tauri-apps/tauri",
  },
  {
    name: "NSIS",
    where: "安装包（Tauri 用 NSIS 打包，安装包里带着 NSIS 的安装程序外壳）",
    license: "Zlib",
    note: "源码里没有，只在打出来的安装包里。https://nsis.sourceforge.io/License",
  },
  {
    name: "Microsoft Edge WebView2 安装程序",
    where: "安装包里带的 WebView2 引导程序（tauri.conf.json 的 embedBootstrapper），没有 WebView2 的电脑装叮咪时顺手装上",
    license: "微软的分发许可",
    ok: true, // 系统组件，不和叮咪的代码合在一起，不用和 AGPL 兼容
    note: "源码里没有，只在打出来的安装包里；WebView2 是系统组件（AGPL 第 1 节说的「系统库」）。https://developer.microsoft.com/microsoft-edge/webview2/",
  },
];

/** 和 AGPL-3.0 能放在一起的许可证（只认这些，别的都标出来看一眼） */
const OK = new Set([
  "MIT", "MIT-0", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib",
  "Unlicense", "0BSD", "CC0-1.0", "BSL-1.0", "Unicode-3.0", "Unicode-DFS-2016", "MPL-2.0", "CDLA-Permissive-2.0",
  "AGPL-3.0-only", "AGPL-3.0-or-later", "OFL-1.1", "Python-2.0", "BlueOak-1.0.0",
]);

function fail(msg) {
  console.error(`\n没生成：${msg}\n`);
  process.exit(1);
}

/**
 * 许可证写法（SPDX 表达式，比如「MIT OR Apache-2.0」「(MIT OR Apache-2.0) AND Unicode-3.0」）能不能和 AGPL 放在一起。
 * OR 有一个能用就行，AND 要都能用。老写法「MIT/Apache-2.0」当成 OR。
 */
function compatible(expr) {
  if (!expr) return false;
  const tokens = expr.replace(/\//g, " OR ").replace(/\(/g, " ( ").replace(/\)/g, " ) ").split(/\s+/).filter(Boolean);
  let i = 0;
  const atom = () => {
    if (tokens[i] === "(") {
      i++;
      const v = or();
      i++; // )
      return v;
    }
    let id = tokens[i++] ?? "";
    if (tokens[i] === "WITH") {
      id += ` WITH ${tokens[i + 1]}`;
      i += 2;
    }
    return OK.has(id) || OK.has(id.replace(/\+$/, ""));
  };
  const and = () => {
    let v = atom();
    while (tokens[i] === "AND") {
      i++;
      v = atom() && v;
    }
    return v;
  };
  const or = () => {
    let v = and();
    while (tokens[i] === "OR") {
      i++;
      v = and() || v;
    }
    return v;
  };
  return or();
}

// ---------- Rust ----------

/** 按一个平台跑 cargo metadata，从公开的包往下找编进程序的依赖（普通依赖和 build 依赖，不含 dev） */
function rustDeps() {
  const found = new Map(); // id -> { pkg, users: Set<label> }
  const platforms = [...new Set(RUST_ROOTS.flatMap((r) => r.platforms))];
  for (const platform of platforms) {
    const r = spawnSync("cargo", ["metadata", "--format-version", "1", "--locked", "--filter-platform", platform], {
      cwd: root,
      encoding: "utf8",
      maxBuffer: 512 * 1024 * 1024,
    });
    if (r.status !== 0) fail(`cargo metadata 出错了（Cargo.lock 和 Cargo.toml 对不上的话先 cargo build 一次）：\n${r.stderr}`);
    const meta = JSON.parse(r.stdout);
    const pkgs = new Map(meta.packages.map((p) => [p.id, p]));
    const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
    for (const rootDef of RUST_ROOTS.filter((x) => x.platforms.includes(platform))) {
      const start = meta.packages.find((p) => p.name === rootDef.name && p.source == null);
      if (!start) fail(`Cargo workspace 里没找到 ${rootDef.name}`);
      const seen = new Set();
      const stack = [start.id];
      while (stack.length) {
        const id = stack.pop();
        if (seen.has(id)) continue;
        seen.add(id);
        const pkg = pkgs.get(id);
        if (pkg.source != null) {
          const e = found.get(id) ?? { pkg, users: new Set() };
          e.users.add(rootDef.label);
          found.set(id, e);
        }
        for (const d of nodes.get(id)?.deps ?? []) {
          if (d.dep_kinds.some((k) => k.kind === null || k.kind === "build")) stack.push(d.pkg);
        }
      }
    }
  }
  return [...found.values()]
    .map(({ pkg, users }) => ({
      name: pkg.name,
      version: pkg.version,
      license: pkg.license ?? (pkg.license_file ? `见包里的 ${pkg.license_file}` : ""),
      spdx: pkg.license ?? "",
      url: pkg.repository ?? pkg.homepage ?? `https://crates.io/crates/${pkg.name}`,
      users: RUST_ROOTS.map((r) => r.label).filter((l) => users.has(l)),
    }))
    .sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version, undefined, { numeric: true }));
}

// ---------- npm ----------

/** 前端代码里 import 的包名（"@codemirror/view" → @codemirror/view，"lucide/xx" → lucide） */
function importedPackages() {
  const names = new Set();
  const walk = (dir) => {
    for (const ent of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, ent.name);
      if (ent.isDirectory()) {
        if (ent.name !== "node_modules" && ent.name !== "dist") walk(p);
      } else if (/\.(ts|mts|js|mjs|html|css)$/.test(ent.name) && !FRONT_SKIP.some((re) => re.test(p))) {
        const src = fs.readFileSync(p, "utf8");
        for (const m of src.matchAll(/(?:from\s*|import\s*\(?\s*|@import\s+)["']([^"'.\/][^"']*)["']/g)) {
          const spec = m[1];
          if (spec.startsWith("node:") || spec.startsWith("@shared")) continue;
          const parts = spec.split("/");
          names.add(spec.startsWith("@") ? parts.slice(0, 2).join("/") : parts[0]);
        }
      }
    }
  };
  for (const d of FRONT_DIRS) walk(path.join(root, d));
  return names;
}

/** 从 package-lock.json 里找出要打包的包和它们的依赖 */
function npmDeps() {
  const lock = JSON.parse(fs.readFileSync(path.join(root, "package-lock.json"), "utf8"));
  const all = lock.packages;
  const top = all[""];
  const imported = importedPackages();
  const declared = { ...top.dependencies, ...top.devDependencies };
  const missing = [...imported].filter((n) => !declared[n]);
  if (missing.length) fail(`前端代码 import 了 package.json 里没有的包：${missing.join("、")}`);
  const starts = new Set([...Object.keys(top.dependencies ?? {}), ...imported]);

  /** 照 node 找包的规矩：从 from 所在的 node_modules 一层层往外找 name */
  const resolve = (from, name) => {
    let base = from;
    for (;;) {
      const key = (base ? base + "/" : "") + "node_modules/" + name;
      if (all[key]) return key;
      if (!base) return null;
      const i = base.lastIndexOf("/node_modules/");
      base = i < 0 ? "" : base.slice(0, i);
    }
  };
  const seen = new Set();
  const stack = [...starts].map((n) => resolve("", n));
  while (stack.length) {
    const key = stack.pop();
    if (!key || seen.has(key)) continue;
    seen.add(key);
    const p = all[key];
    const deps = { ...p.dependencies, ...p.optionalDependencies, ...p.peerDependencies };
    for (const d of Object.keys(deps)) {
      const k = resolve(key, d);
      if (k) stack.push(k);
      else if (!p.optionalDependencies?.[d] && !p.peerDependenciesMeta?.[d]?.optional) fail(`package-lock.json 里找不到 ${key} 要的 ${d}（先 npm install）`);
    }
  }
  const rows = new Map();
  for (const key of seen) {
    const p = all[key];
    const name = key.slice(key.lastIndexOf("node_modules/") + "node_modules/".length);
    let license = typeof p.license === "string" ? p.license : p.license?.type ?? "";
    // 锁文件里偶尔没写，装过的话去 node_modules 里的 package.json 看一眼
    if (!license) {
      try {
        const pj = JSON.parse(fs.readFileSync(path.join(root, key, "package.json"), "utf8"));
        license = typeof pj.license === "string" ? pj.license : pj.license?.type ?? "";
      } catch {}
    }
    rows.set(`${name}@${p.version}`, {
      name,
      version: p.version,
      license,
      spdx: license,
      url: `https://www.npmjs.com/package/${name}`,
    });
  }
  return [...rows.values()].sort((a, b) => a.name.localeCompare(b.name));
}

// ---------- 写出来 ----------

const esc = (s) => String(s).replace(/\|/g, "\\|");

const rust = rustDeps();
const npm = npmDeps();
const flagged = [
  ...rust.filter((r) => !compatible(r.spdx)).map((r) => ({ ...r, kind: "Rust" })),
  ...npm.filter((r) => !compatible(r.spdx)).map((r) => ({ ...r, kind: "npm" })),
  ...MANUAL.filter((m) => !m.ok && !compatible(m.license)).map((m) => ({ name: m.name, version: "", license: m.license, kind: "素材" })),
];

const lines = [
  "# 第三方许可证清单",
  "",
  "叮咪的代码用 [AGPL-3.0](LICENSE) 许可证。叮咪里用到了下面这些别人的代码、字体和图标，它们各自的许可证照旧有效。",
  "",
  "这份清单由 `node scripts/licenses.mjs` 生成，改了依赖以后重新跑一次，不要手改。",
  "只列编进程序、打进界面的；只在开发、打包、测试时用的工具（Vite、TypeScript、Vitest 等）不列。",
  "",
  "## 字体、图标和内嵌的代码",
  "",
  "| 名字 | 用在哪 | 许可证 | 说明 |",
  "| --- | --- | --- | --- |",
  ...MANUAL.map((m) => `| ${esc(m.name)} | ${esc(m.where)} | ${esc(m.license)} | ${esc(m.note)} |`),
  "",
  "团子猫和程序图标是叮咪自己画的，不是第三方素材；它们的使用规则见 [TRADEMARK.md](TRADEMARK.md)。",
  "",
  `## Rust 依赖（${rust.length} 个）`,
  "",
  "电脑端按 Windows 算，服务器按 Linux 和 Windows 算。「用在」：电脑端 = dingmi，服务器 = dingmi-server，同步库 = dingmi-sync。",
  "",
  "| 名字 | 版本 | 许可证 | 用在 |",
  "| --- | --- | --- | --- |",
  ...rust.map((r) => `| [${esc(r.name)}](${r.url}) | ${r.version} | ${esc(r.license || "（没写）")} | ${r.users.join("、")} |`),
  "",
  `## npm 依赖（${npm.length} 个）`,
  "",
  "打进电脑端界面和手机网页的包，以及它们用到的包。",
  "",
  "| 名字 | 版本 | 许可证 |",
  "| --- | --- | --- |",
  ...npm.map((r) => `| [${esc(r.name)}](${r.url}) | ${r.version} | ${esc(r.license || "（没写）")} |`),
  "",
];
if (flagged.length) {
  lines.push(
    "## 要看一眼的",
    "",
    "下面这些的许可证没写、或者不在常见的宽松许可证（MIT、Apache、BSD、ISC 等）里，要人工确认能不能和 AGPL-3.0 放在一起：",
    "",
    "| 哪里的 | 名字 | 版本 | 许可证 |",
    "| --- | --- | --- | --- |",
    ...flagged.map((f) => `| ${f.kind} | ${esc(f.name)} | ${f.version} | ${esc(f.license || "（没写）")} |`),
    "",
  );
}
const md = lines.join("\n");

console.log(`Rust ${rust.length} 个，npm ${npm.length} 个，字体图标等 ${MANUAL.length} 个`);
if (flagged.length) {
  console.log(`\n要看一眼的 ${flagged.length} 个：`);
  for (const f of flagged) console.log(`  ${f.kind}  ${f.name} ${f.version}  ${f.license || "（没写）"}`);
}
if (checkOnly) {
  process.exit(flagged.length ? 1 : 0);
}
fs.writeFileSync(outFile, md);
console.log(`\n写好了：${path.relative(root, outFile)}`);
