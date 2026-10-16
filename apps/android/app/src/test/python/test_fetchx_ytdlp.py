import importlib.util
import sys
import tempfile
import types
import unittest
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path


def load_ytdlp_module():
    yt_dlp = types.ModuleType("yt_dlp")
    yt_dlp.YoutubeDL = object
    yt_dlp.parse_options = object
    utils = types.ModuleType("yt_dlp.utils")
    utils.DownloadCancelled = type("DownloadCancelled", (Exception,), {})
    sys.modules["yt_dlp"] = yt_dlp
    sys.modules["yt_dlp.utils"] = utils

    module_path = (
        Path(__file__).parents[2] / "main" / "python" / "fetchx_ytdlp.py"
    )
    spec = importlib.util.spec_from_file_location("fetchx_ytdlp", module_path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ProgressFileTest(unittest.TestCase):
    def test_concurrent_updates_use_independent_atomic_temporary_files(self):
        ytdlp = load_ytdlp_module()
        with tempfile.TemporaryDirectory() as directory:
            progress_path = str(Path(directory) / "progress")
            lines = [f"FETCHX_PROGRESS|{index}%" for index in range(200)]
            with ThreadPoolExecutor(max_workers=16) as executor:
                list(executor.map(
                    lambda line: ytdlp._write_progress(progress_path, line),
                    lines,
                ))

            self.assertIn(Path(progress_path).read_text().strip(), lines)
            self.assertEqual(list(Path(directory).iterdir()), [Path(progress_path)])


if __name__ == "__main__":
    unittest.main()
