#!/usr/bin/env python3
"""Write latest.json, the static manifest read by tauri-plugin-updater.

Usage: make-latest-json.py --dir DIR --version X.Y.Z[-pre] --tag vX.Y.Z[-pre] --repo OWNER/REPO
                           [--notes TEXT] [--pub-date RFC3339] [--out FILE]

DIR must contain exactly the files the release workflow collects (nothing more, nothing
less), named as in docs/RELEASING.md:

  Portolano-<version>-macos-aarch64.dmg
  Portolano-<version>-macos-aarch64.app.tar.gz      (+ .sig)
  Portolano-<version>-macos-x86_64.dmg
  Portolano-<version>-macos-x86_64.app.tar.gz       (+ .sig)
  Portolano-<version>-windows-x64-setup.exe         (+ .sig)

The Windows installer is itself the update package (Tauri v2). Every .sig must hold a
non-empty one-line base64 signature, whose CONTENT (not a path or URL) goes in latest.json.
Any missing or unexpected file, or an empty or malformed signature, is an error.

Output: latest.json (default DIR/latest.json) in the format

  {"version", "notes", "pub_date", "platforms": {"<os>-<arch>": {"signature", "url"}}}

with urls https://github.com/<repo>/releases/download/<tag>/<file name>. Python 3 standard
library only. Errors are printed as GitHub Actions "::error::" lines and the exit code is 1.
"""
import argparse
import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

SEMVER = re.compile(r"^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$")
REPO = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
URL_SAFE = re.compile(r"^[A-Za-z0-9._-]+$")  # no spaces, no '+', nothing needing escaping
SIGNATURE = re.compile(r"^[A-Za-z0-9+/]+={0,2}$")  # the .sig file is one base64 line
RFC3339 = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$")

# Updater platform key -> (suffix of the update package, extra installer suffixes that must
# also be present). The package suffix is appended to "Portolano-<version>-".
PLATFORMS = {
    "darwin-aarch64": "macos-aarch64.app.tar.gz",
    "darwin-x86_64": "macos-x86_64.app.tar.gz",
    "windows-x86_64": "windows-x64-setup.exe",
}
INSTALLERS = ["macos-aarch64.dmg", "macos-x86_64.dmg"]


class Failure(Exception):
    pass


def expected_files(version: str) -> dict[str, str]:
    """Platform key -> update package file name."""
    return {key: f"Portolano-{version}-{suffix}" for key, suffix in PLATFORMS.items()}


def build_manifest(directory: Path, version: str, tag: str, repo: str, notes: str, pub_date: str) -> dict:
    if not SEMVER.match(version):
        raise Failure(f"'{version}' is not a valid x.y.z[-prerelease] version")
    if tag != f"v{version}":
        raise Failure(f"tag '{tag}' does not match version '{version}'")
    if not REPO.match(repo):
        raise Failure(f"'{repo}' is not an OWNER/REPO name")
    if not RFC3339.match(pub_date):
        raise Failure(f"pub_date '{pub_date}' is not RFC 3339")
    if not directory.is_dir():
        raise Failure(f"{directory} is not a directory")

    packages = expected_files(version)
    wanted = set(packages.values())
    wanted |= {f"{name}.sig" for name in packages.values()}
    wanted |= {f"Portolano-{version}-{suffix}" for suffix in INSTALLERS}

    # latest.json and SHA256SUMS.txt may be left over from an earlier run in the same folder.
    present = {p.name for p in directory.iterdir() if p.name not in ("latest.json", "SHA256SUMS.txt")}
    missing = sorted(wanted - present)
    extra = sorted(present - wanted)
    if missing:
        raise Failure("missing files: " + ", ".join(missing))
    if extra:
        raise Failure("unexpected files (expected exactly %d): %s" % (len(wanted), ", ".join(extra)))

    platforms = {}
    for key, name in packages.items():
        if not URL_SAFE.match(name):
            raise Failure(f"file name '{name}' is not URL-safe")
        for required in (name, f"{name}.sig"):
            if not (directory / required).is_file():
                raise Failure(f"{required} is not a regular file")
        if (directory / name).stat().st_size == 0:
            raise Failure(f"{name} is empty")
        try:
            signature = (directory / f"{name}.sig").read_text(encoding="utf-8").strip()
        except (OSError, UnicodeDecodeError) as exc:
            raise Failure(f"cannot read {name}.sig: {exc}") from exc
        if not signature:
            raise Failure(f"{name}.sig is empty")
        if not SIGNATURE.match(signature):
            raise Failure(f"{name}.sig is not a single line of base64 (it must be the CONTENT of the .sig file)")
        platforms[key] = {
            "signature": signature,
            "url": f"https://github.com/{repo}/releases/download/{tag}/{name}",
        }

    return {"version": version, "notes": notes, "pub_date": pub_date, "platforms": platforms}


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--dir", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--notes", default="")
    parser.add_argument("--pub-date", default=datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"))
    parser.add_argument("--out", type=Path)
    args = parser.parse_args(argv[1:])

    try:
        manifest = build_manifest(args.dir, args.version, args.tag, args.repo, args.notes, args.pub_date)
    except Failure as exc:
        print(f"::error::latest.json: {exc}")
        return 1

    out = args.out or args.dir / "latest.json"
    try:
        out.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    except OSError as exc:
        print(f"::error::cannot write {out}: {exc}")
        return 1
    print(f"Wrote {out} for {args.tag} ({len(manifest['platforms'])} platforms)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
