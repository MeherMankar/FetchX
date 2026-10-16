#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
builder_commit="dd72b161ae5c759fd25a5cab971a3ff710f0bdba"
builder_dir="${FETCHX_FFMPEG_BUILDER_DIR:-${HOME}/.cache/fetchx-ffmpeg-android-maker}"
sdk_root="${ANDROID_SDK_HOME:-/opt/android-sdk}"
ndk_root="${ANDROID_NDK_HOME:-${sdk_root}/ndk/27.2.12479018}"

if [[ ! -x "${ndk_root}/toolchains/llvm/prebuilt/linux-x86_64/bin/clang" ]]; then
  echo "Android NDK 27.2.12479018 for Linux is required at ${ndk_root}" >&2
  exit 1
fi

if [[ ! -d "${builder_dir}/.git" ]]; then
  mkdir -p "$(dirname "${builder_dir}")"
  git init "${builder_dir}"
  git -C "${builder_dir}" remote add origin https://github.com/Javernaut/ffmpeg-android-maker.git
fi

git -C "${builder_dir}" fetch --depth 1 origin "${builder_commit}"
git -C "${builder_dir}" checkout --detach FETCH_HEAD
python3 "${repo_root}/scripts/patch-ffmpeg-android-maker.py" "${builder_dir}"

export ANDROID_SDK_HOME="${sdk_root}"
export ANDROID_NDK_HOME="${ndk_root}"
export FETCHX_BUILD_JOBS="${FETCHX_BUILD_JOBS:-2}"
export HOST_NPROC="${FETCHX_BUILD_JOBS}"

"${builder_dir}/ffmpeg-android-maker.sh" \
  --target-abis=arm64-v8a,x86_64 \
  --android-api-level=26 \
  --source-tar=8.1.2

for abi in arm64-v8a x86_64; do
  source_binary="${builder_dir}/build/ffmpeg/${abi}/bin/ffmpeg"
  destination="${repo_root}/apps/android/app/src/main/jniLibs/${abi}/libfetchx_ffmpeg.so"
  if [[ ! -x "${source_binary}" ]]; then
    echo "Expected FFmpeg executable is missing: ${source_binary}" >&2
    exit 1
  fi
  mkdir -p "$(dirname "${destination}")"
  install -m 0755 "${source_binary}" "${destination}"
  "${ndk_root}/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf" -h "${destination}" >/dev/null
  echo "${abi}: $(stat -c '%s bytes' "${destination}")"
done

license_dir="${repo_root}/apps/android/app/src/main/assets/licenses"
mkdir -p "${license_dir}"
install -m 0644 "${builder_dir}/sources/ffmpeg/ffmpeg-8.1.2/COPYING.LGPLv2.1" \
  "${license_dir}/FFmpeg-LGPL-2.1.txt"
install -m 0644 "${builder_dir}/LICENSE.txt" \
  "${license_dir}/ffmpeg-android-maker-MIT.txt"
