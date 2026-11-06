!macro NSIS_HOOK_PREINSTALL
  ; Tauri's current-user default collides with JobScraper's required data root.
  ; Move only the untouched default; preserve any directory chosen by the user.
  StrCmp $INSTDIR "$LOCALAPPDATA\${PRODUCTNAME}" 0 +2
  StrCpy $INSTDIR "$LOCALAPPDATA\Programs\${PRODUCTNAME}"
  ; The template calls SetOutPath before this hook, so synchronize it after
  ; changing the default directory or the main executable lands in data root.
  SetOutPath $INSTDIR
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Remove opt-in tasks before their executable disappears -- but only on a real uninstall.
  ; Installing over an existing copy runs this same uninstaller first, and the executable
  ; survives that, so deleting regardless switched "Start JobScraper when I sign in" off on
  ; every upgrade, with the checkbox reading its own absent task as "off" and nobody told.
  ; A real uninstall is the one NSIS relocates to %TEMP% before running; the installer is the
  ; only caller that passes "_?=", which keeps the uninstaller in the installation folder.
  ; /UPDATE cannot stand in for it -- the installer forwards that only when the updater
  ; launched it, never on a hand-run reinstall -- and GetOptions never sees "_?=" at all,
  ; because NSIS strips it from the parameters before the script can read them.
  ${If} $EXEDIR != $INSTDIR
    nsExec::ExecToLog '"$SYSDIR\schtasks.exe" /Delete /F /TN "\JobScraper\JobScraper-Sync"'
    nsExec::ExecToLog '"$SYSDIR\schtasks.exe" /Delete /F /TN "\JobScraper\JobScraper-Startup"'
  ${EndIf}
!macroend
