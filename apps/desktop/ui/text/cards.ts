// 提醒卡片、每日开场白、每天收尾上的字（U5）。用词照 U13，语气照需求-0.2 第 7.5 节：
// 按钮不卖萌；带猫说的几句（开场白、收尾）关掉团子猫时换平实的说法，cat 为 true 时是开着团子猫

/** 几分钟前 / 几小时前 / 几天前 */
const agoMin = (m: number) => (m < 60 ? `${m} 分钟前` : m < 24 * 60 ? `${Math.floor(m / 60)} 小时前` : `${Math.floor(m / 1440)} 天前`);

export const remindText = {
  ring: (time: string) => `叮！${time}`,
  late: (time: string) => `过点了 · 原定 ${time}`,
  lateAgo: agoMin,
  more: (n: number) => `还有 ${n} 条`,
  next: (titles: string[]) => `接下来：${titles.join(" · ")}`,
  urgent: "紧急",
  snoozed: (n: number) => `已推迟 ${n} 次`,
  onlyOn: (names: string[]) => `只在${names.join("、")}`,
  openNote: "打开笔记",
  done: "完成",
  doneOnce: "完成这次",
  snooze: (min: number) => `${min} 分钟后`,
  tomorrow: "明天再说",
  pick: "推迟到…",
  allDone: "全部完成",
  allSnooze: (min: number) => `全部 ${min} 分钟后`,
  close: (min: number) => `${min} 分钟后再提醒（回车 = 完成，Esc = ${min} 分钟后）`,
  // 推迟到…
  pickLabel: "推迟到",
  quick: { half: "半小时后", hour: "1 小时后", three: "3 小时后", tonight: "今晚 8 点", tmrMorning: "明早 9 点", tmrAfternoon: "明天下午 2 点", monday: "下周一 9 点" },
  pickHint: "打一句话，比如「周三下午」「明晚 8 点」",
  pickNone: "没看懂，换个说法？",
  pickPast: "这个时间已经过了",
  pickTo: (when: string) => `推迟到${when}`,
  pickGo: "推迟",
  back: "返回",
  // 专注结束后一起弹
  heldWhen: (n: number) => `专注时攒了 ${n} 条`,
  heldTitle: "这些在你专注时到点了",
  later: "稍后",
  // 别的电脑处理了
  elsewhere: (where: string, what: "done" | "moved" | "deleted") =>
    `${where || "别的电脑"}上已经${what === "done" ? "完成了" : what === "moved" ? "改了时间" : "删掉了"}`,
  failed: (why: string) => `没操作成功：${why}`,
};

export const morningText = {
  hello: (cat: boolean, h: number) =>
    cat ? (h < 11 ? "早呀！" : h < 13 ? "中午好呀！" : h < 18 ? "下午好呀！" : "晚上好呀！") : h < 11 ? "早上好" : h < 13 ? "中午好" : h < 18 ? "下午好" : "晚上好",
  /** 上面那段话；top 是最要紧那件的「时间 标题」（已转义） */
  say: (today: number, top: string, late: number, lateYesterday: boolean) => {
    const p: string[] = [];
    if (today) p.push(`今天有 <b>${today} 件事</b>${top ? `，最要紧的是 <b>${top}</b>` : ""}。`);
    if (late) p.push(`${lateYesterday ? "昨天" : "之前"}还剩 ${late} 件没做完，要挪到今天吗？`);
    return p.join("");
  },
  free: (cat: boolean, hotkey: string) =>
    cat ? `今天没有安排，轻松的一天～想到什么随时按 <b>${hotkey}</b> 记下来。` : `今天没有安排。想到什么随时按 <b>${hotkey}</b> 记下来。`,
  aiWriting: "AI 正在写今天的开场白…",
  today: "今天",
  top: "最要紧",
  restMore: (n: number) => `还有 ${n} 件…`,
  late: "之前没做完",
  moveToday: "挪到今天",
  notes: (yesterdayOnly: boolean) => (yesterdayOnly ? "昨天记的笔记" : "最近记的笔记"),
  look: "看看",
  count: (n: number) => `${n} 件`,
  noteCount: (n: number) => `${n} 条`,
  off: "在 设置 → 提醒 里可以关掉",
  openToday: "打开今天",
  start: "开始吧",
  ok: "好的",
  close: "关掉",
};

export const wrapText = {
  hello: (cat: boolean, allDone: boolean) => (allDone ? (cat ? "全做完啦！" : "今天的事都做完了") : cat ? "今天辛苦啦" : "今天辛苦了"),
  say: (cat: boolean, done: number, left: number) => {
    if (!left && !done) return cat ? "今天没有要做的事，早点休息～" : "今天没有安排。";
    if (!left) return cat ? `今天 <b>${done} 件事</b>一件不落，早点休息～` : `今天 <b>${done} 件事</b>都做完了。`;
    const d = done ? (cat ? `做完了 <b>${done} 件</b>` : `今天做完 <b>${done} 件</b>`) : "今天还没有勾掉的事";
    return `${d}，还剩 <b>${left} 件</b>${cat ? "。剩下的挪到明天吗？" : "，挪到明天吗？"}`;
  },
  done: "做完了",
  left: "还剩",
  doneMore: (n: number) => `还有 ${n} 件`,
  moveTmr: "挪到明天",
  allTmr: "全部挪到明天",
  tmrHint: "挪到明天：时间不变，日期改成明天",
  finish: "收工",
  count: (n: number) => `${n} 件`,
  failed: (why: string) => `没操作成功：${why}`,
};

const WD = "日一二三四五六";
const hm = (d: Date) => `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
const md = (d: Date, now: Date) => `${d.getFullYear() !== now.getFullYear() ? `${d.getFullYear()} 年 ` : ""}${d.getMonth() + 1} 月 ${d.getDate()} 日`;
const days = (d: Date, now: Date) =>
  Math.round((new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime() - new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()) / 86400000);

/** 卡片顶上的日期：11 月 6 日 周四 */
export const dateLine = (d: Date) => `${d.getMonth() + 1} 月 ${d.getDate()} 日 周${WD[d.getDay()]}`;

/** 按钮上的时间（U13 时间写法）：今天 15:00 / 明天 09:00 / 周三 15:00 / 11 月 12 日 15:00 */
export function whenShort(d: Date, now = new Date()): string {
  const n = days(d, now);
  if (n === 0) return `今天 ${hm(d)}`;
  if (n === 1) return `明天 ${hm(d)}`;
  if (n > 1 && n < 7) return `周${WD[d.getDay()]} ${hm(d)}`;
  return `${md(d, now)} ${hm(d)}`;
}

/** 过去的时间：今天的只写 11:00，昨天 11:00，再早写 11 月 3 日 11:00 */
export function whenPast(d: Date, now = new Date()): string {
  const n = days(d, now);
  if (n === 0) return hm(d);
  if (n === -1) return `昨天 ${hm(d)}`;
  return `${md(d, now)} ${hm(d)}`;
}

/** 推迟到…输入框右边：11 月 12 日 周三 15:00 */
export const whenFull = (d: Date, now = new Date()) => `${md(d, now)} 周${WD[d.getDay()]} ${hm(d)}`;
