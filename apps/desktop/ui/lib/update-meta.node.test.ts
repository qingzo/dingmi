// update.json 里 version、min_version、sha256 的签名（安全审查第 8 条）：发布脚本签的文字要和叮咪核对时拼的一模一样
// （apps/desktop/src-tauri/src/update.rs 的 meta_text_matches_release_script 对的是同样的字符串）
import { describe, expect, it } from "vitest";
// @ts-expect-error 脚本是 .mjs，没有类型
import { metaText } from "../../../../scripts/update-meta.mjs";

describe("update.json 的签名内容", () => {
  it("和 update.rs 的 meta_text 一样", () => {
    expect(metaText("0.5.0", "0.4.1", "ab12")).toBe("dingmi-update-meta-v1\nversion:0.5.0\nmin_version:0.4.1\nsha256:ab12\n");
    expect(metaText("0.5.0", null, undefined)).toBe("dingmi-update-meta-v1\nversion:0.5.0\nmin_version:\nsha256:\n");
  });
});
