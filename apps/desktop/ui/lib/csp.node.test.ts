// 内容安全策略（CSP，安全审查第 4、7 条）：电脑端 tauri.conf.json 和手机网页都不许内联脚本、内联事件。
// 开了 CSP 以后写在 HTML 字符串里的 onclick= / onerror= 会被浏览器直接拦掉（功能坏了也不报错），这里把代码扫一遍。
// 要读文件，只在 vitest（Node）里跑（tsconfig 里排除了 *.node.test.ts）
import { describe, expect, it } from "vitest";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { renderMd } from "@shared/md";

const root = fileURLToPath(new URL("../../../../", import.meta.url));

function files(dir: string, ext: RegExp): string[] {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    if (statSync(p).isDirectory()) return n === "node_modules" || n === "dist" ? [] : files(p, ext);
    return ext.test(n) && !n.includes(".test.") ? [p] : [];
  });
}

const sources = [
  ...files(join(root, "apps/desktop/ui"), /\.(ts|html)$/),
  ...files(join(root, "apps/mobile"), /\.(ts|html)$/),
  ...files(join(root, "shared"), /\.ts$/),
];

/** 一条 CSP 拆成 指令 → 来源 */
function parse(csp: string): Record<string, string[]> {
  return Object.fromEntries(
    csp
      .split(";")
      .map((d) => d.trim().split(/\s+/))
      .filter((d) => d[0])
      .map(([k, ...v]) => [k, v]),
  );
}

describe("电脑端的 CSP", () => {
  const conf = JSON.parse(readFileSync(join(root, "apps/desktop/src-tauri/tauri.conf.json"), "utf8"));
  const sec = conf.app.security;

  it("开着，脚本只认程序自己带的", () => {
    for (const csp of [sec.csp, sec.devCsp]) {
      expect(typeof csp).toBe("string");
      const d = parse(csp);
      expect(d["script-src"]).toEqual(["'self'"]);
      expect(d["default-src"]).toEqual(["'self'"]);
      expect(d["object-src"]).toEqual(["'none'"]);
      expect(d["base-uri"]).toEqual(["'none'"]);
      // 不能连外面：AI、同步、检查更新都是程序（Rust）去连，网页只和程序说话
      expect(d["connect-src"].filter((s) => /^https?:\/\//.test(s) && !/\.localhost$/.test(s) && !s.startsWith("http://localhost"))).toEqual([]);
      expect(d["img-src"]).not.toContain("https:");
    }
    // 样式里有 style="…"（CodeMirror 也会插 <style>），所以 style-src 要 'unsafe-inline'；
    // Tauri 往 style-src 里加 nonce 的话 'unsafe-inline' 就失效了，要关掉
    expect(sec.dangerousDisableAssetCspModification).toEqual(["style-src"]);
  });

  it("网页里没有内联脚本", () => {
    for (const f of files(join(root, "apps/desktop/ui"), /\.html$/)) {
      const html = readFileSync(f, "utf8");
      for (const m of html.matchAll(/<script\b([^>]*)>/gi)) expect(m[1], relative(root, f)).toMatch(/\bsrc=/);
    }
  });
});

describe("代码里没有 CSP 会拦掉的写法", () => {
  it("HTML 字符串里没有 onclick= / onerror= 这类内联事件，没有 javascript: 链接、eval、new Function", () => {
    const bad: string[] = [];
    for (const f of sources) {
      const src = readFileSync(f, "utf8");
      src.split("\n").forEach((line, i) => {
        const at = `${relative(root, f)}:${i + 1}`;
        // 标签里的 on 开头的属性：<img … onerror="…">、<button onclick=…>
        if (/<[a-z][^<>]*\son[a-z]+\s*=/i.test(line)) bad.push(`${at} 内联事件`);
        if (/["'`]javascript:/i.test(line)) bad.push(`${at} javascript: 链接`);
        if (/\beval\s*\(|\bnew Function\s*\(/.test(line)) bad.push(`${at} eval`);
        if (/setTimeout\(\s*["'`]|setInterval\(\s*["'`]/.test(line)) bad.push(`${at} 字符串当代码`);
      });
    }
    expect(bad).toEqual([]);
  });

  it("手机网页只有打包时填进去的两段内联脚本（服务器按内容算哈希放进 CSP）", () => {
    const html = readFileSync(join(root, "apps/mobile/index.html"), "utf8");
    const inline = [...html.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/gi)].map((m) => m[1]);
    expect(inline).toEqual(["/*NACL*/", "/*APP*/"]);
  });

  it("笔记里的图片：读不到时的提示不写成 onerror=，文件名转义过", () => {
    const html = renderMd("![](att:a-1.png)\n[报价.pdf](att:b.pdf)");
    expect(html).not.toMatch(/\son[a-z]+=/i);
    expect(html).toContain('data-att="a-1.png"');
  });
});
