// 手机版的数据：和电脑端同步的记录格式（apps/desktop/src-tauri/src/sync/records.rs）、周期提醒（recur.rs）、版本号（clock.rs）。
// 字段名和取值都要和电脑端一模一样，电脑端才认得手机改过的记录。
import { isRecurring, nextAfter, type RepeatRule } from "@shared/recur";

/** 格式见 docs/方案/同步方案.md 4.2；month、nth 是 0.2 加的，旧记录没有 */
export type Repeat = RepeatRule;

export interface SReminder {
  uid: string;
  title: string;
  detail: string;
  tag: string | null;
  urgent: boolean;
  /** "YYYY-MM-DD HH:MM" */
  due_at: string;
  repeat: Repeat;
  snoozed_until: string | null;
  done: boolean;
  snooze_count: number;
  last_done_at: string | null;
  completed_at: string | null;
  only_on: string[];
  created_at: string;
  updated_at: string;
  deleted_at: string | null;
  purged: boolean;
  note_uid?: string | null;
  todo?: string | null;
  /** 一句话说得含糊、时间是先定的（0.2 加的，旧记录没有） */
  vague?: boolean;
}

export interface SNote {
  uid: string;
  title: string;
  body: string;
  tag: string | null;
  reminder_uid: string | null;
  created_at: string;
  updated_at: string;
  deleted_at: string | null;
  purged: boolean;
  /** 置顶的时间（电脑端 0.3.5 加的；改笔记时在原记录上改，会原样带着） */
  pinned_at?: string | null;
}

export interface SAttachment {
  uid: string;
  file: string;
  mime: string;
  size: number;
  sha256: string;
  note_uid: string | null;
  created_at: string;
  deleted_at: string | null;
  purged: boolean;
  /** 文件附件的原文件名，图片没有 */
  name?: string;
  /** 叮咪同步的套餐不传它（太大）：原文件只在这台设备上（设备编号），服务器上没有 */
  only_on?: string | null;
}

/** 叮咪同步的套餐对单个图片、单个文件的限额（服务器 /m/api/limits 说的，字节；null 不限） */
export interface Limits {
  max_image: number | null;
  max_file: number | null;
}

/** 这个要传的东西超没超套餐（需求-1.0 第 32.1 节第 5 条）：图片超了要压缩（"image"），文件超了传不了（"file"），没超是 null */
export function overLimit(mime: string, size: number, l: Limits): "image" | "file" | null {
  if (mime.startsWith("image/")) return l.max_image != null && size > l.max_image ? "image" : null;
  return l.max_file != null && size > l.max_file ? "file" : null;
}

export interface SShared {
  uid: string;
  value: string;
}

/** 解密后的一条记录：{"k": 类型, "d": 内容}。手机上用不到的类型（收件箱、设备）原样留着不碰 */
export type Payload =
  | { k: "reminder"; d: SReminder }
  | { k: "note"; d: SNote }
  | { k: "attachment"; d: SAttachment }
  | { k: "shared"; d: SShared }
  | { k: "inbox" | "device"; d: { uid: string; name?: string } };

export interface TagDef {
  id: string;
  name: string;
  color: string;
}

/** 笔记分类（v0.3 起和提醒的分开），电脑上没同步过来时用这几个，和 settings.rs 的默认值一样 */
export const DEFAULT_NOTE_CATS: TagDef[] = [
  { id: "toread", name: "待看", color: "orange" },
  { id: "idea", name: "灵感", color: "purple" },
  { id: "ref", name: "资料", color: "teal" },
];

export const DEFAULT_TAGS: TagDef[] = [
  { id: "work", name: "工作", color: "blue" },
  { id: "life", name: "生活", color: "orange" },
  { id: "health", name: "健康", color: "green" },
  { id: "like", name: "喜好", color: "pink" },
  { id: "cycle", name: "周期", color: "purple" },
];

// ---------- 时间 ----------

const pad = (n: number, w = 2) => String(n).padStart(w, "0");
export const dayStr = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
/** 提醒的时间："YYYY-MM-DD HH:MM" */
export const fmtMin = (d: Date) => `${dayStr(d)} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
/** 创建、修改时间："YYYY-MM-DD HH:MM:SS" */
export const fmtSec = (d: Date) => `${fmtMin(d)}:${pad(d.getSeconds())}`;
export const nowMin = () => fmtMin(new Date());
export const nowSec = () => fmtSec(new Date());

export function parseDT(s: string | null | undefined): Date | null {
  const m = s?.match(/^(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2})/);
  return m ? new Date(+m[1], +m[2] - 1, +m[3], +m[4], +m[5]) : null;
}

export const fireAt = (r: SReminder) => r.snoozed_until ?? r.due_at;
export { isRecurring };

// ---------- 周期提醒（和 recur.rs 一样，代码在 shared/recur.ts） ----------

export { matches, nextAfter, normalize } from "@shared/recur";

// ---------- 版本号（和 clock.rs 一样："<毫秒 13 位>-<计数 4 位>-<设备>"） ----------

export class Clock {
  private ms = 0;
  private n = 0;
  constructor(private device: string) {}

  observe(ver: string) {
    const [ms, n] = ver.split("-", 2).map(Number);
    if (!Number.isFinite(ms) || !Number.isFinite(n)) return;
    if (ms > this.ms || (ms === this.ms && n > this.n)) [this.ms, this.n] = [ms, n];
  }

  next(now = Date.now()): string {
    if (now > this.ms) [this.ms, this.n] = [now, 0];
    else if (this.n >= 9999) [this.ms, this.n] = [this.ms + 1, 0];
    else this.n++;
    return `${pad(this.ms, 13)}-${pad(this.n, 4)}-${this.device}`;
  }
}

/** UUID v7，和电脑端 Uuid::now_v7() 的格式一样 */
export function uuidv7(): string {
  const b = crypto.getRandomValues(new Uint8Array(16));
  let ms = Date.now();
  for (let i = 5; i >= 0; i--) {
    b[i] = ms % 256;
    ms = Math.floor(ms / 256);
  }
  b[6] = (b[6] & 0x0f) | 0x70;
  b[8] = (b[8] & 0x3f) | 0x80;
  const h = [...b].map((x) => x.toString(16).padStart(2, "0")).join("");
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}

// ---------- 提醒和笔记的规则（和电脑端一样） ----------

/** 一条新提醒：手机上「新提醒」和今天页「记成提醒」都从这里来 */
export function blankReminder(due_at: string): SReminder {
  const now = nowSec();
  return {
    uid: uuidv7(), title: "", detail: "", tag: null, urgent: false, due_at,
    repeat: { kind: "none", days: [], monthday: 0, every: 0, month: 0, nth: 0 }, snoozed_until: null, done: false, snooze_count: 0,
    last_done_at: null, completed_at: null, only_on: [], created_at: now, updated_at: now, deleted_at: null, purged: false,
    note_uid: null, todo: null,
  };
}

/** 完成一条提醒：一次性的标记完成，周期的排到下一次 */
export function complete(r: SReminder, now = new Date()): SReminder {
  const out = { ...r, snoozed_until: null };
  if (isRecurring(r.repeat)) {
    out.last_done_at = fmtMin(now);
    const base = parseDT(r.due_at);
    const next = base && nextAfter(r.repeat, base, base > now ? base : now);
    if (next) out.due_at = fmtMin(next);
  } else {
    out.done = true;
    out.completed_at = fmtMin(now);
  }
  return out;
}

export const undoComplete = (r: SReminder): SReminder => ({ ...r, done: false, completed_at: null });

/** 标题空着时，用正文第一行的前 20 个字 */
export function noteTitle(title: string, body: string): string {
  const t = title.trim();
  if (t) return t;
  const first =
    body
      .split("\n")
      .map((l) => l.trim().replace(/^[#>\- ]+/, "").trim())
      .find(Boolean) ?? "";
  const chars = [...first];
  return chars.length ? chars.slice(0, 20).join("") + (chars.length > 20 ? "…" : "") : "无标题";
}

/** 笔记里写着 todo 的那条待办勾上 / 取消（和 db.rs 的 toggle_todo_line 一样）；没找到或者已经是这样了返回 null */
export function toggleTodoLine(body: string, todo: string, done: boolean): string | null {
  const want = todo.trim();
  const lines = body.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const lead = line.length - line.trimStart().length;
    const rest = line.slice(lead);
    if (!(rest.startsWith("- ") || rest.startsWith("* "))) continue;
    const after = rest.slice(2).trimStart();
    const markAt = lead + (rest.length - after.length);
    const checked = after.startsWith("[x]") || after.startsWith("[X]");
    const open = after.startsWith("[ ]");
    if (!(checked || open) || after.slice(3).trim() !== want) continue;
    if (checked === done) return null;
    lines[i] = line.slice(0, markAt) + (done ? "[x]" : "[ ]") + line.slice(markAt + 3);
    return lines.join("\n");
  }
  return null;
}

// ---------- 附件（和 db.rs 的 save_file 一样的规则） ----------

/** 附件单个最大多大 */
export const MAX_ATTACHMENT = 50 * 1024 * 1024;

/** 原文件名只留最后一段，去掉 Windows 文件名里不能用的字符和 Markdown 会误会的方括号 */
export function cleanFileName(name: string): string {
  const base = name.split(/[/\\]/).pop() ?? "";
  // eslint-disable-next-line no-control-regex
  const s = base.replace(/[\u0000-\u001f\u007f<>:"|?*[\]]/g, "_").trim().replace(/\.+$/, "");
  return [...s].slice(0, 120).join("");
}

const IMAGE_EXT: Record<string, string> = {
  "image/png": "png", "image/jpeg": "jpg", "image/jpg": "jpg", "image/gif": "gif", "image/webp": "webp", "image/heic": "heic", "image/bmp": "bmp",
};

/** 存在电脑上的文件名用的扩展名：图片按类型，别的文件沿用原文件名的扩展名（只留字母数字） */
export function extFor(mime: string, name: string): string {
  if (IMAGE_EXT[mime]) return IMAGE_EXT[mime];
  const i = name.lastIndexOf(".");
  const ext = i >= 0 ? [...name.slice(i + 1)].filter((c) => /[a-z0-9]/i.test(c)).slice(0, 10).join("").toLowerCase() : "";
  return ext || "bin";
}
