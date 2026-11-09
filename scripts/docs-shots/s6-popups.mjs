// 到点的提醒弹窗、笔记窗口、桌面组件
export default async ({ page, shot, invoke, wait, all }) => {
  // 1 分钟后的提醒
  await invoke('open_window', { name: 'flash' });
  const f = await page('flash');
  await wait(400);
  const box = f.locator('input, textarea').first();
  await box.fill('');
  await box.type('1分钟后站起来喝杯水', { delay: 10 });
  await wait(500);
  await box.press('Enter');
  console.log('等提醒弹出来……');

  // 等的时候：笔记窗口
  const notes = await invoke('list_notes', {});
  const list = Array.isArray(notes) ? notes : notes.items || [];
  console.log('笔记', list.map((n) => [n.id, n.title]).slice(0, 5));
  const book = list.find((n) => (n.title || n.body || '').includes('书单'));
  if (book) {
    await invoke('open_note_window', { id: book.id }).catch((e) => console.log('open_note_window', String(e)));
    const n = await page('note').catch((e) => (console.log(String(e)), null));
    if (n) await shot(n, 'win-note', { settle: 1200 });
  }

  // 桌面组件：设置里打开
  await invoke('open_main_nav', { view: 'settings', sub: 'widget' });
  const m = await page('main');
  await wait(800);
  const toggles = await m.evaluate(() => [...document.querySelectorAll('[data-set], [data-key]')].map((e) => e.dataset.set || e.dataset.key).slice(0, 20));
  console.log('桌面组件页的开关', toggles);

  for (let i = 0; i < 100; i++) {
    if (all().some((p) => p.url().includes('/remind.html'))) break;
    await wait(1000);
  }
  const r = await page('remind', 5).catch(() => null);
  if (r) await shot(r, 'win-remind', { settle: 1500 });
  else console.log('提醒没弹出来');
};
