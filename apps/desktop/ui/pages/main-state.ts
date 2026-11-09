// 主窗口几块共用的状态和小工具：main.ts（今天、提醒、笔记本、回收站）和 settings.ts（设置）都用
import type { AttachmentInfo, BetaInfo, Device, FocusInfo, FolderCandidate, FolderProbe, InboxEntry, Note, PhoneWait, Reminder, Settings, SyncAccount, SyncStatus, Tag, TermsView, Trash, UpdateStatus } from "../lib/api";
import type { ParseResult } from "../lib/parse";
import { $ } from "@shared/util";

export type View = "today" | "reminders" | "notes" | "settings" | "trash";

/** 同步页正在走的流程（选方式 → 选文件夹 → 第一台 / 加入，或者配对服务器；
 *  叮咪同步：填兑换码开通 official → 设同步密码 setup → 保存恢复码，或者用恢复码找回 recover）；没在走时是 null */
export type SyncFlow = {
  step: "choose" | "pick" | "first" | "join" | "server" | "official" | "pair" | "setup" | "recover" | "fork" | "ways" | "ojoin" | "phone" | "phonepw";
  probe?: FolderProbe;
  cands?: FolderCandidate[];
  /** official：公测免费体验现在怎样（进来时问一次服务器，没开、连不上是 null） */
  beta?: BetaInfo | null;
  /** setup：刚开通时服务器给的账（显示「开通了：标准会员 · 到 …」） */
  account?: SyncAccount;
  /** ojoin：开通时填了用过的码，带过来填好 */
  code?: string;
  /** phone：服务器给的 8 位数字、4 个字；gone 是过期了 / 手机上拒绝了（给人看的话） */
  wait?: PhoneWait;
  gone?: string;
};

export const S = {
  /** 附件：存在本机的文件名 → 情况（文件卡片显示大小、要不要先下载） */
  atts: new Map<string, AttachmentInfo>(),
  reminders: [] as Reminder[],
  notes: [] as Note[],
  trash: { reminders: [], notes: [] } as Trash,
  inbox: [] as InboxEntry[],
  devices: [] as Device[],
  /** 正在用「整理一下」处理的手机收件箱（可能是「全部整理」的几条），保存后标为已处理 */
  inboxParsing: [] as number[],
  /** 手机发来的多了收起来，点「还有 N 条」展开 */
  inboxOpen: false,
  settings: null as Settings | null,
  sync: null as SyncStatus | null,
  /** 锁着时用户点了「用恢复码」 */
  syncByRecovery: false,
  /** 同步页正在走的流程（选方式、选文件夹、设密码……） */
  syncFlow: null as SyncFlow | null,
  focus: { manual: false, auto: false, active: false, held: 0 } as FocusInfo,
  view: "today" as View,
  /** 提醒页上面切的：还没做的 / 周期 / 做完的 */
  rMode: "todo" as "todo" | "cycle" | "done",
  /** 侧栏点了哪个提醒分类（只看这一类），null 是全部 */
  rTag: null as Tag | null,
  q: "",
  noteId: 0,
  /** 笔记本里看「待办」（所有笔记里没勾掉的待办） */
  todoView: false,
  /** 设置里正在看哪一页（text/settings.ts 的 PAGES）；打开时停在上次看的那页 */
  setSec: "general",
  /** 设置画好后跳到哪一行、闪一下（data-row），跳过去就清掉 */
  setRow: "",
  /** 笔记本按哪个笔记分类筛："all" 全部，"none" 没分类的 */
  nCat: "all",
  /** 看笔记时，分类那里展开着可以改 */
  catPick: false,
  /** 设置 → 分类里正在选颜色的那个（"tag:id" 或 "ncat:id"） */
  tagPick: "",
  editing: false,
  /** 开着单独编辑窗口的笔记：主窗口里只看不改，免得两边互相覆盖 */
  noteWins: new Set<number>(),
  parsed: null as ParseResult | null,
  parsing: false,
  recording: "" as "" | keyof Settings,
  update: null as UpdateStatus | null,
  /** 三份协议最近有没有更新、要不要通知（官网 terms.json，见 terms.rs） */
  terms: null as TermsView | null,
};

/** main.ts 填进来的几个动作，设置页要用（免得两个文件互相引用） */
export const hooks = {
  go: (_v: View) => {},
  renderAll: () => {},
  /** 设置存好了：顶上的横幅、今天页的提示跟着变 */
  afterSave: () => {},
  /** 检查更新的结果变了：顶上的横幅、「关于」那页 */
  renderUpdate: () => {},
};

// ---------- 弹窗（设置里那些：换密码、恢复码、导出、恢复……；#modal 是一层 .scrim，里面放 .dialog） ----------

const modal = () => $("#modal");
/** 按 Esc 时怎么关；null 是不让关（恢复码、正在导出） */
export const M = { close: null as (() => void) | null };

export function showModal(html: string) {
  const m = modal();
  m.innerHTML = html;
  m.hidden = false;
  const first = m.querySelector<HTMLElement>("input, textarea");
  setTimeout(() => first?.focus(), 30);
}

export function hideModal() {
  const m = modal();
  m.hidden = true;
  m.innerHTML = "";
  M.close = null;
}

export const modalOpen = () => !modal().hidden;

/** 危险操作要点两次：第一次只把按钮换成确认的字样，3 秒内再点才算数 */
export function armed(b: HTMLElement, ask: string) {
  if (b.classList.contains("sure")) return true;
  const label = b.innerHTML;
  b.classList.add("sure");
  b.textContent = ask;
  setTimeout(() => {
    b.classList.remove("sure");
    b.innerHTML = label;
  }, 3000);
  return false;
}
