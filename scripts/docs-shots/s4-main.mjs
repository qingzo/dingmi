// 主窗口：今天、提醒、笔记
export default async ({ page, shot, invoke, wait }) => {
  await invoke('open_main_nav', { view: 'today' });
  const p = await page('main');
  await wait(1200);
  console.log('主窗口大小', await p.evaluate(() => [innerWidth, innerHeight]));
  await shot(p, 'main-today');
  for (const view of ['reminders', 'notes']) {
    await invoke('open_main_nav', { view });
    await wait(1000);
    await shot(p, `main-${view}`);
  }
  // 设置有哪些页
  await invoke('open_main_nav', { view: 'settings' });
  await wait(1000);
  await shot(p, 'main-settings');
  const subs = await p.evaluate(() =>
    [...document.querySelectorAll('[data-sub], [data-page], [data-set]')].map((e) => [e.tagName, e.dataset.sub || e.dataset.page || e.dataset.set, e.textContent.trim().slice(0, 12)]),
  );
  console.log(JSON.stringify(subs));
};
