// 手机网页网址里的配对信息（安全审查第 7 条）。
// 新的网址：/m/#t=<手机令牌>.<收件箱公钥>&k=<空间密钥>：全在 # 后面，浏览器不会发给服务器，也就不会进 Nginx 的访问日志。
// 以前的网址：/m/<手机令牌>.<收件箱公钥>#k=<空间密钥>：已经加到主屏幕的还在用，照样认；打开后把网址换成新的写法（history.replaceState）。
// 安卓「安装」以后从清单的 /m/ 打开，# 后面是空的：用上次存在这部手机上的配对信息。

/** # 后面的参数。快捷指令是在整个网址后面接 #text=，所以 # 和 & 都当分隔符 */
export function parseHash(hash: string): Map<string, string> {
  return new Map(
    hash
      .replace(/^#/, "")
      .split(/[#&]/)
      .filter((p) => p.includes("="))
      .map((p) => {
        const i = p.indexOf("=");
        let v = p.slice(i + 1);
        try {
          v = decodeURIComponent(v);
        } catch {
          /* 坏掉的 % 原样留着 */
        }
        return [p.slice(0, i), v] as [string, string];
      }),
  );
}

/** 配对的那一段 <令牌>.<收件箱公钥>：只认 base64url 和点（和服务器 valid_seg 一样） */
export const validSeg = (s: string) => s.length > 2 && s.length <= 200 && /^[A-Za-z0-9_.-]+$/.test(s) && s.lastIndexOf(".") > 0;

export interface Pairing {
  /** <令牌>.<收件箱公钥>，认不出来时是空的 */
  seg: string;
  /** 从以前的网址（路径里）认出来的：要换成新的写法 */
  legacy: boolean;
}

/** 从网址（和上次存下的）认出配对信息：# 里的 t= 优先，其次以前的路径，最后是存下的 */
export function readPairing(pathname: string, args: Map<string, string>, saved: string | null): Pairing {
  const t = args.get("t") ?? "";
  if (validSeg(t)) return { seg: t, legacy: false };
  let path = "";
  try {
    path = decodeURIComponent(pathname.split("/")[2] || "");
  } catch {
    /* 坏掉的网址 */
  }
  if (validSeg(path)) return { seg: path, legacy: true };
  return { seg: saved && validSeg(saved) ? saved : "", legacy: false };
}

/** 换成新的写法以后的网址：路径只有 /m/，令牌、公钥、空间密钥在 # 后面 */
export function cleanUrl(seg: string, k: string | undefined): string {
  if (!seg) return "/m/";
  return `/m/#t=${seg}${k ? `&k=${encodeURIComponent(k)}` : ""}`;
}
