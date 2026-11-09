// update.json 里决定叮咪怎么做的几样（版本号、最低版本、安装包的 SHA-256）合成一段文字，发布时用签安装包的那把私钥另外签名，
// 签名放在 update.json 的 meta_signature。叮咪（apps/desktop/src-tauri/src/update.rs 的 meta_text）拼出一模一样的文字核对：
// 对不上、没有签名就不认 min_version，免得更新通道被劫持时有人写个很大的最低版本让所有叮咪停下同步。
// 两边的写法要一模一样，测试 apps/desktop/ui/lib/update-meta.node.test.ts 和 update.rs 的 meta_text_matches_release_script 对的是同一个字符串。

/** 要签名的那段文字：一行一样，没有的写空 */
export function metaText(version, minVersion, sha256) {
  return `dingmi-update-meta-v1\nversion:${version}\nmin_version:${minVersion ?? ""}\nsha256:${sha256 ?? ""}\n`;
}
