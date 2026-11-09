// 笔记相关的界面文字（编辑器、单独的笔记窗口、快速记笔记小窗口）。用词照 UI设计.md 的 U13 用词表
import { fileSize } from "@shared/md";
import { parseDT } from "@shared/util";

const WD = "日一二三四五六";
const hm = (d: Date) => `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;

/** 提醒时间的短说法（U13 时间写法）：今天「14:00」，明天「明天 08:30」，这周「周三 15:00」，更远「11 月 12 日」，跨年才写年份 */
export function whenShort(s: string, now = new Date()): string {
  const d = parseDT(s);
  if (!d) return s;
  const day = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((day(d) - day(now)) / 86400000);
  if (diff === 0) return hm(d);
  if (diff === 1) return `明天 ${hm(d)}`;
  if (diff === -1) return `昨天 ${hm(d)}`;
  // 这周（周一到周日）
  const monday = day(now) - ((now.getDay() + 6) % 7) * 86400000;
  if (day(d) >= monday && day(d) < monday + 7 * 86400000) return `周${WD[d.getDay()]} ${hm(d)}`;
  return `${d.getFullYear() !== now.getFullYear() ? `${d.getFullYear()} 年 ` : ""}${d.getMonth() + 1} 月 ${d.getDate()} 日`;
}

export const T = {
  // ---------- 编辑器 ----------
  placeholder: "写点什么…打 / 插入待办、表格、图片",
  editKeys: [
    "Ctrl + F 查找，Ctrl + H 替换，F3 下一个",
    "Ctrl + B 粗体，Ctrl + Shift + X 删除线，Ctrl + E 代码，Ctrl + K 链接",
    "Ctrl + 1 / 2 / 3 标题，Ctrl + Shift + 8 圆点列表，Ctrl + Shift + 7 编号列表",
    "Ctrl + Enter 勾选待办，Shift + Alt + F 格式化",
    "行首打 / 插入待办、表格、图片、文件、分隔线",
    "表格里 Tab / Shift + Tab 跳格，回车加一行；Ctrl + Alt + T 用格子编辑（不在表格里时插一张新表格）",
    "粘贴网页、Word、Excel 的内容自动转成 Markdown，Ctrl + Shift + V 按纯文本粘贴",
    "图片和文件可以直接粘贴或拖进来",
  ],
  /** 工具栏、选中文字小工具条上的按钮提示 */
  tool: {
    heading: "小标题（Ctrl + 2）",
    bold: "粗体（Ctrl + B）",
    list: "列表（Ctrl + Shift + 8）",
    todo: "待办（Ctrl + Enter）",
    table: "表格（Ctrl + Alt + T）",
    image: "图片",
    file: "文件（单个不超过 50 MB）",
    link: "链接（Ctrl + K）",
    code: "代码（Ctrl + E）",
    remind: "设成提醒",
    copy: "复制",
  },
  /** 行首打 / 弹出的插入菜单；keys 是筛选时也认的说法 */
  slashHead: "插入",
  slash: [
    { id: "todo", name: "待办", md: "- [ ]", icon: "square-check", keys: "daiban db todo 清单 勾选" },
    { id: "table", name: "表格", md: "| |", icon: "table", keys: "biaoge bg table" },
    { id: "heading", name: "小标题", md: "##", icon: "heading", keys: "biaoti bt xiaobiaoti h title 标题" },
    { id: "list", name: "列表", md: "-", icon: "list", keys: "liebiao lb list" },
    { id: "image", name: "图片", md: "", icon: "image", keys: "tupian tp image img 照片 截图" },
    { id: "file", name: "文件", md: "", icon: "paperclip", keys: "wenjian wj file 附件" },
    { id: "hr", name: "分隔线", md: "---", icon: "minus", keys: "fengexian fgx hr line 分割线" },
  ] as const,
  copied: (t: string) => `已复制：${t.length > 30 ? t.slice(0, 30) + "…" : t}`,
  copyFail: "没复制上，再试一次",
  tablePanel: "光标在表格里：Tab 跳格，回车加一行",
  tableGrid: "格子编辑（Ctrl + Alt + T）",
  tableGridShort: "格子编辑",
  tableGridTip: "像电子表格一样改：增删行列、排序、从 Excel 粘贴",
  tableSource: "改源码",
  tableSourceTip: "直接改这张表格的 Markdown",
  tableChanged: "这张表格在别处改过了，这次在格子里的修改没写回去",
  /** 待办后面 */
  setRemind: "设提醒",
  setRemindTip: "到点提醒我做这件事",
  remindTip: "改这条提醒",
  checkTip: (done: boolean) => (done ? "取消勾选" : "勾上"),
  linkTip: (url: string) => `${url}\n点一下用浏览器打开`,
  // 图片和文件
  imgLoading: "正在下载图片…",
  imgView: "看大图",
  imgCopy: "复制图片",
  imgDelete: "删掉这张图片",
  imgCopied: "图片已复制",
  fileHere: "在这台电脑上",
  fileCloud: "还没下载到这台电脑",
  fileMeta: (size: number | undefined, local: boolean) => [size != null ? fileSize(size) : "", local ? "在这台电脑上" : "还没下载到这台电脑"].filter(Boolean).join(" · "),
  /** 叮咪同步的套餐不传太大的：原文件只在一台设备上 */
  fileMetaAway: (size: number, away: string) => `${fileSize(size)} · ${away}`,
  onlyHere: "超过叮咪同步单个文件的大小，没有传上去，只在这台电脑上",
  onlyOn: (pc: string) => `超过叮咪同步单个文件的大小，没有传过来，只在「${pc}」上`,
  someOtherPc: "另一台电脑",
  fileOpen: "打开",
  fileReveal: "打开所在位置",
  fileRunnable: "这种文件不直接打开，在文件夹里选中它",
  /** 打开附件时：不在能直接打开的类型里（程序、脚本、压缩包、带宏的文档……），只在文件夹里选中了 */
  fileRevealed: "这种文件叮咪不直接打开，已经在文件夹里选中它了。确认来源可靠再自己双击",
  fileDownloading: "正在下载…",
  fileDelete: "从笔记里去掉这个文件",
  tooBig: (name: string) => `「${name}」超过 50 MB 了，放不进笔记`,
  notSaved: (name: string, why: string) => `「${name}」没存上：${why}`,
  // 看大图
  lbCopy: "复制",
  lbSave: "另存为",
  lbClose: "关掉（Esc）",
  lbName: (alt: string, i: number, n: number) => `${alt || "图片"} · ${i} / ${n}${n > 1 ? " · ← → 切换" : ""}`,
  lbSaved: (p: string) => `存好了：${p}`,
  // 设提醒的小框
  rpWhen: "什么时候？比如：下周一上午",
  rpChips: ["今晚 8 点", "明早 9 点", "下周一 9 点"],
  rpNone: "没看懂时间，换个说法试试",
  rpCancel: "取消",
  rpDone: "设好了",
  rpDelete: "不提醒了",
  rpTitle: (t: string) => `提醒：${t}`,
  rpSaved: (when: string) => `好的，${when} 提醒你`,
  rpDeleted: "这条提醒删掉了",
  rpTodoDone: "待办勾上了，提醒也完成了",
  // 让 AI 整理
  aiAsk: "让 AI 怎么整理？",
  aiPresets: ["整理成表格", "按时间先后排好", "只分段，不要改成列表"],
  aiInput: "或者自己写一句，比如「按负责人分组」",
  aiRecent: "最近用过：",
  aiCancel: "取消",
  aiPlain: "直接整理",
  aiGo: (ask: string) => (ask ? `按「${ask.length > 10 ? ask.slice(0, 10) + "…" : ask}」整理` : "开始整理"),
  aiButton: "让 AI 整理",
  /** 平时看、双击或点「编辑」才改（2026-11-06 试用后改） */
  edit: "编辑",
  editTip: "双击正文也能改，光标就落在双击的地方",
  doneEdit: "完成",
  doneTip: "回去看排好版的样子（Esc）",
  viewEmpty: "还是空的，双击这里开始写",
  aiBusy: "整理中…",
  aiNoKey: "要先在「设置 → AI」里填 API Key",
  aiEmpty: "先写点东西再整理",
  aiDone: "整理好了",
  aiDoneSub: "只理了格式，内容没增没减",
  aiUndo: "撤销",
  aiShrunk: "整理后的内容比原来少了不少，可能漏了东西",
  aiShrunkSub: "要不要换上？",
  aiUse: "仍然用",
  aiDrop: "不用",
  aiFail: (why: string) => `没整理成：${why}`,

  // ---------- 单独的笔记窗口 ----------
  winTitle: "叮咪笔记",
  title: "标题",
  outline: "目录",
  outlineEmpty: "用 # 、## 、### 开头写标题，这里就有目录了",
  outlineTip: "目录",
  pin: "置顶",
  unpin: "取消置顶",
  pinned: "已置顶",
  unpinned: "已取消置顶",
  preview: "对照着看（Ctrl + Shift + P）",
  previewEmpty: "还没有内容",
  focusMode: "专注写作（F11），Esc 退出",
  min: "最小化",
  max: "最大化",
  close: "关闭",
  more: "更多",
  fmt: "格式化",
  fmtTip: "表格对齐、列表记号统一、编号重排、去掉多余空行，不改字（Shift + Alt + F）",
  keys: "快捷键",
  words: (n: number) => `${n} 字`,
  edited: (ago: string) => (/\d$/.test(ago) ? `${ago} 改过` : `${ago}改过`),
  noCat: "分类",
  catTip: "改分类",
  catNone: "不分类",
  catEmpty: "还没有笔记分类，在「设置 → 分类」里加",
  linked: (t: string) => `关联提醒：${t}`,
  linkedTip: "在主窗口里看这条提醒",
  saving: "正在输入…",
  saved: "已保存",
  autoSave: "自动保存",
  saveFail: (why: string) => `没存上：${why}`,
  pickFirst: "先选用哪一版",
  usedTheirs: "已换成别处的修改",
  noteGone: "这条笔记已经删掉了（在回收站里），这里的修改不会再保存。",
  closeWin: "关掉窗口",
  conflict: "这条笔记在别处（主窗口或者别的电脑）也改过了，和这里的不一样。",
  conflictSub: "选一版留下；没选就关窗口的话，这里的另存一条「冲突副本」，两边都不丢。",
  useTheirs: "用那边的",
  keepMine: "保留我的",
  conflictCopy: (t: string) => `${t}（冲突副本）`,
  notDone: (why: string) => `没改成：${why}`,

  // ---------- 快速记笔记 ----------
  qTitle: "标题（可以不填）",
  qBody: "想到什么写什么…打 / 插入待办、表格",
  qToMain: "转到笔记本里接着写",
  qClose: "关掉（Esc），没保存的留作草稿",
  qAttach: "加图片或文件（单个不超过 50 MB），也可以直接拖进来、粘贴",
  qTidy: "让 AI 整理格式",
  qSave: "保存",
  qDraft: "草稿已存",
  qEmpty: "先写点什么再保存",
  qAdding: (name: string) => `正在加「${name}」…`,
  qSaved: (t: string) => `已存进笔记本：${t}`,
  qSaveFail: (why: string) => `没存上：${why}`,
  qRemove: "去掉",
  qTidyUndone: "整理撤销了",
};

/** 插入菜单里的一项 */
export type SlashId = (typeof T.slash)[number]["id"];
