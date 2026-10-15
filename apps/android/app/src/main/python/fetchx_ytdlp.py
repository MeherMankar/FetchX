import io
import json
import os
import sys
import traceback

from yt_dlp import YoutubeDL, parse_options
from yt_dlp.utils import DownloadCancelled


def _ensure_streams():
    """Ensure sys.stdout/stderr are not None (Chaquopy background threads may leave them None)."""
    if sys.stdout is None:
        sys.stdout = io.StringIO()
    if sys.stderr is None:
        sys.stderr = io.StringIO()


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

    extractor_args = options.setdefault("extractor_args", {})
    youtube_args = extractor_args.setdefault("youtube", {})
    if not youtube_args.get("player_client"):
        youtube_args["player_client"] = ["android", "web"]
    return options, urls


def run(arguments, progress_path, cancel_path):
    _ensure_streams()
    try:
        options, urls = prepare_options(arguments)

        def is_cancelled():
            return os.path.exists(cancel_path)

        def report_progress(progress):
            if is_cancelled():
                raise DownloadCancelled()
            if progress.get("status") != "downloading":
                return

            downloaded = progress.get("downloaded_bytes")
            total = progress.get("total_bytes")
            estimate = progress.get("total_bytes_estimate")
            speed = progress.get("speed")
            eta = progress.get("eta")
            percent = "NA"
            denominator = total or estimate
            if downloaded is not None and denominator:
                percent = f"{downloaded * 100.0 / denominator:.1f}%"
            fields = (
                percent,
                downloaded if downloaded is not None else "NA",
                total if total is not None else "NA",
                estimate if estimate is not None else "NA",
                speed if speed is not None else "NA",
                eta if eta is not None else "NA",
            )
            line = "FETCHX_PROGRESS|" + "|".join(str(value) for value in fields)
            temporary_progress_path = progress_path + ".tmp"
            with open(temporary_progress_path, "w", encoding="utf-8") as progress_file:
                progress_file.write(line + "\n")
            os.replace(temporary_progress_path, progress_path)

        options["progress_hooks"] = [report_progress]
        options["quiet"] = True
        options["no_warnings"] = True
        with YoutubeDL(options) as downloader:
            status = downloader.download(urls)
        return "" if status == 0 else f"yt-dlp exited with status {status}"
    except DownloadCancelled:
        return "" if is_cancelled() else "yt-dlp download was cancelled"
    except Exception as error:
        tb = traceback.format_exc()
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
        }
        with YoutubeDL(options) as downloader:
            info = downloader.extract_info(url, download=False)
        formats = []
        for item in (info or {}).get("formats") or []:
            if item.get("has_drm"):
                continue
            formats.append({
                "format_id": item.get("format_id"),
                "ext": item.get("ext"),
                "resolution": item.get("resolution"),
                "width": item.get("width"),
                "height": item.get("height"),
                "format_note": item.get("format_note"),
                "filesize": item.get("filesize"),
                "filesize_approx": item.get("filesize_approx"),
                "vcodec": item.get("vcodec"),
                "acodec": item.get("acodec"),
                "has_drm": item.get("has_drm"),
            })
            if len(formats) >= 100:
                break
        return json.dumps({"formats": formats}, separators=(",", ":"))
    except Exception as error:
        return f"ERROR:{type(error).__name__}: {error}"
