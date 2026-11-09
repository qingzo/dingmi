// 快捷框上的字（UI设计.md U4、design/ui/u4-quickbox.html）。用词照 U13，语气照需求-0.2 第 7.5 节：
// 按钮、菜单不卖萌；带猫说的几句（mascot 为 true 时）关掉团子猫后换成平实的说法。都是纯文字，用的地方自己转义

export const T = {
  placeholder: "写一句话就记下来，打程序名就打开",
  inputLabel: "快捷框",
  search: "搜索",
  settings: "快捷框设置",

  // 空着时
  alt: (n: number) => `Alt ${n}`,
  next: "下一条：",
  soon: "马上",
  inMin: (m: number) => `${m} 分钟后`,
  inHour: (h: number, m: number) => `${h} 小时${m ? ` ${m} 分钟` : ""}后`,

  // 结果
  rec: (title: string) => `记下来：${title}`,
  reminder: "提醒",
  note: "笔记",
  noTime: "没有时间，存成一条笔记",
  urgent: "紧急",
  count: (n: number) => `一共 ${n} 条`,
  alsoNote: "也记一条笔记",
  onlyOn: (pc: string) => `只在 ${pc}`,
  aiCheck: "回车后 AI 再确认一遍",
  /** 说得含糊时黄色小条：parse.ts 给的「没听清具体时间，先定在明天 9:00」后面接这句 */
  guessTail: "，接着补一句就行",
  noteOnly: "只记成笔记",
  noteOnlySub: "不设提醒，整句存成一条笔记",
  blankNote: "空白笔记",
  remDone: "做完了",

  // 随手算、网页关键字（需求-0.3 第 10 节）
  calcBad: "算不了",
  copied: (v: string) => `已复制 ${v}`,
  webSearch: (site: string, q: string) => `用${site}搜：${q || "……"}`,
  webSub: (q: string) => (q ? "在浏览器里打开" : "打开网站首页"),
  webFail: (why: string) => `打不开浏览器：${why}`,

  // ? 开头搜索
  searchHint: "打要找的字，笔记和提醒一起搜",
  searching: "正在找…",
  notFound: (mascot: boolean) => (mascot ? "没找到呀，换个说法试试？" : "没有找到，换个说法试试。"),

  // 底下一行按键提示
  key: {
    pins: "打开常用",
    searchPrefix: "开头搜索",
    close: "关掉",
    save: "记下来",
    saveVague: "先这样记下",
    vagueTip: "接着打「周三」之类的，预览马上变",
    noteOnly: "只记成笔记",
    pick: "选",
    space: "空格",
    open: "打开",
    admin: "管理员身份",
    dir: "打开所在位置",
    rightClick: "右键",
    more: "更多",
    openMain: "在主窗口里打开",
    copy: "复制结果",
    webGo: "搜索",
    webHome: "打开首页",
    webWhere: "关键字在 设置 → 快捷框 里改",
  },

  // 记的时候
  thinking: "AI 正在理解…",
  saving: "正在记下…",
  saveFail: (why: string) => `没记上：${why}。这句话还在，再按一次回车试试`,
  launchFail: (name: string, why: string) => `「${name}」打不开：${why}`,

  // 记好了：框缩成一行，1.5 秒后自己消失
  savedRem: (mascot: boolean, when: string) => (mascot ? `搞定！${when} 叫你` : `已设好提醒：${when}`),
  savedNote: (mascot: boolean) => (mascot ? "搞定！记进笔记本了" : "已存进笔记本"),
  savedGuess: "时间是先定的，在提醒列表里能改",
  savedAlsoNote: "也记了一条笔记",
  aiFailed: "AI 这次没回话，按本地规则记的",
  undo: "撤销",
  undone: "撤销了，这句话还在",
  undoFail: (why: string) => `没撤销成功：${why}。可以到主窗口里删掉`,

  // 右键菜单（程序、常用）
  menu: {
    open: "打开",
    admin: "用管理员身份打开",
    dir: "打开所在位置",
    pin: "放到常用里",
    unpin: "从常用里拿掉",
    alias: "起个别名…",
    hide: "不在这里显示",
  },
  aliasLabel: "起个短名字，打它就是这个",
  aliasEg: (eg: string) => `比如 ${eg}`,
  aliasOk: "设好",
  pinsFull: (n: number) => `常用里最多放 ${n} 个，先拿掉一个`,
  pinned: (name: string) => `「${name}」放到常用里了，这里和桌面组件上都有`,
  unpinned: (name: string) => `「${name}」从常用里拿掉了`,
  hidden: (name: string) => `以后不显示「${name}」，在 设置 → 快捷框 里能找回来`,
  aliased: (alias: string, name: string) => `以后打 ${alias} 就是「${name}」`,
  cfgFail: (why: string) => `没改成：${why}`,
};
