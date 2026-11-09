// 用手机加入：4 个字和电脑端一样（同一组数据，电脑端见 crypto.rs 的 phone_seal_and_words）、配对码接上空间密钥
import { describe, expect, it } from "vitest";
import { cleanCode, phoneWords, withSpaceKey } from "./join";

describe("用手机加入", () => {
  it("4 个字和电脑端算的一样", async () => {
    expect(await phoneWords(new Uint8Array(32).fill(7))).toBe("鞋峰鹿绿");
  });

  it("配对码后面接上空间密钥（base64url 不带 =）", () => {
    const k = new Uint8Array(32).fill(255);
    expect(withSpaceKey("dingmi://1.2.3.4:7788/PUB/tok", k)).toBe("dingmi://1.2.3.4:7788/PUB/tok/" + "_".repeat(42) + "8");
  });

  it("数字只留 8 位数字", () => {
    expect(cleanCode(" 4817 0293 ")).toBe("48170293");
    expect(cleanCode("1234-5678-9")).toBe("12345678");
  });
});
