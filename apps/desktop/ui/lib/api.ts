import { invoke } from "@tauri-apps/api/core";
import type { Reminder, Repeat, Tag, TagDef } from "@shared/types";
export type { Reminder, Repeat, RepeatKind, Tag, TagDef } from "@shared/types";

export interface Note {
  id: number;
  uid: string;
  title: string;
  body: string;
  tag: Tag | null;
  reminder_id: number | null;
  created_at: string;
  updated_at: string;
  deleted_at: string | null;
  /** 置顶的时间，没置顶是 null */
  pinned_at: string | null;
}

export interface Trash {
  reminders: Reminder[];
  notes: Note[];
}

export interface Settings {
  hk_flash: string;
  hk_qnote: string;
  hk_main: string;
  hk_focus: string;
  ai_base: string;
  ai_model: string;
  ai_key: string;
  morning: boolean;
  auto_focus: boolean;
  snooze_min: number;
  ring_every_min: number;
  sound: boolean;
  autostart: boolean;
  /** 每天收尾，wrap_time 是 "HH:MM" */
  wrap: boolean;
  wrap_time: string;
  /** 手机发来文字后自动「整理一下」 */
  inbox_auto: boolean;
  /** 开启同步后 AI Key 也同步到别的电脑 */
  sync_ai_key: boolean;
  /** 按闪念快捷键时带上别的程序里选中的文字 */
  flash_selection: boolean;
  /** 提醒的分类，几台电脑共用 */
  tags: TagDef[];
  /** 笔记的分类，几台电脑共用 */
  note_cats: TagDef[];
  /** 桌面组件（只属于这台电脑） */
  widget: boolean;
  /** 桌面组件里不显示的几块：next / today / toread / pins / input */
  widget_hide: string[];
  /** 桌面组件用毛玻璃的底（只属于这台电脑）；默认不透明 */
  widget_glass: boolean;
  /** 毛玻璃深浅 0–100，越大越像磨砂，50 是默认 */
  widget_glass_level: number;
  /** 启动器（只属于这台电脑） */
  launcher: LaunchConfig;
  /** 自动检查更新（只属于这台电脑） */
  auto_update: boolean;
  /** 外观，下面几项都只属于这台电脑（见 lib/look.ts） */
  style: LookStyle;
  theme: "system" | "light" | "dark";
  font_size: "s" | "m" | "l";
  /** 团子猫；null 表示没手动改过，跟着风格走 */
  mascot: boolean | null;
  /** 动画：on 启用 / system 跟随 Windows / reduce 减少 */
  motion: "on" | "system" | "reduce";
  /** 快捷框空着时「常用」显示几排（1 或 2；只属于这台电脑） */
  pin_rows: number;
  /** 空格启动（实验）：快捷框里打了字以后按空格也能打开选中的程序（只属于这台电脑） */
  flash_space: boolean;
  /** 随手算：快捷框里打算式直接出结果 */
  calc: boolean;
  /** 网页关键字，几台电脑共用 */
  web_keys: WebKey[];
}

/** 一份协议现在要显示什么（terms.rs View）：recent 是 30 天内更新过的日子（YYYY-MM-DD）；
 *  notice 是还没点过「知道了」的重大修改，started 是已经生效了 */
export interface TermsDoc {
  recent: string | null;
  notice: { version: string; effective: string; summary: string } | null;
  started: boolean;
}

/** 三份协议：用户协议、隐私说明、叮咪同步服务协议（键和官网 terms.json 一样） */
export interface TermsView {
  terms: TermsDoc;
  privacy: TermsDoc;
  sync_terms: TermsDoc;
}

/** 诊断包里的一样东西；path 是临时文件夹里的位置，「打开看看」用 */
export interface DiagPart {
  name: string;
  desc: string;
  path: string;
}

/** 一条网页关键字：快捷框里打「key 内容」用浏览器搜，url 里搜的内容写成 {q} */
export interface WebKey {
  name: string;
  key: string;
  url: string;
}

/** 内置风格：warm 暖云母 / simple 简洁 / calm 安静 */
export type LookStyle = "warm" | "simple" | "calm";

/** 启动器里能打开的：app 开始菜单里的程序 / store 微软商店应用 / desk 桌面上的快捷方式 / 自己加的 folder、file、link */
export type LaunchKind = "app" | "store" | "desk" | "folder" | "file" | "link";

export interface LaunchItem {
  id: string;
  name: string;
  kind: LaunchKind;
  /** 真正在哪（exe、快捷方式指向的地方、文件夹、网址），不知道时为空 */
  path: string;
  /** 名字一段一段的读法：一个汉字一段（多音字有几个），连着的字母数字一段（小写后面跟大写分开：ToDesk → to、desk） */
  pys: string[][];
  /** data:image/png… 拿不到时为空 */
  icon: string;
  /** 卸载、帮助这类，默认不出现 */
  junk: boolean;
  /** 自己加的文件、文件夹已经不在了 */
  missing: boolean;
  /** Windows 记的打开过几次（开始菜单「最常用」那份），刚开始用时拿来排顺序 */
  runs: number;
}

/** 设置里启动器那部分（只属于这台电脑） */
export interface LaunchConfig {
  /** 放在桌面组件「常用」里的 id，按顺序 */
  pins: string[];
  aliases: { alias: string; id: string }[];
  /** 自己加的：target 是路径或网址 */
  /** 自己加的；args 是打开时带的参数（拖进来的快捷方式里带的） */
  custom: { name: string; target: string; args?: string }[];
  hidden: string[];
  /** 关掉的来源：start / store / desktop */
  off: string[];
}

/** 用过几次、最后一次（毫秒）、打什么字时开的是哪个 */
export interface LaunchStats {
  n: Record<string, number>;
  last: Record<string, number>;
  q: Record<string, [string, number]>;
}

export interface FolderPeek {
  total: number;
  files: { name: string; path: string; dir: boolean; modified: number }[];
}

/** 叮咪同步：到期后接着用的那一段（换档时剩下的时长） */
export interface NextPlan {
  plan: string;
  /** 用到哪天（YYYY-MM-DD）；null 是不会到期 */
  until: string | null;
}

/** 叮咪同步：这个空间的账（服务器每次连上时告诉电脑，见 docs/方案/同步方案.md 第 16 节） */
export interface SyncAccount {
  /** 空间的短编号，比如 S-7K2M9，联系客服时报这个 */
  space: string;
  /** 服务器开了兑换码，也就是叮咪同步（自建服务器是 false，不用显示套餐和到期） */
  official: boolean;
  /** 套餐名，比如「标准」 */
  plan: string;
  /** 现在这个套餐到哪天（这一天还能用）；null 是不会到期 */
  expires_at: string | null;
  /** 换档后排在后面的 */
  next: NextPlan[];
  /** 全部到哪天；null 是不会到期 */
  until: string | null;
  /** 离 until 还有几天（那天是 0，过了是负数）；不会到期是 null */
  days_left: number | null;
  /** 到期了：停止同步，续费后接着同步 */
  expired: boolean;
  /** 到期了的话，哪天删服务器上的数据 */
  delete_on: string | null;
  /** 该提醒什么：expire_7d 7 天内到期 / expire_1d 明天（或今天）到期 / expired 已到期 / delete_7d 7 天后删服务器上的数据 */
  remind: "" | "expire_7d" | "expire_1d" | "expired" | "delete_7d";
  /** 用了多少、上限（字节）；quota 是 null 时不限 */
  used: number;
  quota: number | null;
  /** near_full 用了 90% 以上 / full 满了（新的图片和文件先只在这台电脑上，文字照常） */
  storage: "ok" | "near_full" | "full";
  /** 电脑加手机几台、最多几台（null 不限） */
  devices: number;
  max_devices: number | null;
  /** 套餐的单个图片、单个附件最大多大（字节，null 不限；协议 6 起，旧服务器没有） */
  max_image?: number | null;
  max_file?: number | null;
  /** 这个空间留的邮箱（只用来发服务调整、到期和活动通知），没留是 null */
  email?: string | null;
  /** 现在是公测免费体验（到截止日停止；填兑换码马上换成付费的） */
  beta?: boolean;
  /** 试运营暂停了续费：还没到期时填码会被拒，到期后照样能填码接着用 */
  renew_off?: boolean;
  /** 续费最多续到哪天（YYYY-MM-DD），null 不限 */
  renew_until_max?: string | null;
  /** 开着兑换码加入：新电脑填兑换码 + 同步密码就能加入（需求-1.x 第 34.2 节；协议 7 起） */
  join?: boolean;
  /** 用户（或后台）关掉了兑换码加入；false 且 join 也是 false 是还没开过（这项上线前开通的、同步密码不够长） */
  join_off?: boolean;
}

/** 用手机加入：电脑上显示的 8 位数字和两边核对的 4 个字 */
export interface PhoneWait {
  code: string;
  expires_at: string;
  words: string;
}

/** 公测免费体验现在怎样（开通前问服务器） */
export interface BetaInfo {
  /** 能领：开着、没过截止日、有名额 */
  open: boolean;
  /** 还剩几个名额 */
  left: number;
  /** 所有公测空间用到这一天（YYYY-MM-DD） */
  until: string | null;
  plan: string;
  quota: number | null;
  max_devices: number | null;
  max_image: number | null;
  max_file: number | null;
}

export interface SyncStatus {
  /** off 没开启 / locked 配对过但还差同步密码 / connecting 正在连 / online 已连接 / offline 连不上 */
  /** outdated：这个版本太旧（官网的最低版本），同步先停下，等更新 */
  /** expired：叮咪同步已到期（停止同步，详情在 account，续费后自己接着同步） */
  /** keys_lost：凭据管理器里没有这台电脑的同步钥匙了（重装过系统、数据从别的电脑拷来），要重新加入 */
  /** keys_retry：这次没从凭据管理器读出钥匙，30 秒后再试（error 里是原因） */
  state: "off" | "locked" | "connecting" | "online" | "offline" | "outdated" | "expired" | "keys_lost" | "keys_retry";
  /** 连的是叮咪同步 */
  official: boolean;
  /** 上次连上时服务器说的这个空间的账；旧服务器、没连上过是 null */
  account: SyncAccount | null;
  /** 空间满了没传上去、先只在这台电脑上的图片和文件有几个；0 是都传上去了 */
  held_files: number;
  /** 超过叮咪同步套餐单个文件的大小、不传、只在这台电脑上的有几个 */
  only_here_files?: number;
  /** locked 时：空间还没设同步密码（刚用兑换码开通），要设一个（输两次）；syncUnlock 会回恢复码 */
  need_setup: boolean;
  /** 服务器不收这台电脑的原因：deleted 叮咪同步到期太久、数据删了（要用兑换码重新开通）等，见 dingmi_sync::refuse */
  refused: string | null;
  server: string | null;
  device: string | null;
  error: string | null;
  last_online: string | null;
  default_name: string;
  /** locked 时：这台电脑没有空间密钥，要输恢复码 */
  need_recovery: boolean;
  /** server 自建服务器 / folder 网盘文件夹；没开启时是空的 */
  kind: "" | "server" | "folder";
  /** 网盘文件夹方式：「叮咪数据」文件夹的位置 */
  folder: string | null;
}

/** 选中的文件夹里有没有叮咪的数据 */
export interface FolderProbe {
  /** 「叮咪数据」文件夹会放在哪 */
  dir: string;
  /** empty 没有叮咪数据（第一台电脑）/ space 已经在用（加入）/ bad 用不了 */
  state: "empty" | "space" | "bad";
  devices: number;
  error: string | null;
}

/** 这台电脑上找到的网盘同步文件夹 */
export interface FolderCandidate {
  label: string;
  path: string;
  probe: FolderProbe;
}

/** 网盘文件夹里的一台电脑 */
export interface FolderDevice {
  id: string;
  name: string;
  last_seen: string | null;
  me: boolean;
}

/** 另存一份备份 */
export interface BackupStatus {
  on: boolean;
  /** 「叮咪数据」文件夹的位置 */
  dir: string | null;
  /** 上次备份的时间 */
  last: string | null;
  /** 上一次备份出的错 */
  error: string | null;
}

/** 同步过的电脑 */
export interface Device {
  uid: string;
  name: string;
  /** 是不是这台电脑 */
  me: boolean;
}

/** 空间里的一台电脑（服务器方式的同步，「设置 → 同步」的设备列表） */
export interface Pc {
  /** 设备公钥，作废时用 */
  id: string;
  name: string;
  created_at: string;
  last_seen: string | null;
  /** 就是这台电脑 */
  me: boolean;
}

export interface Phone {
  id: string;
  name: string;
  created_at: string;
  last_seen: string | null;
}

/** 添加手机后给手机用的网址：先装根证书（ca_url），再打开收件箱网页（url）。服务器给手机网页绑了域名时 ca_url 是空的，不用装证书 */
export interface PhoneLink {
  id: string;
  url: string;
  ca_url: string;
  fingerprint: string;
}

/** 一个附件（图片或文件） */
export interface AttachmentInfo {
  uid: string;
  /** 存在本机的文件名，笔记里写成 att:<file> */
  file: string;
  /** 文件的原文件名，图片没有 */
  name: string;
  mime: string;
  size: number;
  /** 原文件在不在本机（文件附件点开时才下载） */
  local: boolean;
  /** 叮咪同步的套餐不传它（太大）：原文件只在这台设备上（设备编号）；null 是照常同步的 */
  only_on: string | null;
}

/** 手机发来、还没处理的一条 */
export interface InboxEntry {
  id: number;
  uid: string;
  phone: string;
  received_at: string;
  sent_at: string | null;
  text: string;
  /** 图片文件名，笔记里写成 ![](att:文件名) */
  images: string[];
}

/** 导出全部的结果 */
export interface Exported {
  path: string;
  reminders: number;
  notes: number;
  attachments: number;
  /** 不在这台电脑上、也没能下载下来的附件 */
  missing: string[];
}

/** 导出进度（export-progress 事件）：download 先下载缺的文件 / pack 打包 */
export interface ExportProgress {
  stage: "download" | "pack";
  done: number;
  total: number;
}

/** 选好的导出包里有什么 */
export interface RestorePeek {
  version: string;
  exported_at: string;
  reminders: number;
  notes: number;
  attachments: number;
  /** 开着同步，只能合并 */
  sync_on: boolean;
}

/** 恢复完：合进来（或改了）几条；替换时本机数据备份到哪了 */
export interface Restored {
  reminders: number;
  notes: number;
  attachments: number;
  backup: string | null;
}

export interface FocusInfo {
  manual: boolean;
  auto: boolean;
  active: boolean;
  held: number;
}

export type ReminderAction = "done" | "snooze" | "tomorrow" | "snooze_to" | "undo";

export const emptyRepeat = (): Repeat => ({ kind: "none", days: [], monthday: 0, every: 0, month: 0, nth: 0 });

export const newReminder = (p: Partial<Reminder> = {}): Reminder => ({
  id: 0,
  uid: "",
  title: "",
  detail: "",
  tag: null,
  urgent: false,
  due_at: "",
  repeat: emptyRepeat(),
  snoozed_until: null,
  done: false,
  state: 0,
  snooze_count: 0,
  last_done_at: null,
  completed_at: null,
  only_on: [],
  created_at: "",
  updated_at: "",
  deleted_at: null,
  note_uid: null,
  todo: null,
  vague: false,
  ...p,
});

export const newNote = (p: Partial<Note> = {}): Note => ({
  id: 0,
  uid: "",
  title: "",
  body: "",
  tag: null,
  reminder_id: null,
  created_at: "",
  updated_at: "",
  deleted_at: null,
  pinned_at: null,
  ...p,
});

export type TrashKind = "reminder" | "note";

/** 检查更新的状态（update.rs） */
export interface UpdateStatus {
  current: string;
  /** idle 没有新版本 / available 有新版本 / downloading 正在下载 / ready 下载好了等着装 */
  state: "idle" | "available" | "downloading" | "ready";
  checking: boolean;
  version: string | null;
  notes: string | null;
  date: string | null;
  /** 安装包多大、已经下载了多少（字节） */
  size: number;
  got: number;
  /** 这个新版本点过 ✕ */
  dismissed: boolean;
  /** 本机比官网写的最低版本还旧：一直提示，同步停下 */
  too_old: boolean;
  /** 下载、手动检查出的问题 */
  error: string | null;
  checked_at: string | null;
}

/** 首次引导第 2 步要的：快捷框现在的快捷键（"Alt+Space" 这种写法）、能不能用、自动换过的话原来是哪个 */
export interface OnboardInfo {
  /** 这台电脑第一次用、引导还没走完（不是从设置里重新打开的） */
  first: boolean;
  flash: string;
  /** 登记上了没有：被别的程序占了、又没能换成别的时是 false */
  flash_ok: boolean;
  moved_from: string | null;
}

/** 快捷框里记下了一条（onboard-tried 事件，只在开着引导时发） */
export interface OnboardTried {
  kind: "reminder" | "note";
  title: string;
  /** 提醒什么时候弹，"YYYY-MM-DD HH:MM"；笔记是 null */
  due_at: string | null;
}

/** 上手清单的 7 项（需求-0.2 第 5.2 节），按「今天」页上的顺序：
 * flash 按快捷键记一句话 / card 处理一张提醒卡片 / selection 选中文字再按快捷键 / launch 用快捷键打开程序 /
 * note 记一条笔记 / sync 设置同步 / ai 填 AI Key */
export type ChecklistId = "flash" | "card" | "selection" | "launch" | "note" | "sync" | "ai";

/** 上手清单（只记在这台电脑上）。新勾上一项、点 ✕ 或找回时发 checklist-changed，内容也是这个 */
export interface Checklist {
  items: { id: ChecklistId; done: boolean }[];
  done: number;
  total: number;
  /** 点过 ✕ 或者全做完了：不显示（「设置 → 关于」里找回） */
  hidden: boolean;
}

/** 从别的窗口跳到主窗口：哪一页、打开哪条笔记或提醒 */
export interface Nav {
  view: string;
  noteId: number | null;
  reminderId: number | null;
  /** 笔记本：按哪个笔记分类筛；设置：哪一页 */
  sub: string | null;
  /** 设置：跳到哪一行（那一行闪一下），见 pages/settings.ts 的行 id */
  row?: string | null;
}

export const api = {
  listReminders: () => invoke<Reminder[]>("list_reminders"),
  saveReminder: (reminder: Reminder) => invoke<Reminder>("save_reminder", { reminder }),
  deleteReminder: (id: number) => invoke<void>("delete_reminder", { id }),
  /** until：snooze_to 时推迟到的时间 "YYYY-MM-DD HH:MM" */
  reminderAction: (id: number, action: ReminderAction, until?: string) =>
    invoke<Reminder>("reminder_action", { id, action, until: until ?? null }),
  getQueue: () => invoke<Reminder[]>("get_queue"),
  heldReminders: () => invoke<Reminder[]>("held_reminders"),
  /** 上次专注结束时一起放出来、还没处理的提醒 id */
  focusBatch: () => invoke<number[]>("focus_batch"),
  /** 这条提醒最后是不是在别的电脑上改的：是的话回那台电脑的名字（不知道名字时是空字符串），否则 null */
  reminderChangedOn: (id: number) => invoke<string | null>("reminder_changed_on", { id }),
  listNotes: () => invoke<Note[]>("list_notes"),
  getNote: (id: number) => invoke<Note | null>("get_note", { id }),
  pinNote: (id: number, on: boolean) => invoke<void>("pin_note", { id, on }),
  /** 在单独的窗口里编辑这条笔记（已经开着就切到前面） */
  openNoteWindow: (id: number) => invoke<void>("open_note_window", { id }),
  /** 哪些笔记开着单独的编辑窗口 */
  noteWindows: () => invoke<number[]>("note_windows"),
  saveNote: (note: Note) => invoke<Note>("save_note", { note }),
  deleteNote: (id: number) => invoke<void>("delete_note", { id }),
  /** 分类删掉后，把用到它的提醒和笔记改成没有分类 */
  /** 删掉分类后清掉用着它的：notes 为 true 时是笔记分类 */
  clearTag: (tag: Tag, notes = false) => invoke<number>("clear_tag", { tag, notes }),
  listTrash: () => invoke<Trash>("list_trash"),
  trashAction: (kind: TrashKind, id: number, action: "restore" | "purge") => invoke<void>("trash_action", { kind, id, action }),
  emptyTrash: () => invoke<number>("empty_trash"),
  syncStatus: () => invoke<SyncStatus>("sync_status"),
  /** replace：本机已有数据时用服务器上的替换本机（先自动备份），否则两边合在一起。
   * 第一台电脑建空间时回新的恢复码，要请用户存好 */
  syncPair: (code: string, password: string, name: string, replace: boolean) =>
    invoke<string | null>("sync_pair", { code, password, name, replace, recovery: null }),
  syncNow: () => invoke<void>("sync_now"),
  /** 输同步密码解锁；没有空间密钥的电脑输恢复码（这时不用同步密码）。
   * 刚用兑换码开通（状态里 need_setup）时这就是设同步密码：回新的恢复码，要请用户存好；别的时候回 null */
  syncUnlock: (password: string, recovery: string | null = null) => invoke<string | null>("sync_unlock", { password, recovery }),
  /** 叮咪同步：用兑换码开通。兑换码不对、用过了会直接报错（原话给人看）；成了回这个空间的账，
   * 这台电脑已经登记好（设备名在这里给），下一步调 syncUnlock(同步密码) 设密码、拿恢复码。
   * email 选填（空的不留）；agree 是勾了「同意协议」，没勾不开通 */
  officialRedeem: (code: string, name: string, email: string, agree: boolean) =>
    invoke<SyncAccount>("official_redeem", { code, name, email: email.trim() || null, agree }),
  /** 叮咪同步：公测免费体验现在怎样（开通那一步用）；连不上时报错 */
  officialBetaInfo: () => invoke<BetaInfo>("official_beta_info"),
  /** 叮咪同步：领公测免费体验开通，之后和兑换码开通一样设同步密码 */
  officialBeta: (name: string, email: string, agree: boolean) =>
    invoke<SyncAccount>("official_beta", { name, email: email.trim() || null, agree }),
  /** 叮咪同步：留 / 改邮箱（写了），删掉（空的）。回改好以后的账 */
  officialEmail: (email: string) => invoke<SyncAccount>("official_email", { email: email.trim() || null }),
  /** 叮咪同步：唯一的电脑没了，用恢复码找回。password 写了就顺便换成新的同步密码（忘了原来的），空的不换；
   * 成了以后请用户换一个恢复码（syncNewRecovery） */
  officialRecover: (recovery: string, name: string, password = "") =>
    invoke<SyncAccount>("official_recover", { recovery, name, password }),
  /** 叮咪同步：兑换码 + 同步密码加入（这个空间开通、续费用过的哪个码都行）。replace：本机有数据时用服务器上的替换 */
  officialJoin: (code: string, password: string, name: string, replace: boolean) =>
    invoke<SyncAccount>("official_join", { code, password, name, replace }),
  /** 叮咪同步：打开兑换码加入（输一次同步密码，要 10 位以上） */
  officialJoinOn: (password: string) => invoke<SyncAccount>("official_join_on", { password }),
  /** 叮咪同步：关掉兑换码加入 */
  officialJoinOff: () => invoke<SyncAccount>("official_join_off"),
  /** 用手机加入：要 8 位数字 */
  officialPhoneStart: (name: string) => invoke<PhoneWait>("official_phone_start", { name }),
  /** 用手机加入：等手机允许（最多 20 秒），true 是允许了 */
  officialPhonePoll: () => invoke<boolean>("official_phone_poll"),
  /** 用手机加入：手机允许了，输同步密码加入 */
  officialPhoneJoin: (password: string, name: string, replace: boolean) => invoke<void>("official_phone_join", { password, name, replace }),
  /** 叮咪同步：删掉服务器上的数据（输同步密码确认），删完这台电脑回到不同步 */
  officialDelete: (password: string) => invoke<void>("official_delete", { password }),
  /** 叮咪同步：续费（填兑换码），回续好以后的账；到期了（30 天内）也能续 */
  officialRenew: (code: string) => invoke<SyncAccount>("official_renew", { code }),
  /** 叮咪同步：联系客服用的 6 位核对码，10 分钟有效 */
  officialSupportCode: () => invoke<{ code: string; expires_at: string }>("official_support_code"),
  /** 叮咪同步：连上去问一次现在的账（状态里的 account 是上次连上时的） */
  officialAccount: () => invoke<SyncAccount>("official_account"),
  /** 换一个恢复码，旧的作废，回新的 */
  syncNewRecovery: () => invoke<string>("sync_new_recovery"),
  /** 换同步密码（不用旧密码） */
  syncChangePassword: (password: string) => invoke<void>("sync_change_password", { password }),
  syncFolderCandidates: () => invoke<FolderCandidate[]>("sync_folder_candidates"),
  syncFolderProbe: (path: string) => invoke<FolderProbe>("sync_folder_probe", { path }),
  /** 开启网盘文件夹同步：没有数据就建（设密码），有就加入（核对密码） */
  syncFolderStart: (path: string, password: string, name: string, replace: boolean) =>
    invoke<void>("sync_folder_start", { path, password, name, replace }),
  syncFolderDevices: () => invoke<FolderDevice[]>("sync_folder_devices"),
  syncStop: () => invoke<void>("sync_stop"),
  /** 给另一台电脑生成配对码 */
  syncInvite: () => invoke<{ code: string; expires_at: string }>("sync_invite"),
  backupStatus: () => invoke<BackupStatus>("backup_status"),
  /** 开备份：开着同步时 password 是现在的同步密码；不同步时是新设的（文件夹里已有数据时是当时设的） */
  backupStart: (path: string, password: string) => invoke<BackupStatus>("backup_start", { path, password }),
  backupNow: () => invoke<BackupStatus>("backup_now"),
  /** 关掉备份：只停止往里写，已经写进去的不删 */
  backupOff: () => invoke<void>("backup_off"),
  phoneList: () => invoke<Phone[]>("phone_list"),
  phoneAdd: (name: string) => invoke<PhoneLink>("phone_add", { name }),
  phoneRevoke: (id: string) => invoke<void>("phone_revoke", { id }),
  /** 空间里的电脑（自己那台 me 是 true） */
  pcList: () => invoke<Pc[]>("pc_list"),
  /** 作废空间里的另一台电脑（不能作废自己） */
  pcRevoke: (id: string) => invoke<void>("pc_revoke", { id }),
  inboxList: () => invoke<InboxEntry[]>("inbox_list"),
  inboxToNote: (id: number) => invoke<number>("inbox_to_note", { id }),
  inboxDone: (id: number) => invoke<void>("inbox_done", { id }),
  inboxDelete: (id: number) => invoke<void>("inbox_delete", { id }),
  /** 原始字节直接放进请求体，比转成 JSON 数组快得多 */
  saveAttachment: (bytes: Uint8Array, mime: string, noteUid: string, name = "") =>
    invoke<string>("save_attachment", bytes, { headers: { "x-mime": mime, "x-note": noteUid, "x-name": encodeURIComponent(name) } }),
  listAttachments: () => invoke<AttachmentInfo[]>("list_attachments"),
  attachFiles: (files: string[], noteUid: string) => invoke<void>("attach_files", { files, noteUid }),
  attachmentDir: () => invoke<string>("attachment_dir"),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
  /** 回 true：不是能直接打开的类型，只在文件夹里选中了（界面提示一句） */
  openAttachment: (file: string) => invoke<boolean>("open_attachment", { file }),
  /** 附件另存为：弹出「另存为」，存好回路径，取消时为 null */
  saveAttachmentAs: (file: string) => invoke<string | null>("save_attachment_as", { file }),
  listDevices: () => invoke<Device[]>("list_devices"),
  /** 导出全部：默认存到哪、换个位置（取消时为 null）、导出 */
  exportDefault: () => invoke<string>("export_default"),
  exportPick: (path: string) => invoke<string | null>("export_pick", { path }),
  exportAll: (path: string) => invoke<Exported>("export_all", { path }),
  /** 在资源管理器里打开所在文件夹 */
  revealPath: (path: string) => invoke<void>("reveal_path", { path }),
  /** 导出诊断信息：先把要放进去的几样写好列出来，再打成 zip 放到桌面（回 zip 的位置） */
  diagPrepare: () => invoke<DiagPart[]>("diag_prepare"),
  diagExport: () => invoke<string>("diag_export"),
  /** 从导出包恢复：选包（取消时为 null）、看看里面有什么、恢复 */
  restorePick: () => invoke<string | null>("restore_pick"),
  /** 系统的选文件夹对话框，取消时为 null */
  pickFolder: (title: string) => invoke<string | null>("pick_folder", { title }),
  /** 把 PNG（base64）存到用户选的位置，取消时为 null */
  savePng: (name: string, data: string) => invoke<string | null>("save_png", { name, data }),
  restorePeek: (path: string) => invoke<RestorePeek>("restore_peek", { path }),
  restoreApply: (path: string, replace: boolean) => invoke<Restored>("restore_apply", { path, replace }),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  dataPath: () => invoke<string>("data_path"),
  /** 本机每天的自动备份放在哪、最近一次的时间 */
  localBackups: () => invoke<{ dir: string; last: string | null }>("local_backups"),
  /** 桌面组件能不能开毛玻璃（Windows 11 才有） */
  widgetGlassOk: () => invoke<boolean>("widget_glass_ok"),
  hotkeyNotice: () => invoke<string | null>("hotkey_notice"),
  /** 调试用的测试副本（和正在用的叮咪同时开着时分得清） */
  isTestCopy: () => invoke<boolean>("is_test_copy"),
  getFocus: () => invoke<FocusInfo>("get_focus"),
  setFocus: (on: boolean) => invoke<FocusInfo>("set_focus", { on }),
  /** devices：同步过的电脑的名字，AI 用来理解"到公司提醒我" */
  aiParse: (text: string, devices: string[] = []) => invoke<unknown>("ai_parse", { text, devices }),
  aiMorning: (context: string) => invoke<string>("ai_morning", { context }),
  aiTest: (settings: Settings) => invoke<string>("ai_test", { settings }),
  /** 让 AI 整理笔记的格式（不动内容），回整理后的标题和正文 */
  aiTidyNote: (title: string, body: string, ask = "") => invoke<{ title: string; body: string }>("ai_tidy_note", { title, body, ask }),
  openWindow: (name: "main" | "flash" | "qnote" | "morning" | "wrap") => invoke<void>("open_window", { name }),
  /** 到主窗口的某一页；设置时 sub 是哪一页、row 是那一行（比如 "general", "hk_flash"） */
  openMainNav: (view: string, noteId?: number, reminderId?: number, sub?: string, row?: string) =>
    invoke<void>("open_main_nav", { view, noteId: noteId ?? null, reminderId: reminderId ?? null, sub: sub ?? null, row: row ?? null }),
  takeNav: () => invoke<Nav | null>("take_nav"),
  /** 打开闪念输入的搜索（输入框里先放一个 ?） */
  openFlashSearch: () => invoke<void>("open_flash_search"),
  /** 闪念输入打开时，取走按快捷键时从别的程序带过来的选中文字 */
  takeFlashText: () => invoke<string | null>("take_flash_text"),
  /** 今天在哪台电脑上收尾过了 */
  wrapStatus: () => invoke<boolean>("wrap_status"),
  wrapDone: () => invoke<void>("wrap_done"),
  fitWindow: (height: number) => invoke<void>("fit_window", { height }),
  /** 显示这个窗口但不抢焦点（提醒卡片、开场白、收尾） */
  showQuiet: () => invoke<void>("show_quiet"),
  /** 快捷框拖完了，记下位置 */
  flashMoved: () => invoke<void>("flash_moved"),
  /** 快捷框放回原位（屏幕中间偏上） */
  flashPosReset: () => invoke<void>("flash_pos_reset"),
  /** 主窗口恢复默认大小、放到屏幕中间 */
  mainReset: () => invoke<void>("main_reset"),
  /** 拖进「自己加的」的文件：快捷方式解开成真实程序和参数，网页快捷方式变成网址 */
  launcherResolve: (paths: string[]) => invoke<LaunchConfig["custom"]>("launcher_resolve", { paths }),
  /** 桌面组件画好了，或者换了外观：设置里开了毛玻璃就上毛玻璃（tint 是叠在上面的颜色 r g b a），摆好位置并显示。
   *  回是不是毛玻璃（没开、Windows 10、系统不支持时是 false，网页画不透明的卡片） */
  widgetShow: (tint: [number, number, number, number]) => invoke<boolean>("widget_show", { tint }),
  /** 移动位置后记住；reset 时放回右上角 */
  widgetPlace: (reset = false) => invoke<void>("widget_place", { reset }),
  widgetOff: () => invoke<void>("widget_off"),
  updateStatus: () => invoke<UpdateStatus>("update_status"),
  /** 点按钮检查：连不上时报错「连不上，过会儿再试」 */
  updateCheck: () => invoke<UpdateStatus>("update_check"),
  /** 开始下载新版本，进度从 update-status 事件里来 */
  updateDownload: () => invoke<void>("update_download"),
  /** 这个版本不再提示 */
  updateDismiss: () => invoke<void>("update_dismiss"),
  /** 启动下载好的安装包，叮咪随即退出，装完自动重新打开 */
  updateInstall: () => invoke<void>("update_install"),
  /** 更新后第一次打开时的提示："叮咪已经更新到 x" */
  updateNotice: () => invoke<string | null>("update_notice"),
  /** 协议：哪份最近更新过、有没有要通知的重大修改（官网 terms.json，检查更新时顺便取的；变了发 terms-changed） */
  termsInfo: () => invoke<TermsView>("terms_info"),
  /** 协议通知点了「知道了」 */
  termsSeen: (key: keyof TermsView, version: string) => invoke<void>("terms_seen", { key, version }),
  launcherList: () => invoke<LaunchItem[]>("launcher_list"),
  launcherStats: () => invoke<LaunchStats>("launcher_stats"),
  /** how：open / admin（管理员身份）/ dir（打开所在位置）；query 是当时输入框里的字，下次排前面 */
  launcherOpen: (id: string, how: "open" | "admin" | "dir" = "open", query?: string) => invoke<void>("launcher_open", { id, how, query: query ?? null }),
  /** 重新找一遍（图标也重新取），找好了发 launcher-changed */
  launcherRescan: () => invoke<void>("launcher_rescan"),
  /** 只存启动器那部分设置，存好了发 settings-changed */
  launcherSave: (config: LaunchConfig) => invoke<void>("launcher_save", { config }),
  folderPeek: (path: string) => invoke<FolderPeek>("folder_peek", { path }),
  openPath: (path: string) => invoke<void>("open_path", { path }),
  /** 选一个文件夹（folder）或文件，取消时是 null */
  pickPath: (folder: boolean) => invoke<string | null>("pick_path", { folder }),
  /** 首次引导：第 2 步的快捷键情况 */
  onboardInfo: () => invoke<OnboardInfo>("onboard_info"),
  /** 打开首次引导（「设置 → 关于 → 再看一遍引导」） */
  onboardOpen: () => invoke<void>("onboard_open"),
  /** 引导走完或跳过：以后启动不再打开 */
  onboardDone: () => invoke<void>("onboard_done"),
  /** 放「叮咪怎么用」笔记（分类「资料」）；放过的不再放，回 null */
  onboardGuideNote: (title: string, body: string) => invoke<number | null>("onboard_guide_note", { title, body }),
  /** 上手清单；同步开着、AI Key 填了的顺便勾上 */
  checklistGet: () => invoke<Checklist>("checklist_get"),
  /** 点 ✕ 不再显示（true），或者找回来（false） */
  checklistHide: (hidden: boolean) => invoke<Checklist>("checklist_hide", { hidden }),
};

/** Tauri 命令抛出的错误是字符串，这里统一成可读的文字 */
export const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
