// 手机上记住的主密钥不再明文放在 localStorage（安全审查第 7 条）：用不可导出的 AES-GCM 钥匙加密后再存
import { describe, expect, it } from "vitest";
import { forgetMaster, loadMaster, saveMaster, type KeyPlace } from "./keystore";

/** 内存里的「IndexedDB + localStorage」 */
function memory(): KeyPlace & { ls: Map<string, string>; wrap: CryptoKey | null } {
  const m = {
    ls: new Map<string, string>(),
    wrap: null as CryptoKey | null,
    getWrap: async () => m.wrap,
    setWrap: async (k: CryptoKey | null) => void (m.wrap = k),
    get: (k: string) => m.ls.get(k) ?? null,
    set: (k: string, v: string | null) => void (v === null ? m.ls.delete(k) : m.ls.set(k, v)),
  };
  return m;
}

const master = Uint8Array.from({ length: 32 }, (_, i) => i * 7);
const b64 = (b: Uint8Array) => btoa(String.fromCharCode(...b));

describe("记住主密钥", () => {
  it("存进去的不是明文，钥匙导不出来，取得回来", async () => {
    const p = memory();
    expect(await saveMaster(p, master)).toBe(true);
    // 攻击：一次性把 localStorage 整个读走——里面没有主密钥的明文（也没有 base64 的明文）
    const dump = [...p.ls.values()].join("\n");
    expect(dump).not.toContain(b64(master));
    expect(p.ls.has("key")).toBe(false);
    // 钥匙是不可导出的
    expect(p.wrap!.extractable).toBe(false);
    await expect(crypto.subtle.exportKey("raw", p.wrap!)).rejects.toThrow();
    expect(await loadMaster(p)).toEqual(master);
  });

  it("老手机上存的明文：打开时换成加密的，明文删掉", async () => {
    const p = memory();
    p.set("key", b64(master));
    expect(await loadMaster(p)).toEqual(master);
    expect(p.get("key")).toBeNull();
    expect(p.get("key-w")).not.toBeNull();
    expect([...p.ls.values()].join("\n")).not.toContain(b64(master));
    // 下次打开从加密的取
    expect(await loadMaster(p)).toEqual(master);
  });

  it("钥匙没了、密文被改过：当没记住（要重新输同步密码）", async () => {
    const p = memory();
    await saveMaster(p, master);
    const sealed = p.get("key-w")!;
    p.wrap = null;
    expect(await loadMaster(p)).toBeNull();
    expect(p.get("key-w")).toBeNull();
    await saveMaster(p, master);
    const raw = Uint8Array.from(atob(p.get("key-w")!), (c) => c.charCodeAt(0));
    raw[20] ^= 1;
    p.set("key-w", b64(raw));
    expect(await loadMaster(p)).toBeNull();
    expect(sealed).not.toBe("");
  });

  it("锁上：全删掉", async () => {
    const p = memory();
    await saveMaster(p, master);
    p.set("key", "x");
    await forgetMaster(p);
    expect([p.get("key"), p.get("key-w"), p.wrap]).toEqual([null, null, null]);
    expect(await loadMaster(p)).toBeNull();
  });

  it("存不了（浏览器不让用 IndexedDB）：不退回存明文", async () => {
    const p = memory();
    p.setWrap = async () => {
      throw new Error("不让存");
    };
    expect(await saveMaster(p, master)).toBe(false);
    expect([...p.ls.keys()]).toEqual([]);
  });
});
