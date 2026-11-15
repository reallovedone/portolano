#!/usr/bin/env python3
"""Fail early when the project's version numbers disagree with each other or with a tag.

Usage: check-version.py [TAG]

Checks that these four places hold the same version:
  - Cargo.toml          [workspace.package] version
  - src-tauri/tauri.conf.json   "version"
  - package.json        "version"
  - ui/package.json     "version"
and, when TAG is given (for example "v0.1.0"), that TAG equals "v" + that version.

On success the version is printed on stdout and, when running in GitHub Actions,
appended to $GITHUB_OUTPUT as "version=<x.y.z>" and "prerelease=<true|false>".
Nothing is read from the environment except GITHUB_OUTPUT, so the tag can be passed
as a plain argument from an env var (never interpolated into a shell script).
"""
import json
import os
import re
import sys
from pathlib import Path

try:
    import tomllib  # Python 3.11+ (the GitHub runners)
except ModuleNotFoundError:  # older local Python: fall back to a small parser
    tomllib = None

ROOT = Path(__file__).resolve().parents[2]
SEMVER = re.compile(r"^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$")


def workspace_version() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    if tomllib is not None:
        return tomllib.loads(text)["workspace"]["package"]["version"]
    section = re.search(r"^\[workspace\.package\]\s*$(.*?)(?=^\[|\Z)", text, re.M | re.S)
    match = section and re.search(r'^version\s*=\s*"([^"]+)"', section.group(1), re.M)
    if not match:
        raise KeyError("version in [workspace.package]")
    return match.group(1)


def read_versions() -> dict[str, str]:
    return {
        "Cargo.toml [workspace.package]": workspace_version(),
        "src-tauri/tauri.conf.json": json.loads(
            (ROOT / "src-tauri/tauri.conf.json").read_text(encoding="utf-8")
        )["version"],
        "package.json": json.loads((ROOT / "package.json").read_text(encoding="utf-8"))["version"],
        "ui/package.json": json.loads((ROOT / "ui/package.json").read_text(encoding="utf-8"))[
            "version"
        ],
    }


def main(argv: list[str]) -> int:
    tag = argv[1].strip() if len(argv) > 1 else ""
    try:
        versions = read_versions()
    except (OSError, KeyError, ValueError) as exc:
        print(f"::error::Cannot read the project versions: {exc}")
        return 1

    distinct = set(versions.values())
    if len(distinct) != 1:
        lines = "\n".join(f"  {name}: {value}" for name, value in versions.items())
        print(f"::error::The version differs between files. Make them identical:\n{lines}")
        return 1

    version = distinct.pop()
    if not SEMVER.match(version):
        print(f"::error::'{version}' is not a valid x.y.z[-prerelease] version.")
        return 1

    if tag and tag != f"v{version}":
        print(
            f"::error::The tag '{tag}' does not match the project version '{version}' "
            f"(expected tag 'v{version}'). Bump the version in the four files listed in "
            "docs/RELEASING.md, or tag the right commit."
        )
        return 1

    print(version)
    out = os.environ.get("GITHUB_OUTPUT")
    if out:
        with open(out, "a", encoding="utf-8") as fh:
            fh.write(f"version={version}\n")
            fh.write(f"prerelease={'true' if '-' in version else 'false'}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
