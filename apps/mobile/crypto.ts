// 和电脑端 apps/desktop/src-tauri/src/sync/crypto.rs 一样的加密（docs/方案/同步方案.md 第 5.1 节）：
// 同步密码 → Argon2id → 密码钥匙，和空间密钥（网址 # 后面带来的）一起 HKDF 成锁 1，打开服务器上的盒子 1 拿到主密钥；
// 每条记录用主密钥 XChaCha20-Poly1305 加密，输出 nonce(24) + 密文；
// 手机收件箱仍用 tweetnacl 的 nacl.box（页面里内嵌的全局 nacl）。
import { xchacha20poly1305 } from "@noble/ciphers/chacha.js";
import { argon2id } from "hash-wasm";

const KDF = "argon2id$v=19$m=65536,t=3,p=1";
const NONCE = 24;
const enc = new TextEncoder();

export function toB64(b: Uint8Array): string {
  let s = "";
  for (let i = 0; i < b.length; i += 0x8000) s += String.fromCharCode(...b.subarray(i, i + 0x8000));
  return btoa(s);
}

export function fromB64(s: string): Uint8Array {
  s = s.replace(/-/g, "+").replace(/_/g, "/");
  while (s.length % 4) s += "=";
  return Uint8Array.from(atob(s), (c) => c.charCodeAt(0));
}

/** 用服务器上的 salt 和同步密码算出密码钥匙。iPhone 上要一两秒 */
export async function derive(password: string, salt: string): Promise<Uint8Array> {
  const i = salt.lastIndexOf("$");
  if (i < 0) throw new Error("服务器上的 salt 格式不对");
  if (salt.slice(0, i) !== KDF) throw new Error("这个空间用的加密参数比手机网页新，请更新 dingmi-server");
  return argon2id({
    password,
    salt: fromB64(salt.slice(i + 1)),
    parallelism: 1,
    iterations: 3,
    memorySize: 65536,
    hashLength: 32,
    outputType: "binary",
  });
}

export function seal(key: Uint8Array, aad: string, plain: Uint8Array): Uint8Array {
  const nonce = crypto.getRandomValues(new Uint8Array(NONCE));
  const ct = xchacha20poly1305(key, nonce, enc.encode(aad)).encrypt(plain);
  const out = new Uint8Array(NONCE + ct.length);
  out.set(nonce);
  out.set(ct, NONCE);
  return out;
}

/** 解不开（密钥不对、数据被改过）返回 null */
export function open(key: Uint8Array, aad: string, data: Uint8Array): Uint8Array | null {
  if (data.length < NONCE) return null;
  try {
    return xchacha20poly1305(key, data.subarray(0, NONCE), enc.encode(aad)).decrypt(data.subarray(NONCE));
  } catch {
    return null;
  }
}

/** 锁 1 = HKDF-SHA256(密码钥匙 ‖ 空间密钥, "dingmi-lock-v1") */
export async function lockKey(passwordKey: Uint8Array, spaceKey: Uint8Array): Promise<Uint8Array> {
  const ikm = new Uint8Array(passwordKey.length + spaceKey.length);
  ikm.set(passwordKey);
  ikm.set(spaceKey, passwordKey.length);
  const k = await crypto.subtle.importKey("raw", ikm, "HKDF", false, ["deriveBits"]);
  const bits = await crypto.subtle.deriveBits(
    { name: "HKDF", hash: "SHA-256", salt: new Uint8Array(0), info: enc.encode("dingmi-lock-v1") },
    k,
    256,
  );
  return new Uint8Array(bits);
}

export const BOX1_AAD = "dingmi-box1-v1";

/** 用同步密码和空间密钥打开盒子 1，拿到主密钥 */
export async function unlockBox1(password: string, salt: string, box1: string, spaceKey: Uint8Array): Promise<Uint8Array> {
  const lock = await lockKey(await derive(password, salt), spaceKey);
  const master = open(lock, BOX1_AAD, fromB64(box1));
  if (!master || master.length !== 32) throw new Error("同步密码不对，再试一次");
  return master;
}

export const recordAad = (uid: string, ver: string) => `${uid}|${ver}`;
export const fileAad = (uid: string) => `file|${uid}`;

declare const nacl: {
  box: ((msg: Uint8Array, nonce: Uint8Array, pk: Uint8Array, sk: Uint8Array) => Uint8Array) & {
    keyPair: () => { publicKey: Uint8Array; secretKey: Uint8Array };
  };
  randomBytes: (n: number) => Uint8Array;
};

/** 发到收件箱：临时公钥(32) + nonce(24) + 密文，只有电脑能解开 */
export function sealInbox(pk: Uint8Array, plain: Uint8Array): Uint8Array {
  const eph = nacl.box.keyPair();
  const nonce = nacl.randomBytes(24);
  const ct = nacl.box(plain, nonce, pk, eph.secretKey);
  const out = new Uint8Array(56 + ct.length);
  out.set(eph.publicKey, 0);
  out.set(nonce, 32);
  out.set(ct, 56);
  return out;
}
