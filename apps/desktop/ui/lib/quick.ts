// 一句话直接保存：快捷框和桌面组件共用。一边打字一边预览会被理解成什么，回车后由 AI（或本地规则）拆开存好
import { api, type Device, type Note, type Reminder } from "./api";
import { guessNoteCat, parseLocal, saveParsed, smartParse, type ParsedItem, type ParsedNote } from "./parse";
import { friendlyDT, isRecurring, repeatLabel } from "@shared/util";

/** 本地规则先猜一下：会不会记成提醒（几点、怎么重复、只在哪台电脑）、顺不顺便记笔记 */
export function guess(v: string, devices: Device[]) {
  const items = parseLocal(v, new Date(), devices);
  const r = items.find((i) => i.type === "reminder");
  const rem = r && r.type === "reminder" ? r : undefined;
  return {
    reminder: rem && {
      when: friendlyDT(rem.due) + (isRecurring(rem.repeat) ? " · " + repeatLabel(rem.repeat, rem.due) : "") + (rem.urgent ? " · 紧急" : ""),
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

export interface Saved {
  /** 拆出来的条目 */
  items: ParsedItem[];
  /** 存好的（撤销时删掉这些） */
  reminders: Reminder[];
  notes: Note[];
  /** 想用 AI、但 AI 没成功，按本地规则存的 */
  aiFailed: boolean;
}

/** 拆开存好；asNote 时只记成笔记 */
export async function quickSaveAll(v: string, useAI: boolean, devices: Device[], asNote = false): Promise<Saved> {
  const res = asNote ? { items: [noteOnly(v)], error: undefined } : await smartParse(v, useAI, 12000, devices);
  const saved = await saveParsed(res.items);
  return { items: res.items, ...saved, aiFailed: useAI && !asNote && !!res.error };
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
