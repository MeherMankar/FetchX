# build-android-native.ps1
# Cross-compiles Rust native libraries for Android using cargo-ndk.
#
# Prerequisites:
#   - cargo-ndk installed (cargo install cargo-ndk)
#   - Android NDK installed at D:\Android\Sdk\ndk\<NDK_VERSION>
#   - Rust Android targets: aarch64-linux-android, x86_64-linux-android
#
# Usage:
#   .\scripts\build-android-native.ps1           # optimized release build
#   .\scripts\build-android-native.ps1 -Debug    # debug build with symbols
#   .\scripts\build-android-native.ps1 -Release  # explicit optimized release build

param(
    [switch]$Release,
    [switch]$Debug
)

$ErrorActionPreference = "Stop"

if ($Release -and $Debug) {
    throw "Choose either -Release or -Debug, not both."
}

$NdkVersion = "27.2.12479018"  # update if NDK version changes
$env:ANDROID_NDK_HOME = "D:\Android\Sdk\ndk\$NdkVersion"
$env:ANDROID_NDK_ROOT = "D:\Android\Sdk\ndk\$NdkVersion"
$env:JAVA_HOME = "C:\Program Files\Eclipse Adoptium\jdk-17.0.20.101-hotspot"

$Profile = if ($Debug) { "" } else { "--release" }
$ProfileDirectory = if ($Debug) { "debug" } else { "release" }
$IsReleaseBuild = -not $Debug
$OutDir = "apps/android/bridge/src/main/jniLibs"
$Crate  = "fetchx-android-bridge"

Write-Host "[Phase 20C] Building arm64-v8a (release=$IsReleaseBuild)..."
$arm64Cmd = "cargo ndk -t arm64-v8a -o $OutDir build $Profile -p $Crate"
Write-Host $arm64Cmd
Invoke-Expression $arm64Cmd
if ($LASTEXITCODE -ne 0) { throw "arm64-v8a build failed" }

Write-Host "[Phase 20C] Building native FFmpeg shim for arm64-v8a..."
cargo ndk -t arm64-v8a build $Profile -p fetchx-downloader --bin fetchx-native-ffmpeg
if ($LASTEXITCODE -ne 0) { throw "arm64-v8a native FFmpeg shim build failed" }
$arm64Binary = "target\aarch64-linux-android\$ProfileDirectory\fetchx-native-ffmpeg"
if (!(Test-Path $arm64Binary)) { throw "arm64-v8a native FFmpeg shim is missing: $arm64Binary" }
$arm64Destination = "apps/android/app/src/main/jniLibs/arm64-v8a/libfetchx_ffmpeg_native.so"
New-Item -ItemType Directory -Force -Path (Split-Path $arm64Destination) | Out-Null
Copy-Item -Force $arm64Binary $arm64Destination

Write-Host "[Phase 20C] Building x86_64 (release=$IsReleaseBuild)..."
$x64Cmd = "cargo ndk -t x86_64 -o $OutDir build $Profile -p $Crate"
Write-Host $x64Cmd
Invoke-Expression $x64Cmd
if ($LASTEXITCODE -ne 0) { throw "x86_64 build failed" }

Write-Host "[Phase 20C] Building native FFmpeg shim for x86_64..."
cargo ndk -t x86_64 build $Profile -p fetchx-downloader --bin fetchx-native-ffmpeg
if ($LASTEXITCODE -ne 0) { throw "x86_64 native FFmpeg shim build failed" }
$x64Binary = "target\x86_64-linux-android\$ProfileDirectory\fetchx-native-ffmpeg"
if (!(Test-Path $x64Binary)) { throw "x86_64 native FFmpeg shim is missing: $x64Binary" }
$x64Destination = "apps/android/app/src/main/jniLibs/x86_64/libfetchx_ffmpeg_native.so"
New-Item -ItemType Directory -Force -Path (Split-Path $x64Destination) | Out-Null
Copy-Item -Force $x64Binary $x64Destination

Write-Host ""
Write-Host "[Phase 20C] Done. Libraries:"
Get-ChildItem -Recurse $OutDir, "apps/android/app/src/main/jniLibs" -Include "*.so" |
    Select-Object FullName, Length
