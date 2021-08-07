$ErrorActionPreference = 'Stop'; # stop on all errors
$toolsDir   = "$(Split-Path -parent $MyInvocation.MyCommand.Definition)"
$url        = 'https://github.com/arturh85/factorio-bot-tauri/releases/download/factorio-bot-v__REPLACE_VERSION__/factorio-bot___REPLACE_VERSION___x64.msi' # download url, HTTPS preferred

$packageArgs = @{
  packageName   = $env:ChocolateyPackageName
  unzipLocation = $toolsDir
  fileType      = 'MSI' #only one of these: exe, msi, msu
  url           = $url
  url64bit      = $url64
  softwareName  = 'factorio-bot*' #part or all of the Display Name as you see it in Programs and Features. It should be enough to be unique

  checksum      = '__REPLACE_CHECKSUM__'
  checksumType  = 'sha256' #default is md5, can also be sha1, sha256 or sha512
  checksum64    = '__REPLACE_CHECKSUM__'
  checksumType64= 'sha256' #default is checksumType

  silentArgs    = "/qn"
  validExitCodes= @(0, 3010, 1641)
}

Install-ChocolateyPackage @packageArgs
