# 使用文档的截图

截 `docs/截图/` 里的图（README 和使用说明的配图）。连上调试版叮咪各个窗口的调试接口（WebView2），在界面里输入示例数据、切页面、截图；不动真的鼠标和键盘，但窗口会弹出来、提醒会响。

1. 截图工具装在别处（不进项目）：找个临时目录 `npm i playwright-core@1.65.0`，把这个文件夹的文件拷过去。
2. 用一个全新的测试数据目录启动调试版：

   ```powershell
   $env:DINGMI_DATA_DIR = "D:\dingmi-test\docs"; $env:DINGMI_WEBVIEW_ARGS = "--remote-debugging-port=9333"; npm run tauri dev
   ```

   测试版的密钥都存在凭据管理器的 `dingmi-dev` 下，和别的测试目录共用：这份**不要配对同步**，不然会盖掉别的测试目录的密钥。
3. 按顺序跑：`node run.mjs s1-onboard.mjs`（引导页，要全新的目录）→ `s2-flash` → `s3-data`（真的记几条示例）→ `s4-main` → `s5-settings` → `s6-popups`（等 1 分钟的提醒）→ `s7-widget` → `s8-fix`。改了界面只重截一部分时照 `s9-reshoot` 那样写（它补上「健康」分类，重截快捷框、今天、提醒、设置 → 快捷框和桌面组件）。图先出在 `out/`，看过再拷进 `docs/截图/`。

快捷框空着时那一排「常用」是这台电脑上真的程序；数据页的路径是测试目录（`s8-fix` 换成正式版的默认位置）。
