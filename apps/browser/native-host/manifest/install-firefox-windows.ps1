# FetchX Native Messaging Host — Firefox Installation Script (Windows)
#
# Firefox uses a different registry path and a different extension ID format
# compared to Chrome/Edge.
#
# Usage:
#   .\install-firefox-windows.ps1 [-Uninstall]
#
# The Firefox extension ID is fixed as "FetchX@FetchX.app"
# (set in manifest-firefox/manifest.json).
# No need to pass it as a parameter.

param([switch]$Uninstall)

$HostName    = "com.fetchx.native_host"
$ExtensionId = "FetchX@FetchX.app"
$ManifestDir = "$env:APPDATA\fetchx\native-host"
$ManifestPath = "$ManifestDir\$HostName.json"

$ScriptDir   = Split-Path -Parent $MyInvocation.MyCommand.Definition
$RepoRoot    = Split-Path -Parent (Split-Path -Parent $ScriptDir)
$HostBinary  = Join-Path $RepoRoot "target\release\fetchx-native-host.exe"

# Firefox uses HKCU\Software\Mozilla\NativeMessagingHosts
$FirefoxKey  = "HKCU:\Software\Mozilla\NativeMessagingHosts\$HostName"

if ($Uninstall) {
  Write-Host "Uninstalling Firefox native messaging host..."
  Remove-Item -Path $FirefoxKey -ErrorAction SilentlyContinue
  Remove-Item -Path $ManifestPath -ErrorAction SilentlyContinue
  Write-Host "Done."
  exit 0
}

if (-not (Test-Path $HostBinary)) {
  Write-Error "Native host binary not found at: $HostBinary"
  Write-Error "Build it first: cargo build --release -p fetchx-native-host"
  exit 1
}

New-Item -ItemType Directory -Force -Path $ManifestDir | Out-Null

$Manifest = @{
  name            = $HostName
  description     = "FetchX native messaging host"
  path            = $HostBinary
  type            = "stdio"
  # Firefox uses the extension ID (e.g. "FetchX@FetchX.app")
  # not the chrome-extension:// origin format
  allowed_extensions = @($ExtensionId)
} | ConvertTo-Json -Depth 3

Set-Content -Path $ManifestPath -Value $Manifest -Encoding UTF8
Write-Host "Wrote manifest to: $ManifestPath"

New-Item -Path $FirefoxKey -Force | Out-Null
Set-ItemProperty -Path $FirefoxKey -Name "(Default)" -Value $ManifestPath
Write-Host "Registered Firefox native messaging host: $FirefoxKey"
Write-Host ""
Write-Host "Restart Firefox for changes to take effect."

