!macro NSIS_HOOK_POSTINSTALL
  Call RegisterLomiFileActions
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Call un.RemoveLomiFileActions
!macroend

Function RegisterLomiFileActions
  StrCpy $R9 "$\"$INSTDIR\lomi.exe$\" $\"%1$\""

  ReadRegStr $R8 SHCTX "Software\Classes\Applications\lomi.exe\shell\open\command" ""
  ${If} $R8 == ""
    WriteRegStr SHCTX "Software\Classes\Applications\lomi.exe\shell\open\command" "" "$R9"
  ${EndIf}

  ReadRegStr $R8 SHCTX "Software\Classes\*\shell\dev.lomi.desktop.open\command" ""
  ${If} $R8 == ""
    ReadRegStr $R8 SHCTX "Software\Classes\*\shell\dev.lomi.desktop.open" "MUIVerb"
    ${If} $R8 == ""
      WriteRegStr SHCTX "Software\Classes\*\shell\dev.lomi.desktop.open" "MUIVerb" "Open with Lomi"
      WriteRegStr SHCTX "Software\Classes\*\shell\dev.lomi.desktop.open" "Icon" "$INSTDIR\lomi.exe,0"
      WriteRegStr SHCTX "Software\Classes\*\shell\dev.lomi.desktop.open\command" "" "$R9"
    ${EndIf}
  ${ElseIf} $R8 == $R9
    WriteRegStr SHCTX "Software\Classes\*\shell\dev.lomi.desktop.open" "MUIVerb" "Open with Lomi"
    WriteRegStr SHCTX "Software\Classes\*\shell\dev.lomi.desktop.open" "Icon" "$INSTDIR\lomi.exe,0"
  ${EndIf}
FunctionEnd

Function un.RemoveLomiFileActions
  StrCpy $R9 "$\"$INSTDIR\lomi.exe$\" $\"%1$\""

  ReadRegStr $R8 SHCTX "Software\Classes\Applications\lomi.exe\shell\open\command" ""
  ${If} $R8 == $R9
    DeleteRegValue SHCTX "Software\Classes\Applications\lomi.exe\shell\open\command" ""
    DeleteRegKey /ifempty SHCTX "Software\Classes\Applications\lomi.exe\shell\open\command"
    DeleteRegKey /ifempty SHCTX "Software\Classes\Applications\lomi.exe\shell\open"
    DeleteRegKey /ifempty SHCTX "Software\Classes\Applications\lomi.exe\shell"
    DeleteRegKey /ifempty SHCTX "Software\Classes\Applications\lomi.exe"
    DeleteRegKey /ifempty SHCTX "Software\Classes\Applications"
  ${EndIf}

  ReadRegStr $R8 SHCTX "Software\Classes\*\shell\dev.lomi.desktop.open\command" ""
  ${If} $R8 == $R9
    DeleteRegValue SHCTX "Software\Classes\*\shell\dev.lomi.desktop.open\command" ""
    DeleteRegKey /ifempty SHCTX "Software\Classes\*\shell\dev.lomi.desktop.open\command"

    ReadRegStr $R8 SHCTX "Software\Classes\*\shell\dev.lomi.desktop.open" "MUIVerb"
    ${If} $R8 == "Open with Lomi"
      DeleteRegValue SHCTX "Software\Classes\*\shell\dev.lomi.desktop.open" "MUIVerb"
    ${EndIf}
    ReadRegStr $R8 SHCTX "Software\Classes\*\shell\dev.lomi.desktop.open" "Icon"
    ${If} $R8 == "$INSTDIR\lomi.exe,0"
      DeleteRegValue SHCTX "Software\Classes\*\shell\dev.lomi.desktop.open" "Icon"
    ${EndIf}
    DeleteRegKey /ifempty SHCTX "Software\Classes\*\shell\dev.lomi.desktop.open"
    DeleteRegKey /ifempty SHCTX "Software\Classes\*\shell"
    DeleteRegKey /ifempty SHCTX "Software\Classes\*"
  ${EndIf}
FunctionEnd
