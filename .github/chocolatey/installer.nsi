OutFile "factorio-bot-installer.exe"
InstallDir $PROGRAMFILES\factorio-bot
Section
    SetOutPath $INSTDIR
    File factorio-bot.exe
    WriteUninstaller $INSTDIR\uninstaller.exe
SectionEnd

Section "Uninstall"
    Delete $INSTDIR\uninstaller.exe
    Delete $INSTDIR\factorio-bot.exe
    RMDir $INSTDIR
SectionEnd