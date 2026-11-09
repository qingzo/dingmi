// 网页关键字（需求-0.3 第 10.2 节）：快捷框里打「关键字 + 空格 + 内容」，回车用浏览器搜。
// 只打关键字、不带空格时不触发（tb 照样能找到程序）；带了空格还没写内容，回车打开网站首页
import type { WebKey } from "./api";

export interface WebHit {
  k: WebKey;
  /** 要搜的内容，可能是空的 */
  q: string;
  /** 回车打开的网址 */
  url: string;
}

/** 网站首页：网址里 {q} 之前那段的域名 */
export function homeOf(url: string): string {
  try {
    return new URL(url.replace(/\{q\}/g, "")).origin + "/";
  } catch {
    return url;
  }
}

/** 打的是不是「关键字 + 空格 + 内容」；关键字不分大小写 */
export function webHit(input: string, keys: WebKey[]): WebHit | null {
  const m = /^(\S+)[ 　]+([\s\S]*)$/.exec(input.replace(/^\s+/, ""));
  if (!m) return null;
  const key = m[1].toLowerCase();
  const k = keys.find((x) => x.key.toLowerCase() === key);
  if (!k) return null;
  const q = m[2].trim();
  const url = !q ? homeOf(k.url) : k.url.includes("{q}") ? k.url.replace(/\{q\}/g, encodeURIComponent(q)) : k.url;
  return { k, q, url };
}

/** 设置里改一条时核对：返回出错的原因，没问题是空字符串 */
export function checkKey(k: WebKey, others: WebKey[]): "name" | "key" | "space" | "taken" | "url" | "" {
  if (!k.name.trim()) return "name";
  if (!k.key.trim()) return "key";
  if (/\s/.test(k.key.trim())) return "space";
  if (others.some((x) => x.key.toLowerCase() === k.key.trim().toLowerCase())) return "taken";
  if (!/^https?:\/\/\S+$/i.test(k.url.trim())) return "url";
  return "";
}
