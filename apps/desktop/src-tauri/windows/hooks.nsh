; 安装包的附加步骤（tauri.conf.json 里的 installerHooks）
; 卸载时 Tauri 自带一个「删掉数据」的勾（默认不勾，文字在 windows/SimpChinese.nsh）；
; 勾上的话 Tauri 删掉 %APPDATA% 和 %LOCALAPPDATA% 下的 app.dingmi。自动更新时（/UPDATE）没有这个勾，也不删

!macro NSIS_HOOK_POSTINSTALL
  ; 换了图标：让资源管理器重新读图标，不然快捷方式还显示缓存里的旧图标
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; 勾了删掉数据：先问一遍，默认「否」。是：照删；否：只卸载程序、数据留着；取消：这次不卸载（还什么都没删）
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    MessageBox MB_YESNOCANCEL|MB_ICONEXCLAMATION|MB_DEFBUTTON2 "$(dingmiDeleteConfirm)" /SD IDNO IDYES dingmi_delete IDNO dingmi_keep
    Quit
    dingmi_keep:
    StrCpy $DeleteAppDataCheckboxState 0
    dingmi_delete:
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; 删掉数据时，Windows 凭据管理器里的密钥也删掉（sync/secret.rs：服务名 dingmi，keyring 存成「名字.dingmi」）
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    System::Call 'advapi32::CredDeleteW(w "sync-device-key.dingmi", i 1, i 0) i'
    System::Call 'advapi32::CredDeleteW(w "sync-data-key.dingmi", i 1, i 0) i'
    System::Call 'advapi32::CredDeleteW(w "sync-space-key.dingmi", i 1, i 0) i'
    System::Call 'advapi32::CredDeleteW(w "sync-backup-key.dingmi", i 1, i 0) i'
  ${EndIf}
!macroend
