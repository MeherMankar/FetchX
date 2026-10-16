import io
import json
import os
import sys
import tempfile
import threading
import traceback

from yt_dlp import YoutubeDL, parse_options
from yt_dlp.utils import DownloadCancelled


_progress_write_lock = threading.Lock()


def _ensure_streams():
    """Ensure sys.stdout/stderr are not None (Chaquopy background threads may leave them None)."""
    if sys.stdout is None:
        sys.stdout = io.StringIO()
    if sys.stderr is None:
        sys.stderr = io.StringIO()


def _write_progress(progress_path, line):
    directory = os.path.dirname(progress_path)
    prefix = os.path.basename(progress_path) + "."
    temporary_progress_path = None
    try:
        with _progress_write_lock:
            with tempfile.NamedTemporaryFile(
                mode="w",
                encoding="utf-8",
                dir=directory,
                prefix=prefix,
                suffix=".tmp",
                delete=False,
            ) as progress_file:
                temporary_progress_path = progress_file.name
                progress_file.write(line + "\n")
            os.replace(temporary_progress_path, progress_path)
            temporary_progress_path = None
    finally:
        if temporary_progress_path is not None:
            try:
                os.remove(temporary_progress_path)
            except FileNotFoundError:
                pass


def validate_version():
    _ensure_streams()
    from yt_dlp.version import __version__

    return __version__


def prepare_options(arguments):
    _, _, urls, options = parse_options(list(arguments))
    if not options.get("allowed_extractors"):
        options.pop("allowed_extractors", None)
    if options.get("download_ranges") is None or not callable(options.get("download_ranges")):
        options.pop("download_ranges", None)

    # Remove progress_template so yt-dlp uses progress_hooks instead of writing
    # to stdout. The --progress-template CLI flag sets this dict, which bypasses
    # progress_hooks and routes output to stdout (a StringIO no-op on Android).
    options.pop("progress_template", None)
    # Also remove newline since it's only meaningful for terminal output.
    options.pop("newline", None)

    return options, urls


def run(arguments, progress_path, cancel_path):
    _ensure_streams()
    debug_path = progress_path + ".debug.txt"
    debug_lines = []

    def dbg(msg):
        debug_lines.append(str(msg))

    # Remove any stale cancel file left from a previous run.
    try:
        if os.path.exists(cancel_path):
            os.remove(cancel_path)
            dbg(f"[fetchx_ytdlp] removed stale cancel_path: {cancel_path}")
    except OSError as _e:
        dbg(f"[fetchx_ytdlp] could not remove stale cancel_path: {_e}")

    dbg(f"[fetchx_ytdlp] cancel_path={cancel_path}")
    dbg(f"[fetchx_ytdlp] progress_path={progress_path}")
    dbg(f"[fetchx_ytdlp] arguments={list(arguments)}")

    def flush_debug():
        try:
            with open(debug_path, "w", encoding="utf-8") as f:
                f.write("\n".join(debug_lines) + "\n")
        except OSError:
            pass

    try:
        options, urls = prepare_options(arguments)
        dbg(f"[fetchx_ytdlp] urls={urls}")
        dbg(f"[fetchx_ytdlp] options_keys={sorted(options.keys())}")

        def is_cancelled():
            return os.path.exists(cancel_path)

        fragment_count = [0]

        def report_progress(progress):
            if is_cancelled():
                dbg(f"[fetchx_ytdlp] cancelled by cancel_path at fragment {fragment_count[0]}")
                raise DownloadCancelled()
            status = progress.get("status")
            if status == "downloading":
                frag_index = progress.get("fragment_index")
                frag_count = progress.get("fragment_count")
                if frag_index is not None:
                    fragment_count[0] = frag_index
                    dbg(f"[fetchx_ytdlp] fragment {frag_index}/{frag_count}")
            if status != "downloading":
                dbg(f"[fetchx_ytdlp] progress status={status}")
                return

            downloaded = progress.get("downloaded_bytes")
            total = progress.get("total_bytes")
            estimate = progress.get("total_bytes_estimate")
            speed = progress.get("speed")
            eta = progress.get("eta")
            percent = "NA"
            denominator = total or estimate
            if downloaded is not None and denominator and denominator > 0:
                pct = min(100.0, max(0.0, float(downloaded) * 100.0 / float(denominator)))
                percent = f"{pct:.1f}%"

            def clean_num(val):
                if val is None:
                    return "NA"
                try:
                    num = float(val)
                    if not (num == num):  # NaN check
                        return "NA"
                    return str(int(num))
                except (ValueError, TypeError):
                    return "NA"

            fields = (
                percent,
                clean_num(downloaded),
                clean_num(total),
                clean_num(estimate),
                clean_num(speed),
                clean_num(eta),
            )
            line = "FETCHX_PROGRESS|" + "|".join(fields)
            _write_progress(progress_path, line)

        # Capture all yt-dlp log messages for diagnostics
        log_messages = []

        def capture_debug(msg):
            if msg.startswith("[debug]") or "fragment" in msg.lower() or "hls" in msg.lower():
                log_messages.append(f"DEBUG: {msg}")
                dbg(f"[ytdlp] {msg}")

        def capture_warning(msg):
            log_messages.append(f"WARNING: {msg}")
            dbg(f"[ytdlp] WARNING: {msg}")

        def capture_error(msg):
            log_messages.append(f"ERROR: {msg}")
            dbg(f"[ytdlp] ERROR: {msg}")

        options["progress_hooks"] = [report_progress]
        options["quiet"] = True
        options["no_warnings"] = True
        # Lightweight diagnostic logger — captures errors and HLS fragment messages only.
        # verbose=False to avoid flooding stdout StringIO with thousands of debug lines.
        options["logger"] = type("Logger", (), {
            "debug": lambda self, msg: capture_debug(msg),
            "warning": lambda self, msg: capture_warning(msg),
            "error": lambda self, msg: capture_error(msg),
        })()
        with YoutubeDL(options) as downloader:
            status = downloader.download(urls)
        dbg(f"[fetchx_ytdlp] download() returned status={status}")
        dbg(f"[fetchx_ytdlp] fragments_reached={fragment_count[0]}")
        dbg(f"[fetchx_ytdlp] log_tail={log_messages[-20:]}")
        flush_debug()
        if status != 0:
            detail = "; ".join(log_messages[-5:]) if log_messages else "no details"
            return f"yt-dlp exited with status {status}: {detail}"
        return ""
    except DownloadCancelled:
        dbg("[fetchx_ytdlp] DownloadCancelled exception raised")
        flush_debug()
        return "" if os.path.exists(cancel_path) else "yt-dlp download was cancelled"
    except Exception as error:
        tb = traceback.format_exc()
        dbg(f"[fetchx_ytdlp] EXCEPTION: {type(error).__name__}: {error}")
        dbg(f"[fetchx_ytdlp] TRACEBACK: {tb}")
        flush_debug()
        return f"{type(error).__name__}: {error}\nTraceback:\n{tb}"


def discover_formats(url):
    _ensure_streams()
    try:
        options = {
            "quiet": True,
            "no_warnings": True,
            "skip_download": True,
            "noplaylist": True,
            "ignoreconfig": True,
            "no_color": True,
            "user_agent": "Mozilla/5.0 (Android 13; Mobile; rv:140.0) Gecko/140.0 Firefox/140.0",
            "http_headers": {
                "Accept": "*/*",
                "Accept-Language": "en-US,en;q=0.9",
            },
        }
        with YoutubeDL(options) as downloader:
            info = downloader.extract_info(url, download=False)
        info = info or {}
        formats = []
        raw_formats = info.get("formats")
        # Direct video or single stream fallback
        if not raw_formats and (info.get("url") or info.get("height") or info.get("ext")):
            raw_formats = [info]

        seen_keys = set()
        for item in raw_formats or []:
            if item.get("has_drm"):
                continue
            vcodec = item.get("vcodec")
            if vcodec == "none":
                continue
            fmt_id = item.get("format_id")
            fmt_id_str = str(fmt_id) if fmt_id is not None else "best"
            ext = item.get("ext") or "mp4"
            height = item.get("height")
            res = item.get("resolution") or (f"{height}p" if height else None)

            # Deduplicate same resolution and extension
            dedup_key = (height or res, ext)
            if dedup_key in seen_keys:
                continue
            if height or res:
                seen_keys.add(dedup_key)

            formats.append({
                "format_id": fmt_id_str,
                "ext": ext,
                "resolution": res,
                "width": item.get("width"),
                "height": height,
                "format_note": item.get("format_note"),
                "quality": item.get("quality"),
                "filesize": item.get("filesize"),
                "filesize_approx": item.get("filesize_approx"),
                "vcodec": vcodec,
                "acodec": item.get("acodec"),
                "has_drm": item.get("has_drm"),
            })

        def format_rank(item):
            def numeric_value(key):
                value = item.get(key)
                return value if isinstance(value, (int, float)) else 0

            return (
                numeric_value("height"),
                numeric_value("quality"),
                numeric_value("filesize") or numeric_value("filesize_approx"),
            )

        formats.sort(
            key=format_rank,
            reverse=True,
        )
        formats = formats[:100]
        return json.dumps(
            {
                "title": info.get("title"),
                "extractor_key": info.get("extractor_key") or info.get("extractor"),
                "formats": formats,
            },
            separators=(",", ":"),
        )
    except Exception as error:
        return f"ERROR:{type(error).__name__}: {error}"
