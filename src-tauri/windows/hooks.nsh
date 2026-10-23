; Registers Aditus as a browser so it shows in Windows' default apps list (per user).
!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr HKCU "Software\Classes\AditusURL" "" "Aditus URL"
  WriteRegStr HKCU "Software\Classes\AditusURL" "URL Protocol" ""
  WriteRegStr HKCU "Software\Classes\AditusURL\shell\open\command" "" '"$INSTDIR\${MAINBINARYNAME}.exe" "%1"'
  WriteRegStr HKCU "Software\Clients\StartMenuInternet\Aditus" "" "Aditus"
  WriteRegStr HKCU "Software\Clients\StartMenuInternet\Aditus\Capabilities" "ApplicationName" "Aditus"
  WriteRegStr HKCU "Software\Clients\StartMenuInternet\Aditus\Capabilities" "ApplicationDescription" "Open the way."
  WriteRegStr HKCU "Software\Clients\StartMenuInternet\Aditus\Capabilities\URLAssociations" "http" "AditusURL"
  WriteRegStr HKCU "Software\Clients\StartMenuInternet\Aditus\Capabilities\URLAssociations" "https" "AditusURL"
  WriteRegStr HKCU "Software\Clients\StartMenuInternet\Aditus\shell\open\command" "" '"$INSTDIR\${MAINBINARYNAME}.exe"'
  WriteRegStr HKCU "Software\RegisteredApplications" "Aditus" "Software\Clients\StartMenuInternet\Aditus\Capabilities"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  DeleteRegKey HKCU "Software\Classes\AditusURL"
  DeleteRegKey HKCU "Software\Clients\StartMenuInternet\Aditus"
  DeleteRegValue HKCU "Software\RegisteredApplications" "Aditus"
!macroend
