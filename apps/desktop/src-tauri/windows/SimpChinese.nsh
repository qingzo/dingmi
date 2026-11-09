; 安装包的中文文字（tauri.conf.json 里的 customLanguageFiles）。
; 从 Tauri 自带的 SimpChinese 拷来（tauri-cli 2.12.4），改了 deleteAppData，加了 dingmiDeleteConfirm（windows/hooks.nsh 用）。
; 升级 tauri-cli 后看一眼自带的那份有没有加新的条目，有的话这里也要加，不然那一句会是空的。
; 文件不要带 BOM：Tauri 拷进去时自己加一个（带了就变成两个，makensis 在第 1 行报错）。

LangString addOrReinstall ${LANG_SIMPCHINESE} "添加/重新安装组件"
LangString alreadyInstalled ${LANG_SIMPCHINESE} "已安装"
LangString alreadyInstalledLong ${LANG_SIMPCHINESE} "${PRODUCTNAME} ${VERSION} 已经安装了。选择你想要执行的操作后点击下一步以继续。"
LangString appRunning ${LANG_SIMPCHINESE} "${PRODUCTNAME} 正在运行！请关闭后再试。"
LangString appRunningOkKill ${LANG_SIMPCHINESE} "${PRODUCTNAME} 正在运行！$\n点击确定以终止运行。"
LangString chooseMaintenanceOption ${LANG_SIMPCHINESE} "选择要执行的维护操作。"
LangString chooseHowToInstall ${LANG_SIMPCHINESE} "选择你想要安装 ${PRODUCTNAME} 的方式。"
LangString createDesktop ${LANG_SIMPCHINESE} "创建桌面快捷方式"
LangString dontUninstall ${LANG_SIMPCHINESE} "请勿卸载"
LangString dontUninstallDowngrade ${LANG_SIMPCHINESE} "请勿卸载（此安装程序禁止未卸载就进行版本降级的操作）"
LangString failedToKillApp ${LANG_SIMPCHINESE} "无法终止 ${PRODUCTNAME}。请关闭后再试。"
LangString installingWebview2 ${LANG_SIMPCHINESE} "正在安装 WebView2..."
LangString newerVersionInstalled ${LANG_SIMPCHINESE} "有一个更新版本的 ${PRODUCTNAME} 已经安装了！不推荐你安装旧的版本。如果你真的想要安装这个旧的版本，推荐先卸载当前版本。选择你想要执行的操作后点击下一步以继续。"
LangString older ${LANG_SIMPCHINESE} "旧的"
LangString olderOrUnknownVersionInstalled ${LANG_SIMPCHINESE} "系统中已存在版本为 $R4 的 ${PRODUCTNAME}。推荐先卸载当前版本后再进行安装。选择你想要执行的操作后点击下一步以继续。"
LangString silentDowngrades ${LANG_SIMPCHINESE} "降级操作在此安装程序上已禁用，无法进行安静安装，请使用图形操作界面。$\n"
LangString unableToUninstall ${LANG_SIMPCHINESE} "无法卸载！"
LangString uninstallApp ${LANG_SIMPCHINESE} "卸载 ${PRODUCTNAME}"
LangString uninstallBeforeInstalling ${LANG_SIMPCHINESE} "安装前卸载"
LangString unknown ${LANG_SIMPCHINESE} "未知"
LangString webview2AbortError ${LANG_SIMPCHINESE} "无法安装 WebView2！没有它，此应用就无法运行。尝试重启安装程序。"
LangString webview2DownloadError ${LANG_SIMPCHINESE} "错误：无法下载 WebView2 - $0"
LangString webview2DownloadSuccess ${LANG_SIMPCHINESE} "WebView2 引导程序下载成功"
LangString webview2Downloading ${LANG_SIMPCHINESE} "正在下载 WebView2 引导程序..."
LangString webview2InstallError ${LANG_SIMPCHINESE} "错误：安装 WebView2 时失败，错误代码：$1"
LangString webview2InstallSuccess ${LANG_SIMPCHINESE} "成功安装 WebView2"
LangString deleteAppData ${LANG_SIMPCHINESE} "同时删掉这台电脑上的提醒、笔记和设置"

LangString dingmiDeleteConfirm ${LANG_SIMPCHINESE} "要删掉这台电脑上的叮咪数据吗？$\n$\n提醒、笔记、附件和设置都会删掉，不能恢复。$\n· 开了同步的：别的电脑和服务器上还有，重新配对就回来。$\n· 没开同步的：先选「取消」，打开叮咪「设置 → 数据 → 导出全部」存一份，再来卸载。$\n$\n是：删掉$\n否：只卸载程序，数据留着（重装后还在）$\n取消：先不卸载"
