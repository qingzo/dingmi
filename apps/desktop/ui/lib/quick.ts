// 一句话直接保存：快捷框和桌面组件共用。一边打字一边预览会被理解成什么，回车后由 AI（或本地规则）拆开存好
import { api, newNote, type Device, type Note, type Reminder } from "./api";
import { guessNoteCat, parseLocal, saveParsed, smartParse, type ParsedItem, type ParsedNote } from "./parse";
import { friendlyDay, friendlyDT, isRecurring, NO_DATE, repeatLabel } from "@shared/util";

/** 本地规则先猜一下：会不会记成提醒（几点、怎么重复、只在哪台电脑）、顺不顺便记笔记 */
export function guess(v: string, devices: Device[]) {
  const items = parseLocal(v, new Date(), devices);
  const r = items.find((i) => i.type === "reminder");
  const rem = r && r.type === "reminder" ? r : undefined;
  return {
    reminder: rem && {
      /** 待办写「待办」；只有日期的不写钟点 */
      when: (rem.due === NO_DATE ? "待办" : rem.allday ? friendlyDay(rem.due) : friendlyDT(rem.due)) + (isRecurring(rem.repeat) ? " · " + repeatLabel(rem.repeat, rem.due) : "") + (rem.urgent ? " · 紧急" : ""),
      title: rem.title,
      /** 说得含糊时先定的时间怎么来的（接着补一句时间，这里马上就变） */
      guess: rem.guess,
    },
    /** 一句话拆出了几条提醒 */
    count: items.filter((i) => i.type === "reminder").length,
    note: items.some((i) => i.type === "note"),
    place: rem?.only_on.length ? (devices.find((d) => d.uid === rem.only_on[0])?.name ?? "") : "",
  };
}

/** 「只记成笔记」：不设提醒，整句存成一条笔记 */
export function noteOnly(v: string): ParsedNote {
  const first = v.split("\n")[0];
  return { type: "note", title: first.length > 20 ? first.slice(0, 20) + "…" : first, body: v, tag: guessNoteCat(v), link: false };
}

/** 一起带着的图片（快捷框里粘贴的截图、截图记下来的那张，需求-1.x 第 42.6 节） */
export interface Pic {
  bytes: Uint8Array;
  mime: string;
  name: string;
}

/** 认出是不是同一张图（粘了两次）：大小 + 内容算个指纹（FNV-1a） */
export function picKey(b: Uint8Array): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < b.length; i++) h = Math.imul(h ^ b[i], 0x01000193);
  return `${b.length}:${(h >>> 0).toString(16)}`;
}

export interface Saved {
  /** 拆出来的条目 */
  items: ParsedItem[];
  /** 存好的（撤销时删掉这些） */
  reminders: Reminder[];
  notes: Note[];
  /** 想用 AI、但 AI 没成功，按本地规则存的 */
  aiFailed: boolean;
}

/** 拆开存好；asNote 时只记成笔记；带着图片的放进一条笔记 */
export async function quickSaveAll(v: string, useAI: boolean, devices: Device[], asNote = false, pics: Pic[] = []): Promise<Saved> {
  const res = asNote ? { items: [noteOnly(v)], error: undefined } : await smartParse(v, useAI, 12000, devices);
  const saved = await saveParsed(res.items);
  if (pics.length) await attachPics(saved, v, pics);
  return { items: res.items, ...saved, aiFailed: useAI && !asNote && !!res.error };
}

/** 图片放进笔记（需求-1.x 第 42.6 节）：拆出了笔记就放进第一条；只拆出了提醒，就新建一条笔记（标题同第一条提醒，正文是原话）关联它 */
async function attachPics(saved: { reminders: Reminder[]; notes: Note[] }, v: string, pics: Pic[]) {
  let note = saved.notes[0];
  if (!note) {
    const r = saved.reminders[0];
    note = await api.saveNote(newNote({ title: r?.title ?? v.split("\n")[0].slice(0, 20), body: v, reminder_id: r?.id ?? null }));
    saved.notes.push(note);
  }
  const md: string[] = [];
  for (const p of pics) md.push(`![](att:${await api.saveAttachment(p.bytes, p.mime, note.uid, p.name)})`);
  saved.notes[saved.notes.indexOf(note)] = await api.saveNote({ ...note, body: [note.body.trim(), md.join("\n")].filter(Boolean).join("\n\n") });
}

/** 拆开存好，回存了哪些 */
export async function quickSaveItems(v: string, useAI: boolean, devices: Device[]): Promise<ParsedItem[]> {
  return (await quickSaveAll(v, useAI, devices)).items;
}

/** 撤销刚记下的：删掉，再从回收站里彻底删除（本来就不想要） */
export async function undoSaved(s: Saved) {
  for (const n of s.notes) {
    await api.deleteNote(n.id);
    await api.trashAction("note", n.id, "purge");
  }
  for (const r of s.reminders) {
    await api.deleteReminder(r.id);
    await api.trashAction("reminder", r.id, "purge");
  }
}
