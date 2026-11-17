// 每个窗口只能调它自己用到的命令（安全审查第 4 条，见 scripts/caps.mjs）：
// 页面里多调了一个命令、加了新命令，capabilities 没跟着改的话，那个窗口里的功能会在运行时被 Tauri 拦掉，这里先拦住。
// 要读文件，只在 vitest（Node）里跑
import { describe, expect, it } from "vitest";
// @ts-expect-error 脚本是 .mjs，没有类型
import { check, usage } from "../../../../scripts/caps.mjs";

describe("窗口的权限", () => {
  it("capabilities 和页面代码对得上，命令都登记了", () => {
    expect(check()).toEqual([]);
  });

  it("小窗口拿不到敏感命令", () => {
    const use = usage() as Record<string, string[]>;
    const sensitive = [
      "export_all", "restore_apply", "restore_peek", "reveal_path", "save_settings", "sync_pair", "sync_unlock", "sync_change_password",
      "sync_new_recovery", "phone_add", "pc_revoke", "official_recover", "diag_export", "backup_start", "sync_folder_start", "launcher_resolve",
    ];
    for (const win of ["flash", "qnote", "remind", "morning", "wrap", "widget", "note-*"]) {
      expect(use[win].filter((c) => sensitive.includes(c)), win).toEqual([]);
    }
    // 提醒卡片、开场白、收尾连文件都碰不了。attachment_dir 只回附件文件夹在哪（卡片上显示关联笔记里的图片要拼图片地址，需求-1.x 第 42.6 节），
    // 不读不写文件；图片能不能看由 asset 协议的范围管，哪个窗口都一样
    for (const win of ["remind", "morning", "wrap"]) {
      expect(use[win].filter((c) => c !== "attachment_dir" && /path|attachment|folder|export|restore|launcher/.test(c)), win).toEqual([]);
    }
  });
});
