// 看大图（U7）：深色遮罩铺满窗口；← → 切同一条笔记里的图片；Esc 或点空白处关掉。右上角复制、另存为
import "../styles/editor.css";
import { attachmentSrc } from "@shared/md";
import { icon } from "@shared/icons";
import { esc } from "@shared/util";
import { api, errText } from "./api";
import { T } from "../text/notes";

export interface Pic {
  file: string;
  alt: string;
}

/** 把一张图片（附件）放进剪贴板。剪贴板只认 PNG，别的格式先画一遍转成 PNG */
export async function copyImage(file: string): Promise<void> {
  const blob = await (await fetch(attachmentSrc(file))).blob();
  let png = blob;
  if (blob.type !== "image/png") {
    const bmp = await createImageBitmap(blob);
    const c = document.createElement("canvas");
    c.width = bmp.width;
    c.height = bmp.height;
    c.getContext("2d")!.drawImage(bmp, 0, 0);
    png = await new Promise<Blob>((ok, no) => c.toBlob((b) => (b ? ok(b) : no(new Error(T.copyFail))), "image/png"));
  }
  await navigator.clipboard.write([new ClipboardItem({ "image/png": png })]);
}

/** 点了一张图片（img[data-att]）：用看大图打开，← → 切同一块（一条笔记、一条手机发来的）里的别的图片 */
export function lightboxFrom(img: HTMLElement, say: (text: string) => void) {
  const scope = img.closest(".dmd, .ib, .preview") ?? img.parentElement ?? document.body;
  const all = [...scope.querySelectorAll<HTMLImageElement>("img[data-att]")];
  openLightbox(
    all.map((x) => ({ file: x.dataset.att!, alt: x.alt })),
    Math.max(0, all.indexOf(img as HTMLImageElement)),
    say,
  );
}

let closeOpen: (() => void) | null = null;

export function openLightbox(pics: Pic[], index: number, say: (text: string) => void) {
  closeOpen?.();
  if (!pics.length) return;
  let i = Math.max(0, Math.min(index, pics.length - 1));
  const box = document.createElement("div");
  box.className = "lightbox";
  box.tabIndex = -1;
  box.innerHTML = `<img alt=""><div class="lt">
      <button type="button" data-lb="copy">${icon("copy")}${T.lbCopy}</button>
      <button type="button" data-lb="save">${icon("download")}${T.lbSave}</button>
      <button type="button" data-lb="close" title="${T.lbClose}" aria-label="${T.lbClose}">${icon("x")}</button></div>
    <button type="button" class="lb-nav prev" data-lb="prev" aria-label="上一张">${icon("chevron-left")}</button>
    <button type="button" class="lb-nav next" data-lb="next" aria-label="下一张">${icon("chevron-right")}</button>
    <div class="nm"></div>`;
  const img = box.querySelector("img")!;
  const show = () => {
    const p = pics[i];
    img.src = attachmentSrc(p.file);
    img.alt = p.alt;
    box.querySelector(".nm")!.innerHTML = esc(T.lbName(p.alt, i + 1, pics.length));
    box.querySelectorAll<HTMLElement>(".lb-nav").forEach((b) => (b.hidden = pics.length < 2));
  };
  const go = (d: number) => {
    i = (i + d + pics.length) % pics.length;
    show();
  };
  const close = () => {
    closeOpen = null;
    document.removeEventListener("keydown", key, true);
    box.remove();
  };
  const key = (e: KeyboardEvent) => {
    if (e.key === "Escape") close();
    else if (e.key === "ArrowLeft") go(-1);
    else if (e.key === "ArrowRight") go(1);
    else return;
    e.preventDefault();
    e.stopPropagation();
  };
  box.addEventListener("click", async (e) => {
    const el = e.target as HTMLElement;
    const b = el.closest<HTMLElement>("[data-lb]")?.dataset.lb;
    if (!b) {
      if (el !== img) close();
      return;
    }
    if (b === "close") close();
    else if (b === "prev") go(-1);
    else if (b === "next") go(1);
    else if (b === "copy") await copyImage(pics[i].file).then(() => say(T.imgCopied), (err) => say(errText(err)));
    else if (b === "save") {
      const p = await api.saveAttachmentAs(pics[i].file).catch((err) => void say(errText(err)));
      if (p) say(T.lbSaved(p));
    }
  });
  document.addEventListener("keydown", key, true);
  document.body.append(box);
  closeOpen = close;
  show();
  box.focus();
}
