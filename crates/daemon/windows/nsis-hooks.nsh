; aw-manual NSIS installer hooks: put the installation folder, which contains
; the CLI (aw-manual.exe), on the user's (or, for per-machine installs, the
; system's) PATH, and remove it again on uninstall.

!include "LogicLib.nsh"
!include "WinMessages.nsh"
${UnStrRep}

!macro NSIS_HOOK_POSTINSTALL
  !if "${INSTALLMODE}" == "perMachine"
    ReadRegStr $R0 HKLM "SYSTEM\CurrentControlSet\Control\Session Manager\Environment" "Path"
  !else
    ReadRegStr $R0 HKCU "Environment" "Path"
  !endif
  ${StrLoc} $R1 "$R0;" "$INSTDIR;" ">"
  ${If} $R1 == ""
    ${If} $R0 == ""
      StrCpy $R0 "$INSTDIR"
    ${Else}
      StrCpy $R0 "$R0;$INSTDIR"
    ${EndIf}
    !if "${INSTALLMODE}" == "perMachine"
      WriteRegExpandStr HKLM "SYSTEM\CurrentControlSet\Control\Session Manager\Environment" "Path" "$R0"
    !else
      WriteRegExpandStr HKCU "Environment" "Path" "$R0"
    !endif
    SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  !if "${INSTALLMODE}" == "perMachine"
    ReadRegStr $R0 HKLM "SYSTEM\CurrentControlSet\Control\Session Manager\Environment" "Path"
  !else
    ReadRegStr $R0 HKCU "Environment" "Path"
  !endif
  ${If} $R0 != ""
    ${UnStrRep} $R1 "$R0" ";$INSTDIR" ""
    ${UnStrRep} $R1 "$R1" "$INSTDIR;" ""
    ${If} $R1 == "$INSTDIR"
      StrCpy $R1 ""
    ${EndIf}
    ${If} $R1 != $R0
      !if "${INSTALLMODE}" == "perMachine"
        WriteRegExpandStr HKLM "SYSTEM\CurrentControlSet\Control\Session Manager\Environment" "Path" "$R1"
      !else
        WriteRegExpandStr HKCU "Environment" "Path" "$R1"
      !endif
      SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
    ${EndIf}
  ${EndIf}
!macroend
