!macro NSIS_HOOK_PREINSTALL
  ; Tauri's current-user default collides with JobScraper's required data root.
  ; Move only the untouched default; preserve any directory chosen by the user.
  StrCmp $INSTDIR "$LOCALAPPDATA\${PRODUCTNAME}" 0 +2
  StrCpy $INSTDIR "$LOCALAPPDATA\Programs\${PRODUCTNAME}"
  ; The template calls SetOutPath before this hook, so synchronize it after
  ; changing the default directory or the main executable lands in data root.
  SetOutPath $INSTDIR
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; The bundled semiconductor boards are installed either way; this only decides whether they start
  ; switched on, which otherwise means nineteen clicks on the Sources tab before the first update
  ; reads anything. The answer is a file in the app's own data directory rather than a registry
  ; key: the app already owns that directory, reads it on first run, and deletes the file once it
  ; has acted, so nothing lingers to override the user later.
  IfSilent starter_pack_answered
  MessageBox MB_YESNO|MB_ICONQUESTION "Include semiconductor companies?$\r$\n$\r$\nAMD, Analog Devices, Apple, Arm, Broadcom, Cisco, GlobalFoundries, Google, Intel, Marvell, MediaTek, Microchip, Micron, NVIDIA, NXP, Qualcomm, SK hynix, STMicroelectronics and u-blox.$\r$\n$\r$\nChoose No to install them switched off. Either way you can change any of them later on the Sources tab." IDNO starter_pack_answered
  CreateDirectory "$LOCALAPPDATA\${PRODUCTNAME}"
  ClearErrors
  FileOpen $0 "$LOCALAPPDATA\${PRODUCTNAME}\starter-pack.optin" w
  IfErrors starter_pack_answered
  FileWrite $0 "1"
  FileClose $0
  starter_pack_answered:
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
