import { describe, expect, it } from "vitest";
import type { AttachmentInfo } from "./api";
import { attView, emailOk, fmtSize, formatRedeem, monthDay, PHONE_GAP_MS, phoneDeadline, planName, redeemProblem, usedPct, waitPhone } from "./official";

describe("兑换码", () => {
  it("自动大写、去空格，按 DM-XXXX-XXXX-XXXX-XXXX 分组", () => {
    expect(formatRedeem("dm-7kq4-px9m-h2ta-wc6r")).toBe("DM-7KQ4-PX9M-H2TA-WC6R");
    expect(formatRedeem(" dm 7kq4px9m h2ta wc6r ")).toBe("DM-7KQ4-PX9M-H2TA-WC6R");
    expect(formatRedeem("7KQ4PX9MH2TAWC6R")).toBe("7KQ4-PX9M-H2TA-WC6R");
    // 正在输：不乱加横线
    expect(formatRedeem("d")).toBe("D");
    expect(formatRedeem("dm")).toBe("DM");
    expect(formatRedeem("DM-")).toBe("DM");
    expect(formatRedeem("DM-7KQ")).toBe("DM-7KQ");
    expect(formatRedeem("DM-7KQ4P")).toBe("DM-7KQ4-P");
    // 多出来的截掉
    expect(formatRedeem("DM-7KQ4-PX9M-H2TA-WC6R-XYZ")).toBe("DM-7KQ4-PX9M-H2TA-WC6R");
    // 中文的横线、全角空格也去掉
    expect(formatRedeem("DM—7KQ4　PX9M")).toBe("DM-7KQ4-PX9M");
  });

  it("填完整了没有", () => {
    expect(redeemProblem("")).toBe("empty");
    expect(redeemProblem("DM-7KQ4")).toBe("short");
    expect(redeemProblem("DM-7KQ4-PX9M-H2TA-WC6R")).toBeNull();
    expect(redeemProblem("7KQ4PX9MH2TAWC6R")).toBeNull();
    // 本身就是 DM 开头、没写前缀的 16 位
    expect(redeemProblem("DMQ4PX9MH2TAWC6R")).toBeNull();
    expect(redeemProblem("DM-7KQ4-PX9M-H2TA-WC60")).toBe("confusing");
  });
});

describe("容量和套餐", () => {
  const MB = 1024 ** 2;
  const GB = 1024 ** 3;
  it("写成 MB / GB", () => {
    expect(fmtSize(0)).toBe("0 MB");
    expect(fmtSize(300 * 1024)).toBe("300 KB");
    expect(fmtSize(45 * MB)).toBe("45 MB");
    expect(fmtSize(0.62 * GB)).toBe("0.62 GB");
    expect(fmtSize(GB)).toBe("1 GB");
    expect(fmtSize(1.5 * GB)).toBe("1.5 GB");
    expect(fmtSize(5 * GB)).toBe("5 GB");
    expect(fmtSize(12.34 * GB)).toBe("12.3 GB");
  });

  it("用了百分之几", () => {
    expect(usedPct(5, 10)).toBe(50);
    expect(usedPct(12, 10)).toBe(100);
    expect(usedPct(5, null)).toBeNull();
  });

  it("套餐名", () => {
    expect(planName("标准")).toBe("标准会员");
    expect(planName("大容量")).toBe("大容量会员");
    expect(planName("免费套餐")).toBe("免费套餐");
    expect(planName("标准会员")).toBe("标准会员");
  });
});

describe("公测、邮箱、套餐限额（需求-1.0 第 32.1 节）", () => {
  it("公测体验的套餐名不加「会员」", () => {
    expect(planName("公测体验")).toBe("公测体验");
  });

  it("日期写成几月几日，不是今年的带上年份", () => {
    const y = new Date().getFullYear();
    expect(monthDay(`${y}-05-03`)).toBe("5 月 3 日");
    expect(monthDay(`${y + 1}-12-31`)).toBe(`${y + 1} 年 12 月 31 日`);
    expect(monthDay("不是日期")).toBe("不是日期");
  });

  it("邮箱只做简单核对（和服务器一样）", () => {
    for (const ok of ["me@example.com", " li.qingcai@example.com ", "a+b@example.org"]) expect(emailOk(ok), ok).toBe(true);
    for (const bad of ["", "me@qq", "@qq.com", "me qq@example.com", "me@@example.com", "me@.com", "me@example.com.", "a,b@example.com"]) expect(emailOk(bad), bad).toBe(false);
  });

  it("太大、叮咪同步不传的附件：说清楚在哪台设备上", () => {
    const devices = [
      { uid: "me1", name: "家里电脑", me: true },
      { uid: "pc2", name: "公司电脑", me: false },
    ];
    const att = (o: Partial<AttachmentInfo>): AttachmentInfo => ({
      uid: "u",
      file: "u.pdf",
      name: "合同.pdf",
      mime: "application/pdf",
      size: 20 << 20,
      local: true,
      only_on: null,
      ...o,
    });
    // 照常同步的：没有 away
    expect(attView(att({}), devices)).toEqual({ size: 20 << 20, local: true });
    // 在这台电脑上、没传上去
    expect(attView(att({ only_on: "me1" }), devices).away).toContain("只在这台电脑上");
    // 在公司电脑上，这台没有
    expect(attView(att({ only_on: "pc2", local: false }), devices).away).toContain("只在「公司电脑」上");
    // 不认识的设备
    expect(attView(att({ only_on: "zz", local: false }), devices).away).toContain("另一台电脑");
  });
});

describe("用手机加入：等手机允许", () => {
  const at = (s: string) => new Date(s.replace(" ", "T")).getTime();
  /** 假的钟：sleep 只把钟往后拨；每问一次服务器也算 dt 毫秒 */
  const fake = (start: string, dt = 0) => {
    let t = at(start);
    const asks: number[] = [];
    return {
      asks,
      clock: { now: () => t, sleep: async (ms: number) => void (t += ms) },
      poll: (answers: boolean[] = []) => async () => {
        asks.push(t);
        t += dt;
        return answers.shift() ?? false;
      },
    };
  };

  it("有效期按这台电脑的钟算；对不上的（已经过了、远得不像话）不按它停", () => {
    expect(phoneDeadline("2026-11-05 10:10:30", at("2026-11-05 10:00:00"))).toBe(at("2026-11-05 10:10:30"));
    expect(phoneDeadline("2026-11-05 10:10", at("2026-11-05 10:00:00"))).toBe(at("2026-11-05 10:10:00"));
    expect(phoneDeadline("2026-11-05 09:10:00", at("2026-11-05 10:00:00"))).toBeNull();
    expect(phoneDeadline("2026-11-05 11:10:00", at("2026-11-05 10:00:00"))).toBeNull();
    expect(phoneDeadline("", at("2026-11-05 10:00:00"))).toBeNull();
  });

  it("手机允许了就回 ok；两次问之间至少隔 1.5 秒", async () => {
    const f = fake("2026-11-05 10:00:00");
    expect(await waitPhone("2026-11-05 10:10:00", () => true, f.poll([false, false, true]), f.clock)).toBe("ok");
    expect(f.asks).toHaveLength(3);
    for (let i = 1; i < f.asks.length; i++) expect(f.asks[i] - f.asks[i - 1]).toBeGreaterThanOrEqual(PHONE_GAP_MS);
  });

  it("过了有效期自己停，回 expired", async () => {
    // 服务器每次等满 20 秒才回
    const f = fake("2026-11-05 10:00:00", 20_000);
    expect(await waitPhone("2026-11-05 10:10:00", () => true, f.poll(), f.clock)).toBe("expired");
    expect(f.clock.now()).toBeGreaterThanOrEqual(at("2026-11-05 10:10:00"));
    expect(f.asks.every((t) => t < at("2026-11-05 10:10:00"))).toBe(true);
  });

  it("换了别的方式（alive 变 false）就不再问；这时候手机允许了也不算", async () => {
    const f = fake("2026-11-05 10:00:00");
    let on = true;
    const poll = f.poll([false, false]);
    const end = waitPhone(
      "2026-11-05 10:10:00",
      () => on,
      async () => {
        const r = await poll();
        if (f.asks.length === 2) on = false;
        return r;
      },
      f.clock,
    );
    expect(await end).toBe("stop");
    expect(f.asks).toHaveLength(2);
    const g = fake("2026-11-05 10:00:00");
    let alive = true;
    expect(await waitPhone("2026-11-05 10:10:00", () => alive, async () => ((alive = false), true), g.clock)).toBe("stop");
  });

  it("问的时候出错照样抛出去（界面上显示服务器说的原因）", async () => {
    const f = fake("2026-11-05 10:00:00");
    await expect(waitPhone("2026-11-05 10:10:00", () => true, async () => Promise.reject("这个数字过期了，再要一个"), f.clock)).rejects.toBe("这个数字过期了，再要一个");
  });
});
