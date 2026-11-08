import unittest
from pathlib import Path
from runpy import run_path


normalize_version = run_path(
    str(Path(__file__).with_name("set-version.py"))
)["normalize_version"]


class NormalizeVersionTests(unittest.TestCase):
    def test_normalizes_alpha_shorthand_to_existing_display_format(self):
        self.assertEqual(
            normalize_version("a0.0.4"),
            ("0.0.4-alpha", "alpha.0.0.4"),
        )
        self.assertEqual(
            normalize_version("va0.0.4"),
            ("0.0.4-alpha", "alpha.0.0.4"),
        )
        self.assertEqual(
            normalize_version("b1.2.3"),
            ("1.2.3-beta", "beta.1.2.3"),
        )
        self.assertEqual(
            normalize_version("rc1.2.3"),
            ("1.2.3-rc", "rc.1.2.3"),
        )

    def test_keeps_existing_prerelease_input_formats(self):
        self.assertEqual(
            normalize_version("alpha.0.0.1"),
            ("0.0.1-alpha", "alpha.0.0.1"),
        )
        self.assertEqual(
            normalize_version("1.2.3-rc.1"),
            ("1.2.3-rc.1", "1.2.3-rc.1"),
        )

    def test_accepts_optional_v_prefix(self):
        self.assertEqual(
            normalize_version("v1.2.3"),
            ("1.2.3", "v1.2.3"),
        )

    def test_rejects_non_semver_output(self):
        with self.assertRaises(ValueError):
            normalize_version("a0.0.4.1")
        with self.assertRaises(ValueError):
            normalize_version("not-a-version")


if __name__ == "__main__":
    unittest.main()
