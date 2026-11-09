// 把手机网页打包成一个 HTML 文件：apps/server/web/app.html。dingmi-server 编译时把它打包进程序，
// 运行时再把 tweetnacl 填进 /*NACL*/ 的位置。用法：npm run build:mobile
import { build } from "esbuild";
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const here = (p) => fileURLToPath(new URL(p, import.meta.url));

// 手机上用到的图标名字：apps/mobile 里所有写成字符串的名字（"bell"）
const mobileSrc = readdirSync(here("./"))
  .filter((f) => f.endsWith(".ts") && !f.endsWith(".test.ts"))
  .map((f) => readFileSync(here("./" + f), "utf8"))
  .join("\n");
const usedIcons = new Set([...mobileSrc.matchAll(/"([a-z0-9-]+)"/g)].map((m) => m[1]));

/** 和电脑共用的几个文件，打进手机网页前改一点 */
const trim = {
  name: "mobile-trim",
  setup(b) {
    // 设计变量里的内置字体太大（2.6 MB），手机上用系统字体，@font-face 整段去掉
    b.onLoad({ filter: /styles[\\/]tokens\.css$/ }, (a) => ({
      contents: readFileSync(a.path, "utf8").replace(/@font-face\s*\{[^}]*\}/g, ""),
      loader: "css",
    }));
    // 图标表只留手机上用到的，其余的 lucide 图标就不会打包进来
    b.onLoad({ filter: /shared[\\/]icons\.ts$/ }, (a) => ({
      contents: readFileSync(a.path, "utf8").replace(/^\s*"([a-z0-9-]+)": \w+,\r?\n/gm, (line, name) => (usedIcons.has(name) ? line : "")),
      loader: "ts",
    }));
  },
};

const out = await build({
  entryPoints: [here("./app.ts")],
  bundle: true,
  minify: true,
  write: false,
  format: "iife",
  target: ["safari15"],
  outdir: "out",
  legalComments: "none",
  charset: "utf8",
  alias: {
    // 和电脑端共用的代码（md、表格、util、图标、团子猫）
    "@shared": here("../../shared"),
    // md.ts 用 Tauri 的 convertFileSrc 显示电脑上的图片；手机上图片另外解密，换成一个空函数
    "@tauri-apps/api/core": here("./tauri-stub.ts"),
  },
  plugins: [trim],
  logLevel: "warning",
});

const pick = (ext) => out.outputFiles.find((f) => f.path.endsWith(ext)).text;
// 内嵌进 <script>/<style> 里，不能出现结束标签
const js = pick(".js").replace(/<\/script/gi, "<\/script");
const css = pick(".css").replace(/<\/style/gi, "<\/style");
if (js.includes("/*NACL*/")) throw new Error("打包结果里出现了 /*NACL*/，服务器替换时会弄坏");
if (/woff2|@font-face/.test(css)) throw new Error("字体打进手机网页了，见上面的 mobile-trim");
const html = readFileSync(here("./index.html"), "utf8")
  .replace("/*CSS*/", () => css)
  .replace("/*APP*/", () => js);
writeFileSync(here("../server/web/app.html"), html);
console.log(`apps/server/web/app.html  ${(html.length / 1024).toFixed(0)} KB`);
