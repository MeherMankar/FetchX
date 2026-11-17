# FetchX Browser Extension — Developer Mode Loader
#
# Run this script to find and open the built extension folder so you can
# load it in Chrome / Edge / Brave / Vivaldi in developer mode.
#
# Usage:
#   .\load-dev-extension.ps1              # Chromium (default)
#   .\load-dev-extension.ps1 -Firefox     # Firefox (opens dist-firefox)
#
# Prerequisites: run `npm run build` in this directory first.

param([switch]$Firefox)

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition

if ($Firefox) {
    $ExtDir = Join-Path $ScriptDir "dist-firefox"
    $Browser = "Firefox"
} else {
    $ExtDir = Join-Path $ScriptDir "dist-chromium"
    $Browser = "Chromium"
}

if (-not (Test-Path $ExtDir)) {
    Write-Error "Extension not built yet. Run: npm run build"
    Write-Error "Expected folder: $ExtDir"
    exit 1
}

Write-Host ""
Write-Host "FetchX Extension — $Browser developer mode" -ForegroundColor Cyan
Write-Host "Extension folder: $ExtDir" -ForegroundColor Green
Write-Host ""

if ($Firefox) {
    Write-Host "Firefox steps:" -ForegroundColor Yellow
    Write-Host "  1. Open Firefox and go to: about:debugging#/runtime/this-firefox"
    Write-Host "  2. Click 'Load Temporary Add-on...'"
    Write-Host "  3. Navigate to the folder below and select manifest.json"
    Write-Host "     $ExtDir\manifest.json"
} else {
    Write-Host "Chrome / Edge / Brave / Vivaldi steps:" -ForegroundColor Yellow
    Write-Host "  1. Open your browser and go to the extensions page:"
    Write-Host "       Chrome:  chrome://extensions"
    Write-Host "       Edge:    edge://extensions"
    Write-Host "       Brave:   brave://extensions"
    Write-Host "       Vivaldi: vivaldi://extensions"
    Write-Host "  2. Enable 'Developer mode' (toggle in the top-right corner)"
    Write-Host "  3. Click 'Load unpacked'"
    Write-Host "  4. Select this folder:"
    Write-Host "     $ExtDir" -ForegroundColor Green
}

Write-Host ""
Write-Host "Opening the folder in Explorer..." -ForegroundColor Gray
Start-Process explorer.exe $ExtDir

Write-Host ""
Write-Host "After loading, also run the native host setup:" -ForegroundColor Yellow
Write-Host "  .\apps\browser\native-host\manifest\install-windows.ps1"
Write-Host ""
