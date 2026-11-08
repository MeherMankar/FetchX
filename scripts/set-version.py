#!/usr/bin/env python3
"""
Sets the version across all project files:
- Cargo.toml ([workspace.package] version)
- apps/desktop/src-tauri/tauri.conf.json
- apps/desktop/ui/package.json

Usage:
  python scripts/set-version.py <version>
"""
import os
import sys
import re
import json
from pathlib import Path

def normalize_version(raw: str):
    raw = raw.strip()
    clean = raw
    if re.match(r"^v(?=(?:\d|[-_.]|alpha|beta|rc|[ab]\d))", clean, re.IGNORECASE):
        clean = clean[1:].lstrip("-_. ")

    shorthand = re.fullmatch(r"(a|b|rc)(\d+\.\d+\.\d+)", clean, re.IGNORECASE)
    if shorthand:
        tag = {"a": "alpha", "b": "beta", "rc": "rc"}[shorthand.group(1).lower()]
        clean = f"{tag}.{shorthand.group(2)}"
    elif re.match(r"(?:a|b|rc)\d", clean, re.IGNORECASE):
        raise ValueError("a, b, and rc shorthand must be followed by major.minor.patch")

    # Check if format like alpha.0.0.1 or alpha-0.0.1
    m = re.match(r"^([a-zA-Z][a-zA-Z0-9-]*)[.-]([0-9]+\.[0-9]+\.[0-9]+.*)$", clean)
    if m:
        tag, numbers = m.groups()
        semver = f"{numbers}-{tag}"
        display = clean
    else:
        semver = clean
        display = raw

    semver_pattern = (
        r"(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)"
        r"(?:-(?:0|[1-9]\d*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*)"
        r"(?:\.(?:0|[1-9]\d*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*))*)?"
        r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
    )
    if not re.fullmatch(semver_pattern, semver):
        raise ValueError(
            "version must be valid SemVer, such as 1.2.3, alpha.0.0.4, or 1.2.3-rc.1"
        )
    return semver, display

def main():
    if len(sys.argv) < 2:
        print("Usage: python scripts/set-version.py <version>", file=sys.stderr)
        sys.exit(1)

    raw_input = sys.argv[1]
    try:
        semver, display = normalize_version(raw_input)
    except ValueError as error:
        print(f"Invalid version: {error}", file=sys.stderr)
        sys.exit(2)
    print(f"Applying version: raw='{raw_input}' -> semver='{semver}', display='{display}'")

    root = Path(__file__).resolve().parent.parent

    # 1. Update root Cargo.toml
    cargo_toml_path = root / "Cargo.toml"
    if cargo_toml_path.exists():
        content = cargo_toml_path.read_text(encoding="utf-8")
        new_content = re.sub(
            r'(\[workspace\.package\][^\[]*?\bversion\s*=\s*)"[^"]+"',
            rf'\g<1>"{semver}"',
            content,
            flags=re.DOTALL
        )
        if new_content != content:
            cargo_toml_path.write_text(new_content, encoding="utf-8")
            print(f"Updated {cargo_toml_path} to version {semver}")
        else:
            print(f"Warning: [workspace.package] version not found in {cargo_toml_path}")

    # 2. Update apps/desktop/src-tauri/tauri.conf.json
    tauri_conf_path = root / "apps" / "desktop" / "src-tauri" / "tauri.conf.json"
    if tauri_conf_path.exists():
        with open(tauri_conf_path, "r", encoding="utf-8") as f:
            data = json.load(f)
        data["version"] = semver
        with open(tauri_conf_path, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=2)
            f.write("\n")
        print(f"Updated {tauri_conf_path} to version {semver}")

    # 3. Update apps/desktop/ui/package.json
    ui_package_json = root / "apps" / "desktop" / "ui" / "package.json"
    if ui_package_json.exists():
        with open(ui_package_json, "r", encoding="utf-8") as f:
            data = json.load(f)
        data["version"] = semver
        with open(ui_package_json, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=2)
            f.write("\n")
        print(f"Updated {ui_package_json} to version {semver}")

    # If running in GitHub Actions, export variables to GITHUB_ENV
    github_env = os.environ.get("GITHUB_ENV")
    if github_env:
        with open(github_env, "a", encoding="utf-8") as f:
            f.write(f"APP_VERSION={display}\n")
            f.write(f"FETCHX_APP_VERSION={display}\n")
            f.write(f"SEMVER_VERSION={semver}\n")
        print(f"Exported APP_VERSION, FETCHX_APP_VERSION, and SEMVER_VERSION to GITHUB_ENV")

if __name__ == "__main__":
    main()
