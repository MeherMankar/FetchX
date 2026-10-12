# build-android-native.ps1
# Cross-compiles Rust native libraries for Android using cargo-ndk.
#
# Prerequisites:
#   - cargo-ndk installed (cargo install cargo-ndk)
#   - Android NDK installed at D:\Android\Sdk\ndk\<NDK_VERSION>
#   - Rust Android targets: aarch64-linux-android, x86_64-linux-android
#
# Usage:
#   .\scripts\build-android-native.ps1           # debug build
#   .\scripts\build-android-native.ps1 -Release  # release build

param([switch]$Release)

$ErrorActionPreference = "Stop"

$NdkVersion = "27.2.12479018"  # update if NDK version changes
$env:ANDROID_NDK_HOME = "D:\Android\Sdk\ndk\$NdkVersion"
$env:ANDROID_NDK_ROOT = "D:\Android\Sdk\ndk\$NdkVersion"
$env:JAVA_HOME = "C:\Program Files\Eclipse Adoptium\jdk-17.0.20.101-hotspot"

$Profile = if ($Release) { "--release" } else { "" }
$OutDir = "apps/android/bridge/src/main/jniLibs"
$Crate  = "mydownloader-android-bridge"

Write-Host "[Phase 20C] Building arm64-v8a (release=$Release)..."
$arm64Cmd = "cargo ndk -t arm64-v8a -o $OutDir build $Profile -p $Crate"
Write-Host $arm64Cmd
Invoke-Expression $arm64Cmd
if ($LASTEXITCODE -ne 0) { throw "arm64-v8a build failed" }

Write-Host "[Phase 20C] Building x86_64 (release=$Release)..."
$x64Cmd = "cargo ndk -t x86_64 -o $OutDir build $Profile -p $Crate"
Write-Host $x64Cmd
Invoke-Expression $x64Cmd
if ($LASTEXITCODE -ne 0) { Write-Warning "x86_64 build failed (non-critical)" }

Write-Host ""
Write-Host "[Phase 20C] Done. Libraries:"
Get-ChildItem -Recurse $OutDir -Include "*.so" | Select-Object FullName, Length
