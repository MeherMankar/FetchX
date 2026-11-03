# FetchX Native Messaging Host & Extension — Firefox & Gecko Installation Script (Windows)
#
# Usage:
#   .\install-firefox-windows.ps1 [-Uninstall]
#
# Registers the Native Messaging Host for Mozilla Firefox, LibreWolf, Waterfox, and Floorp.
# The extension must be loaded through the browser UI; a permanent Firefox add-on
# also requires a signed release:
#   HKCU\Software\Mozilla\NativeMessagingHosts\com.fetchx.native_host
#   HKCU\Software\Mozilla\Firefox\Extensions ("FetchX@FetchX.app" = path)
#   HKCU\Software\LibreWolf\NativeMessagingHosts\com.fetchx.native_host
#   HKCU\Software\LibreWolf\Extensions ("FetchX@FetchX.app" = path)
#   HKCU\Software\Waterfox\NativeMessagingHosts\com.fetchx.native_host
#   HKCU\Software\Waterfox\Extensions ("FetchX@FetchX.app" = path)
#   HKCU\Software\Floorp\NativeMessagingHosts\com.fetchx.native_host
#   HKCU\Software\Floorp\Extensions ("FetchX@FetchX.app" = path)

param([switch]$Uninstall)

$HostName    = "com.fetchx.native_host"
$ExtensionId = "FetchX@FetchX.app"
$ManifestDir = "$env:APPDATA\fetchx\native-host"
$ManifestPath = "$ManifestDir\$HostName.firefox.json"

$ScriptDir   = Split-Path -Parent $MyInvocation.MyCommand.Definition
$RepoRoot    = (Resolve-Path (Join-Path $ScriptDir "..\..\..\..")).Path

$HostBinaryRelease = Join-Path $RepoRoot "target\release\fetchx-native-host.exe"
$HostBinaryDebug   = Join-Path $RepoRoot "target\debug\fetchx-native-host.exe"
$HostBinary = if (Test-Path $HostBinaryRelease) { $HostBinaryRelease } else { $HostBinaryDebug }

$GeckoBrowsers = @(
    @{ Name = "Mozilla Firefox"; NativeKey = "HKCU:\Software\Mozilla\NativeMessagingHosts\$HostName"; ExtKey = "HKCU:\Software\Mozilla\Firefox\Extensions" },
    @{ Name = "LibreWolf";       NativeKey = "HKCU:\Software\LibreWolf\NativeMessagingHosts\$HostName"; ExtKey = "HKCU:\Software\LibreWolf\Extensions" },
    @{ Name = "Waterfox";        NativeKey = "HKCU:\Software\Waterfox\NativeMessagingHosts\$HostName"; ExtKey = "HKCU:\Software\Waterfox\Extensions" },
    @{ Name = "Floorp";          NativeKey = "HKCU:\Software\Floorp\NativeMessagingHosts\$HostName"; ExtKey = "HKCU:\Software\Floorp\Extensions" }
)

if ($Uninstall) {
  Write-Host "Uninstalling Firefox & Gecko native messaging hosts..."
  foreach ($G in $GeckoBrowsers) {
    Remove-Item -Path $G.NativeKey -Recurse -ErrorAction SilentlyContinue
    Remove-ItemProperty -Path $G.ExtKey -Name $ExtensionId -ErrorAction SilentlyContinue
  }
  Remove-Item -Path $ManifestPath -ErrorAction SilentlyContinue
  Write-Host "Done."
  exit 0
}

if (-not (Test-Path $HostBinary)) {
  Write-Error "Native host binary not found at: $HostBinary"
  Write-Error "Build it first: cargo build -p fetchx-native-host"
  exit 1
}

New-Item -ItemType Directory -Force -Path $ManifestDir | Out-Null

$Manifest = @{
  name            = $HostName
  description     = "FetchX native messaging host"
  path            = $HostBinary
  type            = "stdio"
  allowed_extensions = @($ExtensionId)
} | ConvertTo-Json -Depth 3

Set-Content -Path $ManifestPath -Value $Manifest -Encoding UTF8
Write-Host "Wrote manifest to: $ManifestPath"

# Register native messaging hosts and remove invalid local-folder extension registrations.
foreach ($G in $GeckoBrowsers) {
  New-Item -Path $G.NativeKey -Force | Out-Null
  Set-ItemProperty -Path $G.NativeKey -Name "(Default)" -Value $ManifestPath
  Remove-ItemProperty -Path $G.ExtKey -Name $ExtensionId -ErrorAction SilentlyContinue
  Write-Host "Registered $($G.Name)"
}

Write-Host ""
Write-Host "Native host setup complete. Load the extension from FetchX Settings > Browser extensions."
