// 手机上记住主密钥（输过同步密码以后下次不用再输）：不再把主密钥明文放在 localStorage（安全审查第 7 条）。
//
// 解密用的 @noble/ciphers（XChaCha20-Poly1305）要主密钥的原始字节，没法直接用 WebCrypto 里「不可导出」的钥匙。
// 所以这样存：生成一把不可导出的 AES-GCM 钥匙（CryptoKey，存在 IndexedDB 里，网页的脚本只能拿它加密解密、拿不到它的字节），
// 用它把主密钥加密后放进 localStorage。取舍：
// - 防得住：一次性把 localStorage（或者浏览器的存储文件、备份）整个拿走——拿到的是密文，钥匙不在里面、也导不出来；
// - 防不住：已经能在网页里一直运行脚本的人（XSS、不怀好意的服务器改了网页）——它可以像网页自己一样调这把钥匙解密。
//   这是「手机网页由服务器下发」这个架构本身的限制，见隐私说明（https://dingmi.app/privacy/）。
// 以前存的明文（localStorage 的 dingmi-key）打开时换成加密的，并删掉明文。

/** 存的地方：IndexedDB 存钥匙，localStorage 存加密后的主密钥。测试里换成内存里的 */
export interface KeyPlace {
  getWrap(): Promise<CryptoKey | null>;
  setWrap(k: CryptoKey | null): Promise<void>;
  get(name: string): string | null;
  set(name: string, v: string | null): void;
}

/** 加密后的主密钥：base64(iv(12) + 密文) */
const SEALED = "key-w";
/** 以前存的明文主密钥 */
const PLAIN = "key";
const AAD = new TextEncoder().encode("dingmi-mobile-master-v1");

const b64 = (b: Uint8Array) => btoa(String.fromCharCode(...b));
const unb64 = (s: string) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));

async function wrapKey(place: KeyPlace, create: boolean): Promise<CryptoKey | null> {
  const k = await place.getWrap();
  if (k || !create) return k;
  const fresh = await crypto.subtle.generateKey({ name: "AES-GCM", length: 256 }, false, ["encrypt", "decrypt"]);
  await place.setWrap(fresh);
  return fresh;
}

/** 记住主密钥。存不了（浏览器不让用 IndexedDB 之类）回 false：这次照常用，下次打开要重新输同步密码 */
export async function saveMaster(place: KeyPlace, master: Uint8Array): Promise<boolean> {
  place.set(PLAIN, null);
  try {
    const k = (await wrapKey(place, true))!;
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const ct = new Uint8Array(await crypto.subtle.encrypt({ name: "AES-GCM", iv, additionalData: AAD }, k, master as BufferSource));
    const out = new Uint8Array(12 + ct.length);
    out.set(iv);
    out.set(ct, 12);
    place.set(SEALED, b64(out));
    return true;
  } catch {
    place.set(SEALED, null);
    return false;
  }
}

/** 取出记住的主密钥；没有、解不开回 null。以前存的明文在这里换成加密的，明文删掉 */
export async function loadMaster(place: KeyPlace): Promise<Uint8Array | null> {
  const plain = place.get(PLAIN);
  if (plain) {
    let master: Uint8Array | null = null;
    try {
      master = unb64(plain);
    } catch {
      /* 坏了就当没有 */
    }
    place.set(PLAIN, null);
    if (master?.length !== 32) return null;
    await saveMaster(place, master);
    return master;
  }
  const sealed = place.get(SEALED);
  if (!sealed) return null;
  try {
    const k = await wrapKey(place, false);
    if (!k) throw new Error("钥匙不见了");
    const raw = unb64(sealed);
    const master = new Uint8Array(await crypto.subtle.decrypt({ name: "AES-GCM", iv: raw.subarray(0, 12), additionalData: AAD }, k, raw.subarray(12)));
    return master.length === 32 ? master : null;
  } catch {
    place.set(SEALED, null);
    return null;
  }
}

/** 忘掉（锁上、手机作废了）：加密的主密钥和钥匙都删掉 */
export async function forgetMaster(place: KeyPlace): Promise<void> {
  place.set(PLAIN, null);
  place.set(SEALED, null);
  try {
    await place.setWrap(null);
  } catch {
    /* 删不了也没关系，没有加密的主密钥它就没用 */
  }
}

// ---------- 浏览器里：钥匙放在 IndexedDB 的 dingmi-keys 里（和同步数据的 dingmi-mobile 分开，锁上清数据时不受影响） ----------

function keyDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const r = indexedDB.open("dingmi-keys", 1);
    r.onupgradeneeded = () => r.result.createObjectStore("keys");
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}

async function idbDo<T>(mode: IDBTransactionMode, f: (s: IDBObjectStore) => IDBRequest): Promise<T> {
  const db = await keyDb();
  try {
    return await new Promise<T>((resolve, reject) => {
      const r = f(db.transaction("keys", mode).objectStore("keys"));
      r.onsuccess = () => resolve(r.result as T);
      r.onerror = () => reject(r.error);
    });
  } finally {
    db.close();
  }
}

/** 浏览器里用的：ls 是 sync.ts 的 localStorage 小工具（键名前面加 dingmi-） */
export function browserPlace(ls: { get(k: string): string | null; set(k: string, v: string | null): void }): KeyPlace {
  return {
    getWrap: async () => (await idbDo<CryptoKey | undefined>("readonly", (s) => s.get("wrap"))) ?? null,
    setWrap: async (k) => void (await idbDo("readwrite", (s) => (k ? s.put(k, "wrap") : s.delete("wrap")))),
    get: ls.get,
    set: ls.set,
  };
}
