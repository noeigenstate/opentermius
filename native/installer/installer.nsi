; OpenTermius Windows installer (NSIS 3, Modern UI 2).
;
; Build through scripts/package.ps1, which stages opentermius.exe and passes
; /DVERSION=<version from Cargo.toml>. The installer is per-user by default and
; never requires administrator rights.

Unicode true
SetCompressor /SOLID lzma
SetCompressorDictSize 64
CRCCheck on
RequestExecutionLevel user
ManifestDPIAware true

!ifndef VERSION
  !error "VERSION is not defined. Run scripts/package.ps1 or pass /DVERSION=<version>."
!endif

!define APP_NAME "OpenTermius"
!define APP_EXE "opentermius.exe"
!define APP_REG_KEY "Software\OpenTermius"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\OpenTermius"
!define STAGED_EXE "staging\opentermius.exe"

Name "${APP_NAME}"
BrandingText "${APP_NAME}"
OutFile "..\releases\OpenTermius-${VERSION}-windows-x64-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\OpenTermius"
InstallDirRegKey HKCU "${APP_REG_KEY}" "InstallDir"
ShowInstDetails show
ShowUninstDetails show

VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=1033 "ProductName" "${APP_NAME}"
VIAddVersionKey /LANG=1033 "FileDescription" "${APP_NAME} Setup"
VIAddVersionKey /LANG=1033 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "CompanyName" "OpenTermius"
VIAddVersionKey /LANG=1033 "LegalCopyright" "OpenTermius"

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "nsDialogs.nsh"

!define MUI_ICON "..\..\assets\icon.ico"
!define MUI_UNICON "..\..\assets\icon.ico"
!define MUI_ABORTWARNING

!define MUI_LANGDLL_REGISTRY_ROOT "HKCU"
!define MUI_LANGDLL_REGISTRY_KEY "${APP_REG_KEY}"
!define MUI_LANGDLL_REGISTRY_VALUENAME "InstallerLanguage"
!define MUI_LANGDLL_WINDOWTITLE "OpenTermius Setup"
!define MUI_LANGDLL_INFO "Select the installer language. / 请选择安装语言。"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP_EXE}"
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
UninstPage custom un.DataPageCreate un.DataPageLeave
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH

!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "SimpChinese"

; ---------------------------------------------------------------------------
; Languages

LangString SecCore        ${LANG_ENGLISH}   "OpenTermius (required)"
LangString SecCore        ${LANG_SIMPCHINESE} "OpenTermius（必需）"
LangString SecCoreDesc    ${LANG_ENGLISH}   "Install the OpenTermius application, Start Menu shortcut and uninstaller."
LangString SecCoreDesc    ${LANG_SIMPCHINESE} "安装 OpenTermius 程序、开始菜单快捷方式和卸载程序。"
LangString SecDesktop     ${LANG_ENGLISH}   "Create a Desktop shortcut"
LangString SecDesktop     ${LANG_SIMPCHINESE} "创建桌面快捷方式"
LangString SecDesktopDesc ${LANG_ENGLISH}   "Place an OpenTermius shortcut on the Desktop."
LangString SecDesktopDesc ${LANG_SIMPCHINESE} "在桌面放置 OpenTermius 快捷方式。"

LangString AppRunning     ${LANG_ENGLISH}   "OpenTermius is currently running. Close it, then click Retry to continue."
LangString AppRunning     ${LANG_SIMPCHINESE} "OpenTermius 正在运行。请关闭它，然后点击“重试”继续。"

LangString DataPageTitle    ${LANG_ENGLISH}   "Remove application data"
LangString DataPageTitle    ${LANG_SIMPCHINESE} "删除应用数据"
LangString DataPageSubtitle ${LANG_ENGLISH}   "Choose whether to keep your local configuration."
LangString DataPageSubtitle ${LANG_SIMPCHINESE} "选择是否保留本地配置。"
LangString DataPageText     ${LANG_ENGLISH}   "Your hosts, credentials and preferences are stored in:"
LangString DataPageText     ${LANG_SIMPCHINESE} "主机、凭据和偏好设置保存在："
LangString DataPageKeep     ${LANG_ENGLISH}   "This data is kept by default so a reinstall can use it again."
LangString DataPageKeep     ${LANG_SIMPCHINESE} "默认保留这些数据，以便重新安装后继续使用。"
LangString DataPageCheck    ${LANG_ENGLISH}   "Also remove my data (hosts, credentials and preferences)"
LangString DataPageCheck    ${LANG_SIMPCHINESE} "同时删除我的数据（主机、凭据和偏好设置）"

; ---------------------------------------------------------------------------
; Variables

Var RemoveData
Var DataCheckbox
Var AppCheckResult

; ---------------------------------------------------------------------------
; Running instance handling
;
; tasklist output is localized, so match the image name through findstr instead
; of relying on the localized "no tasks" text. A non-zero findstr exit code
; means that OpenTermius is not running.

Function CheckAppClosed
  ${Do}
    nsExec::ExecToStack 'cmd /c tasklist /FI "IMAGENAME eq ${APP_EXE}" /NH 2>nul | findstr /I /C:"${APP_EXE}" >nul'
    Pop $AppCheckResult
    Pop $0
    ${If} $AppCheckResult != 0
      ${ExitDo}
    ${EndIf}
    ${If} ${Silent}
      SetErrorLevel 2
      Abort
    ${EndIf}
    MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(AppRunning)" IDRETRY retry IDCANCEL abort
    retry:
  ${Loop}
  Return

  abort:
    SetErrorLevel 3
    Abort
FunctionEnd

Function un.CheckAppClosed
  ${Do}
    nsExec::ExecToStack 'cmd /c tasklist /FI "IMAGENAME eq ${APP_EXE}" /NH 2>nul | findstr /I /C:"${APP_EXE}" >nul'
    Pop $AppCheckResult
    Pop $0
    ${If} $AppCheckResult != 0
      ${ExitDo}
    ${EndIf}
    ${If} ${Silent}
      SetErrorLevel 2
      Abort
    ${EndIf}
    MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(AppRunning)" IDRETRY retry IDCANCEL abort
    retry:
  ${Loop}
  Return

  abort:
    SetErrorLevel 3
    Abort
FunctionEnd

; ---------------------------------------------------------------------------
; Installer

Function .onInit
  SetShellVarContext current
  SetRegView 64
  !insertmacro MUI_LANGDLL_DISPLAY
  Call CheckAppClosed
FunctionEnd

Section "$(SecCore)" SecCore
  SectionIn RO
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  File "${STAGED_EXE}"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateDirectory "$SMPROGRAMS"
  ; Remove shortcuts from an older install so the current component selection wins.
  Delete "$SMPROGRAMS\OpenTermius.lnk"
  Delete "$DESKTOP\OpenTermius.lnk"
  CreateShortCut "$SMPROGRAMS\OpenTermius.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}" 0

  WriteRegStr HKCU "${APP_REG_KEY}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${APP_REG_KEY}" "Version" "${VERSION}"

  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "OpenTermius"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${APP_EXE},0"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1

  StrCpy $0 0
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  ${If} $0 == ""
    StrCpy $0 1024
  ${EndIf}
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "EstimatedSize" $0
SectionEnd

Section "$(SecDesktop)" SecDesktop
  SetShellVarContext current
  CreateShortCut "$DESKTOP\OpenTermius.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}" 0
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecCore} "$(SecCoreDesc)"
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "$(SecDesktopDesc)"
!insertmacro MUI_FUNCTION_DESCRIPTION_END

; ---------------------------------------------------------------------------
; Uninstaller

Function un.onInit
  SetShellVarContext current
  SetRegView 64
  !insertmacro MUI_UNGETLANGUAGE

  ${GetParameters} $R0
  ClearErrors
  ${GetOptions} $R0 "/REMOVEDATA" $R1
  ${IfNot} ${Errors}
    StrCpy $RemoveData 1
  ${EndIf}

  Call un.CheckAppClosed
FunctionEnd

Function un.DataPageCreate
  !insertmacro MUI_HEADER_TEXT "$(DataPageTitle)" "$(DataPageSubtitle)"
  nsDialogs::Create 1018
  Pop $0
  ${If} $0 == error
    Abort
  ${EndIf}

  ${NSD_CreateLabel} 0 0 100% 10u "$(DataPageText)"
  Pop $0
  ${NSD_CreateLabel} 0 12u 100% 10u "$APPDATA\OpenTermius"
  Pop $0
  ${NSD_CreateLabel} 0 28u 100% 24u "$(DataPageKeep)"
  Pop $0
  ${NSD_CreateCheckbox} 0 58u 100% 12u "$(DataPageCheck)"
  Pop $DataCheckbox
  ${If} $RemoveData == 1
    ${NSD_Check} $DataCheckbox
  ${Else}
    ${NSD_Uncheck} $DataCheckbox
  ${EndIf}

  nsDialogs::Show
FunctionEnd

Function un.DataPageLeave
  ${NSD_GetState} $DataCheckbox $0
  StrCpy $RemoveData 0
  ${If} $0 == ${BST_CHECKED}
    StrCpy $RemoveData 1
  ${EndIf}
FunctionEnd

Section "Uninstall"
  SetShellVarContext current

  Delete "$DESKTOP\OpenTermius.lnk"
  Delete "$SMPROGRAMS\OpenTermius.lnk"
  RMDir "$SMPROGRAMS\OpenTermius"

  Delete "$INSTDIR\${APP_EXE}"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  DeleteRegKey HKCU "${APP_REG_KEY}"

  ${If} $RemoveData == 1
    RMDir /r "$APPDATA\OpenTermius"
  ${EndIf}
SectionEnd
