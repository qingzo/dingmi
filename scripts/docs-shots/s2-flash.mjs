// 快捷框：空着、一句话认出时间、周期、几件事、只记成笔记
export default async ({ page, shot, invoke, wait }) => {
  const show = async () => {
    await invoke('open_window', { name: 'flash' });
    const p = await page('flash');
    await wait(500);
    return p;
  };
  let p = await show();
  const inp = p.locator('input, textarea').first();
  await inp.fill('');
  await shot(p, 'flash-empty');
  const cases = [
    ['flash-time', '明天下午三点和小王开会'],
    ['flash-repeat', '每周一早上9点交周报'],
    ['flash-multi', '周五晚上7点看电影，周六上午10点去超市买菜'],
    ['flash-vague', '下周找时间整理照片'],
    ['flash-note', '书单：《人类简史》《置身事内》'],
  ];
  for (const [name, text] of cases) {
    if (p.isClosed() || !(await p.evaluate(() => document.visibilityState === 'visible'))) p = await show();
    const box = p.locator('input, textarea').first();
    await box.fill('');
    await box.type(text, { delay: 15 });
    await wait(700);
    await shot(p, name);
  }
  await p.locator('input, textarea').first().fill('');
};
