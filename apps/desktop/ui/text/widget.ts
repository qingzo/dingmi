// 桌面组件上的字（UI设计.md U8、design/ui/u8-widget.html）。带猫的几句（mascot 为 true 时）关掉团子猫后换成平实的说法（需求-0.2 第 7.5 节）

export const T = {
  more: "更多",
  menu: {
    open: "打开叮咪",
    move: "移动位置",
    pins: "编辑常用",
    parts: "设置这几块",
    off: "关掉桌面组件",
  },
  /** 11 月 6 日 */
  date: (m: number, d: number) => `${m} 月 ${d} 日`,
  /** 日期下面那一行（HTML，数字加粗） */
  left: (n: number) => `还有 <b>${n} 件事</b>`,
  tomorrow: (n: number) => `明天还有 <b>${n} 件事</b>`,
  allDone: (mascot: boolean) => (mascot ? "今天的事做完啦" : "今天的事都做完了"),
  free: (mascot: boolean) => (mascot ? "今天没事啦，我先眯一会儿" : "今天没有安排"),
  /** 白天全做完了，「今天」那块换成这一条 */
  allOk: (n: number, mascot: boolean) => (mascot ? `${n} 件都做完了，喝口水歇会儿` : `${n} 件都做完了`),

  // 下一条的倒计时
  soon: "马上",
  inMin: (m: number) => `${m} 分钟后`,
  inHour: (h: number, m: number) => `${h} 小时${m ? ` ${m} 分钟` : ""}后`,
  yesterday: "昨天",

  today: "今天",
  pins: "常用",
  fold: "收起",
  unfold: "展开",
  moreRows: (n: number) => `还有 ${n} 条`,
  noToday: "今天没有提醒",
  noToread: (cat: string) => `没有${cat}的笔记`,
  blankNote: "空白笔记",
  done: "完成",
  undo: "取消完成",

  // 输入框
  placeholder: "写一句，回车就记好",
  inputLabel: "写一句",
  /** 回车：提醒 · 明天 9:00 */
  enter: "回车：",
  asReminder: "提醒",
  asNote: "笔记",
  alsoNote: "顺便记一条笔记",
  toNotebook: "存进笔记本",
  onlyOn: (pc: string) => `只在 ${pc}`,
  aiCheck: "回车后 AI 再确认一下",
  thinking: "AI 正在理解…",
  saving: "正在记下…",
  saveFail: (why: string) => `没记下来：${why}。再按一次回车试试`,
  savedRem: (when: string, title: string, note: boolean) => `记好了：${when} ${title}${note ? "，也记了一条笔记" : ""}`,
  savedNote: (title: string) => `记好了，存进笔记本：${title}`,

  // 移动位置
  moveHint: "按住拖到想放的地方",
  moveHome: "放回右上角",
  moveDone: "好了",
};

/** 常用（lib/pins.ts） */
export const P = {
  edit: "完成",
  add: "添加",
  addTitle: "添加程序、文件夹、文件或网址",
  seeFiles: "看看里面的文件",
  unpin: "从这里拿掉",
  openDir: (name: string) => `打开「${name}」文件夹`,
  emptyDir: "文件夹是空的",
  moreFiles: (n: number) => `还有 ${n} 个，打开文件夹看全部`,
  search: "搜程序、文件夹…",
  searchLabel: "搜要添加的",
  noMatch: "没找到，可以用下面的按钮选",
  pickFolder: "选文件夹…",
  pickFile: "选文件…",
  addUrl: "加网址…",
  dragTip: "也可以直接把文件、文件夹拖到组件上",
  urlPh: "网址，比如 https://…",
  urlLabel: "网址",
  namePh: "名字（可以不填）",
  nameLabel: "名字",
  urlAdd: "加上",
  cancel: "取消",
  badUrl: "这个网址不对，检查一下再加",
  already: (n: string) => `「${n}」已经在这里了`,
  full: (max: number) => `最多放 ${max} 个，先拿掉一个`,
  fullNow: (max: number) => `放满 ${max} 个了`,
  added: (n: string) => `「${n}」放上来了`,
  saveFail: (why: string) => `没存好：${why}`,
  drop: "松开，放到「常用」里",
};
