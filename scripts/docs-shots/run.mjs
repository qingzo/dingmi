// 截图驱动：node run.mjs <步骤文件.mjs>
// 步骤文件 export default async ({ page, shot, invoke, wait, b }) => { ... }
import { chromium } from 'playwright-core';
import { mkdirSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const OUT = process.env.SHOT_DIR || 'out';
mkdirSync(OUT, { recursive: true });
const b = await chromium.connectOverCDP('http://127.0.0.1:9333');
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

function all() {
  return b.contexts().flatMap((c) => c.pages());
}
/** 按窗口的网页名找（main 是 index.html），等最多 10 秒 */
async function page(name, secs = 10) {
  const file = name === 'main' ? 'index' : name;
  for (let i = 0; i < secs * 5; i++) {
    const p = all().find((p) => {
      const path = new URL(p.url()).pathname;
      return path === `/${file}.html` || (file === 'index' && path === '/');
    });
    if (p) {
      await p.waitForLoadState('domcontentloaded').catch(() => {});
      return p;
    }
    await wait(200);
  }
  throw new Error(`没找到窗口 ${name}：${all().map((p) => p.url()).join(', ')}`);
}
/** 截图前去掉「测试」标签，等动画停下 */
async function shot(p, name, opts = {}) {
  await p.evaluate(() => document.querySelectorAll('#brand .tag').forEach((e) => e.remove())).catch(() => {});
  await wait(opts.settle ?? 600);
  const path = `${OUT}/${name}.png`;
  await p.screenshot({ path, ...opts.pw });
  console.log('截了', path);
}
/** 调叮咪的命令（从任意一个窗口里调） */
async function invoke(cmd, args = {}) {
  const p = all().find((p) => p.url().startsWith('http://localhost:1420/'));
  return p.evaluate(([c, a]) => window.__TAURI_INTERNALS__.invoke(c, a), [cmd, args]);
}

const step = await import(pathToFileURL(process.argv[2]).href);
try {
  await step.default({ page, shot, invoke, wait, b, all });
} finally {
  await b.close();
}
