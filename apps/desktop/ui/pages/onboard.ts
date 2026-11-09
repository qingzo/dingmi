// 首次引导（需求-0.2 第 5 节、U10）：第 1 步你是第一次用吗（或者加入同步、从导出包恢复）→ 第 2 步试一下快捷框 → 第 3 步几个开关。
// 只在这台电脑第一次打开叮咪时由 Rust 打开（onboard.rs）；「设置 → 关于 → 再看一遍引导」也能打开
import "../styles/ui.css";
import "../styles/onboard.css";
import { mascotOn } from "../lib/look";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  api,
  errText,
  type FolderCandidate,
  type FolderProbe,
  type OnboardInfo,
  type OnboardTried,
  type PhoneWait,
  type RestorePeek,
  type Settings,
} from "../lib/api";
import { icon, type IconName } from "@shared/icons";
import { cat } from "@shared/mascot";
import { $, copyText, esc, friendlyDT, hotkeyText, parseDT } from "@shared/util";
import { waitPhone } from "../lib/official";
import { T, guideNote } from "../text/onboard";

const win = getCurrentWindow();
const card = $("#card");

type Screen = "hello" | "sync" | "restore" | "try" | "switches";
type Choice = "first" | "sync" | "restore";
type Note = { kind: "ok" | "warn" | "err"; title: string; text: string; code?: string; /** 带一个「去设置」，跳到 设置 → 通用 → 快捷键 */ keys?: boolean };

const S = {
  screen: "hello" as Screen,
  choice: "first" as Choice,
  info: null as OnboardInfo | null,
  syncOn: false,
  /** 团子猫挥过手了：再画第 1 步时不再挥 */
  waved: false,
  busy: false,
  /** 这一屏上的出错小条 */
  msg: null as Note | null,
  /** 第 2 步顶上的小条：刚加入同步、刚恢复完 */
  after: null as Note | null,
  // 加入同步
  /** folder 网盘文件夹 / server 自建服务器的配对码 / official 叮咪同步（平时也是配对码，原来的电脑都没了时 rec 换成恢复码） */
  mode: "folder" as "folder" | "server" | "official",
  /** 叮咪同步走哪一种（需求-1.x 第 34 节）：ways 先选 / code 兑换码 + 同步密码 / pair 配对码 / rec 恢复码 / phone 用手机加入 / phonepw 手机允许了 */
  osub: "ways" as "ways" | "code" | "pair" | "rec" | "phone" | "phonepw",
  redeem: "",
  /** 用手机加入：8 位数字、4 个字；gone 是过期了、手机上点了「不是我」 */
  wait: null as PhoneWait | null,
  gone: "",
  cands: undefined as FolderCandidate[] | undefined,
  probe: null as FolderProbe | null,
  pw: "",
  code: "",
  /** 叮咪同步：恢复码，和（可选的）新同步密码 */
  rc: "",
  newPw: "",
  name: "",
  // 从导出包恢复
  pkg: null as { path: string; peek: RestorePeek } | null,
  picking: false,
  // 第 2 步：用户真的用快捷框记下了一条
  tried: null as OnboardTried | null,
  copied: false,
  // 第 3 步
  sw: { autostart: true, widget: false, morning: true, wrap: true, wrap_time: "18:00" },
};

const STEP: Record<Screen, number> = { hello: 1, sync: 1, restore: 1, try: 2, switches: 3 };

const dots = (n: number) => `<span class="dots" aria-label="${T.stepOf(n)}">${[1, 2, 3].map((i) => `<i class="${i < n ? "done" : i === n ? "on" : ""}"></i>`).join("")}</span>`;
const banner = (n: Note) =>
  `<div class="banner ${n.kind}" role="${n.kind === "err" ? "alert" : "status"}">${icon(n.kind === "ok" ? "circle-check" : n.kind === "warn" ? "info" : "circle-alert")}<div><b>${esc(n.title)}</b>${esc(n.text)}${
    n.code ? `<span class="code">${esc(n.code)}</span><button class="btn sm" data-copy="${esc(n.code)}">${icon("copy")}${T.try.copy}</button>` : ""
  }</div>${n.keys ? `<button class="btn sm" data-goset>${T.try.toSettings}</button>` : ""}</div>`;
const btn = (label: string, attrs: string, cls = "") => `<button class="btn ${cls}" ${attrs} ${S.busy ? "disabled" : ""}>${label}</button>`;

// ---------- 每一屏 ----------

function hello(): [string, string] {
  const on = mascotOn();
  const opt = (c: Choice, ic: IconName, [b, small]: string[], off = false) =>
    `<button class="opt ${S.choice === c ? "on" : ""}" data-choice="${c}" aria-pressed="${S.choice === c}" ${off ? "disabled" : ""}><span class="ic">${icon(ic)}</span><div><b>${b}</b><small>${small}</small></div><span class="radio ${S.choice === c ? "on" : ""}"></span></button>`;
  const body = `<div class="hero">${on ? `<span class="cat mascot ${S.waved ? "" : "play"}">${cat("wave")}</span>` : ""}<div><h3 id="ob-title">${on ? T.hello.title : T.hello.titlePlain}</h3><p class="s">${on ? T.hello.sub : T.hello.subPlain}</p></div></div>
    <div class="opts">${opt("first", "sparkles", T.hello.first)}${opt("sync", "refresh-cw", S.syncOn ? [T.hello.sync[0], T.hello.syncOn] : T.hello.sync, S.syncOn)}${opt("restore", "archive-restore", T.hello.restore)}</div>
    ${S.msg ? banner(S.msg) : ""}`;
  S.waved = true;
  return [body, `<span class="sp"></span>${btn(T.next, "data-go=next", "pri lg")}`];
}

function syncScreen(): [string, string] {
  const t = T.sync;
  const way = (k: typeof S.mode, label: string) => `<button data-way="${k}" aria-pressed="${S.mode === k}">${label}</button>`;
  const seg = `<div class="seg" role="tablist">${way("folder", t.folder)}${way("server", t.server)}${way("official", t.official)}</div>`;
  const status = (p: FolderProbe) => (p.state === "space" ? t.has(p.devices) : p.state === "empty" ? t.empty : (p.error ?? t.bad));
  const row = (label: string, path: string, p: FolderProbe) =>
    `<button class="pick ${S.probe?.dir === p.dir ? "on" : ""}" data-folder="${esc(path)}" ${p.state === "bad" ? "disabled" : ""}>${icon("folder-sync", "s16")}<span class="t"><b>${esc(label)}</b><small>${esc(path)}</small></span><span class="st" data-st="${p.state}">${esc(status(p))}</span></button>`;
  const known = new Set((S.cands ?? []).map((c) => c.probe.dir));
  const picked = S.probe && !known.has(S.probe.dir) ? row(S.probe.dir.split(/[\\/]/).filter(Boolean).slice(-2, -1)[0] ?? S.probe.dir, S.probe.dir, S.probe) : "";
  const name = `<label>${t.name}<small>${t.nameSub}</small><input class="field" id="ob-name" value="${esc(S.name)}" spellcheck="false"></label>`;
  const pw = `<label>${t.password}<small>${t.passwordSub}</small><input class="field" type="password" id="ob-pw" value="${esc(S.pw)}" autocomplete="current-password"></label>`;
  const folder = `<div class="picks">
      ${
        S.cands === undefined
          ? `<div class="pick wait"><span class="spin"></span><span class="t">${t.finding}</span></div>`
          : S.cands.map((c) => row(c.label, c.path, c.probe)).join("") || `<div class="pick wait">${icon("folder-x", "s16")}<span class="t"><b>${t.none}</b><small>${t.noneSub}</small></span></div>`
      }
      ${picked}
      <button class="pick" data-go="other">${icon("folder-open", "s16")}<span class="t"><b>${t.other}</b><small>${t.otherSub}</small></span></button>
    </div>
    <div class="form two">${pw}${name}</div>`;
  const code = (top = "") =>
    `<div class="form">${top}<label>${t.code}<small>${t.codeSub}</small><input class="field" id="ob-code" value="${esc(S.code)}" placeholder="${t.codeHolder}" spellcheck="false" autocomplete="off"></label><div class="form two">${pw}${name}</div></div>`;
  const o = S.mode === "official" ? S.osub : null;
  const rec = o === "rec";
  const other = `<button type="button" class="lnk" data-osub="ways">${t.otherWays}</button>`;
  // 叮咪同步：先选四种加法里的一种（需求-1.x 第 34 节）；原来的电脑都没了用恢复码 + 名字 +（忘了原来的才填）新的同步密码（需求-1.0 第 30.4 节）
  const opt = ([k, ic, b, small]: string[], i: number) =>
    `<button class="opt" data-osub="${k}"><span class="ic">${icon(ic as IconName)}</span><div><b>${b}${i ? "" : `<em class="tagr">${T.sync.best}</em>`}</b><small>${small}</small></div>${icon("chevron-right")}</button>`;
  const digits = (c: string) => `<div class="digits">${[...c].map((d, i) => `${i === 4 ? "<i></i>" : ""}<span>${esc(d)}</span>`).join("")}</div>`;
  const words = (w: string) => `<div class="words">${[...w].map((d) => `<span>${esc(d)}</span>`).join("")}</div>`;
  const official =
    o === "ways"
      ? `<p class="s">${t.waysSub}</p><div class="opts">${t.ways.map(opt).join("")}</div>`
      : o === "rec"
        ? `<div class="form"><p class="s">${t.officialSub} ${other}</p><label>${t.rc}<small>${t.rcSub}</small><input class="field" id="ob-rc" value="${esc(S.rc)}" placeholder="DMR-…" spellcheck="false" autocomplete="off"></label>
    <div class="form two"><label>${t.newPw}<small>${t.newPwSub}</small><input class="field" type="password" id="ob-newpw" value="${esc(S.newPw)}" autocomplete="new-password"></label>${name}</div></div>`
        : o === "code"
          ? `<div class="form"><p class="s">${other}</p><label>${t.redeem}<small>${t.redeemSub}</small><input class="field mono" id="ob-redeem" value="${esc(S.redeem)}" placeholder="DM-XXXX-XXXX-XXXX-XXXX" spellcheck="false" autocomplete="off"></label><div class="form two">${pw}${name}</div></div>`
          : o === "phone"
            ? `<div class="form"><p class="s">${t.phoneSub} ${other}</p><div class="phone-wait">${
                S.gone
                  ? `<p class="s">${esc(S.gone)}</p><button class="btn sm pri" data-go="phone-again">${t.phoneAgain}</button>`
                  : S.wait
                    ? `${digits(S.wait.code)}${words(S.wait.words)}<span class="wait"><span class="spin"></span>${esc(t.phoneWait(S.wait.expires_at.slice(11, 16)))}</span>`
                    : `<span class="wait"><span class="spin"></span>${t.phoneAsking}</span>`
              }</div></div>`
            : o === "phonepw"
              ? `<div class="form"><p class="s"><b>${t.phoneOk}</b> ${other}</p><div class="form two">${pw}${name}</div></div>`
              : code(`<p class="s">${t.officialCode} ${other}</p>`);
  const body = `<div><h3 id="ob-title">${t.title}</h3><p class="s">${t.sub}</p></div>${seg}${S.mode === "folder" ? folder : S.mode === "server" ? code() : official}${S.msg ? banner(S.msg) : ""}`;
  const waiting = o === "ways" || o === "phone";
  const go = waiting ? "" : S.busy ? `<button class="btn pri lg" disabled><span class="spin"></span>${rec ? t.recovering : t.joining}</button>` : btn(rec ? t.recover : t.join, "data-go=join", "pri lg");
  return [body, `${btn(T.back, "data-go=back", "ghost")}<span class="sp"></span>${go}`];
}

function restoreScreen(): [string, string] {
  const t = T.restore;
  const p = S.pkg;
  const body = `<div><h3 id="ob-title">${t.title}</h3>${S.picking ? `<p class="s">${t.picking}</p>` : ""}</div>
    ${
      p
        ? `<div class="pkg">${icon("archive")}<div><b>${esc(t.summary(p.peek.exported_at.slice(0, 10), p.peek.reminders, p.peek.notes, p.peek.attachments))}</b><small>${esc(p.path)}</small></div></div><p class="hint">${t.hint}</p>`
        : !S.picking && !S.msg
          ? `<p class="s">${t.cancelled}</p>`
          : ""
    }
    ${S.msg ? banner(S.msg) : ""}`;
  const go = S.busy ? `<button class="btn pri lg" disabled><span class="spin"></span>${t.going}</button>` : p ? btn(t.go, "data-go=restore", "pri lg") : "";
  return [body, `${btn(T.back, "data-go=back", "ghost")}<span class="sp"></span>${S.picking ? "" : btn(p ? t.again : t.pick, "data-go=pick", p ? "" : "pri lg")}${go}`];
}

/** 提醒什么时候弹：一小时之内写「5 分钟后」，再远写「明天 15:00」。
 *  提醒只记到分钟，「1 分钟后」记下来可能只差几十秒：不满一分钟的往上算，照样写「1 分钟后」 */
function triedText(t: OnboardTried) {
  if (t.kind === "note" || !t.due_at) return T.try.okNote;
  const d = parseDT(t.due_at);
  const min = d ? Math.ceil((d.getTime() - Date.now()) / 60000) : -1;
  return min >= 1 && min <= 60 ? T.try.okSoon(min) : T.try.okAt(friendlyDT(t.due_at));
}

function tryScreen(): [string, string] {
  const t = T.try;
  const info = S.info;
  const k = info?.flash ?? "";
  const keys = hotkeyText(k).split(" + ");
  const warn: Note | null = !k
    ? { kind: "warn", title: t.unset[0], text: t.unset[1], keys: true }
    : info && !info.flash_ok
      ? { kind: "warn", title: t.broken(k)[0], text: t.broken(k)[1], keys: true }
      : info?.moved_from
        ? { kind: "warn", title: t.moved(info.moved_from, k)[0], text: t.moved(info.moved_from, k)[1], keys: true }
        : null;
  const usable = !!k && info?.flash_ok !== false;
  const body = `${S.after ? banner(S.after) : ""}<div><h3 id="ob-title">${t.title}</h3><p class="s">${t.sub}</p></div>
    <div class="keyshow ${usable ? "" : "off"}">${k ? keys.map((x) => `<kbd>${esc(x)}</kbd>`).join("<span>+</span>") : `<kbd>?</kbd>`}</div>
    ${
      S.tried
        ? `<div class="banner ok" role="status">${icon("circle-check")}<div><b>${t.ok}</b>${esc(triedText(S.tried))}</div></div>`
        : `${warn ? banner(warn) : ""}<div class="sample">${icon("quote")}<span>${t.sample}</span><button class="btn sm" data-copy="${t.sample}">${icon(S.copied ? "check" : "copy")}${S.copied ? t.copied : t.copy}</button></div>
           ${usable ? `<div class="wait"><span class="pulse"></span>${t.wait}</div>` : ""}`
    }
    ${warn && !S.tried ? "" : `<div class="tips"><div>${icon("text-select")}${t.tipSel}</div><div>${icon("app-window")}${t.tipApp}</div></div>`}`;
  return [body, `${btn(T.back, "data-go=back", "ghost")}<span class="sp"></span>${S.tried ? "" : btn(T.notNow, "data-go=next")}${btn(T.next, "data-go=next", "pri lg")}`];
}

function switchesScreen(): [string, string] {
  const t = T.switches;
  const link = ([label, url]: string[]) => `<button type="button" class="lnk" data-url="${url}">${label}</button>`;
  const tog = (k: "autostart" | "widget" | "morning" | "wrap", label: string) =>
    `<button class="tog" role="switch" aria-checked="${S.sw[k]}" aria-label="${label}" data-sw="${k}" ${S.busy ? "disabled" : ""}></button>`;
  const row = (k: "autostart" | "widget" | "morning" | "wrap", ic: IconName, [name, small]: string[], extra = "") =>
    `<div class="srow"><span class="ic">${icon(ic)}</span><div>${name}<small>${small}</small></div>${extra}${tog(k, name)}</div>`;
  const times: string[] = [];
  for (let h = 16; h <= 22; h++) times.push(`${h}:00`, `${h}:30`);
  if (!times.includes(S.sw.wrap_time)) times.push(S.sw.wrap_time);
  times.sort();
  const sel = `<select class="field" id="ob-wrap" aria-label="${t.wrapTime}" ${S.sw.wrap && !S.busy ? "" : "disabled"}>${times.map((x) => `<option ${x === S.sw.wrap_time ? "selected" : ""}>${x}</option>`).join("")}</select>`;
  const body = `<div><h3 id="ob-title">${t.title}</h3><p class="s">${t.sub}</p></div>
    <div class="srows">${row("autostart", "power", t.autostart)}${row("widget", "layout-panel-top", t.widget)}${row("morning", "sunrise", t.morning)}${row("wrap", "sunset", t.wrap, sel)}</div>
    ${S.msg ? banner(S.msg) : ""}
    <p class="agree">${t.agree}${link(t.terms)}${t.and}${link(t.privacy)}</p>`;
  const go = S.busy ? `<button class="btn pri lg" disabled><span class="spin"></span>${T.start}</button>` : btn(T.start, "data-go=start", "pri lg");
  return [body, `${btn(T.back, "data-go=back", "ghost")}<span class="sp"></span>${go}`];
}

const SCREENS: Record<Screen, () => [string, string]> = { hello, sync: syncScreen, restore: restoreScreen, try: tryScreen, switches: switchesScreen };

let shown = false;
let drawn: Screen | null = null;

function render() {
  const [body, ft] = SCREENS[S.screen]();
  const fresh = drawn !== S.screen;
  drawn = S.screen;
  // 重画时正在输入的框不丢焦点（比如网盘列表找好了）
  const focused = fresh ? "" : (document.activeElement as HTMLElement | null)?.id;
  card.innerHTML =`<div class="hd" data-tauri-drag-region>${dots(STEP[S.screen])}<span class="sp" data-tauri-drag-region></span><button class="skip" data-go="skip" ${S.busy ? "disabled" : ""}>${T.skip}</button></div>
    <div class="bd" ${fresh ? "" : 'style="animation:none"'}>${body}</div><div class="ft">${ft}</div>`;
  if (focused) document.getElementById(focused)?.focus();
  requestAnimationFrame(async () => {
    await api.fitWindow(card.offsetHeight + 28).catch(() => {});
    if (!shown) {
      shown = true;
      await win.show();
      await win.setFocus();
    }
  });
}

function go(screen: Screen) {
  // 离开加入同步这一屏（上一步、加入成功）：不再等手机
  if (screen !== "sync") stopPhone();
  S.screen = screen;
  S.msg = null;
  render();
  if (screen === "try") void loadInfo();
}

const fail = (e: unknown, title = T.fail): Note => ({ kind: "err", title, text: errText(e) });

async function loadInfo() {
  S.info = await api.onboardInfo().catch(() => S.info);
  if (S.screen === "try") render();
}

// ---------- 第 1 步的几条路 ----------

/** 放「叮咪怎么用」：按这台电脑上实际设的快捷键写。放不进去也不耽误往下走 */
async function placeNote() {
  try {
    const s = await api.getSettings();
    const n = guideNote({ flash: s.hk_flash, qnote: s.hk_qnote, main: s.hk_main, focus: s.hk_focus, snooze: s.snooze_min, ring: s.ring_every_min });
    await api.onboardGuideNote(n.title, n.body);
  } catch {
    // 没放上就算了，引导照常走
  }
}

async function startSync() {
  go("sync");
  const st = await api.syncStatus().catch(() => null);
  S.name ||= st?.device || st?.default_name || "";
  if (S.cands === undefined) S.cands = await api.syncFolderCandidates().catch(() => []);
  if (S.screen === "sync") render();
}

/** 选了一个文件夹：要里面已经有叮咪的数据（另一台电脑在用）才能加入 */
async function useFolder(path: string) {
  S.msg = null;
  try {
    const p = await api.syncFolderProbe(path);
    if (p.state === "bad") S.msg = { kind: "err", title: T.fail, text: p.error ?? T.sync.bad };
    else if (p.state === "empty") S.msg = { kind: "warn", title: T.sync.empty, text: T.sync.emptyPick };
    else S.probe = p;
  } catch (e) {
    S.msg = fail(e);
  }
  render();
  if (S.probe?.state === "space") $<HTMLInputElement>("#ob-pw")?.focus();
}

async function join() {
  const t = T.sync;
  S.msg = null;
  const need = (text: string) => {
    S.msg = { kind: "err", title: T.fail, text };
    render();
  };
  if (S.mode === "official" && S.osub === "rec") return recover();
  if (S.mode === "official" && (S.osub === "code" || S.osub === "phonepw")) return joinOfficial();
  const byCode = S.mode !== "folder";
  if (!byCode && !S.probe) return need(t.needFolder);
  if (byCode && !S.code.trim()) return need(t.needCode);
  if (!S.pw) return need(t.needPw);
  if (byCode && [...S.pw].length < 8) return need(t.pwShort);
  S.busy = true;
  render();
  try {
    let recovery: string | null = null;
    if (S.mode === "folder") await api.syncFolderStart(S.probe!.dir, S.pw, S.name, false);
    else recovery = await api.syncPair(S.code.trim(), S.pw, S.name, false);
    S.pw = "";
    S.syncOn = true;
    S.after = recovery
      ? { kind: "warn", title: t.recovery, text: t.recoverySub, code: recovery }
      : { kind: "ok", title: t.joined, text: t.joinedSub };
    S.busy = false;
    go("try");
  } catch (e) {
    S.busy = false;
    S.msg = fail(e);
    render();
  }
}

/** 叮咪同步：兑换码 + 同步密码加入，或者手机允许以后输同步密码（需求-1.x 第 34 节） */
async function joinOfficial() {
  const t = T.sync;
  const need = (text: string) => {
    S.msg = { kind: "err", title: T.fail, text };
    render();
  };
  if (S.osub === "code" && !S.redeem.trim()) return need(t.needRedeem);
  if (!S.pw) return need(t.needPw);
  S.busy = true;
  render();
  try {
    if (S.osub === "code") await api.officialJoin(S.redeem.trim(), S.pw, S.name, false);
    else await api.officialPhoneJoin(S.pw, S.name, false);
    S.pw = "";
    S.syncOn = true;
    S.after = { kind: "ok", title: t.joined, text: t.joinedSub };
    S.busy = false;
    go("try");
  } catch (e) {
    S.busy = false;
    S.msg = fail(e);
    render();
  }
}

/** 用手机加入：要数字、等手机允许（一次最多等 20 秒，隔一会儿接着再等，过了有效期就停）；
 *  换到别的方式、换成网盘或自建服务器、退回上一步都不等了（stopPhone） */
async function startPhone() {
  S.osub = "phone";
  S.wait = null;
  S.gone = "";
  S.msg = null;
  render();
  const mine = {};
  phoneRun = mine;
  const alive = () => phoneRun === mine && S.screen === "sync" && S.mode === "official" && S.osub === "phone";
  try {
    const w = await api.officialPhoneStart(S.name);
    if (!alive()) return;
    S.wait = w;
  } catch (e) {
    if (!alive()) return;
    S.gone = errText(e);
  }
  render();
  if (!S.wait || S.gone) return;
  try {
    const end = await waitPhone(S.wait.expires_at, () => alive() && !S.gone, api.officialPhonePoll);
    if (!alive()) return;
    if (end === "ok") {
      S.osub = "phonepw";
      render();
      $<HTMLInputElement>("#ob-pw")?.focus();
    } else if (end === "expired") {
      S.gone = T.sync.phoneOver;
      render();
    }
  } catch (e) {
    if (!alive()) return;
    S.gone = errText(e);
    render();
  }
}
let phoneRun: object | null = null;

/** 不等手机了：这一轮作废（后台不再问服务器，手机那边允许了也不跳过来）；再回到叮咪同步时重新选方式 */
function stopPhone() {
  phoneRun = null;
  if (S.osub === "phone") S.osub = "ways";
}

/** 叮咪同步：用恢复码找回。成了以后建议换一个新的恢复码（在「设置 → 同步」里） */
async function recover() {
  const t = T.sync;
  const need = (text: string) => {
    S.msg = { kind: "err", title: T.fail, text };
    render();
  };
  if (!S.rc.trim()) return need(t.needRc);
  if (S.newPw && [...S.newPw].length < 8) return need(t.pwShort);
  S.busy = true;
  render();
  try {
    await api.officialRecover(S.rc.trim(), S.name, S.newPw);
    S.rc = "";
    S.newPw = "";
    S.syncOn = true;
    S.after = { kind: "ok", title: t.recovered, text: t.recoveredSub };
    S.busy = false;
    go("try");
  } catch (e) {
    S.busy = false;
    S.msg = fail(e);
    render();
  }
}

async function pickPackage() {
  S.picking = true;
  S.msg = null;
  render();
  const path = await api.restorePick().catch(() => null);
  S.picking = false;
  if (path) {
    try {
      S.pkg = { path, peek: await api.restorePeek(path) };
    } catch (e) {
      S.pkg = null;
      S.msg = fail(e);
    }
  }
  if (S.screen === "restore") render();
}

async function restore() {
  if (!S.pkg) return;
  S.busy = true;
  S.msg = null;
  render();
  try {
    const r = await api.restoreApply(S.pkg.path, false);
    const n = r.reminders + r.notes + r.attachments;
    S.after = { kind: "ok", title: T.restore.done, text: n ? T.restore.doneSub(r.reminders, r.notes, r.attachments) : T.restore.nothingSub };
    S.busy = false;
    S.pkg = null;
    go("try");
  } catch (e) {
    S.busy = false;
    S.msg = fail(e);
    render();
  }
}

// ---------- 走完 ----------

async function loadSwitches() {
  const s = await api.getSettings().catch(() => null);
  if (s) S.sw = { autostart: s.autostart, widget: s.widget, morning: s.morning, wrap: s.wrap, wrap_time: s.wrap_time || "18:00" };
}

const same = (s: Settings) => (Object.keys(S.sw) as (keyof typeof S.sw)[]).every((k) => s[k] === S.sw[k]);

/** 「同意并开始」：存好开关（没改的不存，免得把默认值当成改动同步出去），关掉引导，打开主窗口的「今天」 */
async function start() {
  S.busy = true;
  S.msg = null;
  render();
  try {
    const cur = await api.getSettings();
    if (!same(cur)) await api.saveSettings({ ...cur, ...S.sw });
  } catch (e) {
    // 快捷键登记不上时也会报错，但设置已经存好了：存好了就照常往下走
    const now = await api.getSettings().catch(() => null);
    if (!now || !same(now)) {
      S.busy = false;
      S.msg = { kind: "err", title: T.switches.saveFail, text: T.switches.saveFailSub(errText(e)) };
      return render();
    }
  }
  await finish();
}

let leaving = false;

async function finish() {
  await api.onboardDone().catch(() => {});
  await api.openMainNav("today").catch(() => {});
  leaving = true;
  await win.close();
}

// Alt + F4 关掉：当成「跳过」，不然什么窗口都没有，像是叮咪没打开
win.onCloseRequested((e) => {
  if (leaving) return;
  e.preventDefault();
  if (!S.busy) void skip();
});

/** 「跳过」：后面的都用默认值。还在第 1 步、选的是「第一次用」的话照样放说明笔记 */
async function skip() {
  S.busy = true;
  render();
  if (S.screen === "hello" && S.choice === "first" && S.info?.first) await placeNote();
  await finish();
}

// ---------- 点击、输入 ----------

card.addEventListener("click", async (e) => {
  const el = e.target as HTMLElement;
  if (S.busy) return;
  const choice = el.closest<HTMLElement>("[data-choice]");
  if (choice) {
    S.choice = choice.dataset.choice as Choice;
    // 只换选中的样子，不重画（重画会让团子猫再挥一次手）
    card.querySelectorAll<HTMLElement>("[data-choice]").forEach((o) => {
      const on = o === choice;
      o.classList.toggle("on", on);
      o.setAttribute("aria-pressed", String(on));
      o.querySelector(".radio")?.classList.toggle("on", on);
    });
    return;
  }
  // 最后一步那行小字里的《用户协议》《隐私说明》：用浏览器打开，引导还开着
  const url = el.closest<HTMLElement>("[data-url]")?.dataset.url;
  if (url) {
    try {
      await api.openUrl(url);
    } catch (err) {
      S.msg = fail(err);
      render();
    }
    return;
  }
  // 快捷键被占时的「去设置」：主窗口直接跳到 设置 → 通用 → 快捷框那一行，引导还开着
  if (el.closest("[data-goset]")) return void api.openMainNav("settings", undefined, undefined, "general", "hk_flash").catch(() => {});
  const copy = el.closest<HTMLElement>("[data-copy]");
  if (copy) {
    await copyText(copy.dataset.copy ?? "");
    if (S.screen === "try" && copy.dataset.copy === T.try.sample) {
      S.copied = true;
      render();
    }
    return;
  }
  // 根元素上有 look.ts 设的 data-mode（深浅色），这里换个名字免得 closest 找到它
  const way = el.closest<HTMLElement>("[data-way]");
  if (way) {
    S.mode = way.dataset.way as typeof S.mode;
    S.msg = null;
    // 换成网盘、自建服务器：不再等手机
    if (S.mode !== "official") stopPhone();
    return render();
  }
  const osub = el.closest<HTMLElement>("[data-osub]")?.dataset.osub;
  if (osub) {
    S.msg = null;
    if (osub === "phone") return startPhone();
    phoneRun = null;
    S.osub = osub as typeof S.osub;
    render();
    $<HTMLInputElement>(osub === "code" ? "#ob-redeem" : osub === "rec" ? "#ob-rc" : "#ob-code")?.focus();
    return;
  }
  const folder = el.closest<HTMLElement>("[data-folder]");
  if (folder) return useFolder(folder.dataset.folder ?? "");
  const sw = el.closest<HTMLElement>("[data-sw]");
  if (sw) {
    const k = sw.dataset.sw as "autostart" | "widget" | "morning" | "wrap";
    S.sw[k] = !S.sw[k];
    sw.setAttribute("aria-checked", String(S.sw[k]));
    if (k === "wrap") $<HTMLSelectElement>("#ob-wrap").disabled = !S.sw.wrap;
    return;
  }
  const act = el.closest<HTMLElement>("[data-go]")?.dataset.go;
  if (!act) return;
  if (act === "skip") return skip();
  if (act === "phone-again") return startPhone();
  if (act === "next") {
    if (S.screen === "hello") {
      // 刚加入同步、又退回第 1 步的：不用再加入一次
      if (S.choice === "sync") return S.syncOn ? go("try") : startSync();
      if (S.choice === "restore") {
        go("restore");
        return pickPackage();
      }
      S.busy = true;
      render();
      await placeNote();
      S.busy = false;
      return go("try");
    }
    if (S.screen === "try") {
      await loadSwitches();
      return go("switches");
    }
  }
  if (act === "back") return go(S.screen === "switches" ? "try" : "hello");
  if (act === "other") {
    const path = await api.pickFolder(T.sync.pickTitle).catch(() => null);
    if (path) await useFolder(path);
    return;
  }
  if (act === "join") return join();
  if (act === "pick") return pickPackage();
  if (act === "restore") return restore();
  if (act === "start") return start();
});

card.addEventListener("input", (e) => {
  const el = e.target as HTMLInputElement;
  if (el.id === "ob-pw") S.pw = el.value;
  else if (el.id === "ob-code") S.code = el.value;
  else if (el.id === "ob-rc") S.rc = el.value;
  else if (el.id === "ob-redeem") S.redeem = el.value;
  else if (el.id === "ob-newpw") S.newPw = el.value;
  else if (el.id === "ob-name") S.name = el.value;
});

card.addEventListener("change", (e) => {
  const el = e.target as HTMLSelectElement;
  if (el.id === "ob-wrap") S.sw.wrap_time = el.value;
});

card.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && S.screen === "sync" && (e.target as HTMLElement).matches("input")) {
    e.preventDefault();
    if (!S.busy) void join();
  }
});

// 快捷框里记下了一条：第 2 步打勾。一句话拆成提醒和笔记时，说提醒的那句
listen<OnboardTried>("onboard-tried", (e) => {
  if (S.tried?.kind === "reminder" && e.payload.kind === "note") return;
  S.tried = e.payload;
  if (S.screen === "try") render();
});

// 团子猫开关换了：第 1 步顶上的猫和那两句跟着换
listen<Settings>("settings-changed", () => {
  if (S.screen === "hello" && !S.busy) setTimeout(render);
});

(async () => {
  const [info, sync] = await Promise.all([api.onboardInfo().catch(() => null), api.syncStatus().catch(() => null)]);
  S.info = info;
  S.syncOn = !!sync && sync.state !== "off";
  render();
})();
