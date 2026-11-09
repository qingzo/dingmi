// 用快捷框真的记几条（回车），攒示例数据
export default async ({ page, invoke, wait }) => {
  const items = [
    '明天下午三点和小王开会',
    '每周一早上9点交周报',
    '周六上午10点去超市买菜',
    '今天晚上8点给妈妈打电话',
    '每天中午12点半吃药',
    '下周三之前交水电费',
    '11月20日 小李生日',
    '下周找时间整理照片',
  ];
  for (const text of items) {
    await invoke('open_window', { name: 'flash' });
    const p = await page('flash');
    await wait(400);
    const box = p.locator('input, textarea').first();
    await box.fill('');
    await box.type(text, { delay: 10 });
    await wait(500);
    await box.press('Enter');
    await wait(900);
    console.log('记了', text);
  }
  // 两条笔记：Tab 切到「只记成笔记」再回车
  for (const text of ['书单：《人类简史》《置身事内》《被讨厌的勇气》', '路由器后台 192.168.1.1，Wi-Fi 名字 Home-5G']) {
    await invoke('open_window', { name: 'flash' });
    const p = await page('flash');
    await wait(400);
    const box = p.locator('input, textarea').first();
    await box.fill('');
    await box.type(text, { delay: 10 });
    await wait(500);
    await box.press('Tab');
    await wait(200);
    await box.press('Enter');
    await wait(900);
    console.log('记了笔记', text);
  }
  const rs = await invoke('list_reminders', {});
  console.log('提醒', Array.isArray(rs) ? rs.length : rs);
};
