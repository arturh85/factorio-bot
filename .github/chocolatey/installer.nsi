OutFile "factorio-bot-installer.exe"
InstallDir $PROGRAMFILES64\factorio-bot
Section
    SetOutPath $INSTDIR
    File factorio-bot.exe
    WriteUninstaller $INSTDIR\uninstaller.exe

    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\factorio-bot" "DisplayName" "factorio-bot"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\factorio-bot" "UninstallString" "$INSTDIR\uninstaller.exe"

    CreateShortCut $SMPROGRAMS\factorio-bot.lnk $INSTDIR\factorio-bot.exe
SectionEnd

Section "Uninstall"
    Delete $INSTDIR\uninstaller.exe
    Delete $INSTDIR\factorio-bot.exe
    Delete $SMPROGRAMS\factorio-bot.lnk
    DeleteRegKey HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\factorio-bot"
    RMDir $INSTDIR
SectionEnd