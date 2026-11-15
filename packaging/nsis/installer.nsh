; NSIS installer/uninstaller hooks for Hybrid Player (Rust/Windows)
; Parity source: `hybrid-player/electron-builder.json` & `hybrid-player/src/main/windows-shell-integration.js`
; AppUserModelID: com.hybridplayer.app
; Matches `electron-builder` NSIS flags:
;   oneClick = false
;   allowToChangeInstallationDirectory = true
;   createDesktopShortcut = true
;   createStartMenuShortcut = true

!define HYBRID_APP_ID "com.hybridplayer.app"
!define HYBRID_PRODUCT_NAME "Hybrid Player"
!define HYBRID_CAP_ROOT "Software\HybridPlayer\Capabilities"
!define HYBRID_REG_APPS "Software\RegisteredApplications"

!macro HYBRID_REGISTER_EXT EXT DESC
  WriteRegStr SHCTX "Software\Classes\HybridPlayer.${EXT}" "" "${DESC}"
  WriteRegStr SHCTX "Software\Classes\HybridPlayer.${EXT}\DefaultIcon" "" "$INSTDIR\resources\assets\icons\video.ico,0"
  WriteRegStr SHCTX "Software\Classes\HybridPlayer.${EXT}\shell\open\command" "" '"$INSTDIR\Hybrid Player.exe" "%1"'
  WriteRegStr SHCTX "Software\Classes\.${EXT}\OpenWithProgids" "HybridPlayer.${EXT}" ""
  WriteRegStr SHCTX "${HYBRID_CAP_ROOT}\FileAssociations" ".${EXT}" "HybridPlayer.${EXT}"
!macroend

!macro HYBRID_UNREGISTER_EXT EXT
  DeleteRegValue SHCTX "Software\Classes\.${EXT}\OpenWithProgids" "HybridPlayer.${EXT}"
  DeleteRegKey SHCTX "Software\Classes\HybridPlayer.${EXT}"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Register AUMID in HKCU so Windows 11 SMTC & taskbar pinning resolve `com.hybridplayer.app`.
  WriteRegStr HKCU "Software\Classes\AppUserModelId\${HYBRID_APP_ID}" "DisplayName" "${HYBRID_PRODUCT_NAME}"
  WriteRegStr HKCU "Software\Classes\AppUserModelId\${HYBRID_APP_ID}" "IconUri" "$INSTDIR\resources\assets\icons\icon.ico"

  ; Register Capabilities + all 10 file associations matching `electron-builder.json`.
  WriteRegStr SHCTX "${HYBRID_CAP_ROOT}" "ApplicationName" "${HYBRID_PRODUCT_NAME}"
  WriteRegStr SHCTX "${HYBRID_CAP_ROOT}" "ApplicationDescription" "Next-generation desktop video player."
  WriteRegStr SHCTX "${HYBRID_REG_APPS}" "${HYBRID_PRODUCT_NAME}" "${HYBRID_CAP_ROOT}"

  !insertmacro HYBRID_REGISTER_EXT "mp4" "MP4 Video"
  !insertmacro HYBRID_REGISTER_EXT "mkv" "MKV Video"
  !insertmacro HYBRID_REGISTER_EXT "webm" "WebM Video"
  !insertmacro HYBRID_REGISTER_EXT "avi" "AVI Video"
  !insertmacro HYBRID_REGISTER_EXT "mov" "QuickTime Video"
  !insertmacro HYBRID_REGISTER_EXT "mp3" "MP3 Audio"
  !insertmacro HYBRID_REGISTER_EXT "flac" "FLAC Audio"
  !insertmacro HYBRID_REGISTER_EXT "wav" "WAV Audio"
  !insertmacro HYBRID_REGISTER_EXT "ogg" "OGG Audio"
  !insertmacro HYBRID_REGISTER_EXT "m4a" "M4A Audio"

  ; Desktop & Start Menu shortcuts (also tagged with `com.hybridplayer.app` by `hybrid_win32_shell::create_start_menu_shortcut` at runtime).
  CreateShortCut "$DESKTOP\${HYBRID_PRODUCT_NAME}.lnk" "$INSTDIR\Hybrid Player.exe" "" "$INSTDIR\resources\assets\icons\icon.ico" 0
  CreateDirectory "$SMPROGRAMS\${HYBRID_PRODUCT_NAME}"
  CreateShortCut "$SMPROGRAMS\${HYBRID_PRODUCT_NAME}\${HYBRID_PRODUCT_NAME}.lnk" "$INSTDIR\Hybrid Player.exe" "" "$INSTDIR\resources\assets\icons\icon.ico" 0

  ; Notify Explorer of updated file associations.
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0x0000, p 0, p 0)'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  Delete "$DESKTOP\${HYBRID_PRODUCT_NAME}.lnk"
  Delete "$SMPROGRAMS\${HYBRID_PRODUCT_NAME}\${HYBRID_PRODUCT_NAME}.lnk"
  RMDir "$SMPROGRAMS\${HYBRID_PRODUCT_NAME}"

  !insertmacro HYBRID_UNREGISTER_EXT "mp4"
  !insertmacro HYBRID_UNREGISTER_EXT "mkv"
  !insertmacro HYBRID_UNREGISTER_EXT "webm"
  !insertmacro HYBRID_UNREGISTER_EXT "avi"
  !insertmacro HYBRID_UNREGISTER_EXT "mov"
  !insertmacro HYBRID_UNREGISTER_EXT "mp3"
  !insertmacro HYBRID_UNREGISTER_EXT "flac"
  !insertmacro HYBRID_UNREGISTER_EXT "wav"
  !insertmacro HYBRID_UNREGISTER_EXT "ogg"
  !insertmacro HYBRID_UNREGISTER_EXT "m4a"

  DeleteRegValue SHCTX "${HYBRID_REG_APPS}" "${HYBRID_PRODUCT_NAME}"
  DeleteRegKey SHCTX "${HYBRID_CAP_ROOT}"
  DeleteRegKey HKCU "Software\Classes\AppUserModelId\${HYBRID_APP_ID}"

  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0x0000, p 0, p 0)'
!macroend
