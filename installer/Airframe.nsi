Unicode true
Name "Airframe"
Caption "Airframe Setup"
OutFile "${SETUP_EXE}"
InstallDir "$LOCALAPPDATA\Airframe"
RequestExecutionLevel user
SetCompressor /SOLID lzma
ShowInstDetails nevershow
ShowUninstDetails nevershow
AutoCloseWindow true

!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "x64.nsh"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Airframe"

Page instfiles
UninstPage uninstConfirm
UninstPage instfiles

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "Airframe requires 64-bit Windows." /SD IDOK
    SetErrorLevel 1
    Quit
  ${EndIf}
  SetRegView 64
  SetShellVarContext current
FunctionEnd

Section "Airframe"
  DetailPrint "Installing Airframe"
  SetOutPath "$INSTDIR"
  ClearErrors
  File /oname=Airframe.new "${APP_EXE}"
  IfErrors failed

  ; Replace the executable only after setup has extracted the entire new file.
  Delete "$INSTDIR\Airframe.previous"
  IfFileExists "$INSTDIR\Airframe.exe" 0 replace
  ClearErrors
  Rename "$INSTDIR\Airframe.exe" "$INSTDIR\Airframe.previous"
  IfErrors in_use

  replace:
  ClearErrors
  Rename "$INSTDIR\Airframe.new" "$INSTDIR\Airframe.exe"
  IfErrors rollback
  Delete "$INSTDIR\Airframe.previous"

  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\Airframe.lnk" "$INSTDIR\Airframe.exe"
  CreateShortcut "$DESKTOP\Airframe.lnk" "$INSTDIR\Airframe.exe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "Airframe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "Airframe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\Airframe.exe"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1

  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "/UPDATE" $1
  IfErrors 0 done
  IfSilent done
  ExecShell "open" "$INSTDIR\Airframe.exe"
  Goto done

  in_use:
  MessageBox MB_ICONEXCLAMATION "Close all Airframe windows and run setup again." /SD IDOK
  Goto failed
  rollback:
  Rename "$INSTDIR\Airframe.previous" "$INSTDIR\Airframe.exe"
  failed:
  Delete "$INSTDIR\Airframe.new"
  SetErrorLevel 1
  Abort "Airframe setup could not complete."
  done:
SectionEnd

Section "Uninstall"
  SetRegView 64
  SetShellVarContext current
  Delete "$SMPROGRAMS\Airframe.lnk"
  Delete "$DESKTOP\Airframe.lnk"
  Delete "$INSTDIR\Airframe.exe"
  Delete "$INSTDIR\Airframe.new"
  Delete "$INSTDIR\Airframe.previous"
  Delete "$INSTDIR\Uninstall.exe"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  RMDir "$INSTDIR"
SectionEnd
