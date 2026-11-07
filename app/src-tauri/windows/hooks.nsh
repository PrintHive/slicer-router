; Keep Slicer Router listed under "Open with" and in Default apps for .3mf even
; after a slicer rewrites the .3mf default value, which they do each time they
; start unless their "Associate .3mf files" option is off.
;
; The ProgId must match the fileAssociations name in tauri.windows.conf.json.

!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr SHCTX "Software\Classes\.3mf\OpenWithProgids" "PrintHive.SlicerRouter.3mf" ""
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DeleteRegValue SHCTX "Software\Classes\.3mf\OpenWithProgids" "PrintHive.SlicerRouter.3mf"
!macroend
