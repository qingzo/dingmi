import { describe, expect, it } from "vitest";
import { checkKey, homeOf, webHit } from "./webkey";

const KEYS = [
  { name: "百度", key: "bd", url: "https://www.baidu.com/s?wd={q}" },
  { name: "淘宝", key: "tb", url: "https://s.taobao.com/search?q={q}" },
  { name: "看板", key: "kb", url: "https://example.com/board" },
];

describe("网页关键字", () => {
  it("关键字 + 空格 + 内容", () => {
    expect(webHit("bd 明天天气", KEYS)).toMatchObject({ q: "明天天气", url: "https://www.baidu.com/s?wd=%E6%98%8E%E5%A4%A9%E5%A4%A9%E6%B0%94" });
    expect(webHit("BD a&b", KEYS)?.url).toBe("https://www.baidu.com/s?wd=a%26b");
    // 中文输入法的全角空格也算
    expect(webHit("tb　键盘", KEYS)?.q).toBe("键盘");
  });

  it("只打关键字不触发；带了空格没写内容打开首页", () => {
    expect(webHit("tb", KEYS)).toBeNull();
    expect(webHit("tb ", KEYS)).toMatchObject({ q: "", url: "https://s.taobao.com/" });
    expect(webHit("xx 天气", KEYS)).toBeNull();
    expect(webHit("bd天气", KEYS)).toBeNull();
  });

  it("网址里没有 {q} 的直接打开", () => {
    expect(webHit("kb 随便", KEYS)?.url).toBe("https://example.com/board");
    expect(homeOf("https://fanyi.baidu.com/#auto/zh/{q}")).toBe("https://fanyi.baidu.com/");
  });

  it("设置里核对", () => {
    const others = KEYS.slice(1);
    expect(checkKey({ name: "百度", key: "bd", url: "https://www.baidu.com/s?wd={q}" }, others)).toBe("");
    expect(checkKey({ name: "", key: "bd", url: "https://a.com" }, others)).toBe("name");
    expect(checkKey({ name: "x", key: " ", url: "https://a.com" }, others)).toBe("key");
    expect(checkKey({ name: "x", key: "b d", url: "https://a.com" }, others)).toBe("space");
    expect(checkKey({ name: "x", key: "TB", url: "https://a.com" }, others)).toBe("taken");
    expect(checkKey({ name: "x", key: "zz", url: "baidu.com" }, others)).toBe("url");
  });
});
