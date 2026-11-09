// 快捷框的空格启动（实验，需求-1.x 第 37 节）：开着时，打了字以后按空格，下面两种情况算「打开选中的程序」，别的时候照常是打一个空格：
//   1. 用方向键主动选中了一个程序；
//   2. 程序对得很准、排在第一行（flash.ts render 里「像程序名」那条），而且输入里还没有空格。比如打 wx 再按空格就打开微信。
// 输入法正在选字时按的空格是给输入法的，不管。快捷框空着时方向键选中常用、按空格打开，本来就有，不归这里管（flash.ts emptyKey）

export interface SpaceCtx {
  /** 设置里开着「空格启动」 */
  on: boolean;
  /** 输入法正在组字（isComposing / keyCode 229） */
  composing: boolean;
  /** 输入框里现在的字（还没加上这个空格） */
  value: string;
  /** 选中的那行是程序 */
  app: boolean;
  /** 那行是用方向键主动选中的 */
  picked: boolean;
  /** 那行是「像程序名」排到第一的（选中的就是第一行） */
  top: boolean;
  /** 再打一个空格就成了网页关键字（「bd 天气」的 bd）：这时空格要留给关键字 */
  webKey: boolean;
}

/** 这次按空格算不算打开选中的程序 */
export function spaceOpens(c: SpaceCtx): boolean {
  if (!c.on || c.composing || !c.value.trim() || !c.app) return false;
  if (c.picked) return true;
  return c.top && !/\s/.test(c.value) && !c.webKey;
}
