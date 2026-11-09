// 今天页认出时间（需求-0.4 第 31 节，设计稿 u11-mobile.html「认出时间：记成提醒」）：
// 输入框里的一句话用和电脑一样的本地规则认，认出带时间的提醒就在下面出一条「记成提醒」，点了直接存成提醒。
import { parseLocal, type ParsedReminder } from "../desktop/ui/lib/parse";
import { blankReminder, fmtMin, normalize, parseDT, type Repeat, type SReminder } from "./model";

export type { ParsedReminder };

export interface HitState {
  /** 输过同步密码（能看、能记） */
  unlocked: boolean;
  /** 长按「发送」换成了「存成笔记」 */
  noteMode: boolean;
  /** 选了几个图片、文件 */
  files: number;
}

/** 认出来的提醒；不该出（没输同步密码、存成笔记、带了图片文件、没认出提醒）时是空的 */
export function reminderHits(text: string, st: HitState, now = new Date()): ParsedReminder[] {
  if (!st.unlocked || st.noteMode || st.files > 0 || !text.trim()) return [];
  return parseLocal(text, now).filter((x): x is ParsedReminder => x.type === "reminder");
}

/** 存的时候用的时间和重复：照编辑面板保存时一样，排到下一次（显示和存的是同一个） */
export function hitWhen(p: ParsedReminder, now = new Date()): { due: string; repeat: Repeat } {
  const r = p.repeat;
  const repeat = { kind: r.kind, days: [...r.days], monthday: r.monthday, every: r.every, month: r.month ?? 0, nth: r.nth ?? 0 };
  return { due: fmtMin(normalize(repeat, parseDT(p.due) ?? now, now)), repeat };
}

/** 认出来的一条变成要存的提醒：和手机上新建提醒一样从 blankReminder 来 */
export function hitToReminder(p: ParsedReminder, now = new Date()): SReminder {
  const { due, repeat } = hitWhen(p, now);
  return {
    ...blankReminder(due),
    title: p.title,
    detail: p.detail,
    tag: p.tag,
    urgent: p.urgent,
    repeat,
    // 时间是先定的（没说几点、说得含糊）：和电脑上「整理一下」一样标成含糊
    vague: !!p.guess,
  };
}
