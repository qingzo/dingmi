// 电脑端和手机网页都用到的数据类型。电脑端从 lib/api.ts 引用，那里原样转出去

/** 分类的 id（默认的是 work / life / health / like / cycle，自己加的是随机的） */
export type Tag = string;

export interface TagDef {
  id: Tag;
  name: string;
  /** 色板里的名字，见 TAG_COLORS */
  color: string;
}
/** 格式见 docs/方案/同步方案.md 4.2 */
export type RepeatKind = "" | "none" | "daily" | "weekdays" | "weekly" | "monthly" | "interval" | "yearly" | "month_last" | "month_nth" | "lunar_yearly";

export interface Repeat {
  kind: RepeatKind;
  /** 星期几，1=周一 … 7=周日 */
  days: number[];
  /** 几号（农历时是农历的几日），0 沿用开始日期 */
  monthday: number;
  /** 每几天一次；每周时是每几周（0、1 每周，2 每两周） */
  every: number;
  /** 几月（农历时是农历的几月），0 沿用开始日期。0.2 加的，旧记录没有 */
  month?: number;
  /** 每月第几个周几：1–5，-1 最后一个。0.2 加的 */
  nth?: number;
}

export interface Reminder {
  id: number;
  /** 跨设备唯一的编号，新建时留空 */
  uid: string;
  title: string;
  detail: string;
  tag: Tag | null;
  urgent: boolean;
  /** "YYYY-MM-DD HH:MM" */
  due_at: string;
  repeat: Repeat;
  snoozed_until: string | null;
  done: boolean;
  /** 0 等待 1 弹出中 2 专注时攒着 3 已看过 */
  state: number;
  snooze_count: number;
  last_done_at: string | null;
  completed_at: string | null;
  /** 只在这几台电脑上提醒（设备 ID），空表示所有电脑 */
  only_on: string[];
  created_at: string;
  updated_at: string;
  /** 在回收站里时是删除的时间 */
  deleted_at: string | null;
  /** 从笔记里的待办设的提醒：哪条笔记（uid）、哪一行待办。完成时把那一行勾上 */
  note_uid: string | null;
  todo: string | null;
  /** 一句话说得含糊、时间是先定的（"改天""过几天"）；改过时间后变回 false */
  vague: boolean;
}
