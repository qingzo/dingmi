// 叮咪同步（收费的官方同步）界面上要算的几样：兑换码怎么分组显示、容量写成多少 GB、套餐名、还剩几天。
// 只管算，不碰界面；规则见 docs/方案/同步方案.md 第 16 节

import type { FileInfo } from "@shared/md";
import type { AttachmentInfo, Device } from "./api";
import { T as TN } from "../text/notes";

/** 兑换码里不会有的字（容易看错，生成时就去掉了）：0 和 O、1 和 I */
const CONFUSING = /[01IO]/;

/** 兑换码只留字母和数字、变大写 */
const bare = (s: string) => s.toUpperCase().replace(/[^0-9A-Z]/g, "");

/** 兑换码按 DM-XXXX-XXXX-XXXX-XXXX 分组显示：自动大写、去掉空格和多余的横线；DM 可写可不写，写了就当开头 */
export function formatRedeem(text: string): string {
  const raw = bare(text);
  const [head, body] = raw.startsWith("DM") ? ["DM", raw.slice(2, 18)] : ["", raw.slice(0, 16)];
  const groups = body.match(/.{1,4}/g) ?? [];
  return [head, ...groups].filter(Boolean).join("-");
}

/** 输入框里改成分组的样子，光标还停在原来那个字后面 */
export function reformatRedeem(input: HTMLInputElement) {
  const before = input.value;
  const next = formatRedeem(before);
  if (next === before) return;
  // 光标前面有几个字母数字
  const at = bare(before.slice(0, input.selectionStart ?? before.length)).length;
  input.value = next;
  let pos = 0;
  for (let n = 0; pos < next.length && n < at; pos++) if (/[0-9A-Z]/.test(next[pos])) n++;
  input.setSelectionRange(pos, pos);
}

/** 兑换码填完整了没有：完整的回 null，不然回差在哪（给人看的字由调用的地方定） */
export function redeemProblem(text: string): "empty" | "short" | "confusing" | null {
  const raw = bare(text);
  if (!raw) return "empty";
  const body = raw.length === 18 && raw.startsWith("DM") ? raw.slice(2) : raw;
  if (body.length !== 16) return "short";
  if (CONFUSING.test(body)) return "confusing";
  return null;
}

/** 字节写成「320 MB」「0.62 GB」「1 GB」 */
export function fmtSize(bytes: number): string {
  const gb = 1024 ** 3;
  const mb = 1024 ** 2;
  if (bytes >= gb) {
    const v = bytes / gb;
    // 1 GB、1.5 GB、1.25 GB；10 GB 以上只留一位小数
    return `${v >= 10 ? Math.round(v * 10) / 10 : Math.round(v * 100) / 100} GB`;
  }
  if (bytes >= 100 * mb) return `${(bytes / gb).toFixed(2)} GB`;
  if (bytes >= mb) return `${Math.round(bytes / mb)} MB`;
  if (bytes > 0) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  return "0 MB";
}

/** 用了多少（百分比，0–100）；不限容量是 null */
export function usedPct(used: number, quota: number | null): number | null {
  if (!quota) return null;
  return Math.min(100, Math.max(0, (used / quota) * 100));
}

/** 套餐名：「标准」写成「标准会员」；本来就带「会员」「套餐」的不加 */
export const planName = (plan: string) => (!plan ? "叮咪同步" : /(会员|套餐|体验)$/.test(plan) ? plan : `${plan}会员`);

/** 附件给文件卡片、图片看的情况：叮咪同步的套餐不传太大的，原文件只在一台设备上时加一句 away（需求-1.0 第 32.1 节第 5 条） */
export function attView(a: AttachmentInfo, devices: Device[]): FileInfo {
  const info: FileInfo = { size: a.size, local: a.local };
  if (!a.only_on) return info;
  const me = devices.find((d) => d.me)?.uid;
  if (a.local || a.only_on === me) return { ...info, away: TN.onlyHere };
  const pc = devices.find((d) => d.uid === a.only_on)?.name ?? TN.someOtherPc;
  return { ...info, away: TN.onlyOn(pc) };
}

/** 「5 月 3 日」：公测用到哪天、续费最多续到哪天（YYYY-MM-DD → 给人看的） */
export function monthDay(day: string): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day);
  if (!m) return day;
  const sameYear = Number(m[1]) === new Date().getFullYear();
  return `${sameYear ? "" : `${m[1]} 年 `}${Number(m[2])} 月 ${Number(m[3])} 日`;
}

/** 邮箱的简单核对（和服务器一样：一个 @，后面是带点的域名，没有空格） */
export function emailOk(s: string): boolean {
  const e = s.trim();
  if (!e || e.length > 100 || /[\s<>()[\],;:"\\]/.test(e)) return false;
  const at = e.indexOf("@");
  if (at <= 0 || e.indexOf("@", at + 1) >= 0) return false;
  const host = e.slice(at + 1);
  return host.includes(".") && !host.startsWith(".") && !host.endsWith(".") && !host.includes("..");
}

/** 用手机加入：两次问服务器之间至少隔多久（服务器平时会等手机 20 秒才回，回得快的时候别一直问） */
export const PHONE_GAP_MS = 1500;
/** 服务器给的数字一般 10 分钟有效；算出来比这还长的，多半是两边钟点（时区）对不上 */
const PHONE_TTL_MAX_MS = 15 * 60_000;

/** 数字什么时候过期（按这台电脑的钟）。expires_at 是服务器那边的「YYYY-MM-DD HH:MM:SS」；
 *  算出来已经过了、或者远得不像话（两边时区不一样），就不按它停，等服务器说过期 */
export function phoneDeadline(expiresAt: string, now: number): number | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2})(?::(\d{2}))?/.exec(expiresAt);
  if (!m) return null;
  const end = new Date(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +(m[6] ?? 0)).getTime();
  return end > now && end - now <= PHONE_TTL_MAX_MS ? end : null;
}

/** 等手机允许：一轮一轮问服务器（每轮服务器最多等 20 秒），两轮之间至少隔 PHONE_GAP_MS，过了有效期就停。
 *  alive() 变成 false（换了别的方式、退回上一步、又要了一个新数字）就不问了。
 *  回 ok 手机允许了 / expired 数字过期了 / stop 不用等了；问的时候出错照样抛出去 */
export async function waitPhone(
  expiresAt: string,
  alive: () => boolean,
  poll: () => Promise<boolean>,
  clock: { now: () => number; sleep: (ms: number) => Promise<void> } = { now: () => Date.now(), sleep: (ms) => new Promise((r) => setTimeout(r, ms)) },
): Promise<"ok" | "expired" | "stop"> {
  const end = phoneDeadline(expiresAt, clock.now());
  while (alive()) {
    if (end !== null && clock.now() >= end) return "expired";
    if (await poll()) return alive() ? "ok" : "stop";
    if (!alive()) break;
    await clock.sleep(PHONE_GAP_MS);
  }
  return "stop";
}
