import { describe, expect, it } from "vitest";
import { findRows, markName, matchRow, rowsOf } from "./setfind";
import { ROWS } from "../text/settings";

const ALL = rowsOf(ROWS);
const top = (q: string) => findRows(q, ALL).map((f) => `${f.row.page}/${f.row.id}`);
const esc = (s: string) => s.replace(/</g, "&lt;");

describe("搜设置", () => {
  it("常见叫法也能找到（需求-0.2 第 6.2 节）", () => {
    expect(top("自启")[0]).toBe("general/autostart");
    expect(top("开机")[0]).toBe("general/autostart");
    expect(top("暗色")[0]).toBe("general/theme");
    expect(top("夜间")[0]).toBe("general/theme");
    expect(top("夜间模式")[0]).toBe("general/theme");
    expect(top("勿扰")).toContain("remind/auto_focus");
    expect(top("皮肤")[0]).toBe("general/style");
    expect(top("启动器")).toContain("quick/sources");
    expect(top("空格")[0]).toBe("quick/flash_space");
    expect(top("空格启动")[0]).toBe("quick/flash_space");
    expect(top("打开程序")).toContain("quick/flash_space");
    expect(top("同步服务协议")[0]).toBe("about/sync_terms");
  });

  it("名字里整段有的排最前，按顺序有这几个字的也算", () => {
    expect(top("快捷框")[0]).toBe("general/hk_flash");
    expect(top("换同步密码")[0]).toBe("sync/chpw");
    // 一样像的，名字短的在前
    expect(top("同步").slice(0, 2)).toEqual(["sync/way", "ai/sync_ai_key"]);
    // 「自动」：名字里有的都出来，越靠前的排越前；说明里才有的排后面
    expect(top("自动").slice(0, 4)).toEqual(["about/auto_update", "general/autostart", "data/autobackup", "remind/auto_focus"]);
    // 隔着几个字：开机……启动
    expect(matchRow({ page: "p", id: "x", name: "开机自动启动" }, "开启")?.marks).toEqual([[0, 1], [4, 5]]);
  });

  it("叮咪同步的几行（需求-1.0 第 31.2 节 E）", () => {
    expect(top("叮咪同步")[0]).toBe("sync/official");
    expect(top("兑换码")).toContain("sync/official");
    expect(top("兑换码")).toContain("sync/renew");
    expect(top("续费")[0]).toBe("sync/renew");
    expect(top("到期")).toContain("sync/account");
    expect(top("到期")).toContain("sync/renew");
    expect(top("客服")[0]).toBe("sync/support");
    expect(top("空间编号")[0]).toBe("sync/space_id");
    expect(top("官方同步")).toContain("sync/official");
    expect(top("恢复码")).toContain("sync/official_recover");
  });

  it("关于里的联系我们、用户协议（需求-1.0 第 32.1 节）", () => {
    expect(top("联系我们")[0]).toBe("about/contact");
    expect(top("QQ群")).toContain("about/contact");
    expect(top("邮箱")).toContain("about/contact");
    expect(top("用户协议")[0]).toBe("about/terms");
    expect(top("协议")).toContain("about/terms");
    // 「客服」还是先到叮咪同步的联系客服
    expect(top("客服")[0]).toBe("sync/support");
  });

  it("不管大小写和空格", () => {
    expect(top("api key")[0]).toBe("ai/ai_key");
    expect(top("APIKEY")[0]).toBe("ai/ai_key");
    expect(top("Key")).toContain("ai/sync_ai_key");
    expect(matchRow({ page: "p", id: "x", name: "API Key" }, "ik")?.marks).toEqual([[2, 3], [4, 5]]);
  });

  it("只在小字说明里有的也能找到，但排在后面", () => {
    const f = matchRow({ page: "p", id: "x", name: "提示音", sub: "到点时「叮」一声" }, "叮");
    expect(f?.score).toBe(30);
    expect(f?.marks).toEqual([]);
    expect(top("剪贴板")).toContain("general/flash_selection");
  });

  it("隔得太开不算，什么都没打不出结果", () => {
    expect(matchRow({ page: "p", id: "x", name: "开一二三四五六七八启" }, "开启")).toBeNull();
    expect(findRows("   ", ALL)).toEqual([]);
    expect(findRows("没有这种设置啦", ALL)).toEqual([]);
  });

  it("每一行都有名字，id 不重复", () => {
    for (const r of ALL) expect(r.name.length).toBeGreaterThan(0);
    expect(new Set(ALL.map((r) => `${r.page}/${r.id}`)).size).toBe(ALL.length);
  });

  it("标出来的字套上 <mark>", () => {
    const f = matchRow({ page: "p", id: "x", name: "开机自动启动" }, "自启")!;
    expect(markName("开机自动启动", f.marks, esc)).toBe("开机<mark>自</mark>动<mark>启</mark>动");
    expect(markName("a<b", [[0, 1]], esc)).toBe("<mark>a</mark>&lt;b");
  });
});
