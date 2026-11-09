// 第二轮：测试数据补上「健康」分类、吃药改成健康；重截快捷框、今天、提醒、设置 → 快捷框、设置 → 桌面组件
import s2 from './s2-flash.mjs';
export default async (ctx) => {
  const { page, shot, invoke, wait } = ctx;
  const st = await invoke('get_settings');
  if (!st.tags.some((t) => t.id === 'health')) {
    const i = st.tags.findIndex((t) => t.id === 'life');
    st.tags.splice(i + 1, 0, { id: 'health', name: '健康', color: 'green' });
  }
  st.widget_glass_level = 50;
  await invoke('save_settings', { settings: st });
  for (const r of await invoke('list_reminders')) {
    if (/吃药|体检|跑步|健身/.test(r.title) && r.tag !== 'health') {
      await invoke('save_reminder', { reminder: { ...r, tag: 'health' } });
      console.log('改成健康：', r.title);
    }
  }
  await wait(800);
  await s2(ctx);
  await invoke('open_main_nav', { view: 'today' });
  const p = await page('main');
  await wait(1200);
  await shot(p, 'main-today');
  await invoke('open_main_nav', { view: 'reminders' });
  await wait(1000);
  await shot(p, 'main-reminders');
  for (const [sub, name] of [['quick', 'settings-quick'], ['widget', 'settings-widget-on']]) {
    await invoke('open_main_nav', { view: 'settings', sub });
    await wait(1000);
    console.log(sub, '→', await p.evaluate(() => document.querySelector('h4')?.textContent));
    await shot(p, name);
  }
};
