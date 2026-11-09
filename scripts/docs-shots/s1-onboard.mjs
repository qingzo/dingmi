// 第一次打开：引导页一步一步截
export default async ({ page, shot, wait }) => {
  const p = await page('onboard', 60);
  await shot(p, 'onboard-1');
  for (let i = 2; i <= 6; i++) {
    const next = p.locator('button:visible', { hasText: /下一步|开始用|好的|完成|知道了/ }).last();
    if (!(await next.count())) break;
    const label = await next.innerText();
    await next.click();
    await wait(900);
    if (p.isClosed()) {
      console.log('引导页关了（点的是', label, '）');
      break;
    }
    await shot(p, `onboard-${i}`);
    console.log('第', i, '步前点的是', label);
  }
};
