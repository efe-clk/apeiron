; The Windows installer. Built with:
;   makensis /DVERSION=<version> /DOUT=<file> packaging\windows.nsi
; from the root of the repository, after a release build and make_icons.py
; into target\icons.
Unicode true
Name "Apeiron ${VERSION}"
OutFile "${OUT}"
InstallDir "$PROGRAMFILES64\Apeiron"
RequestExecutionLevel admin
Icon "..\target\icons\apeiron.ico"
UninstallIcon "..\target\icons\apeiron.ico"
SetCompressor /SOLID lzma

!include "MUI2.nsh"
!define MUI_ICON "..\target\icons\apeiron.ico"
!define MUI_UNICON "..\target\icons\apeiron.ico"
!insertmacro MUI_PAGE_LICENSE "..\LICENSE-MIT"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "Turkish"

Section
  SetOutPath "$INSTDIR"
  File "..\target\release\apeiron.exe"
  File "..\target\release\apeiron-desktop.exe"
  File "..\target\icons\apeiron.ico"
  File "..\LICENSE-MIT"
  File "..\LICENSE-APACHE"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateShortcut "$SMPROGRAMS\Apeiron.lnk" "$INSTDIR\apeiron-desktop.exe" "" "$INSTDIR\apeiron.ico"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Apeiron" "DisplayName" "Apeiron"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Apeiron" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Apeiron" "DisplayIcon" "$INSTDIR\apeiron.ico"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Apeiron" "UninstallString" '"$INSTDIR\uninstall.exe"'
SectionEnd

; Removes the program only; repositories and settings belong to the user.
Section "Uninstall"
  Delete "$SMPROGRAMS\Apeiron.lnk"
  Delete "$INSTDIR\apeiron.exe"
  Delete "$INSTDIR\apeiron-desktop.exe"
  Delete "$INSTDIR\apeiron.ico"
  Delete "$INSTDIR\LICENSE-MIT"
  Delete "$INSTDIR\LICENSE-APACHE"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\Apeiron"
SectionEnd
