Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!include "WinVer.nsh"

Name "Aula Studio"
OutFile "${OUTPUT_FILE}"
InstallDir "$LOCALAPPDATA\Programs\Aula Studio"
InstallDirRegKey HKCU "Software\AulaStudio\Installer" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID zlib
BrandingText "Aula Studio"
VIProductVersion "${APP_VERSION}.0"
VIAddVersionKey /LANG=1033 "ProductName" "Aula Studio"
VIAddVersionKey /LANG=1033 "FileDescription" "Aula Studio Setup"
VIAddVersionKey /LANG=1033 "FileVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=1033 "LegalCopyright" "MIT licensed; see included LICENSE"

!define MUI_ABORTWARNING
!define MUI_ICON "${REPO_ROOT}\assets\app-icon.ico"
!define MUI_UNICON "${REPO_ROOT}\assets\app-icon.ico"
!define MUI_WELCOMEPAGE_TEXT "This wizard installs Aula Studio for your Windows account.$\r$\n$\r$\nChoose an installation folder, then launch the app from the Start menu. Connect your AULA F75 using wired USB."
!define MUI_FINISHPAGE_RUN "$INSTDIR\aula-studio.exe"
!define MUI_FINISHPAGE_RUN_UNCHECKED
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${REPO_ROOT}\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH
!insertmacro MUI_LANGUAGE "English"

Function .onInit
    SetShellVarContext current
    ${IfNot} ${RunningX64}
        MessageBox MB_OK|MB_ICONSTOP "Aula Studio requires 64-bit Windows."
        Abort
    ${EndIf}
    ${IfNot} ${AtLeastWin10}
        MessageBox MB_OK|MB_ICONSTOP "Aula Studio requires Windows 10 or later."
        Abort
    ${EndIf}
    FindWindow $0 "" "Aula Studio | F75"
    ${If} $0 != 0
        MessageBox MB_OK|MB_ICONINFORMATION "Close Aula Studio normally before installing."
        Abort
    ${EndIf}
FunctionEnd

Section "Aula Studio"
    SetShellVarContext current
    !include "${INSTALL_FILES}"
    SetOutPath "$INSTDIR"
    WriteUninstaller "$INSTDIR\Uninstall.exe"
    CreateDirectory "$SMPROGRAMS\Aula Studio"
    CreateShortcut "$SMPROGRAMS\Aula Studio\Aula Studio.lnk" "$INSTDIR\aula-studio.exe" "" "$INSTDIR\aula-studio.exe" 0
    WriteRegStr HKCU "Software\AulaStudio\Installer" "InstallDir" "$INSTDIR"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AulaStudio" "DisplayName" "Aula Studio"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AulaStudio" "DisplayVersion" "${APP_VERSION}"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AulaStudio" "DisplayIcon" "$INSTDIR\aula-studio.exe"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AulaStudio" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AulaStudio" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AulaStudio" "QuietUninstallString" '$\"$INSTDIR\Uninstall.exe$\" /S'
    WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AulaStudio" "NoModify" 1
    WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AulaStudio" "NoRepair" 1
SectionEnd

Function un.onInit
    SetShellVarContext current
    FindWindow $0 "" "Aula Studio | F75"
    ${If} $0 != 0
        MessageBox MB_OK|MB_ICONINFORMATION "Close Aula Studio normally before uninstalling."
        Abort
    ${EndIf}
FunctionEnd

Section "Uninstall"
    SetShellVarContext current
    !include "${UNINSTALL_FILES}"
    Delete "$INSTDIR\Uninstall.exe"
    RMDir "$INSTDIR"
    Delete "$SMPROGRAMS\Aula Studio\Aula Studio.lnk"
    RMDir "$SMPROGRAMS\Aula Studio"
    DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\AulaStudio"
    DeleteRegKey HKCU "Software\AulaStudio\Installer"
    DeleteRegKey /ifempty HKCU "Software\AulaStudio"
    ; Profiles and recovery checkpoints in LOCALAPPDATA\AulaStudio are preserved.
SectionEnd
