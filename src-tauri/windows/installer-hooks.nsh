!macro NSIS_HOOK_PREINSTALL
  ; Tauri's current-user default collides with JobScraper's required data root.
  ; Move only the untouched default; preserve any directory chosen by the user.
  StrCmp $INSTDIR "$LOCALAPPDATA\${PRODUCTNAME}" 0 +2
  StrCpy $INSTDIR "$LOCALAPPDATA\Programs\${PRODUCTNAME}"
  ; The template calls SetOutPath before this hook, so synchronize it after
  ; changing the default directory or the main executable lands in data root.
  SetOutPath $INSTDIR
!macroend
