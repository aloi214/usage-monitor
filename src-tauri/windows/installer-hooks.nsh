; The product rename changes Tauri's NSIS uninstall key and default install path.
; Do not guess ownership from an executable name or execute an UninstallString.
; Require the user to remove the registered legacy app while keeping their data.
;
; Reviewed against tauri-cli-v2.11.4:
; https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi
; PREINSTALL runs before application files, registry values and shortcuts are
; changed, but after the WebView2 section and SetOutPath (directory creation).

!macro RiceMonitorAbortInstall MESSAGE
  DetailPrint "${MESSAGE}"
  ${IfNot} ${Silent}
    ${If} $PassiveMode <> 1
      MessageBox MB_OK|MB_ICONSTOP "${MESSAGE}"
    ${EndIf}
  ${EndIf}
  SetErrorLevel 2
  Abort "${MESSAGE}"
!macroend

!macro RiceMonitorCheckLegacyRegistration ROOT ACCESS
  ; Open only this exact key, even if its values are incomplete or empty.
  ; KEY_QUERY_VALUE is sufficient; the WOW64 flag selects a view without
  ; changing the registry view selected by Tauri's SetContext macro.
  System::Call 'advapi32::RegOpenKeyExW(p ${ROOT}, w "Software\Microsoft\Windows\CurrentVersion\Uninstall\Pane Private", i 0, i ${ACCESS}, *p .r1) i.r0'
  ${If} $0 = 0
    System::Call 'advapi32::RegCloseKey(p r1) i.r0'
    !insertmacro RiceMonitorAbortInstall "A Pane Private installation is registered. Exit Pane Private, uninstall Pane Private from Windows Settings > Apps, then run this installer again. Keep your app data: leave any remove-app-data option unchecked."
  ${ElseIf} $0 <> 2
    ; ERROR_FILE_NOT_FOUND (2) is the only confirmed absence.
    !insertmacro RiceMonitorAbortInstall "Could not check the previous Pane Private installation (Windows error $0). Installation stopped. Check registry access and try again."
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREINSTALL
  Push $0
  Push $1

  ; HKCU / HKLM, KEY_QUERY_VALUE | KEY_WOW64_32KEY.
  !insertmacro RiceMonitorCheckLegacyRegistration 0x80000001 0x0201
  !insertmacro RiceMonitorCheckLegacyRegistration 0x80000002 0x0201
  ${If} ${RunningX64}
    ; HKCU / HKLM, KEY_QUERY_VALUE | KEY_WOW64_64KEY.
    !insertmacro RiceMonitorCheckLegacyRegistration 0x80000001 0x0101
    !insertmacro RiceMonitorCheckLegacyRegistration 0x80000002 0x0101
  ${EndIf}

  ; tauri-plugin-single-instance 2.4.2 uses "<identifier>-sim" on Windows.
  ; The identifier is deliberately retained and the semver feature is disabled.
  ; This also catches a running portable legacy copy without matching pane.exe
  ; from some other app. Only open the existing mutex; never create or wait on it.
  ; ?e captures GetLastError immediately, before other calls can overwrite it.
  System::Call 'kernel32::OpenMutexW(i 0x00100000, i 0, w "local.pane.private-sim") p.r0 ?e'
  Pop $1
  ${If} $0 P<> 0
    System::Call 'kernel32::CloseHandle(p r0) i.r0'
    !insertmacro RiceMonitorAbortInstall "Exit Pane Private or rice monitor from its tray menu, then run this installer again. A private app instance is still running in this Windows session."
  ${ElseIf} $1 <> 2
    !insertmacro RiceMonitorAbortInstall "Could not check whether the private app is running (Windows error $1). Installation stopped. Exit Pane Private or rice monitor and try again."
  ${EndIf}

  Pop $1
  Pop $0
!macroend
