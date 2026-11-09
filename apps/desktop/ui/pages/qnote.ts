import "../styles/ui.css";
import "../styles/qnote.css";
import "../lib/look";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, errText, newNote, type Tag } from "../lib/api";
import { createNoteEditor } from "../lib/editor";
import { attachmentSrc, setAttachmentDir, shrunkMuch } from "@shared/md";
import { askTidy } from "../lib/tidyask";
import { icon, type IconName } from "@shared/icons";
import { $, esc, noteCatList, setNoteCats, tagColor } from "@shared/util";
import { T } from "../text/notes";

// 快速记笔记小窗口（U7，Ctrl + Alt + N）：宽 460，在快捷框的位置弹出。标题可以不填，正文是实时预览；
// 拖进来、粘贴的图片和文件放在下面一排，保存时接在正文后面。没保存就关掉的内容留作草稿，下次打开还在。
// 右上角 ⤢ 存好后转到主窗口的笔记本里接着写

const win = getCurrentWindow();
const title = $<HTMLInputElement>("#title");
const tip = $("#tip");
const DRAFT = "dingmi-qnote-draft";
/** 附件单个最大多大（和主窗口、手机、服务器一致） */
const MAX_ATTACHMENT = 50 * 1024 * 1024;

/** 这次加的附件。笔记要到保存时才有，存好再把它们归到笔记下面 */
interface Att {
  file: string;
  name: string;
  image: boolean;
}
let atts: Att[] = [];
/** 旧版本的草稿：附件已经写在正文里了 */
let inBody: string[] = [];
let tag: Tag | null = null;

function iconBtn(sel: string, ic: IconName, tipText: string) {
  const b = $(sel);
  b.innerHTML = icon(ic);
  b.title = tipText;
  b.setAttribute("aria-label", tipText);
}
iconBtn("#tomain", "maximize-2", T.qToMain);
iconBtn("#close", "x", T.qClose);
iconBtn("#tidy", "sparkles", T.qTidy);
iconBtn("#attach", "paperclip", T.qAttach);
$("#save").innerHTML = `${T.qSave}<kbd>Ctrl ↵</kbd>`;
title.placeholder = T.qTitle;
title.setAttribute("aria-label", T.title);

const say = (text: string) => (tip.textContent = text);

const ed = createNoteEditor($("#body"), "", {
  placeholder: T.qBody,
  onChange: () => saveDraft(),
  onFiles: (files) => void addFiles(files),
  onMessage: say,
});

// ---------- 草稿 ----------

function loadDraft() {
  try {
    const d = JSON.parse(localStorage.getItem(DRAFT) ?? "null");
    if (!d) return;
    // 先放附件和分类：换正文时会顺手存一次草稿
    atts = Array.isArray(d.atts) ? d.atts : [];
    inBody = Array.isArray(d.files) ? d.files.filter((f: unknown) => typeof f === "string") : [];
    tag = d.tag ?? null;
    title.value = d.title ?? "";
    ed.setValue(d.body ?? "");
  } catch {
    /* 草稿坏了就算了 */
  }
}
function saveDraft() {
  try {
    if (!title.value && !ed.value && !atts.length) localStorage.removeItem(DRAFT);
    else {
      localStorage.setItem(DRAFT, JSON.stringify({ title: title.value, body: ed.value, tag, atts, files: inBody }));
      say(T.qDraft);
    }
  } catch {
    /* 存不了草稿不影响使用 */
  }
}
function clear() {
  title.value = "";
  ed.setValue("");
  atts = [];
  inBody = [];
  tag = null;
  try {
    localStorage.removeItem(DRAFT);
  } catch {
    /* 没关系 */
  }
  drawAtts();
  drawCats();
  say("");
}

// ---------- 分类、附件 ----------

function drawCats() {
  $("#tags").innerHTML = noteCatList()
    .map((c) => `<button type="button" class="tag qcat" style="${tagColor(c.color)}" data-tag="${esc(c.id)}" aria-pressed="${c.id === tag}">${esc(c.name)}</button>`)
    .join("");
}
$("#tags").addEventListener("click", (e) => {
  const c = (e.target as HTMLElement).closest<HTMLElement>("[data-tag]");
  if (!c) return;
  tag = tag === c.dataset.tag ? null : c.dataset.tag!;
  drawCats();
  saveDraft();
});

function drawAtts() {
  const box = $("#att");
  box.hidden = !atts.length;
  box.innerHTML = atts
    .map((a, i) =>
      a.image
        ? `<span class="th"><img src="${attachmentSrc(a.file)}" alt=""><button type="button" data-rm="${i}" title="${T.qRemove}" aria-label="${T.qRemove}">${icon("x")}</button></span>`
        : `<span class="fl" title="${esc(a.name)}">${icon("file-text")}<span>${esc(a.name)}</span><button type="button" data-rm="${i}" title="${T.qRemove}" aria-label="${T.qRemove}">${icon("x")}</button></span>`,
    )
    .join("");
}
$("#att").addEventListener("click", (e) => {
  const b = (e.target as HTMLElement).closest<HTMLElement>("[data-rm]");
  if (!b) return;
  atts.splice(Number(b.dataset.rm), 1);
  drawAtts();
  saveDraft();
});

async function addFiles(list: File[]) {
  for (const f of list) {
    if (f.size > MAX_ATTACHMENT) {
      say(T.tooBig(f.name));
      continue;
    }
    say(T.qAdding(f.name));
    try {
      const image = f.type.startsWith("image/");
      const file = await api.saveAttachment(new Uint8Array(await f.arrayBuffer()), f.type, "", image ? "" : f.name);
      atts.push({ file, name: f.name.replace(/[[\]]/g, "_") || "文件", image });
      drawAtts();
      saveDraft();
    } catch (e) {
      say(T.notSaved(f.name, errText(e)));
    }
  }
  ed.focus();
}

$("#attach").onclick = () => ed.pickFiles(false);
document.addEventListener("dragover", (e) => {
  if ([...(e.dataTransfer?.items ?? [])].some((i) => i.kind === "file")) e.preventDefault();
});
document.addEventListener("drop", (e) => {
  const list = [...(e.dataTransfer?.files ?? [])];
  if (!list.length || e.defaultPrevented) return;
  e.preventDefault();
  void addFiles(list);
});

// ---------- 让 AI 整理：只理格式不动内容，整理完直接换上，底下可以撤销；少了一大截的不换 ----------

let beforeTidy: { title: string; body: string } | null = null;
$("#tidy").addEventListener("click", async () => {
  const btn = $<HTMLButtonElement>("#tidy");
  if (!ed.value.trim()) return void say(T.aiEmpty);
  const ask = await askTidy(btn);
  if (ask === null) return;
  btn.disabled = true;
  btn.innerHTML = `<span class="spin"></span>`;
  say(T.aiBusy);
  try {
    const r = await api.aiTidyNote(title.value, ed.value, ask);
    if (shrunkMuch(ed.value, r.body)) say(T.aiShrunk);
    else {
      beforeTidy = { title: title.value, body: ed.value };
      title.value = r.title;
      ed.setValue(r.body);
      saveDraft();
      tip.innerHTML = `${T.aiDone} <button type="button" class="undo" id="undo-tidy">${T.aiUndo}</button>`;
    }
  } catch (e) {
    say(T.aiFail(errText(e)));
  } finally {
    btn.disabled = false;
    btn.innerHTML = icon("sparkles");
  }
});
tip.addEventListener("click", (e) => {
  if (!(e.target as HTMLElement).closest("#undo-tidy") || !beforeTidy) return;
  title.value = beforeTidy.title;
  ed.setValue(beforeTidy.body);
  beforeTidy = null;
  saveDraft();
  say(T.qTidyUndone);
});

// ---------- 保存 ----------

let saving = false;
/** 存进笔记本；回存好的笔记 id */
async function store(): Promise<number | null> {
  if (saving) return null;
  if (!title.value.trim() && !ed.value.trim() && !atts.length) {
    ed.focus();
    say(T.qEmpty);
    return null;
  }
  saving = true;
  try {
    const lines = atts.map((a) => (a.image ? `![](att:${a.file})` : `[${a.name}](att:${a.file})`));
    const text = ed.value.replace(/\s+$/, "");
    const body = lines.length ? `${text}${text ? "\n\n" : ""}${lines.join("\n")}` : text;
    const n = await api.saveNote(newNote({ title: title.value, body, tag }));
    // 正文里删掉了的附件不归进来
    const used = [...atts.map((a) => a.file), ...inBody.filter((f) => body.includes(`(att:${f})`))];
    if (used.length) await api.attachFiles(used, n.uid);
    tip.innerHTML = `<span class="ok">${icon("check")}${esc(T.qSaved(n.title))}</span>`;
    return n.id;
  } catch (e) {
    say(T.qSaveFail(errText(e)));
    return null;
  } finally {
    saving = false;
  }
}

async function save() {
  if ((await store()) == null) return;
  // 窗口关掉只是藏起来（见 win.rs），下次打开要是空白的
  setTimeout(() => {
    void win.close();
    clear();
  }, 700);
}

$("#save").onclick = () => void save();
$("#close").onclick = () => void win.close();
$("#tomain").onclick = async () => {
  const nid = await store();
  if (nid == null) return;
  clear();
  await api.openMainNav("notes", nid).catch((err) => say(errText(err)));
  void win.close();
};
document.addEventListener("keydown", (e) => {
  if (e.defaultPrevented) return;
  if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
    e.preventDefault();
    void save();
  } else if (e.key === "Escape") {
    void win.close();
  }
});
title.addEventListener("input", saveDraft);
title.addEventListener("keydown", (e) => {
  if ((e.key === "Enter" && !e.ctrlKey) || e.key === "ArrowDown") {
    e.preventDefault();
    ed.view.dispatch({ selection: { anchor: 0 } });
    ed.focus();
  }
});

// 窗口跟着内容变高变矮（顶边不动，见 win.rs 的 fit）
let lastH = 0;
new ResizeObserver(() => {
  const h = $("#card").offsetHeight + 28;
  if (h === lastH) return;
  lastH = h;
  void api.fitWindow(h);
}).observe($("#card"));

/** 读一遍设置（分类可能在别处改过） */
async function prepare() {
  const [s, dir] = await Promise.all([api.getSettings().catch(() => null), api.attachmentDir().catch(() => "")]);
  setNoteCats(s?.note_cats);
  setAttachmentDir(dir);
  drawCats();
  drawAtts();
}

// 藏起来的窗口又被打开（窗口由 win.rs 显示出来）
listen("qnote-open", async () => {
  await prepare();
  ed.focus();
});

(async () => {
  loadDraft();
  await prepare();
  await win.show();
  await win.setFocus();
  ed.focus();
})();
