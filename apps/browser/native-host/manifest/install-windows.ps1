# FetchX Native Messaging Host — Windows Installation Script
#
# Run this script after:
#   1. Building the native host:  cargo build --release -p fetchx-native-host
#   2. Loading the unpacked extension in Chrome to get your extension ID.
#
# Usage:
#   .\install-windows.ps1 -ExtensionId "abcdefghijklmnopabcdefghijklmnop" [-Uninstall]
#
# The script writes the native messaging host manifest to the Windows Registry
# key that Chrome/Edge reads:
#   HKCU\Software\Google\Chrome\NativeMessagingHosts\com.fetchx.native_host
#   HKCU\Software\Microsoft\Edge\NativeMessagingHosts\com.fetchx.native_host

param(
    [Parameter(Mandatory=$true)]
    [string]$ExtensionId,

    [switch]$Uninstall
)

$HostName = "com.fetchx.native_host"
$ManifestDir = "$env:APPDATA\fetchx\native-host"
$ManifestPath = "$ManifestDir\$HostName.json"

# Derive the native host binary path relative to this script.
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$RepoRoot = (Resolve-Path (Join-Path $ScriptDir "..\..\..\..")).Path
$HostBinary = Join-Path $RepoRoot "target\release\fetchx-native-host.exe"

$ChromeKey = "HKCU:\Software\Google\Chrome\NativeMessagingHosts\$HostName"
$EdgeKey   = "HKCU:\Software\Microsoft\Edge\NativeMessagingHosts\$HostName"

if ($Uninstall) {
    Write-Host "Uninstalling native messaging host..."
    Remove-Item -Path $ChromeKey -ErrorAction SilentlyContinue
    Remove-Item -Path $EdgeKey   -ErrorAction SilentlyContinue
    Remove-Item -Path $ManifestPath -ErrorAction SilentlyContinue
    Write-Host "Done."
    exit 0
}

# Validate inputs.
if (-not (Test-Path $HostBinary)) {
    Write-Error "Native host binary not found at: $HostBinary"
    Write-Error "Build it first: cargo build --release -p fetchx-native-host"
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
Write-Host "Wrote manifest to: $ManifestPath"

# Register in the Windows Registry for Chrome and Edge.
foreach ($Key in @($ChromeKey, $EdgeKey)) {
    New-Item -Path $Key -Force | Out-Null
    Set-ItemProperty -Path $Key -Name "(Default)" -Value $ManifestPath
    Write-Host "Registered: $Key"
}

Write-Host ""
Write-Host "Installation complete."
Write-Host "Restart Chrome/Edge for changes to take effect."
