// 手机网页网址里的配对信息（安全审查第 7 条）：新网址令牌在 # 后面；以前的网址照样认，打开后换成新的写法
import { describe, expect, it } from "vitest";
import { cleanUrl, parseHash, readPairing } from "./pair";

const SEG = "Tok_en-1.PubKey_-x";

describe("配对信息", () => {
  it("新网址：令牌、公钥、空间密钥都在 # 后面", () => {
    const args = parseHash(`#t=${SEG}&k=c3BhY2U`);
    expect(readPairing("/m/", args, null)).toEqual({ seg: SEG, legacy: false });
    expect(args.get("k")).toBe("c3BhY2U");
    // 快捷指令在后面接 #text=
    expect(parseHash(`#t=${SEG}&k=a#text=%E4%BD%A0%E5%A5%BD`).get("text")).toBe("你好");
  });

  it("以前的网址（已经加到主屏幕的）照样能用，标成要换写法", () => {
    expect(readPairing(`/m/${SEG}`, parseHash("#k=abc"), null)).toEqual({ seg: SEG, legacy: true });
    expect(readPairing(`/m/${encodeURIComponent(SEG)}`, new Map(), null)).toEqual({ seg: SEG, legacy: true });
  });

  it("安卓从主屏幕的 /m/ 打开（# 后面是空的）：用存下的", () => {
    expect(readPairing("/m/", new Map(), SEG)).toEqual({ seg: SEG, legacy: false });
    expect(readPairing("/m", new Map(), null)).toEqual({ seg: "", legacy: false });
    // # 里的优先于存下的（换了一部手机的链接）
    expect(readPairing("/m/", parseHash("#t=New.Pk"), SEG).seg).toBe("New.Pk");
  });

  it("乱写的不认", () => {
    for (const bad of ["#t=a\"b.c", "#t=<x>.y", "#t=nodot", "#t=.x"]) expect(readPairing("/m/", parseHash(bad), null).seg, bad).toBe("");
    expect(readPairing("/m/%E0%A4%A", new Map(), null).seg).toBe("");
    expect(readPairing("/m/", new Map(), "bad\"saved").seg).toBe("");
  });

  it("换成新写法后路径里没有令牌", () => {
    const url = cleanUrl(SEG, "c3Bh/Y2U=");
    expect(url).toBe(`/m/#t=${SEG}&k=c3Bh%2FY2U%3D`);
    expect(url.split("#")[0]).toBe("/m/");
    expect(readPairing("/m/", parseHash(url.slice(url.indexOf("#"))), null).seg).toBe(SEG);
    expect(parseHash(url.slice(url.indexOf("#"))).get("k")).toBe("c3Bh/Y2U=");
    expect(cleanUrl(SEG, undefined)).toBe(`/m/#t=${SEG}`);
  });
});
