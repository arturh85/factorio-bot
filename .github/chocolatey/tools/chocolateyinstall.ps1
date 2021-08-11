$ErrorActionPreference = 'Stop'; # stop on all errors
$toolsDir   = "$(Split-Path -parent $MyInvocation.MyCommand.Definition)"
$url        = 'https://github.com/arturh85/factorio-bot-tauri/releases/download/factorio-bot-v__REPLACE_VERSION__/factorio-bot-installer.exe'

$packageArgs = @{
  packageName   = $env:ChocolateyPackageName
  unzipLocation = $toolsDir
  fileType      = 'EXE'
  url           = $url
  softwareName  = 'factorio-bot*'
  checksum      = '__REPLACE_CHECKSUM__'
  checksumType  = 'sha256'
  silentArgs     = '/S'
  validExitCodes= @(0, 3010)
}

Install-ChocolateyPackage @packageArgs
