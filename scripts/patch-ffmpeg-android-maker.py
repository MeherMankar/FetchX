#!/usr/bin/env python3
import sys
from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    source = path.read_text(encoding="utf-8")
    if source.count(old) == 0 and source.count(new) == 1:
        return
    if source.count(old) != 1:
        raise RuntimeError(f"Expected one patch target in {path}")
    path.write_text(source.replace(old, new), encoding="utf-8")


root = Path(sys.argv[1])
replace_once(
    root / "scripts/export-host-variables.sh",
    "  HOST_NPROC=$(nproc)\n",
    "  HOST_NPROC=${FETCHX_BUILD_JOBS:-$(nproc)}\n",
)
replace_once(
    root / "scripts/ffmpeg/build.sh",
    "  --enable-shared \\\n  --disable-static \\\n",
    "  --disable-shared \\\n  --enable-static \\\n",
)
replace_once(
    root / "ffmpeg-android-maker.sh",
    """function prepareOutput() {
  OUTPUT_LIB=${OUTPUT_DIR}/lib/${ANDROID_ABI}
  mkdir -p ${OUTPUT_LIB}
  cp ${BUILD_DIR_FFMPEG}/${ANDROID_ABI}/lib/*.so ${OUTPUT_LIB}

  OUTPUT_HEADERS=${OUTPUT_DIR}/include/${ANDROID_ABI}
  mkdir -p ${OUTPUT_HEADERS}
  cp -r ${BUILD_DIR_FFMPEG}/${ANDROID_ABI}/include/* ${OUTPUT_HEADERS}
}""",
    """function prepareOutput() {
  return 0
}""",
)
replace_once(
    root / "ffmpeg-android-maker.sh",
    """  TEXT_REL_STATS_FILE=${STATS_DIR}/text-relocations.txt
  ${FAM_READELF} --dynamic ${BUILD_DIR_FFMPEG}/${ANDROID_ABI}/lib/*.so | grep 'TEXTREL\\|File' >> ${TEXT_REL_STATS_FILE}

  if grep -q TEXTREL ${TEXT_REL_STATS_FILE}; then
    echo "There are text relocations in output files:"
    cat ${TEXT_REL_STATS_FILE}
    exit 1
  fi""",
    """  if ${FAM_READELF} --dynamic ${BUILD_DIR_FFMPEG}/${ANDROID_ABI}/bin/ffmpeg | grep -q TEXTREL; then
    echo "FFmpeg contains text relocations"
    exit 1
  fi""",
)
