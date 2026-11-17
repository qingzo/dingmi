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
  /** 常用（固定的）和最近用过的都有时，两块上面的小字 */
  pinsLab: "常用",
  recentLab: "最近用过",
  soon: "马上",
  inMin: (m: number) => `${m} 分钟后`,
  inHour: (h: number, m: number) => `${h} 小时${m ? ` ${m} 分钟` : ""}后`,

  // 结果
  rec: (title: string) => `记下来：${title}`,
  reminder: "提醒",
  note: "笔记",
  noTime: "没有时间，存成一条笔记；开头写「待办」就记成待办",
  todo: "待办",
  todoWhen: "有空再做",
  dayOnly: "那天不弹窗",
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
  // 带着的图片（需求-1.x 第 42.6 节）
  picName: "截图",
  picDel: "不要这张图",
  picNote: "回车时图片放进一条笔记；拆出提醒的，笔记跟着提醒",
  picFull: (n: number) => `最多带 ${n} 张图`,
  picSame: "这张图已经贴过了",
  picOnly: "存成笔记",
  picOnlySub: "还没写字：图片存成一条笔记；写一句带时间的话，就是提醒加上这条带图的笔记",
  // 搜文件（需求-1.x 第 21 节）
  files: "搜文件",
  fileHint: "打文件名，比如「f 合同」；用 Everything 时也能写 *.pdf、ext:docx 合同",
  fileNone: "没找到这个名字的文件",
  fileEverything: "用的是 Everything",
  fileIndex: "用的是 Windows 搜索索引，只搜用户文件夹里的",
  fileOff: "Windows 搜索服务关着，搜不了文件",
  fileAll: "搜整台电脑：一键开启",
  fileIdle: "Everything 没在运行",
  fileIdleGo: "打开 Everything",
  fileFolder: "文件夹",
  fileCopied: (name: string) => `复制好了「${name}」的路径`,
  fileNoted: (name: string) => `「${name}」放进笔记本了`,
  filePinned: (name: string) => `「${name}」放进常用了`,
  fileFail: (why: string) => `没成：${why}`,
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
    filePrefix: "开头搜文件",
    copyPath: "复制路径",
  },

  // 记的时候
  thinking: "AI 正在理解…",
  saving: "正在记下…",
  saveFail: (why: string) => `没记上：${why}。这句话还在，再按一次回车试试`,
  launchFail: (name: string, why: string) => `「${name}」打不开：${why}`,

  // 记好了：框缩成一行，1.5 秒后自己消失
  savedRem: (mascot: boolean, when: string) => (mascot ? `搞定！${when} 叫你` : `已设好提醒：${when}`),
  savedNote: (mascot: boolean) => (mascot ? "搞定！记进笔记本了" : "已存进笔记本"),
  savedTodo: (mascot: boolean) => (mascot ? "搞定！放进待办了" : "已放进待办"),
  savedDay: (mascot: boolean, day: string) => (mascot ? `搞定！${day}那天放在「今天」里` : `已记好：${day}那天放在「今天」里`),
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
    toNote: "放进笔记本",
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
