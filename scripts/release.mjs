// 发布新版本（需求-0.1 第 2.6 节）：
//   npm run release -- keygen                 生成签名密钥（只做一次），公钥写进 apps/desktop/src-tauri/release-pubkey.txt
//   npm run release -- "更新了什么" [--min-version x.y.z] [--url <下载地址开头>]...
// 用私钥给 npm run tauri build 打好的安装包签名，生成 update.json（带安装包的 SHA-256），
// version、min_version、sha256 另外签一个名（meta_signature，见 scripts/update-meta.mjs：没有它叮咪不认 min_version）。
// 只在本地 release/ 里留安装包和 update.json，不再自己往官网放：到后台「版本」里上传这两个，点「发布」（需求-1.x 第 36.5 节）。
// 私钥在 %USERPROFILE%\.dingmi-release\updater.key（测试时可以用环境变量 DINGMI_RELEASE_DIR 换一个目录）。
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { metaText } from "./update-meta.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const pubkeyFile = path.join(root, "apps/desktop/src-tauri/release-pubkey.txt");
const keyDir = process.env.DINGMI_RELEASE_DIR || path.join(os.homedir(), ".dingmi-release");
const keyFile = path.join(keyDir, "updater.key");
const releaseDir = path.join(root, "release");
const DEFAULT_URL = "https://dingmi.app/download/";

function fail(msg) {
  console.error(`\n${{ keygen: "没生成", backfill: "没补上" }[process.argv[2]] ?? "没发布"}：${msg}\n`);
  process.exit(1);
}

/** 运行 tauri 命令行（不经过 shell，路径里有空格、中文也没事） */
function tauri(args) {
  const cli = createRequire(import.meta.url).resolve("@tauri-apps/cli/tauri.js");
  const r = spawnSync(process.execPath, [cli, ...args], { encoding: "utf8" });
  if (r.status !== 0) fail(`tauri ${args[0]} ${args[1]} 出错了：\n${r.stderr || r.stdout}`);
  return r.stdout;
}

/** a 比 b 新吗（按 . 分开逐段比数字，和叮咪里的 update.rs 一样） */
function newer(a, b) {
  const pa = a.split(".").map(Number), pb = b.split(".").map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    if (i >= pa.length) return false;
    if (i >= pb.length) return true;
    if (pa[i] !== pb[i]) return pa[i] > pb[i];
  }
  return false;
}
const validVersion = (v) => /^\d+(\.\d+)*$/.test(v) && v.length <= 20;

/** 仓库里的公钥：# 开头的是说明，第一行别的就是公钥 */
function repoPubkey() {
  return fs.readFileSync(pubkeyFile, "utf8").split(/\r?\n/).map((l) => l.trim()).find((l) => l && !l.startsWith("#")) ?? "";
}

/** 用公钥核对 minisign 签名（sigB64 是 tauri signer 生成的 .sig 文件内容）。对得上回可信注释，对不上回 null */
function verify(pubLine, sigB64, data) {
  try {
    const pk = Buffer.from(pubLine, "base64"); // "Ed" + 8 字节编号 + 32 字节公钥
    const lines = Buffer.from(sigB64, "base64").toString("utf8").split(/\r?\n/);
    const sig = Buffer.from(lines[1], "base64"); // "ED"/"Ed" + 8 字节编号 + 64 字节签名
    const trusted = lines[2].replace(/^trusted comment: /, "");
    const global = Buffer.from(lines[3], "base64");
    if (pk.length !== 42 || sig.length !== 74 || !pk.subarray(2, 10).equals(sig.subarray(2, 10))) return null;
    const key = crypto.createPublicKey({ key: { kty: "OKP", crv: "Ed25519", x: pk.subarray(10).toString("base64url") }, format: "jwk" });
    const alg = sig.subarray(0, 2).toString();
    const msg = alg === "ED" ? crypto.createHash("blake2b512").update(data).digest() : data;
    const sigBytes = sig.subarray(10);
    const ok = crypto.verify(null, msg, key, sigBytes) && crypto.verify(null, Buffer.concat([sigBytes, Buffer.from(trusted)]), key, global);
    return ok ? trusted : null;
  } catch {
    return null;
  }
}

/** 用私钥给文件签名，回 .sig 文件的内容（base64） */
function sign(file, version) {
  const pass = process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ?? "";
  tauri(["signer", "sign", "-f", keyFile, "-p", pass, "--app-version", version, file]);
  const sig = fs.readFileSync(`${file}.sig`, "utf8").trim();
  fs.rmSync(`${file}.sig`);
  return sig;
}

/** 写临时文件再改名：别人读到的要么是旧的、要么是完整的新的 */
function writeAtomic(file, content) {
  fs.writeFileSync(`${file}.tmp`, content);
  fs.renameSync(`${file}.tmp`, file);
}
const sha256 = (buf) => crypto.createHash("sha256").update(buf).digest("hex");

/** update.json 的字段顺序固定：sha256 跟在 size 后面，meta_signature 跟在 signature 后面 */
function updateJson(info) {
  const { version, date, notes, urls, size, sha256: sum, signature, meta_signature: meta, ...rest } = info;
  return `${JSON.stringify({ version, date, notes, urls, size, ...(sum ? { sha256: sum } : {}), signature, ...(meta ? { meta_signature: meta } : {}), ...rest }, null, 2)}\n`;
}

/** 给 version、min_version、sha256 签名（和安装包同一把私钥），签完用仓库里的公钥核对一遍。回签名（base64） */
function signMeta(version, min, sum) {
  const text = metaText(version, min, sum);
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "dingmi-meta-"));
  const file = path.join(dir, "update-meta.txt");
  fs.writeFileSync(file, text);
  const sig = sign(file, version);
  fs.rmSync(dir, { recursive: true, force: true });
  if (!verify(repoPubkey(), sig, Buffer.from(text))?.split("\t").includes(`version:${version}`)) fail("update.json 的签名核对不上，没发布。再试一次，还不行就看看 tauri 命令行的版本");
  return sig;
}

// ---------- 生成密钥 ----------

function keygen() {
  if (fs.existsSync(keyFile)) {
    fail(`${keyFile} 已经有签名私钥了，不会覆盖。\n真要换一对，先把旧的挪走；换了以后，已经装好的叮咪就收不到之后的更新了，要重新下载安装。`);
  }
  fs.mkdirSync(keyDir, { recursive: true });
  tauri(["signer", "generate", "--ci", "-p", "", "-w", keyFile]);
  const pubText = Buffer.from(fs.readFileSync(`${keyFile}.pub`, "utf8").trim(), "base64").toString("utf8");
  const pub = pubText.split(/\r?\n/)[1]?.trim();
  if (!pub) fail(`生成的公钥 ${keyFile}.pub 读不出来`);
  const today = new Date().toLocaleDateString("sv-SE");
  fs.writeFileSync(
    pubkeyFile,
    `# 叮咪检查更新时用这把公钥核对安装包的签名（minisign 格式），${today} 用 npm run release -- keygen 生成。\n` +
      `# 私钥在开发电脑的 %USERPROFILE%\\.dingmi-release\\updater.key，不进仓库。换了这把公钥，已经装好的叮咪就认不出新的安装包了。\n` +
      `${pub}\n`,
  );
  console.log(`
生成好了：
  私钥：${keyFile}
  公钥：已经写进 ${path.relative(root, pubkeyFile)}，记得提交

很要紧：现在就把私钥另存一份离线备份（U 盘或密码管理器），不要放进仓库和网盘。
私钥丢了，已经装好的叮咪就再也收不到更新。
`);
}

// ---------- 发布 ----------

function args(a = process.argv.slice(2)) {
  const o = { notes: null, min: null, urls: [] };
  for (let i = 0; i < a.length; i++) {
    const need = () => a[++i] ?? fail(`${a[i - 1]} 后面要写值`);
    if (a[i] === "--min-version") o.min = need();
    else if (a[i] === "--url") o.urls.push(need());
    else if (a[i].startsWith("--")) fail(`不认识 ${a[i]}`);
    else if (o.notes === null) o.notes = a[i];
    else fail(`多了一个参数「${a[i]}」：更新说明有空格时用引号括起来`);
  }
  return o;
}

/** 几处版本号要一样（需求-0.1 第 2.6 节第 1 条） */
function readVersion() {
  const conf = JSON.parse(fs.readFileSync(path.join(root, "apps/desktop/src-tauri/tauri.conf.json"), "utf8"));
  const v = conf.version;
  const cargo = (p) => fs.readFileSync(path.join(root, p), "utf8").match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  const pkg = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8")).version;
  const lock = JSON.parse(fs.readFileSync(path.join(root, "package-lock.json"), "utf8"));
  const all = {
    "apps/desktop/src-tauri/Cargo.toml": cargo("apps/desktop/src-tauri/Cargo.toml"),
    "apps/server/Cargo.toml": cargo("apps/server/Cargo.toml"),
    "package.json": pkg,
    "package-lock.json": lock.version,
  };
  const diff = Object.entries(all).filter(([, x]) => x !== v);
  if (diff.length) fail(`版本号对不上：tauri.conf.json 是 ${v}，${diff.map(([f, x]) => `${f} 是 ${x}`).join("，")}。改成一样的再打包`);
  if (!validVersion(v)) fail(`版本号 ${v} 不对，只能是数字和点`);
  return v;
}

/** 打包签名，在 release/ 里留安装包和 update.json，到后台「版本」里上传这两个（需求-1.x 第 36.5 节）。
 *  release/site/update.json 记着上次打的是哪一版（进仓库，两台电脑都会发版）：版本号要比它大，min_version、下载地址默认沿用它的 */
function publish() {
  const o = args();
  if (!o.notes?.trim()) fail('要写更新了什么：npm run release -- "更新了什么"（换行写 \\n）');
  const notes = o.notes.replace(/\\n/g, "\n").trim();
  const version = readVersion();
  const setup = path.join(root, `target/release/bundle/nsis/叮咪_${version}_x64-setup.exe`);
  if (!fs.existsSync(setup)) fail(`找不到安装包 ${path.relative(root, setup)}，先 npm run tauri build`);

  const lastPath = path.join(releaseDir, "site", "update.json");
  let prev = null;
  if (fs.existsSync(lastPath)) {
    try {
      prev = JSON.parse(fs.readFileSync(lastPath, "utf8"));
    } catch {
      fail(`${lastPath} 看不懂，先看看是不是坏了`);
    }
    if (!newer(version, prev.version)) fail(`上次打的已经是 ${prev.version} 了，先把版本号改大再打包`);
  }
  const min = o.min ?? prev?.min_version ?? null;
  if (min !== null && (!validVersion(min) || newer(min, version))) fail(`最低版本 ${min} 不对：要是数字和点，而且不能比 ${version} 新`);
  if (o.min && o.min !== prev?.min_version) console.log(`最低版本：${prev?.min_version ?? "没写"} → ${o.min}（比它旧的叮咪会停下同步，提示更新）`);
  const bases = o.urls.length ? o.urls : prev?.urls?.length ? prev.urls.map((u) => u.slice(0, u.lastIndexOf("/") + 1)) : [DEFAULT_URL];
  const name = `dingmi_${version}_x64-setup.exe`;
  const urls = bases.map((b) => (b.endsWith("/") ? b : `${b}/`) + name);

  // 签之前核对私钥和仓库里的公钥是一对：不是的话，签出来没有一台叮咪会装
  if (!fs.existsSync(keyFile)) fail(`没有签名私钥 ${keyFile}：第一次发布先 npm run release -- keygen；换了电脑就从离线备份拷回来`);
  const pub = repoPubkey();
  const probe = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "dingmi-release-")), "probe.bin");
  fs.writeFileSync(probe, crypto.randomBytes(64));
  const probeOk = verify(pub, sign(probe, version), fs.readFileSync(probe));
  fs.rmSync(path.dirname(probe), { recursive: true, force: true });
  if (!probeOk) fail(`私钥 ${keyFile} 和仓库里的公钥（${path.relative(root, pubkeyFile)}）不是一对。\n用离线备份的私钥换回来；或者公钥是测试用的，先 npm run release -- keygen`);

  // 本地 release/ 只留最新的一份（site/ 里那份「上次打的」另外管）
  fs.mkdirSync(releaseDir, { recursive: true });
  for (const f of fs.readdirSync(releaseDir)) {
    const p = path.join(releaseDir, f);
    if (fs.statSync(p).isFile()) fs.rmSync(p);
  }
  const local = path.join(releaseDir, name);
  fs.copyFileSync(setup, local);
  const bytes = fs.readFileSync(local);
  const signature = sign(local, version);
  const trusted = verify(pub, signature, bytes);
  if (!trusted?.split("\t").includes(`version:${version}`)) fail("签好的安装包核对不上，没发布。再试一次，还不行就看看 tauri 命令行的版本");

  const info = {
    version,
    date: new Date().toLocaleDateString("sv-SE"),
    notes,
    urls,
    size: bytes.length,
    sha256: sha256(bytes), // 官网下载页「核对安装包」显示的，用户用 Get-FileHash 对
    signature,
    ...(min ? { min_version: min } : {}),
  };
  // 决定叮咪怎么做的这几样另外签名：叮咪不认没签名的 min_version
  info.meta_signature = signMeta(version, min, info.sha256);
  const json = updateJson(info);
  fs.writeFileSync(path.join(releaseDir, "update.json"), json);
  fs.mkdirSync(path.dirname(lastPath), { recursive: true });
  writeAtomic(lastPath, json);

  console.log(`
打包签好了：叮咪 ${version}（${(bytes.length / 1048576).toFixed(1)} MB${min ? `，最低版本 ${min}` : ""}）
  安装包：${local}
  update.json：${path.join(releaseDir, "update.json")}
  SHA-256：${info.sha256}
  下载地址：${urls.join("\n            ")}

接下来：
  1. 打开后台 → 版本 → 上传新版本，把上面这两个文件一起拖进去，看一眼更新说明，点「发布」。
  2. 把安装包也换到网盘的分享文件夹里（蓝奏云不收 .exe，压成 zip 再传），链接有变的话到后台「网站信息 → 网盘下载」改。
  3. 提交 ${path.relative(root, lastPath)}（记着上次打的是哪一版）。
`);
}

if (process.argv[2] === "keygen") keygen();
else publish();
