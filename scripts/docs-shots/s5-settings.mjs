// 设置各页 + 几个小窗口
export default async ({ page, shot, invoke, wait }) => {
  await invoke('checklist_hide', {}).catch((e) => console.log('checklist_hide', String(e)));
  const subs = ['remind', 'notes', 'quick', 'widget', 'ai', 'sync', 'data', 'about'];
  const p = await page('main');
  for (const sub of subs) {
    await invoke('open_main_nav', { view: 'settings', sub });
    await wait(900);
    const title = await p.evaluate(() => document.querySelector('.set-page:not([hidden]) h2, section:not([hidden]) > h2')?.textContent);
    console.log(sub, '→', title);
    await shot(p, `settings-${sub}`);
  }
  for (const name of ['qnote', 'morning', 'wrap']) {
    await invoke('open_window', { name });
    const w = await page(name).catch((e) => (console.log(String(e)), null));
    if (w) await shot(w, `win-${name}`, { settle: 1200 });
  }
};
