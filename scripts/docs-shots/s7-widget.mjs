// 关掉提醒弹窗；打开桌面组件截一张
export default async ({ page, shot, invoke, wait, all }) => {
  const r = all().find((p) => p.url().includes('/remind.html'));
  if (r) await r.locator('button', { hasText: '完成' }).first().click().catch((e) => console.log(String(e)));
  await invoke('open_main_nav', { view: 'settings', sub: 'widget' });
  const m = await page('main');
  await wait(800);
  await m.locator('[data-set="widget"]').first().click();
  await wait(2500);
  const w = await page('widget', 15).catch((e) => (console.log(String(e)), null));
  if (w) {
    console.log('组件大小', await w.evaluate(() => [innerWidth, innerHeight]));
    await shot(w, 'win-widget', { settle: 1500 });
  }
  await shot(m, 'settings-widget-on');
};
