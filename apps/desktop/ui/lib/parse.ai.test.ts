// 拿 parse.cases.ts 抽查 AI（需求-0.2 8.6）：提示词和 ai_parse 一样，从 ai.rs 里原样读出来。
// 要真的调 DeepSeek，平时跳过。用法（PowerShell）：
//   $env:DINGMI_AI_KEY = "sk-..."; npx vitest run parse.ai
// 结果写在 release/ai-check.md。AI 每次说法不完全一样，标题只记下来不算错；时间、周期、紧急、含糊、条数对不上算错。
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { setNoteCats, setTags } from "@shared/util";
import { CASES, type Case } from "./parse.cases";
import { fixLunar, normalizeAI, type ParsedReminder } from "./parse";
import type { Device } from "./api";

const KEY = process.env.DINGMI_AI_KEY ?? "";
const BASE = (process.env.DINGMI_AI_BASE ?? "https://api.deepseek.com").replace(/\/$/, "");
const MODEL = process.env.DINGMI_AI_MODEL ?? "deepseek-chat";
const NOW = "2026-11-05 10:00";
const WEEK = ["日", "一", "二", "三", "四", "五", "六"];

const root = new URL("../../../../", import.meta.url);
const rs = readFileSync(new URL("apps/desktop/src-tauri/src/ai.rs", root), "utf8");
// 和 parse_prompt 一样：没有分类时 tag 一律 null
const PROMPT = rs
  .match(/const PARSE_PROMPT: &str = r#"([\s\S]*?)"#;/)![1]
  .replace("{TAG_IDS}", "null")
  .replace("{NOTE_IDS}", "null")
  .replace("{TAG_RULE}", "- 用户没有设提醒分类，提醒的 tag 一律 null。\n- 用户没有设笔记分类，笔记的 tag 一律 null。");

const dt = (s: string) => {
  const [d, t] = s.split(" ");
  const [y, m, dd] = d.split("-").map(Number);
  const [h, mi] = t.split(":").map(Number);
  return new Date(y, m - 1, dd, h, mi);
};

async function ask(c: Case, now: string): Promise<unknown> {
  const names = (c.devices ?? []).map(([, n]) => n);
  let user = `现在是 ${now} 星期${WEEK[dt(now).getDay()]}。\n`;
  if (names.length > 1) user += `用户的电脑：${names.join("、")}\n`;
  user += `用户写的是：${c.say.trim()}`;
  for (let i = 0; ; i++) {
    const res = await fetch(`${BASE}/chat/completions`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: `Bearer ${KEY}` },
      body: JSON.stringify({
        model: MODEL,
        messages: [
          { role: "system", content: PROMPT },
          { role: "user", content: user },
        ],
        temperature: 0.3,
        stream: false,
        response_format: { type: "json_object" },
      }),
    });
    if (res.ok) return JSON.parse((await res.json()).choices[0].message.content);
    // 限流、服务器忙：等一下再试两次
    if (i < 2 && (res.status === 429 || res.status >= 500)) {
      await new Promise((r) => setTimeout(r, 3000 * (i + 1)));
      continue;
    }
    throw new Error(`AI 服务出错了（${res.status}）${await res.text()}`);
  }
}

type Row = { say: string; now: string; want: string; got: string; diff: string[] };
const wrong: Row[] = [];
const titles: Row[] = [];
let done = 0;

const show = (r: { title: string; due: string; kind: string; urgent: boolean; guess: boolean }[], notes: number) =>
  r.map((x) => `${x.title} · ${x.due}${x.kind !== "none" ? ` · ${x.kind}` : ""}${x.urgent ? " · 紧急" : ""}${x.guess ? " · 含糊" : ""}`).join("；") +
  (notes ? `（笔记 ${notes} 条）` : "");

describe.skipIf(!KEY)("AI 抽查：测试句子", () => {
  beforeAll(() => {
    setTags([]);
    setNoteCats([]);
  });

  afterAll(() => {
    const cell = (s: string) => s.replace(/\|/g, "\\|").replace(/\n/g, " ");
    const table = (rows: Row[], diff: boolean) =>
      [`| 句子 | 应该是 | AI 给的${diff ? " | 哪里不对" : ""} |`, `| --- | --- | ---${diff ? " | ---" : ""} |`]
        .concat(rows.map((r) => `| ${cell(r.say)}${r.now !== NOW ? `（${r.now}）` : ""} | ${cell(r.want)} | ${cell(r.got)}${diff ? ` | ${r.diff.join("、")}` : ""} |`))
        .join("\n");
    const md = [
      `# AI 抽查结果`,
      ``,
      `${new Date().toLocaleString("zh-CN")} · 模型 ${MODEL} · 共 ${CASES.length} 句，跑完 ${done} 句，对不上 ${wrong.length} 句，只有标题不一样 ${titles.length} 句`,
      ``,
      `## 对不上的`,
      ``,
      wrong.length ? table(wrong, true) : "没有",
      ``,
      `## 只有标题不一样（不算错，看看说法能不能接受）`,
      ``,
      titles.length ? table(titles, false) : "没有",
      ``,
    ].join("\n");
    mkdirSync(new URL("release/", root), { recursive: true });
    writeFileSync(new URL("release/ai-check.md", root), md);
  });

  for (const c of CASES) {
    const now = c.now ?? NOW;
    it.concurrent(`${c.say}${c.now ? `（${c.now}）` : ""}`, { timeout: 120_000 }, async () => {
      const devices: Device[] = (c.devices ?? []).map(([uid, name]) => ({ uid, name, me: false }));
      // 和 smartParse 一样：说了农历的，日子用本地规则算的
      const items = fixLunar(normalizeAI(await ask(c, now), devices, dt(now)), c.say, dt(now), devices);
      done++;
      // 拆成几条时 AI 给的先后不一定一样（先晚上再早上），两边都按时间排好再对
      const byDue = <T extends { due: string }>(a: T, b: T) => a.due.localeCompare(b.due);
      const full = (due: string) => (due.length === 11 ? `${now.slice(0, 4)}-${due}` : due);
      const rems = items.filter((i): i is ParsedReminder => i.type === "reminder").sort(byDue);
      const wants = c.want.map((w) => ({ ...w, due: full(w.due) })).sort(byDue);
      const got = rems.map((x) => ({ title: x.title, due: x.due, kind: x.repeat.kind, urgent: x.urgent, guess: !!x.guess }));
      const want = wants.map((w) => ({
        title: w.title,
        due: w.due,
        kind: w.repeat?.kind ?? "none",
        urgent: w.urgent ?? false,
        guess: !!w.guess,
      }));
      const diff: string[] = [];
      if (got.length !== want.length) diff.push(`提醒 ${got.length} 条（应该 ${want.length} 条）`);
      want.forEach((w, i) => {
        const g = got[i];
        if (!g) return;
        if (g.due !== w.due) diff.push("时间");
        if (g.kind !== w.kind) diff.push("周期");
        else if (wants[i].repeat) {
          for (const [k, v] of Object.entries(wants[i].repeat!))
            if (JSON.stringify(rems[i].repeat[k as keyof ParsedReminder["repeat"]]) !== JSON.stringify(v)) diff.push(`周期的 ${k}`);
        }
        if (g.urgent !== w.urgent) diff.push("紧急");
        if (g.guess !== w.guess) diff.push("含糊");
        if (c.only && JSON.stringify(rems[i].only_on) !== JSON.stringify(c.only)) diff.push("在哪台电脑");
      });
      const notes = items.length - rems.length;
      if (notes !== (c.notes ?? 0)) diff.push(`笔记 ${notes} 条（应该 ${c.notes ?? 0} 条）`);
      const row = { say: c.say, now, want: show(want, c.notes ?? 0), got: show(got, notes), diff };
      if (diff.length) wrong.push(row);
      else if (got.some((g, i) => g.title !== want[i].title)) titles.push(row);
      expect(diff).toEqual([]);
    });
  }
});
