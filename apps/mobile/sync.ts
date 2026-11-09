// 手机版的同步：和电脑端一样从服务器拉记录、推本机改过的，规则见同步方案第 14 节。
// 拉到的密文原样存进 IndexedDB（没网时也能看），打开时解开放进内存；手机上改的先进待上传队列。
import { fromB64, open, recordAad, seal, toB64 } from "./crypto";
import { Clock, nowSec, uuidv7, type Payload, type SNote } from "./model";

interface Stored {
  uid: string;
  ver: string;
  blob: string;
}

interface Remote extends Stored {
  seq: number;
}

/** 服务器回 401：这部手机已经在电脑上作废了 */
export class Revoked extends Error {}

// ---------- IndexedDB ----------

const DB = "dingmi-mobile";
let dbp: Promise<IDBDatabase> | null = null;

function idb(): Promise<IDBDatabase> {
  dbp ??= new Promise((resolve, reject) => {
    const r = indexedDB.open(DB, 1);
    r.onupgradeneeded = () => {
      r.result.createObjectStore("items", { keyPath: "uid" });
      r.result.createObjectStore("pending", { keyPath: "uid" });
    };
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
  return dbp;
}

function req<T>(r: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    r.onsuccess = () => resolve(r.result);
    r.onerror = () => reject(r.error);
  });
}

async function all(store: string): Promise<Stored[]> {
  return req((await idb()).transaction(store).objectStore(store).getAll());
}

async function write(ops: { store: string; put?: Stored; del?: string }[]) {
  if (!ops.length) return;
  const t = (await idb()).transaction(["items", "pending"], "readwrite");
  for (const o of ops) {
    if (o.put) t.objectStore(o.store).put(o.put);
    if (o.del) t.objectStore(o.store).delete(o.del);
  }
  await new Promise<void>((resolve, reject) => {
    t.oncomplete = () => resolve();
    t.onerror = () => reject(t.error);
  });
}

/** 锁上时清掉缓存的全部数据 */
export async function wipe() {
  if (dbp) (await dbp).close();
  dbp = null;
  await new Promise<void>((resolve) => {
    const r = indexedDB.deleteDatabase(DB);
    r.onsuccess = r.onerror = r.onblocked = () => resolve();
  });
}

// ---------- 本地存储的小东西 ----------

export const ls = {
  get: (k: string) => {
    try {
      return localStorage.getItem("dingmi-" + k);
    } catch {
      return null;
    }
  },
  set: (k: string, v: string | null) => {
    try {
      if (v === null) localStorage.removeItem("dingmi-" + k);
      else localStorage.setItem("dingmi-" + k, v);
    } catch {
      /* 存不了就算了，下次打开重新拉 */
    }
  },
};

/** 手机的设备编号，版本号的最后一段 */
function deviceId(): string {
  let d = ls.get("device");
  if (!d) {
    d = [...crypto.getRandomValues(new Uint8Array(6))].map((b) => b.toString(16).padStart(2, "0")).join("");
    ls.set("device", d);
  }
  return d;
}

// ---------- 引擎 ----------

export class Engine {
  /** 解开的全部记录：uid → {版本, 内容} */
  items = new Map<string, { ver: string; p: Payload }>();
  /** 手机上改了、还没推上去的 */
  pending = new Set<string>();
  lastSync = ls.get("last-sync");
  private clock = new Clock(deviceId());
  status: "idle" | "syncing" | "ok" | "offline" | "revoked" = "idle";
  error = "";
  private busy: Promise<void> | null = null;
  private again = false;

  constructor(
    private token: string,
    private key: Uint8Array,
    private onChange: () => void,
  ) {}

  private decrypt(s: Stored): Payload | null {
    const plain = open(this.key, recordAad(s.uid, s.ver), fromB64(s.blob));
    if (!plain) return null;
    try {
      const p = JSON.parse(new TextDecoder().decode(plain)) as Payload;
      return p?.d?.uid === s.uid ? p : null;
    } catch {
      return null;
    }
  }

  /** 打开时：从 IndexedDB 里读出上次的内容 */
  async load() {
    const [items, pending] = await Promise.all([all("items"), all("pending")]);
    for (const s of items) {
      const p = this.decrypt(s);
      if (p) this.items.set(s.uid, { ver: s.ver, p });
      this.clock.observe(s.ver);
    }
    for (const s of pending) this.pending.add(s.uid);
  }

  private async api(path: string, init?: RequestInit) {
    const r = await fetch(path, { ...init, headers: { ...init?.headers, Authorization: "Bearer " + this.token } });
    if (r.status === 401) throw new Revoked(await r.text().catch(() => ""));
    if (!r.ok) throw new Error((await r.text().catch(() => "")) || "服务器回了 " + r.status);
    return r;
  }

  /** 改一条（新建、修改、删除都是这个）：给新版本号，加密存好，排进待上传 */
  async save(p: Payload) {
    const uid = p.d.uid;
    const ver = this.clock.next();
    const blob = toB64(seal(this.key, recordAad(uid, ver), new TextEncoder().encode(JSON.stringify(p))));
    const s = { uid, ver, blob };
    await write([
      { store: "items", put: s },
      { store: "pending", put: s },
    ]);
    this.items.set(uid, { ver, p });
    this.pending.add(uid);
    this.onChange();
    void this.sync();
  }

  /** 推上去再拉下来。同时只跑一次，跑的时候又有人叫就再跑一遍。不会抛错，结果看 status */
  sync(): Promise<void> {
    if (this.busy) {
      this.again = true;
      return this.busy;
    }
    this.status = "syncing";
    this.onChange();
    this.busy = (async () => {
      try {
        do {
          this.again = false;
          const rejected = await this.push();
          await this.pull(rejected);
          // 被服务器拒了的（别处有更新的版本）：拉完已经合并好，冲突的笔记也另存了副本
          if (rejected.size) await this.dropPending([...rejected]);
        } while (this.again);
        this.status = "ok";
        this.error = "";
        this.lastSync = nowSec();
        ls.set("last-sync", this.lastSync);
      } catch (e) {
        this.status = e instanceof Revoked ? "revoked" : "offline";
        this.error = e instanceof TypeError ? "连不上服务器" : e instanceof Error ? e.message : String(e);
      } finally {
        this.busy = null;
        this.onChange();
      }
    })();
    return this.busy;
  }

  private async dropPending(uids: string[]) {
    await write(uids.map((del) => ({ store: "pending", del })));
    uids.forEach((u) => this.pending.delete(u));
  }

  /** 把待上传的推上去，返回被服务器拒了的 uid */
  private async push(): Promise<Set<string>> {
    const rejected = new Set<string>();
    const list = await all("pending");
    for (let i = 0; i < list.length; ) {
      const batch: Stored[] = [];
      let size = 0;
      while (i < list.length && (batch.length === 0 || size + list[i].blob.length < 3_000_000)) {
        size += list[i].blob.length;
        batch.push(list[i++]);
      }
      const r = await this.api("/m/api/push", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ items: batch }),
      });
      const { acks } = (await r.json()) as { acks: { uid: string; ok: boolean }[] };
      const done: string[] = [];
      for (const a of acks) {
        const sent = batch.find((b) => b.uid === a.uid);
        // 推的时候又改过的，留着下次推
        if (sent && this.items.get(a.uid)?.ver !== sent.ver) continue;
        if (a.ok) done.push(a.uid);
        else rejected.add(a.uid);
      }
      await this.dropPending(done);
    }
    return rejected;
  }

  private async pull(rejected: Set<string>) {
    let since = Number(ls.get("since") ?? 0);
    let changed = false;
    for (;;) {
      const r = await this.api(`/m/api/pull?since=${since}`);
      const { items, more, epoch } = (await r.json()) as { items: Remote[]; more: boolean; epoch?: string };
      // 服务器拿旧备份恢复过（epoch 变了）：这里记的拉取位置比服务器上的还靠后，从头拉一遍
      if (epoch) {
        const seen = ls.get("epoch");
        ls.set("epoch", epoch);
        if (seen && seen !== epoch && since > 0) {
          since = 0;
          continue;
        }
      }
      const ops: { store: string; put?: Stored; del?: string }[] = [];
      const copies: SNote[] = [];
      for (const it of items) {
        since = Math.max(since, it.seq);
        const local = this.items.get(it.uid);
        if (local && local.ver >= it.ver) continue;
        const p = this.decrypt(it);
        if (!p) continue;
        this.clock.observe(it.ver);
        // 手机上也改过这条笔记、还没推上去：先把手机上的另存一份，免得丢了
        if (local && (this.pending.has(it.uid) || rejected.has(it.uid))) {
          if (local.p.k === "note" && p.k === "note" && !local.p.d.deleted_at && (local.p.d.title !== p.d.title || local.p.d.body !== p.d.body))
            copies.push(local.p.d);
          this.pending.delete(it.uid);
          ops.push({ store: "pending", del: it.uid });
        }
        this.items.set(it.uid, { ver: it.ver, p });
        ops.push({ store: "items", put: { uid: it.uid, ver: it.ver, blob: it.blob } });
        changed = true;
      }
      await write(ops);
      ls.set("since", String(since));
      for (const n of copies) {
        const now = nowSec();
        await this.save({ k: "note", d: { ...n, uid: uuidv7(), title: `${n.title}（冲突副本）`, created_at: now, updated_at: now } });
      }
      if (!more) break;
    }
    if (changed) this.onChange();
  }
}
