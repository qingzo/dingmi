// 设置 → 同步（UI设计.md U9 的 9 个同步画面）：选方式、选网盘文件夹、第一台 / 加入、配对服务器、开着同步、出问题、锁着；
// 换同步密码、恢复码、备份、手机。流程和后台调用跟 0.1 一样，只换了样子和字。
// 叮咪同步（需求-1.0 第 18、30、31 节，u9-settings 的「官方同步 · 开通 / 已开通」）：填兑换码 + 设备名开通 → 设同步密码 →
// 保存恢复码；已开通时的套餐、到期、容量、续费、联系客服；到期、删了、设备满了的提示；用恢复码找回。
// 新电脑方便加入（需求-1.x 第 34 节、U18）：开通前先问「第一次开通 / 已经开通过」，已经开通过的四种加法（兑换码 + 同步密码、
// 用手机、配对码、恢复码）；只有一台时的提示、老空间补开、兑换码加入的开关、删掉服务器上的数据（第 35 节）
import qrcode from "qrcode-generator";
import { api, errText, type BackupStatus, type BetaInfo, type FolderCandidate, type FolderProbe, type PhoneLink, type SyncAccount, type SyncStatus } from "../lib/api";
import { cat } from "@shared/mascot";
import { mascotOn } from "../lib/look";
import { emailOk, fmtSize, monthDay, planName, redeemProblem, reformatRedeem, usedPct, waitPhone } from "../lib/official";
import { icon, type IconName } from "@shared/icons";
import { $, $$, addDays, ago, dayStr, esc, toast } from "@shared/util";
import { armed, hideModal, S, type SyncFlow } from "./main-state";
import { busy, dialog, h5, head, note, row, rows, tog } from "./settings-ui";
import { ROWS, T, TS } from "../text/settings";

const R = ROWS.sync;

/** 开通时填了用过的码、那个空间能用兑换码加入：后台报错前面带着这个（sync::JOINABLE_MARK） */
const JOINABLE = "joinable:";

/** 同步出问题（还差同步密码、版本太旧、找不到同步文件夹、叮咪同步到期了 / 数据删了）：导航里「同步」右边一个小红点，
 *  和托盘一样（需求-0.2 第 7.2 节） */
export const syncBad = (s: SyncStatus | null) =>
  !!s && (s.state === "locked" || s.state === "outdated" || s.state === "expired" || s.state === "keys_lost" || (s.state === "offline" && (s.kind === "folder" || s.refused === "deleted")));

/** 叮咪同步该提醒快到期了（7 天内、明天） */
export const expiringSoon = (a: SyncAccount | null | undefined) => !!a && !a.expired && (a.remind === "expire_7d" || a.remind === "expire_1d");

/** 「 · 到 2027-11-06」/「 · 不会到期」 */
const untilText = (a: SyncAccount) => (a.expires_at ? TS.until(a.expires_at) : TS.forever);

/** 同步页：主要那块、手机、备份分开画，免得互相重画时清掉正在填的东西 */
export const syncPageHtml = () => `<div id="sync-box"></div><div id="phone-sec"></div><div id="backup-box"></div>`;

function syncMsg(text: string, good = false) {
  const m = document.querySelector<HTMLElement>("#sync-msg");
  if (!m) return void (text && toast(text));
  m.className = `fhelp ${good ? "ok" : "err"}`;
  m.textContent = text;
}
const msgLine = `<p class="fhelp err" id="sync-msg"></p>`;

/** 面包屑：同步 › 网盘文件夹 › 第一台电脑；前面几段点了退回去 */
const crumb = (...parts: [string, string?][]) =>
  `<div class="crumb">${parts.map(([t, back], i) => (i ? icon("chevron-right") : "") + (back ? `<button data-crumb="${back}">${esc(t)}</button>` : `<span>${esc(t)}</span>`)).join("")}</div>`;

const acts = (go: string) => `<div class="acts"><button class="btn" data-sync-back>${T.back}</button>${go}</div>`;

// ---------- 画 ----------

export function renderSync() {
  const box = document.querySelector<HTMLElement>("#sync-box");
  const s = S.sync;
  if (!box || !s) return;
  const flow = S.syncFlow ?? (s.state === "off" ? { step: "choose" as const } : null);
  box.innerHTML = flow
    ? flowHtml(flow, s)
    : s.state === "keys_lost" || s.state === "keys_retry"
      ? keysHtml(s)
      : s.state === "locked"
      ? s.need_setup ? setupHtml(s.account) : lockedHtml(s)
      : s.kind === "folder" ? folderOnHtml(s) : s.official ? officialOnHtml(s) : serverOnHtml(s);
  if (!flow && s.kind === "folder" && s.state !== "locked") void loadFolderDevices();
  renderPhone();
  void renderBackup();
}

/** 「这台电脑已经有 12 条提醒…」：加入别人的同步时选合并还是替换 */
function mergeHtml(replace: string[]) {
  if (!(S.reminders.length + S.notes.length)) return "";
  const opt = (mode: string, [name, sub]: string[], on: boolean) =>
    `<button class="choice" data-mode="${mode}" aria-pressed="${on}"><span class="radio${on ? " on" : ""}"></span><div>${esc(name)}<small>${esc(sub)}</small></div></button>`;
  return `<h5>${esc(TS.mergeTitle(S.reminders.length, S.notes.length))}</h5><div class="srows" id="sy-mode">${opt("merge", TS.merge, true)}${opt("replace", replace, false)}</div>`;
}

const pwField = (id: string, auto: string, meter = false) =>
  `<label class="field"><input type="password" id="${id}" autocomplete="${auto}" aria-label="${TS.pw}">${meter ? `<span class="pwm" id="${id}-m"></span>` : ""}</label>`;
const codeRow = () =>
  row("code", { name: TS.code, sub: TS.codeSub }, `<input class="field mono" id="sy-code" placeholder="${TS.codePh}" spellcheck="false" aria-label="${TS.code}">`);
const nameRow = (s: SyncStatus) =>
  row("name", { name: TS.name, sub: TS.nameSub }, `<input class="field" id="sy-name" value="${esc(s.device || s.default_name)}" aria-label="${TS.name}">`);

function flowHtml(f: SyncFlow, s: SyncStatus): string {
  if (f.step === "choose") {
    const way = ([k, ic, name, text, foot]: string[]) => {
      const card = `<button class="way" data-sync-pick="${k}">${icon(ic as IconName, "s20")}<b>${name}</b><span>${text}</span><em>${foot}</em></button>`;
      // 叮咪同步：卡片里再放一个「去买兑换码」（按钮里不能套按钮，压在卡片下边）
      return k === "official" ? `<div class="way-box" data-row="official">${card}<button class="lnk way-buy" data-buy>${icon("external-link")}${TS.buy}</button></div>` : card;
    };
    return `${head(TS.crumb, T.hint.sync as string)}
      ${s.state !== "off" ? note("soft", "info", "", esc(TS.switching)) : ""}
      <div class="ways" data-row="way">${TS.ways.map(way).join("")}</div>
      ${note("soft", "smartphone", "", esc(TS.phoneNeedServer))}
      ${s.state !== "off" ? `<div class="acts"><button class="btn" data-sync-back>${T.back}</button></div>` : ""}${msgLine}`;
  }
  if (f.step === "pick") {
    const cands = f.cands;
    const list =
      cands === undefined
        ? row("finding", { name: TS.finding })
        : cands.length
          ? cands.map(candRow).join("")
          : row("none", { name: TS.noDrive, sub: TS.noDriveSub });
    return `${crumb([TS.crumb, "choose"], [TS.folder])}${head(TS.pickTitle, TS.pickHint)}
      ${rows(
        list,
        row(
          "other",
          { name: TS.other, sub: TS.otherSub },
          `<span class="keyw"><input class="field" id="sy-dir" placeholder="${esc(TS.otherPh)}" spellcheck="false" aria-label="${TS.other}"><button class="btn sm" id="sy-dir-go">${TS.useThis}</button><button class="btn sm" id="sy-dir-pick">${TS.choose}</button></span>`,
        ),
      )}
      ${note("warn", "triangle-alert", "", esc(TS.nutWarn))}${msgLine}${acts("")}`;
  }
  if (f.step === "first" || f.step === "join") {
    const p = f.probe!;
    const first = f.step === "first";
    return `${crumb([TS.crumb, "choose"], [TS.folder, "pick"], [first ? TS.first : TS.join])}
      ${head(first ? TS.firstTitle : TS.joinTitle, first ? TS.firstHint(p.dir) : TS.joinHint(p.devices))}
      ${rows(
        row("pw", { name: TS.pw, sub: first ? TS.pwSub : "" }, pwField("sy-pw", first ? "new-password" : "current-password", first)),
        first ? row("pw2", { name: TS.pw2 }, pwField("sy-pw2", "new-password")) : "",
        nameRow(s),
      )}
      ${first ? "" : mergeHtml(TS.replaceFolder)}
      ${first ? note("warn", "key-round", TS.forgetTitle, esc(TS.forget)) : ""}
      ${msgLine}${acts(`<button class="btn pri" id="sy-folder-go">${first ? TS.start : TS.join}</button>`)}`;
  }
  if (f.step === "official") return officialHtml(s);
  if (f.step === "fork") return forkHtml(s);
  if (f.step === "ways") return waysHtml(s);
  if (f.step === "ojoin") return ojoinHtml(s, f.code ?? "");
  if (f.step === "phone") return phoneHtml(f);
  if (f.step === "phonepw") return phonePwHtml(s);
  if (f.step === "setup") return setupHtml(f.account ?? s.account);
  if (f.step === "recover") return recoverHtml(s);
  if (f.step === "pair") return pairHtml(s);
  // 自建服务器：粘贴配对码
  return `${crumb([TS.crumb, "choose"], [TS.server])}${head(TS.serverTitle, TS.serverHint)}
    ${rows(
      codeRow(),
      nameRow(s),
      row("pw", { name: TS.pw, sub: TS.pwSub }, pwField("sy-pw", "new-password", true)),
      row("pw2", { name: TS.pw2 }, pwField("sy-pw2", "new-password")),
    )}
    ${mergeHtml(TS.replaceServer)}
    ${note("warn", "key-round", TS.forgetTitle, esc(TS.forget))}
    ${msgLine}${acts(`<button class="btn pri" id="sy-pair">${TS.start}</button>`)}`;
}

/** 网盘的小方块：坚果云绿、OneDrive 蓝……认不出的灰色 */
function drive(label: string) {
  const [c, ch] =
    /坚果/.test(label) ? ["green", "坚"]
    : /onedrive/i.test(label) ? ["blue", "O"]
    : /百度/.test(label) ? ["blue", "百"]
    : /dropbox/i.test(label) ? ["teal", "D"]
    : /icloud/i.test(label) ? ["gray", "i"]
    : ["gray", [...label][0] ?? "?"];
  return `<span class="drive" style="--c:var(--tc-${c})">${esc(ch)}</span>`;
}

function candRow(c: FolderCandidate) {
  const p = c.probe;
  const [cls, st] = p.state === "space" ? ["ok", TS.hasData(p.devices)] : p.state === "empty" ? ["", TS.noData] : ["bad", p.error ?? TS.cantUse];
  return `<button class="srow drow" data-folder="${esc(c.path)}" ${p.state === "bad" ? "disabled" : ""}>
    ${drive(c.label)}<div>${esc(c.label)}<small class="path">${esc(c.path)}</small></div><span class="ago ${cls}">${esc(st)}</span>${icon("chevron-right", "s16")}</button>`;
}

// ---------- 叮咪同步：开通、设同步密码、用恢复码找回 ----------

/** 这台电脑上已经有提醒、笔记时说一句：开通 / 找回后会传上去、合在一起 */
const localNote = (text: (r: number, n: number) => string) =>
  S.reminders.length + S.notes.length ? note("soft", "info", "", esc(text(S.reminders.length, S.notes.length))) : "";

/** 兑换码输入框；下面一行放出错的原话（兑换码不存在、用过了、输错太多次……） */
const redeemField = (id: string) =>
  `<span class="redeem-w"><input class="field mono" id="${id}" data-redeem placeholder="${TS.redeemPh}" spellcheck="false" autocomplete="off" aria-label="${TS.redeem}"><span class="fhelp err" id="${id}-msg" role="alert"></span></span>`;

/** 选填的邮箱框（开通时、改邮箱的对话框） */
const emailField = (id: string, value = "") =>
  `<input class="field" id="${id}" type="email" value="${esc(value)}" placeholder="${TS.emailPh}" spellcheck="false" autocomplete="email" aria-label="${TS.email}">`;

/** 开通前的「我已阅读并同意《…》《…》《…》，知道数据存放在香港的服务器上」（32.1 第 7 条）：没勾点不了开通 */
const agreeHtml = () =>
  `<label class="agree" data-row="agree"><input type="checkbox" id="sy-agree"><span>${esc(TS.agreePre)}${TS.agreeDocs
    .map(([name, url]) => `<button type="button" class="lnk" data-doc="${esc(url)}">${esc(name)}</button>`)
    .join("")}${esc(TS.agreePost)}</span></label>`;

/** 公测免费体验那一块（32.1 第 4 条）：服务器开着、有名额、没过截止日才有；名额满了、关了就不显示 */
function betaHtml(b: BetaInfo | null | undefined) {
  if (!b?.open || !b.until) return "";
  const size = b.quota ? fmtSize(b.quota) : TS.noLimit;
  return note(
    "ok",
    "sparkles",
    esc(TS.betaTitle(b.left, monthDay(b.until))),
    esc(TS.betaSub(planName(b.plan), size, b.max_devices)),
    `<button class="btn sm pri" id="sy-beta-go" data-needs-agree disabled>${TS.betaGo}</button>`,
  );
}

/** 开通那一步的公测情况：进来时问一次服务器（连不上就不显示），问到了只重画那一块，不清掉正在填的 */
async function loadBeta() {
  const b = await api.officialBetaInfo().catch(() => null);
  const f = S.syncFlow;
  if (!f || f.step !== "official") return;
  f.beta = b;
  const box = document.querySelector<HTMLElement>("#sy-beta");
  if (box) box.innerHTML = betaHtml(b);
  agreeChanged();
}

/** 勾没勾「同意」：开通、免费体验按钮跟着能不能点 */
function agreeChanged() {
  const on = document.querySelector<HTMLInputElement>("#sy-agree")?.checked ?? false;
  $$<HTMLButtonElement>("#sync-box [data-needs-agree]").forEach((b) => (b.disabled = !on));
}

/** 第 1 步：兑换码 + 这台电脑的名字 +（选填）邮箱 + 勾同意 → 开通（需求-1.0 第 31.4 节 B 的第 7 条、32.1 第 4、6、7 条）；
 *  公测开着的话多一个「免费体验」，走一样的后续（设同步密码 → 存恢复码），只是不填兑换码 */
function officialHtml(s: SyncStatus) {
  const again = s.refused === "deleted";
  return `${crumb([TS.crumb, "choose"], [TS.official, "fork"], [TS.firstStep])}${head(TS.openTitle, TS.openHint)}
    ${again ? note("warn", "info", TS.deletedTitle, esc(s.error || TS.deletedText)) : s.state !== "off" ? note("soft", "info", "", esc(TS.switching)) : ""}
    ${again ? "" : note("soft", "laptop", "", `<button class="lnk" data-sync-pick="ways">${TS.firstLink}</button>`)}
    ${rows(
      row("redeem", { name: TS.redeem }, redeemField("sy-redeem"), { sub: `${esc(TS.redeemSub)}<button class="lnk" data-buy>${TS.buy}</button>` }),
      nameRow(s),
      row("email", { name: `${TS.email}（${TS.emailOpt}）`, sub: TS.emailSub }, `<span class="redeem-w">${emailField("sy-email")}<span class="fhelp err" id="sy-email-msg" role="alert"></span></span>`),
    )}
    ${agreeHtml()}
    <div id="sy-beta">${betaHtml(S.syncFlow?.beta)}</div>
    ${localNote(TS.openLocal)}
    <p class="hint trial">${esc(TS.trial)}</p>
    ${msgLine}${acts(`<button class="btn pri" id="sy-redeem-go" data-needs-agree disabled>${TS.openGo}</button>`)}`;
}

// ---------- 新电脑方便加入（需求-1.x 第 34 节、U18） ----------

/** 进叮咪同步先问一句：第一次开通 / 已经在别的电脑上开通过（再开通一次就是另一个空间） */
function forkHtml(s: SyncStatus) {
  const card = (pick: string, ic: IconName, [name, text]: [string, string]) =>
    `<button class="way" data-sync-pick="${pick}">${icon(ic, "s20")}<b>${esc(name)}</b><span>${esc(text)}</span></button>`;
  return `${crumb([TS.crumb, "choose"], [TS.official])}${head(TS.official, TS.forkHint)}
    ${s.state !== "off" ? note("soft", "info", "", esc(TS.switching)) : ""}
    <div class="ways fork" data-row="official">${card("first", "sparkles", TS.forkFirst)}${card("ways", "monitor-smartphone", TS.forkJoin)}</div>
    ${note("soft", "info", "", esc(TS.forkNote))}${msgLine}${acts("")}`;
}

/** 已经开通过：四种加法，按方便程度排 */
function waysHtml(s: SyncStatus) {
  const opt = ([pick, ic, name, text, rowId]: [string, string, string, string, string], i: number) =>
    `<button class="choice opt${i ? "" : " rec"}" data-sync-pick="${pick}" data-row="${rowId}"><span class="opt-ic">${icon(ic as IconName)}</span><div>${esc(name)}${i ? "" : `<em class="tagr">${TS.waysBest}</em>`}<small>${esc(text)}</small></div>${icon("chevron-right")}</button>`;
  return `${crumb([TS.crumb, "choose"], [TS.official, "fork"], [TS.waysStep])}${head(TS.waysTitle, TS.waysHint)}
    ${s.state !== "off" ? note("soft", "info", "", esc(TS.switching)) : ""}
    <div class="srows opts">${TS.joinWays.map(opt).join("")}</div>${msgLine}${acts("")}`;
}

/** 兑换码 + 同步密码加入 */
function ojoinHtml(s: SyncStatus, code: string) {
  return `${crumb([TS.crumb, "choose"], [TS.official, "fork"], [TS.waysStep, "ways"], [TS.ojoinStep])}${head(TS.ojoinTitle, TS.ojoinHint)}
    ${rows(
      row("redeem", { name: TS.redeem, sub: TS.ojoinCodeSub }, redeemField("sy-jcode")),
      row("pw", { name: TS.pw, sub: TS.ojoinPwSub }, pwField("sy-pw", "current-password")),
      nameRow(s),
    )}
    ${mergeHtml(TS.replaceServer)}
    ${msgLine}${acts(`<button class="btn pri" id="sy-ojoin">${TS.join}</button>`)}`.replace(`id="sy-jcode"`, `id="sy-jcode" value="${esc(code)}"`);
}

/** 用手机加入：电脑上显示 8 位数字和 4 个字，等手机允许 */
function phoneHtml(f: SyncFlow) {
  const w = f.wait;
  const digits = (code: string) => `<div class="digits">${[...code].map((c, i) => `${i === 4 ? "<i></i>" : ""}<span>${esc(c)}</span>`).join("")}</div>`;
  const words = (t: string) => `<div class="words">${[...t].map((c) => `<span>${esc(c)}</span>`).join("")}</div>`;
  const body = f.gone
    ? note("warn", "clock", esc(f.gone), "", `<button class="btn sm pri" id="sy-phone-again">${TS.phoneAgain}</button>`)
    : w
      ? `<div class="panel phone-wait">${digits(w.code)}<p>${esc(TS.phoneWords)}</p>${words(w.words)}<span class="wait">${icon("loader-circle")}${esc(TS.phoneWait(w.expires_at.slice(11, 16)))}</span></div>`
      : `<div class="panel phone-wait"><span class="wait">${icon("loader-circle")}${TS.phoneAsking}</span></div>`;
  return `${crumb([TS.crumb, "choose"], [TS.official, "fork"], [TS.waysStep, "ways"], [TS.phoneStep])}${head(TS.phoneTitle, TS.phoneJoinHint)}
    ${body}${note("soft", "info", "", esc(TS.phoneNoPhone))}${msgLine}${acts("")}`;
}

/** 手机允许了：输同步密码 */
function phonePwHtml(s: SyncStatus) {
  return `${crumb([TS.crumb, "choose"], [TS.official, "fork"], [TS.waysStep, "ways"], [TS.phonePwStep])}${head(TS.phonePwTitle, TS.phonePwHint)}
    ${rows(row("pw", { name: TS.pw, sub: TS.ojoinPwSub }, pwField("sy-pw", "current-password")), nameRow(s))}
    ${mergeHtml(TS.replaceServer)}
    ${msgLine}${acts(`<button class="btn pri" id="sy-phone-join">${TS.join}</button>`)}`;
}

/** 要数字、等手机允许（一次最多等 20 秒，隔一会儿接着再等，过了有效期就停）；换到别的步骤就不等了 */
async function startPhone() {
  const s = S.sync;
  const f: SyncFlow = { step: "phone" };
  S.syncFlow = f;
  renderSync();
  try {
    f.wait = await api.officialPhoneStart(s?.device || s?.default_name || "");
  } catch (e) {
    f.gone = errText(e);
  }
  if (S.syncFlow !== f) return;
  renderSync();
  if (!f.wait || f.gone) return;
  try {
    const end = await waitPhone(f.wait.expires_at, () => S.syncFlow === f && !f.gone, api.officialPhonePoll);
    if (S.syncFlow !== f) return;
    if (end === "ok") {
      S.syncFlow = { step: "phonepw" };
      renderSync();
      document.querySelector<HTMLInputElement>("#sy-pw")?.focus();
    } else if (end === "expired") {
      f.gone = TS.phoneOver;
      renderSync();
    }
  } catch (e) {
    if (S.syncFlow !== f) return;
    f.gone = errText(e);
    renderSync();
  }
}

/** 这台电脑已经在另一个叮咪同步空间里（多半是在这台电脑上又开通了一次）：加入别的空间前先说清楚。回 true 是接着加入 */
function leaveSpaceOk(): Promise<boolean> {
  const s = S.sync;
  const a = s?.account;
  if (!s || s.state === "off" || !s.official || !a?.official || s.refused === "deleted") return Promise.resolve(true);
  return new Promise((done) => {
    dialog(
      `<h4>${TS.twoTitle}</h4><p>${esc(TS.twoText(a.days_left))}</p>${note("soft", "headset", "", esc(TS.twoSupport))}
      <div class="acts"><button type="button" class="btn" id="two-no">${TS.twoNo}</button><button type="button" class="btn pri" id="two-go">${TS.twoGo}</button></div>`,
    );
    $("#two-no").onclick = () => {
      hideModal();
      done(false);
    };
    $("#two-go").onclick = () => {
      hideModal();
      done(true);
    };
  });
}

/** 凭据管理器里没有这台电脑的钥匙了（重新加入）/ 这次没读出来（30 秒后再试） */
function keysHtml(s: SyncStatus) {
  if (s.state === "keys_retry") {
    return `${note("warn", "key-round", TS.keysRetryTitle, esc(TS.keysRetryText) + (s.error ? `<br><small>${esc(s.error)}</small>` : "") + diagLink())}
      ${head(s.kind === "folder" ? TS.onFolder : s.official ? TS.onOfficial : TS.onServer)}${moreHtml(s, "", false)}${msgLine}`;
  }
  const go = s.official ? "ways" : "server";
  return `${note("warn", "key-round", TS.keysLostTitle, esc(TS.keysLostText), `<button class="btn sm pri" data-sync-pick="${go}">${TS.keysLostGo}</button>`)}
    ${head(s.official ? TS.onOfficial : TS.onServer)}${moreHtml(s, "", false)}${msgLine}`;
}

/** 已开通、空间里只有这一台：另一台电脑怎么加 */
function tipHtml(a: SyncAccount) {
  const items = [a.join ? TS.tipCode : "", TS.tipPhone, TS.tipPair].filter(Boolean).map((t) => `<li>${esc(t)}</li>`).join("");
  return `<div class="join-tip">${mascotOn() ? `<span class="join-tip-cat">${cat("wave")}</span>` : ""}<div><b>${TS.tipTitle}</b><ul>${items}</ul></div><button class="btn sm" data-tip-invite>${TS.invite}</button></div>`;
}

/** 打开兑换码加入：输一次同步密码 */
function showJoinOn() {
  dialog(
    `<h4>${TS.joinOnTitle}</h4><p>${esc(TS.joinOnText)}</p>
    <label class="fl"><span>${TS.pw}</span>${pwField("jo-pw", "current-password")}</label>
    <p class="fhelp err" id="jo-err"></p>
    <div class="acts"><button type="button" class="btn" data-close>${T.cancel}</button><button class="btn pri" id="jo-go">${TS.joinOnGo}</button></div>`,
    { form: "jo" },
  );
  $<HTMLFormElement>("#jo").onsubmit = async (e) => {
    e.preventDefault();
    const pw = $<HTMLInputElement>("#jo-pw").value;
    const err = await busy($<HTMLButtonElement>("#jo-go"), TS.joinOnBusy, async () => {
      await api.officialJoinOn(pw);
      S.sync = await api.syncStatus().catch(() => S.sync);
    });
    if (err) return void ($("#jo-err").textContent = err);
    hideModal();
    renderSync();
    toast(TS.joinOnOk);
  };
  setTimeout(() => document.querySelector<HTMLInputElement>("#jo-pw")?.focus(), 40);
}

/** 关掉兑换码加入：说清楚得到什么、少了什么 */
function showJoinOff() {
  dialog(
    `<h4>${TS.joinOffTitle}</h4><p>${esc(TS.joinOffText)}</p><p>${esc(TS.joinOffCost)}</p>
    <div class="acts"><button type="button" class="btn" data-close>${TS.joinOffNo}</button><button type="button" class="btn pri" id="jf-go">${TS.joinOffGo}</button></div>`,
  );
  $<HTMLButtonElement>("#jf-go").onclick = async (e) => {
    const err = await busy(e.currentTarget as HTMLButtonElement, TS.joinOffGo, async () => {
      await api.officialJoinOff();
      S.sync = await api.syncStatus().catch(() => S.sync);
    });
    if (err) return toast(err);
    hideModal();
    renderSync();
    toast(TS.joinOffOk);
  };
}

/** 删掉服务器上的数据（需求-1.x 第 35.3 节）：说清楚，输同步密码确认 */
function showDeleteServer() {
  dialog(
    `<h4>${TS.delTitle}</h4><ul class="del-list">${TS.delItems.map((t) => `<li>${esc(t)}</li>`).join("")}</ul>
    <label class="fl"><span>${TS.delPw}</span>${pwField("del-pw", "current-password")}</label>
    <p class="fhelp err" id="del-err"></p>
    <div class="acts"><button type="button" class="btn" data-close>${T.cancel}</button><button class="btn pri danger" id="del-go">${TS.delGo}</button></div>`,
    { form: "del" },
  );
  $<HTMLFormElement>("#del").onsubmit = async (e) => {
    e.preventDefault();
    const pw = $<HTMLInputElement>("#del-pw").value;
    if (!pw) return void ($("#del-err").textContent = TS.needPw);
    const err = await busy($<HTMLButtonElement>("#del-go"), TS.delBusy, async () => {
      await api.officialDelete(pw);
      S.sync = await api.syncStatus().catch(() => S.sync);
    });
    if (err) return void ($("#del-err").textContent = err);
    hideModal();
    S.syncFlow = null;
    renderSync();
    toast(TS.delOk);
  };
}

/** 开通前核对邮箱、同意：不行回给人看的一句（显示在对应的地方），行回 null */
function openCheck(): string | null {
  const email = document.querySelector<HTMLInputElement>("#sy-email")?.value ?? "";
  if (email.trim() && !emailOk(email)) {
    $("#sy-email-msg").textContent = TS.emailBad;
    return TS.emailBad;
  }
  $("#sy-email-msg").textContent = "";
  if (!document.querySelector<HTMLInputElement>("#sy-agree")?.checked) return TS.needAgree;
  return null;
}

/** 第 2 步：开通了，设同步密码（输两次）。兑换码已经用掉了，所以没有「返回」；中途退出的下次进来还是这一步 */
function setupHtml(a: SyncAccount | null | undefined) {
  const fresh = S.syncFlow?.step === "setup";
  const got = a?.official ? note("ok", "badge-check", esc(TS.opened(planName(a.plan), untilText(a))), a.quota ? esc(TS.openedSize(fmtSize(a.quota))) : "") : "";
  return `${crumb([TS.official], [TS.setupStep])}${got}${head(TS.setupTitle, TS.setupHint)}
    ${fresh ? "" : note("soft", "info", "", esc(TS.setupLater))}
    ${rows(
      row("pw", { name: TS.pw, sub: TS.pwSub }, pwField("sy-pw", "new-password", true)),
      row("pw2", { name: TS.pw2 }, pwField("sy-pw2", "new-password")),
    )}
    ${note("soft", "ticket", "", esc(TS.joinPwHint))}
    ${note("warn", "key-round", TS.forgetTitle, esc(TS.forgetOfficial))}
    ${msgLine}<div class="acts"><button class="btn pri" id="sy-setup">${TS.setupGo}</button></div>`;
}

/** 已经在别的电脑上开通了：粘贴那台电脑生成的配对码 + 同一个同步密码（配对码里带着空间密钥，密码只输一次） */
function pairHtml(s: SyncStatus) {
  return `${crumb([TS.crumb, "choose"], [TS.official, "fork"], [TS.waysStep, "ways"], [TS.pairStep])}${head(TS.pairTitle, TS.pairHint)}
    ${s.state !== "off" ? note("soft", "info", "", esc(TS.switching)) : ""}
    ${rows(
      codeRow(),
      nameRow(s),
      row("pw", { name: TS.pw, sub: TS.pairPwSub }, pwField("sy-pw", "current-password")),
    )}
    ${mergeHtml(TS.replaceServer)}
    ${msgLine}${acts(`<button class="btn pri" id="sy-pair">${TS.join}</button>`)}`;
}

/** 用恢复码加入：不用别的电脑开着（家里、公司各自加），唯一的电脑没了也走这里（需求-1.0 第 30.4 节）。
 *  恢复码 + 这台电脑的名字 +（忘了才填）新的同步密码；别的电脑照常同步 */
function recoverHtml(s: SyncStatus) {
  return `${crumb([TS.crumb, "choose"], [TS.official, "fork"], [TS.waysStep, "ways"], [TS.recoverStep])}${head(TS.recoverTitle, TS.recoverHint)}
    ${s.state !== "off" ? note("soft", "info", "", esc(TS.switching)) : ""}
    ${rows(
      row("recovery", { name: TS.recoverRc, sub: TS.recoverRcSub }, `<input class="field mono" id="sy-rc" placeholder="${TS.rcPh}" spellcheck="false" autocomplete="off" aria-label="${TS.rc}">`),
      nameRow(s),
      row("pw", { name: TS.recoverPw, sub: TS.recoverPwSub }, pwField("sy-pw", "new-password", true)),
      row("pw2", { name: TS.pw2 }, pwField("sy-pw2", "new-password")),
    )}
    ${localNote(TS.recoverLocal)}
    ${note("warn", "key-round", "", esc(TS.recoverWarn))}
    ${msgLine}${acts(`<button class="btn pri" id="sy-recover">${TS.recoverGo}</button>`)}`;
}

// ---------- 叮咪同步：已开通 ----------

/** 「续费」按钮（顶上的提示条和账的那一块都有） */
const renewBtn = (cls = "") => `<button class="btn${cls ? ` ${cls}` : ""}" data-renew>${TS.renewBtn}</button>`;

/** 顶上的提示条：数据删了 / 已到期 / 快到期（需求-1.0 第 18.4 节） */
function officialTop(s: SyncStatus) {
  const a = s.account;
  if (s.refused === "deleted") return note("err", "cloud-off", TS.deletedTitle, esc(s.error || TS.deletedText), `<button class="btn sm pri" data-reopen>${TS.reopen}</button>`);
  if (s.state === "expired" || a?.expired) {
    const del = a?.delete_on ?? null;
    const soon = a?.remind === "delete_7d" && del ? `<br>${esc(TS.deleteSoon(del))}` : "";
    return note("err", "cloud-off", TS.expiredTitle, esc(TS.expiredText(del)) + soon, renewBtn("sm pri"));
  }
  if (expiringSoon(a)) return note("warn", "alarm-clock", esc(TS.remind7(a!.days_left ?? 0)), esc(TS.remind7Text(a!.until ?? "")), renewBtn("sm pri"));
  if (s.state === "outdated") return note("warn", "cloud-off", TS.oldTitle, esc(TS.old));
  if (s.state === "offline") return note("warn", "wifi-off", TS.offTitle, esc(TS.off) + diagLink());
  return "";
}

/** 账的那一块：套餐、到期、用了多少（进度条）、几台设备、续费。快到期、快满时变黄，满了、到期了变红 */
function accountHtml(a: SyncAccount, s: SyncStatus) {
  const pct = usedPct(a.used, a.quota);
  const soon = expiringSoon(a);
  const lvl = a.expired || a.storage === "full" ? "err" : a.storage === "near_full" || soon ? "warn" : "";
  const plan = planName(a.plan);
  const line = a.expired ? `${plan} · ${TS.expiredLine(a.delete_on)}` : `${plan}${untilText(a)}${a.days_left !== null ? ` · ${TS.daysLeft(a.days_left)}` : ""}`;
  const next = a.next.length ? TS.thenPlan(a.next.map((n) => TS.thenOne(planName(n.plan), n.until)).join("，")) : "";
  const meter = a.storage === "full" || (pct ?? 0) >= 100 ? "err" : a.storage === "near_full" || (pct ?? 0) >= 90 ? "warn" : "";
  const room = a.storage === "full" ? TS.full : a.storage === "near_full" ? TS.nearFull : "";
  const held = s.held_files ? TS.held(s.held_files) : "";
  // 套餐限单个图片 / 文件多大（32.1 第 5 条）；超了不传的有几个
  const limits = TS.limitsLine(a.max_image ? fmtSize(a.max_image) : null, a.max_file ? fmtSize(a.max_file) : null);
  const here = s.only_here_files ? TS.onlyHere(s.only_here_files, a.max_file ? fmtSize(a.max_file) : null) : "";
  const beta = a.beta && a.until && !a.expired ? TS.betaNote(monthDay(a.until)) : "";
  return `<div class="acct${lvl ? ` ${lvl}` : ""}" data-row="account">
    <div class="acct-top"><div class="acct-main"><small>${esc(line)}</small>
      <div class="acct-big">${esc(fmtSize(a.used))}<span>${esc(a.quota ? TS.usedOf(fmtSize(a.quota)) : ` · ${TS.noLimit}`)}</span></div></div>
      <span data-row="renew">${renewBtn(lvl ? "pri" : "")}</span></div>
    ${pct !== null ? `<div class="meter${meter ? ` ${meter}` : ""}" role="progressbar" aria-valuenow="${Math.round(pct)}" aria-valuemin="0" aria-valuemax="100"><i style="width:${pct.toFixed(1)}%"></i></div>` : ""}
    <div class="acct-foot"><span>${esc(TS.devicesOf(a.devices, a.max_devices))} <button class="lnk" data-see-devices>${TS.seeDevices}</button></span>${next ? `<span>${esc(next)}</span>` : ""}${limits ? `<span>${esc(limits)}</span>` : ""}</div>
    ${beta ? `<p class="acct-warn">${icon("sparkles")}<span>${esc(beta)}</span></p>` : ""}
    ${room || held ? `<p class="acct-warn">${icon(a.storage === "full" ? "circle-alert" : "info")}<span>${esc([room, held].filter(Boolean).join(""))}</span></p>` : ""}
    ${here ? `<p class="acct-warn">${icon("paperclip")}<span>${esc(here)}</span></p>` : ""}
  </div>`;
}

function officialOnHtml(s: SyncStatus) {
  const a = s.account?.official ? s.account : null;
  const deleted = s.refused === "deleted";
  const label = deleted ? TS.deletedTitle : (TS.serverState[s.state] ?? "");
  const dot = s.state === "online" ? (s.error ? "bad" : "ok") : s.state === "connecting" ? "busy" : "bad";
  const sub =
    deleted || s.state === "expired"
      ? ""
      : s.state === "offline"
        ? [s.error, s.last_online && TS.serverLast(ago(s.last_online))].filter(Boolean).join(" · ")
        : s.state === "online" && s.error
          ? s.error
          : s.state === "online" && s.last_online
            ? TS.serverSince(ago(s.last_online))
            : "";
  // 新电脑方便加入：只有这一台时说另一台怎么加；这项上线前开通的（没开过、也没关过）请输一次同步密码补开
  const live = a && !deleted;
  const tip = live && a.devices <= 1 ? tipHtml(a) : "";
  const old = live && a.join === false && !a.join_off && !a.beta
    ? note("warn", "ticket", TS.oldJoinTitle, esc(TS.oldJoinText), `<button class="btn sm pri" data-join-on>${TS.joinOnBtn}</button>`)
    : "";
  const joinRow = live && a.join !== undefined
    ? row("join", R.join, `<button class="tog" role="switch" data-join-toggle aria-checked="${!!a.join}" aria-label="${esc(R.join.name)}"></button>`, { sub: esc(a.join ? TS.joinSubOn : TS.joinSubOff) })
    : "";
  const delRow = live ? row("delete_server", R.delete_server, `<button class="btn sm ghost danger" data-del-server>${TS.delBtn}</button>`) : "";
  return `${officialTop(s)}${tip}${old}${head(TS.onOfficial, TS.serverOnHint).replace("<h4>", '<h4 data-row="official">')}
    ${a ? accountHtml(a, s) : deleted ? "" : note("soft", "info", "", esc(TS.noAccount))}
    ${rows(
      `<div class="srow" data-row="server"><div><span class="sdot ${dot}"></span>${esc(label)}${sub ? `<small>${esc(sub)}</small>` : ""}</div><span class="path">${esc(s.server?.replace(/:\d+$/, "") ?? "")}</span></div>`,
      row("now", { name: TS.thisPc }, `<button class="btn sm" id="sy-now">${TS.syncNow}</button>`, { sub: esc(s.device ?? "") }),
      a?.space
        ? row("space_id", { name: TS.spaceId, sub: TS.spaceIdSub }, `<span class="keyw"><code class="space-id">${esc(a.space)}</code><button class="btn sm ghost icon" id="sy-space-copy" title="${T.copy}" aria-label="${T.copy}">${icon("copy")}</button></span>`)
        : "",
      deleted ? "" : row("support", { name: TS.support, sub: TS.supportSub }, `<button class="btn sm" id="sy-support">${TS.supportBtn}</button>`),
      // 通知邮箱（32.1 第 6 条）：协议 6 的服务器才有（account 里带 email 这一项）
      a && !deleted && a.email !== undefined
        ? row("email", { name: TS.emailRow }, `<button class="btn sm" data-email-edit>${a.email ? TS.emailEdit : TS.emailAdd}</button>`, {
            sub: `${a.email ? esc(a.email) : TS.emailNone} · ${esc(TS.emailSub)}`,
          })
        : "",
    )}
    ${moreHtml(
      s,
      // 服务器上的数据删了：加电脑、换恢复码、换密码都做不了，只留换同步方式和停止同步
      deleted
        ? ""
        : row("invite", R.invite, `<button class="btn sm" id="sy-invite">${TS.invite}</button>`, { sub: esc(TS.inviteSub) }) +
            `<div id="sy-invite-box"></div>` +
            joinRow +
            row("recovery", R.recovery, `<button class="btn sm" id="sy-new-rc">${TS.newRc}</button>`) +
            delRow,
      !deleted,
    )}${msgLine}`;
}

/** 设备满了（31.1 第 2 条）：说清楚，带一个「看设备」跳到手机列表 */
const deviceFullHtml = (max: number) =>
  note("warn", "smartphone", esc(TS.deviceFull(max)), esc(TS.deviceFullHow), `<button class="btn sm" data-see-devices>${TS.seeDevices}</button>`);

/** 服务器说的「这个空间已经有 10 台设备了…」：认出来换成上面那块；回台数，不是这句时回 0 */
const deviceFullIn = (err: string) => Number(/已经有\s*(\d+)\s*台设备/.exec(err)?.[1] ?? 0);

/** 照上次连上时服务器说的账，已经满了：不用去试 */
function devicesFull(): number {
  const a = S.sync?.account;
  return a?.official && a.max_devices && a.devices >= a.max_devices ? a.max_devices : 0;
}

/** 跳到同步页里的某一块（比如手机列表），那一块闪一下 */
function jumpTo(id: string) {
  const el = document.querySelector<HTMLElement>(`#set [data-row="${id}"]`);
  if (!el) return;
  el.scrollIntoView({ block: "start", behavior: "smooth" });
  el.classList.add("flash");
  setTimeout(() => el.classList.remove("flash"), 1400);
}

function lockedHtml(s: SyncStatus) {
  const byCode = s.kind === "server" && (s.need_recovery || S.syncByRecovery);
  const folder = s.kind === "folder";
  return `${note("warn", "lock", TS.lockedTitle(byCode), esc(TS.lockedHint))}
    ${head(folder ? TS.onFolder : TS.onServer)}
    ${rows(
      row("this", { name: TS.thisPc }, "", { sub: esc(`${s.device ?? ""} · ${TS.where(folder, (folder ? s.folder : s.server) ?? "")}`) }),
      byCode
        ? row(
            "recovery",
            { name: TS.rc, sub: s.need_recovery ? TS.rcSubPair : TS.rcSub },
            `<span class="keyw"><input class="field mono" id="sy-rc" placeholder="${TS.rcPh}" spellcheck="false" autocomplete="off" aria-label="${TS.rc}"><button class="btn pri sm" id="sy-unlock-rc">${TS.unlock}</button></span>`,
          )
        : row("pw", { name: TS.pw }, `<span class="keyw">${pwField("sy-pw", "current-password")}<button class="btn pri sm" id="sy-unlock">${TS.unlock}</button></span>`),
      !byCode && s.kind === "server" ? row("forgot", { name: TS.forgot, sub: TS.forgotSub }, `<button class="btn sm ghost" id="sy-use-rc">${TS.useRc}</button>`) : "",
      row("stop", { name: TS.quit, sub: TS.quitSub }, `<button class="btn sm ghost danger" id="sy-stop">${TS.stopBtn}</button>`),
    )}${msgLine}`;
}

/** 开着同步时最下面的「更多」；chpw：有没有「换同步密码」（叮咪同步的数据删了时没有） */
function moreHtml(s: SyncStatus, extra = "", chpw = true) {
  const folder = s.kind === "folder";
  return `${h5(TS.more)}${rows(
    extra,
    chpw ? row("chpw", R.chpw, `<button class="btn sm" id="sy-chpw">${TS.chpwBtn}</button>`, { sub: esc(TS.chpwSub(TS.others(folder))) }) : "",
    row("switch", R.switch, `<button class="btn sm" id="sy-switch">${TS.switchBtn}</button>`, { sub: esc(TS.switchSub) }),
    row("stop", R.stop, `<button class="btn sm ghost danger" id="sy-stop">${TS.stopBtn}</button>`, { sub: esc(TS.stopSub(folder)) }),
  )}`;
}

/** 同步出错的提示里：还是不行就导出诊断信息发给作者（需求-0.3 第 12.2 节；点了跳到 关于 → 导出诊断信息，见 settings.ts） */
const diagLink = () => `<br><button class="lnk" data-godiag>${icon("file-text")}${esc(TS.diagTip)}</button>`;

function folderOnHtml(s: SyncStatus) {
  const lost = s.state === "offline";
  const old = s.state === "outdated";
  const why = s.error && s.error !== "找不到同步文件夹" ? `<br><small>${esc(s.error)}</small>` : "";
  const top = lost
    ? note("err", "folder-x", TS.lostTitle, esc(TS.lost) + why + diagLink(), `<button class="btn sm" id="sy-repick">${TS.repick}</button><button class="btn sm pri" id="sy-retry">${TS.retry}</button>`)
    : old
      ? note("warn", "cloud-off", TS.oldTitle, esc(TS.old))
      : "";
  const last = s.last_online ? (lost ? TS.lastSeen(ago(s.last_online)) : TS.lastSync(ago(s.last_online))) : "";
  return `${top}${head(TS.onFolder, lost || old ? "" : TS.folderHint)}
    ${rows(
      row("dir", { name: TS.syncDir }, `<button class="btn sm ghost" id="sy-open-dir">${T.open}</button>`, { sub: `<span class="path">${esc(s.folder ?? "")}</span>` }),
      row("now", { name: TS.thisPc }, `<button class="btn sm" id="sy-now">${TS.syncNow}</button>`, { sub: esc((s.device ?? "") + last) }),
    )}
    ${h5(R.devices.name, { id: "devices" })}<p class="hint">${esc(TS.devicesHint)}</p>
    <div class="srows" id="sy-devs">${row("reading", { name: TS.reading })}</div>
    ${moreHtml(s)}${msgLine}`;
}

function serverOnHtml(s: SyncStatus) {
  const label = TS.serverState[s.state] ?? "";
  // 连上了但带着 error：上传被服务器拒了（比如空间满了）
  const dot = s.state === "online" ? (s.error ? "bad" : "ok") : s.state === "offline" || s.state === "outdated" ? "bad" : "busy";
  const sub =
    s.state === "offline"
      ? [s.error, s.last_online && TS.serverLast(ago(s.last_online))].filter(Boolean).join(" · ")
      : s.state === "online" && s.error
        ? s.error
        : s.state === "online" && s.last_online
          ? TS.serverSince(ago(s.last_online))
          : "";
  const top = s.state === "outdated" ? note("warn", "cloud-off", TS.oldTitle, esc(TS.old)) : s.state === "offline" ? note("warn", "wifi-off", TS.offTitle, esc(TS.off) + diagLink()) : "";
  return `${top}${head(TS.onServer, TS.serverOnHint)}
    ${rows(
      `<div class="srow" data-row="server"><div><span class="sdot ${dot}"></span>${esc(label)}${sub ? `<small>${esc(sub)}</small>` : ""}</div><span class="path">${esc(s.server ?? "")}</span></div>`,
      row("now", { name: TS.thisPc }, `<button class="btn sm" id="sy-now">${TS.syncNow}</button>`, { sub: esc(s.device ?? "") }),
    )}
    ${moreHtml(
      s,
      row("invite", R.invite, `<button class="btn sm" id="sy-invite">${TS.invite}</button>`, { sub: esc(TS.inviteSub) }) +
        `<div id="sy-invite-box"></div>` +
        row("recovery", R.recovery, `<button class="btn sm" id="sy-new-rc">${TS.newRc}</button>`),
    )}${msgLine}`;
}

async function loadFolderDevices() {
  const box = document.querySelector<HTMLElement>("#sy-devs");
  if (!box) return;
  try {
    const list = await api.syncFolderDevices();
    const day = 24 * 3600 * 1000;
    box.innerHTML = list.length
      ? list
          .map((d) => {
            const old = !d.last_seen || Date.now() - new Date(d.last_seen.replace(" ", "T")).getTime() > day;
            return row("dev", { name: d.name }, `<span class="ago ${d.me || !old ? "ok" : "stale"}">${d.last_seen ? esc(ago(d.last_seen)) : TS.never}</span>`, { sub: d.me ? TS.me : null });
          })
          .join("")
      : row("dev", { name: TS.noDevices });
  } catch (e) {
    box.innerHTML = row("dev", { name: TS.cantRead }, "", { sub: esc(errText(e)) });
  }
}

// ---------- 手机（只有服务器方式有） ----------

/** 上次画的是哪种：list 手机列表 / gone 叮咪同步到期了 / no 用不了手机；null 没画 */
let phonesShown: string | null = null;

/** 开着服务器同步才有手机；连接状态来回变时不重画，免得关掉正在看的二维码 */
function renderPhone() {
  const sec = document.querySelector<HTMLElement>("#phone-sec");
  const s = S.sync;
  if (!sec || !s) return;
  const step = S.syncFlow?.step;
  // 没开同步（选方式的卡片下面已经说了）、正在走开同步的流程、锁着时不显示
  if (s.state === "off" || s.state === "locked" || step) {
    sec.innerHTML = "";
    phonesShown = null;
    return;
  }
  // 叮咪同步到期了、服务器上的数据删了：手机也用不了
  const gone = s.official && (s.state === "expired" || s.refused === "deleted");
  const show = s.kind === "server" && !gone && ["online", "offline", "connecting"].includes(s.state);
  const shownKey = show ? "list" : gone ? `gone-${s.refused ?? s.state}` : "no";
  if (shownKey === phonesShown && sec.innerHTML) return;
  phonesShown = shownKey;
  if (!show) {
    const why = !gone ? TS.phoneNo(s.kind === "folder") : s.refused === "deleted" ? TS.phoneDeleted : TS.phoneExpired;
    sec.innerHTML = `${h5(R.phone.name, { id: "phone" })}<p class="hint">${esc(why)}</p>`;
    return;
  }
  sec.innerHTML = `${h5(TS.pcs, { id: "pcs" })}<p class="hint">${esc(TS.pcsHint)}</p><div class="srows" id="pc-box"></div>
    ${h5(R.phone.name, { id: "phone" })}<p class="hint">${esc(TS.phoneHint)}</p>
    ${rows(row("inbox_auto", R.inbox_auto, tog("inbox_auto", R.inbox_auto.name)))}
    <div class="srows" id="phone-box"></div><div id="phone-new"></div><p class="fhelp err" id="phone-msg"></p>`;
  void loadPcs();
  void loadPhones();
}

/** 空间里的电脑：自己那台不给作废（要停就「停止同步」） */
async function loadPcs() {
  const box = document.querySelector<HTMLElement>("#pc-box");
  if (!box) return;
  box.innerHTML = row("reading", { name: TS.reading });
  try {
    const list = await api.pcList();
    box.innerHTML = list
      .map((p) =>
        row("pc", { name: p.me ? `${p.name}${TS.pcThis}` : p.name }, p.me ? "" : `<button class="btn sm ghost danger" data-pc-revoke="${esc(p.id)}">${TS.phoneRevoke}</button>`, {
          sub: esc(TS.pcSub(ago(p.created_at), p.last_seen ? TS.pcSeen(ago(p.last_seen)) : TS.pcNever)),
        }),
      )
      .join("");
  } catch (e) {
    box.innerHTML = row("pc", { name: TS.pcCantRead }, `<button class="btn sm" id="pc-reload">${TS.phoneReload}</button>`, { sub: esc(errText(e)) });
  }
}

async function loadPhones() {
  const box = document.querySelector<HTMLElement>("#phone-box");
  if (!box) return;
  box.innerHTML = row("reading", { name: TS.reading });
  try {
    const list = await api.phoneList();
    box.innerHTML =
      list
        .map((p) =>
          row("ph", { name: p.name }, `<button class="btn sm ghost danger" data-phone-revoke="${esc(p.id)}">${TS.phoneRevoke}</button>`, {
            sub: esc(TS.phoneAdded(ago(p.created_at), p.last_seen ? TS.phoneSeen(ago(p.last_seen)) : TS.phoneNever)),
          }),
        )
        .join("") +
      row(
        "phone_add",
        { name: TS.phoneAdd, sub: TS.phoneAddSub },
        `<span class="keyw"><input class="field" id="ph-name" value="${TS.phoneDefault}" aria-label="${TS.phoneName}"><button class="btn pri sm" id="ph-add">${TS.phoneAddBtn}</button></span>`,
      );
  } catch (e) {
    box.innerHTML = row("ph", { name: TS.phoneCantRead }, `<button class="btn sm" id="ph-reload">${TS.phoneReload}</button>`, { sub: esc(errText(e)) });
  }
}

const qrSvg = (text: string) => {
  const q = qrcode(0, "M");
  q.addData(text);
  q.make();
  return q.createSvgTag({ cellSize: 4, margin: 2 });
};

/** 刚添加的那部手机：说明按 iPhone / 安卓切换（名字里有「安卓」「Android」这类时先选安卓） */
let phoneShown: { name: string; link: PhoneLink; os: string } | null = null;

function showPhoneLink(name: string, link: PhoneLink, os = /安卓|android|华为|小米|oppo|vivo|荣耀|三星|pixel/i.test(name) ? "android" : "ios") {
  phoneShown = { name, link, os };
  const fp = link.fingerprint;
  const android = os === "android";
  // 服务器给手机网页绑了域名（正规证书）时 ca_url 是空的：不用装证书，只有打开网页这一步
  const trusted = !link.ca_url;
  // 安卓装证书用 .crt 文件；iPhone 用描述文件
  const ca = android ? `${link.ca_url}.crt` : link.ca_url;
  const caStep = trusted
    ? ""
    : `<div class="step"><div class="qr">${qrSvg(ca)}</div><div><b>${TS.phoneCa}</b><p>${android ? TS.phoneCaAndroid : TS.phoneCaHow}</p><p class="path">${esc(TS.phoneFp(fp.slice(0, 11), fp.slice(-11)))}</p></div></div>`;
  // 安卓的分享菜单不对外承诺（2026-11-06 作者定），两种手机都教复制、粘贴
  const share = TS.phonePasteIos;
  $("#phone-new").innerHTML = `<div class="phone-new">
    <div class="phead"><b>${esc(trusted ? TS.phoneStepsOne(name) : TS.phoneSteps(name))}</b><div class="seg" role="group">${TS.phoneOs.map(([k, l]) => `<button data-phos="${k}" aria-pressed="${k === os}">${l}</button>`).join("")}</div></div>
    ${caStep}
    <div class="step"><div class="qr">${qrSvg(link.url)}</div><div><b>${trusted ? TS.phoneOpenOnly : TS.phoneOpen}</b><p>${android ? TS.phoneOpenAndroid : TS.phoneOpenHow}</p><p>${share}</p></div></div>
    <div class="acts"><button class="btn sm" id="ph-copy">${icon("copy")}${TS.phoneCopy}</button><button class="btn sm pri" id="ph-done">${T.done}</button></div>
  </div>`;
  $("#ph-copy").dataset.url = link.url;
}

// ---------- 备份 ----------

/** 备份的情况；还没读过是 null */
let backup: BackupStatus | null = null;
/** 没开备份时点了「选地方…」又取消了：在下面粘贴文件夹的位置 */
let backupPicking = false;
/** 上次画出来的内容：没变就不重画，免得清掉正在输入的文件夹位置 */
let backupShown = "";

/** 「备份」那一块：不同步时在选方式的卡片下面，开着同步时在最下面；正在走开同步的流程时藏起来 */
async function renderBackup(reload = true) {
  if (reload) backup = await api.backupStatus().catch(() => backup);
  const box = document.querySelector<HTMLElement>("#backup-box");
  if (!box) return;
  const step = S.syncFlow?.step;
  box.hidden = (!!step && step !== "choose") || S.sync?.state === "locked";
  const html = backupHtml();
  if (html !== backupShown || !box.innerHTML) {
    box.innerHTML = html;
    backupShown = html;
  }
}

function backupHtml(): string {
  const b = backup;
  if (!b) return "";
  const syncing = !!S.sync && S.sync.state !== "off";
  const top = h5(R.backup.name, { id: "backup" });
  if (!b.on) {
    return `${top}<p class="hint">${esc(TS.backupOff(syncing))}</p>${rows(
      row("backup_pick", { name: TS.backupOne, sub: TS.backupOneSub }, `<button class="btn sm" id="bk-pick">${TS.backupPick}</button>`),
      backupPicking
        ? row(
            "backup_dir",
            { name: TS.backupWhere, sub: TS.backupWhereSub(S.sync?.kind === "folder") },
            `<span class="keyw"><input class="field" id="bk-dir" placeholder="${esc(TS.backupDirPh)}" spellcheck="false" aria-label="${TS.backupWhere}"><button class="btn sm" id="bk-dir-go">${TS.backupNext}</button></span>`,
          )
        : "",
    )}<p class="fhelp err" id="bk-msg"></p>`;
  }
  const last = TS.backupLastSub(b.last ? backupTime(b.last) : TS.backupNever);
  return `${top}<p class="hint">${esc(TS.backupOnSub(syncing))}</p>${rows(
    row("backup_dir", { name: TS.backupDir }, `<button class="btn sm ghost" id="bk-open">${T.open}</button>`, { sub: `<span class="path">${esc(b.dir ?? "")}</span>` }),
    row("backup_last", { name: TS.backupLast }, `<button class="btn sm" id="bk-now">${TS.backupNow}</button>`, {
      sub: esc(last) + (b.error ? `<span class="bad">${esc(b.error)}</span>` : ""),
    }),
    row("backup_off", { name: TS.backupStop, sub: TS.backupStopSub }, `<button class="btn sm ghost danger" id="bk-off">${TS.backupStopBtn}</button>`),
  )}<p class="fhelp err" id="bk-msg"></p>`;
}

/** 今天 14:20 / 昨天 14:20 / 11 月 3 日 14:20 */
export function backupTime(s: string) {
  const d = new Date(s.replace(" ", "T"));
  const hm = s.slice(11, 16);
  if (dayStr(d) === dayStr(new Date())) return `今天 ${hm}`;
  if (dayStr(d) === dayStr(addDays(new Date(), -1))) return `昨天 ${hm}`;
  return `${d.getMonth() + 1} 月 ${d.getDate()} 日 ${hm}`;
}

function backupMsg(text: string) {
  const m = document.querySelector<HTMLElement>("#bk-msg");
  if (m) m.textContent = text;
  else if (text) toast(text);
}

/** 选好了备份文件夹：看看里面有没有叮咪的数据，再在弹窗里输密码 */
async function pickBackupFolder(path: string) {
  if (!path.trim()) return backupMsg(TS.needDir);
  backupMsg("");
  const p = await api.syncFolderProbe(path);
  if (p.state === "bad") return backupMsg(p.error ?? TS.badDir);
  const norm = (x: string) => x.toLowerCase().replace(/[\\/]+$/, "");
  if (S.sync?.kind === "folder" && norm(p.dir) === norm(S.sync.folder ?? "")) return backupMsg(TS.backupSame);
  showBackupPassword(p);
}

/** 开备份的弹窗：开着同步时输同步密码；不同步时设一个（文件夹里已经有数据时输当时设的） */
function showBackupPassword(p: FolderProbe) {
  const syncing = !!S.sync && S.sync.state !== "off";
  const has = p.state === "space";
  const twice = !syncing && !has;
  const label = syncing ? TS.backupPwSync : has ? TS.backupPwOld : TS.backupPwNew;
  dialog(
    `<h4>${TS.backupTitle}</h4><p>${esc(TS.backupAt(p.dir, has))}</p>
    <label class="fl"><span>${label}</span><input class="field" type="password" id="bkpw-1" autocomplete="${twice ? "new-password" : "current-password"}"></label>
    ${twice ? `<label class="fl"><span>${TS.pw2}</span><input class="field" type="password" id="bkpw-2" autocomplete="new-password"></label>${note("warn", "key-round", TS.backupForgetTitle, esc(TS.backupForget))}` : ""}
    <p class="fhelp err" id="bkpw-err"></p>
    <div class="acts"><button type="button" class="btn" data-close>${T.cancel}</button><button class="btn pri" id="bkpw-go">${TS.backupGo}</button></div>`,
    { form: "bkpw" },
  );
  $<HTMLFormElement>("#bkpw").onsubmit = async (e) => {
    e.preventDefault();
    const pw = $<HTMLInputElement>("#bkpw-1").value;
    const fail = (t: string) => ($("#bkpw-err").textContent = t);
    if (!pw) return fail(TS.backupNeedPw(syncing));
    if (twice) {
      if ([...pw].length < 8) return fail(TS.backupPwShort);
      if (pw !== $<HTMLInputElement>("#bkpw-2").value) return fail(TS.pwDiff2);
    }
    const err = await busy($<HTMLButtonElement>("#bkpw-go"), syncing || has ? TS.checkingPw : TS.backupWriting, async () => {
      backup = await api.backupStart(p.dir, pw);
    });
    if (err) return fail(err);
    backupPicking = false;
    hideModal();
    await renderBackup(false);
    toast(backup?.error ? TS.backupOkButMissed : TS.backupDone);
  };
}

// ---------- 换同步密码、恢复码 ----------

function showChangePassword() {
  const others = TS.others(S.sync?.kind === "folder");
  dialog(
    `<h4>${TS.chpwTitle}</h4><p>${esc(TS.chpwWhy(others))}</p>
    <label class="fl"><span>${TS.newPw}<small>${TS.newPwSub}</small></span>${pwField("chpw-1", "new-password", true)}</label>
    <label class="fl"><span>${TS.pw2}</span>${pwField("chpw-2", "new-password")}</label>
    <p class="fhelp err" id="chpw-err"></p>
    <div class="acts"><button type="button" class="btn" data-close>${T.cancel}</button><button class="btn pri" id="chpw-go">${TS.chpwGo}</button></div>`,
    { form: "chpw" },
  );
  $<HTMLFormElement>("#chpw").onsubmit = async (e) => {
    e.preventDefault();
    const pw = $<HTMLInputElement>("#chpw-1").value;
    const fail = (t: string) => ($("#chpw-err").textContent = t);
    if ([...pw].length < 8) return fail(TS.pwShort);
    if (pw !== $<HTMLInputElement>("#chpw-2").value) return fail(TS.pwDiff2);
    const err = await busy($<HTMLButtonElement>("#chpw-go"), TS.chpwBusy, () => api.syncChangePassword(pw));
    if (err) return fail(err);
    hideModal();
    toast(TS.chpwOk);
  };
}

/** 恢复卡：恢复码、它的二维码和几句说明画成一张 PNG，回 base64（打印或存进手机相册）。
 *  这张图离开叮咪单独看，颜色不跟着风格变 */
function recoveryCard(code: string): string {
  const scale = 2;
  const [w, h] = [640, 400];
  const cv = document.createElement("canvas");
  cv.width = w * scale;
  cv.height = h * scale;
  const g = cv.getContext("2d")!;
  g.scale(scale, scale);
  g.fillStyle = "#fffcf8";
  g.fillRect(0, 0, w, h);
  g.strokeStyle = "#e2d6ca";
  g.strokeRect(12.5, 12.5, w - 25, h - 25);
  const ui = `"Microsoft YaHei UI", "Segoe UI", sans-serif`;
  g.fillStyle = "#2a2421";
  g.font = `600 22px ${ui}`;
  g.fillText(TS.card.title, 40, 62);
  g.fillStyle = "#6f655e";
  g.font = `13px ${ui}`;
  g.fillText(TS.card.made(new Date().toLocaleDateString("zh-CN")), 40, 88);
  // 恢复码分两行，每行 4 组
  const groups = code.replace(/^DMR-/, "").split("-");
  g.fillStyle = "#2a2421";
  g.font = `600 24px Consolas, "Cascadia Mono", monospace`;
  g.fillText("DMR-" + groups.slice(0, 4).join("-"), 40, 150);
  g.fillText("    " + groups.slice(4).join("-"), 40, 186);
  g.fillStyle = "#6f655e";
  g.font = `13px ${ui}`;
  TS.card.lines.forEach((t, i) => g.fillText(t, 40, 250 + i * 24));
  // 二维码：以后在别的设备上找回时可以直接拍
  const q = qrcode(0, "M");
  q.addData(code);
  q.make();
  const n = q.getModuleCount();
  const cell = Math.floor(150 / n);
  const [qx, qy] = [w - 40 - cell * n, 120];
  g.fillStyle = "#2a2421";
  for (let r = 0; r < n; r++) for (let c = 0; c < n; c++) if (q.isDark(r, c)) g.fillRect(qx + c * cell, qy + r * cell, cell, cell);
  return cv.toDataURL("image/png").split(",")[1];
}

/** 显示恢复码请用户存好：叮咪不保存它，只显示这一次。勾了「我已经存好了」才能关；关了以后调 after */
function showRecovery(code: string, fresh: boolean, after?: () => void) {
  dialog(
    `<h4>${fresh ? TS.rcFresh : TS.rcNew}</h4>
    <code class="rc-code">${esc(code)}</code>
    <div class="rc-acts"><button type="button" class="btn sm" id="rc-copy">${icon("copy")}${T.copy}</button><button type="button" class="btn sm" id="rc-img">${icon("image-down")}${TS.rcImg}</button></div>
    <p>${TS.rcWhy}${fresh ? "" : TS.rcOld}</p>
    ${note("warn", "key-round", "", esc(TS.rcKeep))}
    <label class="rc-ok"><input type="checkbox" id="rc-ok">${TS.rcOk}</label>
    <div class="acts"><button type="button" class="btn pri" id="rc-done" disabled>${T.done}</button></div>`,
    { sticky: true },
  );
  $("#rc-img").onclick = async () => {
    try {
      const saved = await api.savePng(`叮咪恢复码_${new Date().toISOString().slice(0, 10)}.png`, recoveryCard(code));
      if (saved) toast(TS.rcSaved);
    } catch (e) {
      toast(TS.rcSaveFail(errText(e)));
    }
  };
  $("#rc-copy").onclick = async () => {
    await navigator.clipboard.writeText(code);
    toast(TS.rcCopied);
  };
  $<HTMLInputElement>("#rc-ok").onchange = (e) => ($<HTMLButtonElement>("#rc-done").disabled = !(e.target as HTMLInputElement).checked);
  $("#rc-done").onclick = () => {
    hideModal();
    after?.();
  };
  // 先停在「复制」上，免得一打开就勾上「我已经存好了」那一格
  setTimeout(() => $("#rc-copy")?.focus(), 40);
}

// ---------- 叮咪同步：续费、联系客服、找回以后换恢复码 ----------

/** 兑换码框：输入时自动大写、分组；改了就清掉下面的出错提示 */
function redeemInput(el: HTMLInputElement) {
  reformatRedeem(el);
  const m = document.getElementById(`${el.id}-msg`);
  if (m) m.textContent = "";
}

/** 兑换码没填完整时给人看的一句；完整了回 "" */
function redeemCheck(code: string) {
  const p = redeemProblem(code);
  return p === "empty" ? TS.needRedeem : p === "short" ? TS.redeemShort : p === "confusing" ? TS.redeemConfusing : "";
}

/** 续费：填兑换码（需求-1.0 第 18.3 节）。数据已经删了的，续费就是重新开通，走开通那几步 */
export function showRenew() {
  if (S.sync?.refused === "deleted") return void reopen();
  const a = S.sync?.account;
  // 试运营的续费限制（服务器提前说了的）：暂停续费（到期了照样能填）、最多续到哪天
  const limit = a?.renew_off && !a.expired ? TS.renewOff : a?.renew_until_max ? TS.renewMax(monthDay(a.renew_until_max)) : "";
  dialog(
    `<h4>${TS.renewTitle}</h4><p>${esc(TS.renewHint(a?.until ?? null))}</p>${limit ? note("warn", "info", "", esc(limit)) : ""}
    <label class="fl"><span>${TS.redeem}<small>${TS.redeemSub}<button type="button" class="lnk" id="rn-buy">${TS.buy}</button></small></span>
      <input class="field mono" id="rn-code" data-redeem placeholder="${TS.redeemPh}" spellcheck="false" autocomplete="off"></label>
    <p class="fhelp err" id="rn-code-msg" role="alert"></p>
    <div class="acts"><button type="button" class="btn" data-close>${T.cancel}</button><button class="btn pri" id="rn-go">${TS.renewGo}</button></div>`,
    { form: "renew" },
  );
  const inp = $<HTMLInputElement>("#rn-code");
  inp.oninput = () => redeemInput(inp);
  $("#rn-buy").onclick = () => void api.openUrl(TS.buyUrl);
  setTimeout(() => inp.focus(), 40);
  $<HTMLFormElement>("#renew").onsubmit = async (e) => {
    e.preventDefault();
    const fail = (t: string) => ($("#rn-code-msg").textContent = t);
    const bad = redeemCheck(inp.value);
    if (bad) return fail(bad);
    let acc: SyncAccount | null = null;
    const err = await busy($<HTMLButtonElement>("#rn-go"), TS.renewing, async () => {
      acc = await api.officialRenew(inp.value);
    });
    if (err || !acc) return fail(err ?? "");
    const got: SyncAccount = acc;
    hideModal();
    if (S.sync) S.sync.account = got;
    if (S.view === "settings" && S.setSec === "sync") renderSync();
    toast(TS.renewed(planName(got.plan), untilText(got)));
  };
}

/** 数据删了以后再填码：当新开通（服务器那边会新建一个空间） */
function reopen() {
  S.syncFlow = { step: "official" };
  renderSync();
  void loadBeta();
  setTimeout(() => document.querySelector<HTMLInputElement>("#sy-redeem")?.focus(), 40);
}

/** 通知邮箱（32.1 第 6 条）：留、改、删 */
function showEmail() {
  const now = S.sync?.account?.email ?? "";
  dialog(
    `<h4>${TS.emailTitle}</h4><p>${esc(TS.emailSub)}</p>
    <label class="fl"><span>${TS.email}</span>${emailField("em-addr", now)}</label>
    <p class="fhelp err" id="em-msg" role="alert"></p>
    <div class="acts">${now ? `<button type="button" class="btn ghost" id="em-del">${TS.emailDel}</button>` : ""}<button type="button" class="btn" data-close>${T.cancel}</button><button class="btn pri" id="em-go">${TS.emailSave}</button></div>`,
    { form: "email" },
  );
  const inp = $<HTMLInputElement>("#em-addr");
  setTimeout(() => inp.focus(), 40);
  const save = async (b: HTMLButtonElement, email: string) => {
    let acc: SyncAccount | null = null;
    const err = await busy(b, TS.emailSaving, async () => {
      acc = await api.officialEmail(email);
    });
    if (err || !acc) return void ($("#em-msg").textContent = err ?? "");
    hideModal();
    if (S.sync) S.sync.account = acc;
    if (S.view === "settings" && S.setSec === "sync" && !S.syncFlow) renderSync();
    toast(email ? TS.emailSaved : TS.emailDeleted);
  };
  const del = document.querySelector<HTMLButtonElement>("#em-del");
  if (del) del.onclick = () => void save(del, "");
  $<HTMLFormElement>("#email").onsubmit = (e) => {
    e.preventDefault();
    const v = inp.value.trim();
    if (v && !emailOk(v)) return void ($("#em-msg").textContent = TS.emailBad);
    void save($<HTMLButtonElement>("#em-go"), v);
  };
}

/** 联系客服（需求-1.0 第 30.6 节）：要一个 6 位核对码，10 分钟有效；「去找客服」打开官网「联系我们」（第 32.1 节第 3 条） */
async function showSupport(b: HTMLButtonElement) {
  syncMsg("");
  let sc: { code: string; expires_at: string } | null = null;
  const err = await busy(b, TS.supportBusy, async () => {
    sc = await api.officialSupportCode();
  });
  if (err || !sc) return syncMsg(err ?? "");
  const got: { code: string; expires_at: string } = sc;
  const space = S.sync?.account?.space ?? "";
  dialog(
    `<h4>${TS.supportTitle}</h4><p>${esc(TS.supportWhy)}</p>
    <code class="rc-code sc-code">${esc(got.code)}</code>
    <div class="rc-acts"><button type="button" class="btn sm" id="sc-copy">${icon("copy")}${TS.supportCopy}</button></div>
    <p class="hint">${esc(TS.supportUntil(got.expires_at.slice(11, 16)))}</p>
    ${space ? rows(row("space_id", { name: TS.spaceId, sub: TS.spaceIdSub }, `<code class="space-id">${esc(space)}</code>`)) : ""}
    <div class="acts"><button type="button" class="btn" id="sc-go">${icon("external-link")}${TS.supportGo}</button><button type="button" class="btn pri" data-close>${T.done}</button></div>`,
  );
  // 去找客服：官网「联系我们」（需求-1.0 第 32.1 节第 3 条），对话框留着，核对码还要复制
  $("#sc-go").onclick = () => void api.openUrl(TS.supportUrl).catch((err) => toast(errText(err)));
  $("#sc-copy").onclick = async () => {
    await navigator.clipboard.writeText(got.code);
    toast(TS.supportCopied);
  };
}

/** 用恢复码找回以后：建议换一个新的恢复码，再存一次（需求-1.0 第 30.4 节） */
function offerNewRecovery() {
  dialog(
    `<h4>${TS.recoverNewTitle}</h4><p>${esc(TS.recoverNewText)}</p>
    <div class="acts"><button type="button" class="btn" data-close>${TS.recoverNewLater}</button><button type="button" class="btn pri" id="rcn-go">${TS.recoverNewGo}</button></div>`,
  );
  $<HTMLButtonElement>("#rcn-go").onclick = async (e) => {
    const b = e.currentTarget as HTMLButtonElement;
    let code = "";
    const err = await busy(b, TS.rcMaking, async () => {
      code = await api.syncNewRecovery();
    });
    if (err) return toast(err);
    showRecovery(code, false);
  };
}

// ---------- 点了什么 ----------

async function act(b: HTMLButtonElement, text: string, run: () => Promise<unknown>, done: string) {
  syncMsg("");
  const err = await busy(b, text, run);
  if (err) syncMsg(err);
  else toast(done);
}

/** 选了一个文件夹：看里面有没有叮咪的数据，决定是「第一台电脑」还是「加入」 */
async function pickFolder(path: string) {
  if (!path.trim()) return syncMsg(TS.needDir);
  syncMsg("");
  const probe = await api.syncFolderProbe(path);
  if (probe.state === "bad") return syncMsg(probe.error ?? TS.badDir);
  S.syncFlow = { step: probe.state === "empty" ? "first" : "join", probe };
  renderSync();
}

/** 去选网盘文件夹那一步，找到的网盘过一会儿才出来 */
async function toPick() {
  S.syncFlow = { step: "pick" };
  renderSync();
  const cands = await api.syncFolderCandidates();
  if (S.syncFlow?.step === "pick") {
    S.syncFlow.cands = cands;
    renderSync();
  }
}

const replacing = () => document.querySelector('#sy-mode [data-mode="replace"]')?.getAttribute("aria-pressed") === "true";

/** 同步页里的按钮，处理了返回 true */
export async function syncClick(el: HTMLElement): Promise<boolean> {
  if (!el.closest("#sync-box, #phone-sec, #backup-box")) return false;
  const b = el.closest<HTMLButtonElement>("button");
  if (!b) return true;
  const id = b.id;
  if (b.dataset.phos && phoneShown) {
    showPhoneLink(phoneShown.name, phoneShown.link, b.dataset.phos);
    return true;
  }
  if ("buy" in b.dataset) {
    await api.openUrl(TS.buyUrl);
    return true;
  }
  // 开通前要同意的协议：在浏览器里打开官网的那一页
  if (b.dataset.doc) {
    await api.openUrl(b.dataset.doc);
    return true;
  }
  if ("emailEdit" in b.dataset) {
    showEmail();
    return true;
  }
  if ("renew" in b.dataset) {
    showRenew();
    return true;
  }
  if ("reopen" in b.dataset) {
    reopen();
    return true;
  }
  if ("seeDevices" in b.dataset) {
    jumpTo("pcs");
    return true;
  }
  if ("joinOn" in b.dataset) {
    showJoinOn();
    return true;
  }
  if ("joinToggle" in b.dataset) {
    if (S.sync?.account?.join) showJoinOff();
    else showJoinOn();
    return true;
  }
  if ("delServer" in b.dataset) {
    showDeleteServer();
    return true;
  }
  if ("tipInvite" in b.dataset) {
    jumpTo("invite");
    document.querySelector<HTMLButtonElement>("#sy-invite")?.click();
    return true;
  }
  if ("joinable" in b.dataset) {
    S.syncFlow = { step: "ojoin", code: $<HTMLInputElement>("#sy-redeem").value };
    renderSync();
    document.querySelector<HTMLInputElement>("#sy-pw")?.focus();
    return true;
  }
  const pick = b.dataset.syncPick;
  if (pick) {
    syncMsg("");
    // 叮咪同步：先问「第一次开通 / 已经开通过」（需求-1.x 第 34.5 节）
    const step = pick === "official" ? "fork" : pick === "first" ? "official" : pick;
    if (step === "phone") void startPhone();
    else if (["server", "official", "recover", "pair", "fork", "ways", "ojoin"].includes(step)) {
      S.syncFlow = { step: step as SyncFlow["step"] };
      renderSync();
      if (step === "official") void loadBeta();
      const first = { official: "#sy-redeem", recover: "#sy-rc", ojoin: "#sy-jcode", server: "#sy-code", pair: "#sy-code" }[step as string];
      if (first) document.querySelector<HTMLInputElement>(first)?.focus();
    } else await toPick();
    return true;
  }
  // 四种加法里的每一种 → 退回「已经开通过」
  const st = S.syncFlow?.step;
  if (b.dataset.crumb === "ways" || ("syncBack" in b.dataset && (st === "recover" || st === "pair" || st === "ojoin" || st === "phone" || st === "phonepw"))) {
    S.syncFlow = { step: "ways" };
    renderSync();
    return true;
  }
  // 第一次开通、已经开通过 → 退回先问的那一步
  if (b.dataset.crumb === "fork" || ("syncBack" in b.dataset && (st === "official" || st === "ways"))) {
    S.syncFlow = { step: "fork" };
    renderSync();
    return true;
  }
  if (b.dataset.crumb === "choose" || ("syncBack" in b.dataset && (S.syncFlow?.step === "pick" || S.syncFlow?.step === "server" || S.syncFlow?.step === "fork"))) {
    S.syncFlow = S.sync?.state === "off" ? null : { step: "choose" };
    renderSync();
    return true;
  }
  if (b.dataset.crumb === "pick" || ("syncBack" in b.dataset && (S.syncFlow?.step === "first" || S.syncFlow?.step === "join"))) {
    await toPick();
    return true;
  }
  if ("syncBack" in b.dataset) {
    S.syncFlow = null;
    renderSync();
    return true;
  }
  if (b.dataset.folder !== undefined) {
    await pickFolder(b.dataset.folder);
    return true;
  }
  const mode = b.dataset.mode;
  if (mode) {
    $$("#sy-mode [data-mode]").forEach((c) => {
      c.setAttribute("aria-pressed", String(c === b));
      c.querySelector(".radio")?.classList.toggle("on", c === b);
    });
    return true;
  }
  switch (id) {
    case "sy-dir-go":
      await pickFolder($<HTMLInputElement>("#sy-dir").value);
      return true;
    case "sy-dir-pick": {
      const path = await api.pickFolder(TS.pickDlg);
      if (path) await pickFolder(path);
      return true;
    }
    case "sy-folder-go": {
      const f = S.syncFlow;
      if (!f?.probe) return true;
      const pw = $<HTMLInputElement>("#sy-pw").value;
      if (f.step === "first") {
        if ([...pw].length < 8) { syncMsg(TS.pwShort); return true; }
        if (pw !== $<HTMLInputElement>("#sy-pw2").value) { syncMsg(TS.pwDiff); return true; }
      } else if (!pw) {
        syncMsg(TS.needPw);
        return true;
      }
      const name = $<HTMLInputElement>("#sy-name").value;
      const replace = replacing();
      await act(b, f.step === "first" ? TS.starting : TS.checkingPw, async () => {
        await api.syncFolderStart(f.probe!.dir, pw, name, replace);
        S.syncFlow = null;
        renderSync();
      }, f.step === "first" ? TS.started : TS.joined);
      return true;
    }
    case "sy-pair": {
      const code = $<HTMLInputElement>("#sy-code").value.trim();
      const pw = $<HTMLInputElement>("#sy-pw").value;
      if (!code) { syncMsg(TS.needCode); return true; }
      if ([...pw].length < 8) { syncMsg(TS.pwShort); return true; }
      // 加入叮咪同步那一页只输一次（密码是别的电脑设好的）；自建服务器可能是第一台，要输两次
      const pw2 = document.querySelector<HTMLInputElement>("#sy-pw2");
      if (pw2 && pw !== pw2.value) { syncMsg(TS.pwDiff); return true; }
      const name = $<HTMLInputElement>("#sy-name").value;
      const replace = replacing();
      if (S.syncFlow?.step === "pair" && !(await leaveSpaceOk())) return true;
      await act(b, TS.connecting, async () => {
        const recovery = await api.syncPair(code, pw, name, replace);
        S.syncFlow = null;
        renderSync();
        if (recovery) showRecovery(recovery, true);
      }, TS.paired);
      return true;
    }
    // 叮咪同步：兑换码 + 设备名 → 开通 → 下一步设同步密码
    case "sy-redeem-go": {
      const inp = $<HTMLInputElement>("#sy-redeem");
      const fail = (t: string) => ($("#sy-redeem-msg").textContent = t);
      const bad = redeemCheck(inp.value);
      if (bad) {
        fail(bad);
        inp.focus();
        return true;
      }
      fail("");
      syncMsg("");
      const no = openCheck();
      if (no) {
        syncMsg(no);
        return true;
      }
      const name = $<HTMLInputElement>("#sy-name").value;
      const email = $<HTMLInputElement>("#sy-email").value;
      const err = await busy(b, TS.opening, async () => {
        const account = await api.officialRedeem(inp.value, name, email, true);
        S.syncFlow = { step: "setup", account };
        renderSync();
        document.querySelector<HTMLInputElement>("#sy-pw")?.focus();
      });
      // 这个码已经开过空间、那个空间能用兑换码加入：多半是自己在别的电脑上开的，接到「输同步密码加入」
      if (err?.startsWith(JOINABLE)) {
        fail("");
        $("#sync-msg").outerHTML = note("warn", "monitor-smartphone", "", esc(err.slice(JOINABLE.length)), `<button class="btn sm pri" data-joinable>${TS.joinableGo}</button>`) + msgLine;
      } else if (err) fail(err);
      agreeChanged();
      return true;
    }
    // 兑换码 + 同步密码加入
    case "sy-ojoin": {
      const inp = $<HTMLInputElement>("#sy-jcode");
      const bad = redeemCheck(inp.value);
      if (bad) {
        $("#sy-jcode-msg").textContent = bad;
        inp.focus();
        return true;
      }
      const pw = $<HTMLInputElement>("#sy-pw").value;
      if (!pw) { syncMsg(TS.needPw); return true; }
      const name = $<HTMLInputElement>("#sy-name").value;
      const replace = replacing();
      if (!(await leaveSpaceOk())) return true;
      syncMsg("");
      const err = await busy(b, TS.joining, async () => {
        await api.officialJoin(inp.value, pw, name, replace);
        S.syncFlow = null;
        S.sync = await api.syncStatus().catch(() => S.sync);
        renderSync();
        toast(TS.joinedOk);
      });
      if (err) syncMsg(err);
      return true;
    }
    case "sy-phone-again":
      void startPhone();
      return true;
    // 手机允许了：输同步密码加入
    case "sy-phone-join": {
      const pw = $<HTMLInputElement>("#sy-pw").value;
      if (!pw) { syncMsg(TS.needPw); return true; }
      const name = $<HTMLInputElement>("#sy-name").value;
      const replace = replacing();
      if (!(await leaveSpaceOk())) return true;
      syncMsg("");
      const err = await busy(b, TS.joining, async () => {
        await api.officialPhoneJoin(pw, name, replace);
        S.syncFlow = null;
        S.sync = await api.syncStatus().catch(() => S.sync);
        renderSync();
        toast(TS.joinedOk);
      });
      if (err) syncMsg(err);
      return true;
    }
    // 公测免费体验：不填兑换码，之后一样设同步密码、存恢复码（32.1 第 4 条）
    case "sy-beta-go": {
      syncMsg("");
      const no = openCheck();
      if (no) {
        syncMsg(no);
        return true;
      }
      const name = $<HTMLInputElement>("#sy-name").value;
      const email = $<HTMLInputElement>("#sy-email").value;
      const err = await busy(b, TS.opening, async () => {
        const account = await api.officialBeta(name, email, true);
        S.syncFlow = { step: "setup", account };
        renderSync();
        document.querySelector<HTMLInputElement>("#sy-pw")?.focus();
      });
      if (err) {
        syncMsg(err);
        // 名额刚好领完、关了：重新问一次，那一块跟着变
        void loadBeta();
      }
      agreeChanged();
      return true;
    }
    // 开通后设同步密码：拿回恢复码，存好了才算开通完
    case "sy-setup": {
      const pw = $<HTMLInputElement>("#sy-pw").value;
      if ([...pw].length < 8) { syncMsg(TS.pwShort); return true; }
      if (pw !== $<HTMLInputElement>("#sy-pw2").value) { syncMsg(TS.pwDiff); return true; }
      syncMsg("");
      const a = S.syncFlow?.account ?? S.sync?.account ?? null;
      const finish = async () => {
        S.syncFlow = null;
        S.sync = await api.syncStatus().catch(() => S.sync);
        renderSync();
        if (a?.official) toast(TS.openDone(planName(a.plan), untilText(a)));
      };
      const err = await busy(b, TS.settingUp, async () => {
        const rc = await api.syncUnlock(pw);
        if (rc) showRecovery(rc, true, () => void finish());
        else await finish();
      });
      if (err) syncMsg(err);
      return true;
    }
    // 用恢复码找回（唯一的电脑没了）
    case "sy-recover": {
      const rc = $<HTMLInputElement>("#sy-rc").value.trim();
      const pw = $<HTMLInputElement>("#sy-pw").value;
      if (!rc) { syncMsg(TS.needRc); return true; }
      if (pw && [...pw].length < 8) { syncMsg(TS.pwShort); return true; }
      if (pw && pw !== $<HTMLInputElement>("#sy-pw2").value) { syncMsg(TS.pwDiff); return true; }
      const name = $<HTMLInputElement>("#sy-name").value;
      if (!(await leaveSpaceOk())) return true;
      syncMsg("");
      const err = await busy(b, TS.recovering, async () => {
        await api.officialRecover(rc, name, pw);
        S.syncFlow = null;
        renderSync();
        // 只是多加一台电脑的不用换恢复码；忘了密码换了新的，多半是原来的电脑没了，再问一句要不要换
        if (pw) offerNewRecovery();
        else toast(TS.recovered);
      });
      if (err) syncMsg(err);
      return true;
    }
    case "sy-support":
      await showSupport(b);
      return true;
    case "sy-space-copy":
      await navigator.clipboard.writeText(S.sync?.account?.space ?? "");
      toast(TS.spaceCopied);
      return true;
    case "sy-open-dir":
      await api.openPath(S.sync?.folder ?? "");
      return true;
    case "sy-chpw":
      showChangePassword();
      return true;
    case "sy-switch":
      syncMsg("");
      S.syncFlow = { step: "choose" };
      renderSync();
      return true;
    case "sy-repick":
      syncMsg("");
      await toPick();
      return true;
    case "sy-retry":
      await api.syncNow();
      toast(TS.retrying);
      return true;
    case "sy-now":
      await api.syncNow();
      toast(TS.syncing);
      return true;
    case "sy-unlock": {
      const pw = $<HTMLInputElement>("#sy-pw").value;
      if (!pw) { syncMsg(TS.needPw); return true; }
      await act(b, TS.checkingPw, () => api.syncUnlock(pw), TS.unlocked);
      return true;
    }
    case "sy-use-rc":
      S.syncByRecovery = true;
      renderSync();
      return true;
    case "sy-unlock-rc": {
      const rc = $<HTMLInputElement>("#sy-rc").value.trim();
      if (!rc) { syncMsg(TS.needRc); return true; }
      await act(b, TS.checkingPw, async () => {
        await api.syncUnlock("", rc);
        S.syncByRecovery = false;
      }, TS.unlocked);
      return true;
    }
    case "sy-new-rc": {
      if (!armed(b, TS.newRcSure)) return true;
      syncMsg("");
      b.disabled = true;
      try {
        showRecovery(await api.syncNewRecovery(), false);
      } catch (e) {
        syncMsg(errText(e));
      }
      b.disabled = false;
      return true;
    }
    case "sy-invite": {
      syncMsg("");
      // 叮咪同步的设备满了：配对码生成了那台电脑也进不来，先说清楚
      const full = devicesFull();
      if (full) {
        $("#sy-invite-box").innerHTML = deviceFullHtml(full);
        return true;
      }
      b.disabled = true;
      try {
        const inv = await api.syncInvite();
        const at = inv.expires_at;
        $("#sy-invite-box").innerHTML = `<div class="srow invite"><div><code class="path">${esc(inv.code)}</code><small>${esc(TS.inviteUntil(+at.slice(5, 7), +at.slice(8, 10), at.slice(11, 16)))}</small></div>
          <button class="btn sm pri" id="sy-copy">${icon("copy")}${T.copy}</button></div>`;
        $("#sy-copy").dataset.code = inv.code;
      } catch (e) {
        const n = deviceFullIn(errText(e));
        if (n) $("#sy-invite-box").innerHTML = deviceFullHtml(n);
        else syncMsg(errText(e));
      }
      b.disabled = false;
      return true;
    }
    case "sy-copy":
      await navigator.clipboard.writeText(b.dataset.code ?? "");
      toast(TS.inviteCopied);
      return true;
    case "sy-stop":
      if (!armed(b, TS.stopSure)) return true;
      await act(b, TS.stopping, () => api.syncStop(), TS.stopped);
      return true;
    // 备份
    case "bk-pick": {
      const path = await api.pickFolder(TS.backupDlg);
      if (path) await pickBackupFolder(path);
      else {
        backupPicking = true;
        await renderBackup(false);
        document.querySelector<HTMLInputElement>("#bk-dir")?.focus();
      }
      return true;
    }
    case "bk-dir-go":
      await pickBackupFolder($<HTMLInputElement>("#bk-dir").value);
      return true;
    case "bk-open":
      await api.openPath(backup?.dir ?? "");
      return true;
    case "bk-now": {
      const err = await busy(b, TS.backupBusy, async () => {
        backup = await api.backupNow();
      });
      if (err) {
        toast(err);
        backup = await api.backupStatus().catch(() => backup);
      } else toast(backup?.error ? TS.backupMissed : TS.backupDone);
      backupShown = "";
      await renderBackup(false);
      return true;
    }
    case "bk-off":
      if (!armed(b, TS.backupStopSure)) return true;
      await api.backupOff();
      backupPicking = false;
      await renderBackup();
      toast(TS.backupStopped);
      return true;
    // 手机
    case "ph-reload":
      await loadPhones();
      return true;
    case "ph-add": {
      const name = $<HTMLInputElement>("#ph-name").value.trim() || TS.phoneDefault;
      $("#phone-msg").textContent = "";
      // 叮咪同步的设备满了（31.1 第 2 条）：说清楚，手机列表就在上面
      const full = devicesFull();
      if (full) {
        phoneShown = null;
        $("#phone-new").innerHTML = deviceFullHtml(full);
        return true;
      }
      b.disabled = true;
      try {
        showPhoneLink(name, await api.phoneAdd(name));
        await loadPhones();
      } catch (e) {
        const n = deviceFullIn(errText(e));
        if (n) {
          phoneShown = null;
          $("#phone-new").innerHTML = deviceFullHtml(n);
        } else $("#phone-msg").textContent = errText(e);
        b.disabled = false;
      }
      return true;
    }
    case "ph-copy":
      await navigator.clipboard.writeText(b.dataset.url ?? "");
      toast(TS.phoneCopied);
      return true;
    case "ph-done":
      phoneShown = null;
      $("#phone-new").innerHTML = "";
      return true;
  }
  if (b.id === "pc-reload") {
    void loadPcs();
    return true;
  }
  const pcRevoke = b.dataset.pcRevoke;
  const revoke = b.dataset.phoneRevoke;
  if (revoke || pcRevoke) {
    if (!armed(b, revoke ? TS.phoneRevokeSure : TS.pcRevokeSure)) return true;
    try {
      if (revoke) {
        await api.phoneRevoke(revoke);
        toast(TS.phoneRevoked);
        await loadPhones();
      } else {
        await api.pcRevoke(pcRevoke!);
        toast(TS.pcRevoked);
        await loadPcs();
      }
      // 叮咪同步：设备数跟着变（账的那一块、设备满了的判断都用它）
      if (S.sync?.official) {
        const a = await api.officialAccount().catch(() => null);
        if (a && S.sync) {
          S.sync.account = a;
          if (!S.syncFlow) renderSync();
        }
      }
    } catch (e) {
      toast(errText(e));
    }
    return true;
  }
  return false;
}

/** 设同步密码时右边实时说够不够长；兑换码框自动大写、分组 */
export function syncInput(el: HTMLElement) {
  if (el instanceof HTMLInputElement && "redeem" in el.dataset) return redeemInput(el);
  if (el.id === "sy-agree") return agreeChanged();
  const m = el.id && document.querySelector<HTMLElement>(`#${el.id}-m`);
  if (!m) return;
  const n = [...(el as HTMLInputElement).value].length;
  m.className = `pwm${n >= 8 ? " ok" : ""}`;
  m.textContent = !n ? "" : n >= 8 ? TS.pwLong : TS.pwShortBy(8 - n);
}

/** 同步状态变了：同步页开着、又没在填表时重画 */
export function settingsSyncChanged(before: SyncStatus["state"] | undefined, now: SyncStatus) {
  // 还在填表（选方式、设密码、待解锁）且状态没变时不重画，免得清掉正在输入的内容
  const filling = !!S.syncFlow || (before === now.state && (before === "off" || before === "locked"));
  if (!filling) renderSync();
}

