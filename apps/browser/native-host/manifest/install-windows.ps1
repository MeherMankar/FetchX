# FetchX Native Messaging Host & Extension — Windows Installation Script
#
# Usage:
#   .\install-windows.ps1 [-ExtensionId "iemejekblepcfhklcadedfdmgnhjifio"] [-Uninstall]
#
# Registers the Native Messaging Host for Chromium browsers. The extension itself
# must be loaded through the browser UI until FetchX is published in the Chrome Web Store:
#   Google Chrome, Chrome Beta/Dev/Canary, Microsoft Edge, Edge Beta/Dev/Canary,
#   Brave Browser, Brave Beta/Nightly, Vivaldi, Opera, Opera GX, Arc, Yandex, Thorium, Chromium.

param(
    [string]$ExtensionId = "iemejekblepcfhklcadedfdmgnhjifio",
    [switch]$Uninstall
)

$HostName = "com.fetchx.native_host"
$ManifestDir = "$env:APPDATA\fetchx\native-host"
$ManifestPath = "$ManifestDir\$HostName.json"

# Derive paths relative to this script.
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$RepoRoot = (Resolve-Path (Join-Path $ScriptDir "..\..\..\..")).Path

$HostBinaryRelease = Join-Path $RepoRoot "target\release\fetchx-native-host.exe"
$HostBinaryDebug   = Join-Path $RepoRoot "target\debug\fetchx-native-host.exe"
$HostBinary = if (Test-Path $HostBinaryRelease) { $HostBinaryRelease } else { $HostBinaryDebug }

$BrowserKeys = @(
    "Google\Chrome",
    "Google\Chrome Beta",
    "Google\Chrome Dev",
    "Google\Chrome SxS",
    "Chromium",
    "Microsoft\Edge",
    "Microsoft\Edge Beta",
    "Microsoft\Edge Dev",
    "Microsoft\Edge Canary",
    "BraveSoftware\Brave-Browser",
    "BraveSoftware\Brave-Browser-Beta",
    "BraveSoftware\Brave-Browser-Nightly",
    "Vivaldi",
    "Opera Software\Opera Stable",
    "Opera Software\Opera GX Stable",
    "Opera Software\Opera Next",
    "Opera Software\Opera Developer",
    "The Browser Company\Arc",
    "Yandex\YandexBrowser",
    "Thorium"
)

if ($Uninstall) {
    Write-Host "Uninstalling FetchX Native Messaging Host registrations..."
    foreach ($B in $BrowserKeys) {
        Remove-Item -Path "HKCU:\Software\$B\NativeMessagingHosts\$HostName" -Recurse -ErrorAction SilentlyContinue
        Remove-Item -Path "HKCU:\Software\$B\Extensions\$ExtensionId" -Recurse -ErrorAction SilentlyContinue
    }
    Remove-Item -Path $ManifestPath -ErrorAction SilentlyContinue
    Write-Host "Done."
    exit 0
}

# Validate inputs.
if (-not (Test-Path $HostBinary)) {
    Write-Error "Native host binary not found at: $HostBinary"
    Write-Error "Build it first: cargo build -p fetchx-native-host"
    exit 1
}

if ($ExtensionId -notmatch '^[a-z]{32}$') {
    Write-Error "Extension ID must be 32 lowercase letters. Got: $ExtensionId"
    exit 1
}

# Write the manifest JSON.
New-Item -ItemType Directory -Force -Path $ManifestDir | Out-Null
$Manifest = @{
    name             = $HostName
    description      = "FetchX native messaging host"
    path             = $HostBinary
    type             = "stdio"
    allowed_origins  = @("chrome-extension://$ExtensionId/")
} | ConvertTo-Json -Depth 3

Set-Content -Path $ManifestPath -Value $Manifest -Encoding UTF8
Write-Host "Wrote native messaging host manifest to: $ManifestPath"

# Register native messaging hosts and remove invalid local-folder extension registrations.
foreach ($B in $BrowserKeys) {
    $NativeKey = "HKCU:\Software\$B\NativeMessagingHosts\$HostName"
    $ExtKey    = "HKCU:\Software\$B\Extensions\$ExtensionId"

    New-Item -Path $NativeKey -Force | Out-Null
    Set-ItemProperty -Path $NativeKey -Name "(Default)" -Value $ManifestPath
    Remove-Item -Path $ExtKey -Recurse -ErrorAction SilentlyContinue

    Write-Host "Registered browser: $B"
}

Write-Host ""
Write-Host "Native host setup complete. Load the extension from FetchX Settings > Browser extensions."
