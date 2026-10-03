param(
  [Parameter(Mandatory = $true)][string]$Installer,
  [Parameter(Mandatory = $true)][string]$Executable
)
$ErrorActionPreference = 'Stop'
$directory = Split-Path -Parent $Executable
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Prometeu'
$existing = Get-ItemProperty $key -ErrorAction SilentlyContinue
if ($existing -and $existing.InstallLocation.Trim('"').TrimEnd('\') -ne $directory.TrimEnd('\')) {
  throw 'An existing Prometeu installation uses a different directory'
}
if (Get-Process prometeu-wsl-desktop -ErrorAction SilentlyContinue) {
  throw 'Close the native Prometeu window before installing'
}
$process = Start-Process -FilePath $Installer -ArgumentList @('/S', "/D=$directory") -PassThru
if (-not $process.WaitForExit(120000)) { throw 'Installer did not finish within two minutes' }
if ($process.ExitCode -ne 0) { throw "Installer failed with $($process.ExitCode)" }
if (-not (Test-Path -LiteralPath $Executable)) { throw 'Installed executable is missing' }
$registration = Get-ItemProperty $key
if ($registration.InstallLocation.Trim('"').TrimEnd('\') -ne $directory.TrimEnd('\')) {
  throw 'Installer registered an unexpected location'
}
$shortcutPath = Join-Path ([Environment]::GetFolderPath('Programs')) 'Prometeu.lnk'
if (-not (Test-Path -LiteralPath $shortcutPath)) { throw 'Start menu shortcut is missing' }
$shortcut = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcutPath)
if ($shortcut.TargetPath -ne $Executable) { throw 'Start menu shortcut targets a different executable' }
Write-Output "Installed Prometeu $($registration.DisplayVersion) at $Executable with a Start menu shortcut"
