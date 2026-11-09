// 电脑和手机网页都用的界面文字。用词照 UI设计.md U13，语气照需求-0.2 第 7.5 节。
// cat：团子猫开着没有，关掉时换成平实的说法

/** 内置风格（需求-0.2 第 29 节）：id、名字、一句说明 */
export const STYLES = [
  ["warm", "暖云母", "暖橘、圆润，默认的样子"],
  ["simple", "简洁", "黑白灰，线条干净"],
  ["calm", "安静", "低饱和的灰绿，适合一直开着"],
] as const;

export type StyleId = (typeof STYLES)[number][0];

export const TEXT = {
  /** 今天没有安排 */
  todayEmpty: (cat: boolean) => (cat ? "今天没事啦，我先眯一会儿。" : "今天没有安排。"),
  /** 做完一件 */
  done: (cat: boolean) => (cat ? "搞定！" : "已完成"),
  /** 搜不到 */
  notFound: (cat: boolean) => (cat ? "没找到呀，换个说法试试？" : "没有找到，换个说法试试。"),
};
