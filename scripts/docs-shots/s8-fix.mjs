// 数据页：测试目录换成装好的叮咪的默认位置再截
export default async ({ page, shot, invoke, wait }) => {
  const m = await page('main');
  await invoke('open_main_nav', { view: 'settings', sub: 'data' });
  await wait(900);
  const n = await m.evaluate(() => {
    let k = 0;
    const walk = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    while (walk.nextNode()) {
      const t = walk.currentNode;
      if (t.nodeValue.includes('D:\\dingmi-test\\docs')) {
        t.nodeValue = t.nodeValue.replace('D:\\dingmi-test\\docs', 'C:\\Users\\你的用户名\\AppData\\Roaming\\app.dingmi');
        k++;
      }
    }
    return k;
  });
  console.log('换了', n, '处路径');
  await shot(m, 'settings-data');
};
