#!/usr/bin/env python3
"""Print the release notes of a version, taken from CHANGELOG.md.

Usage: changelog-notes.py VERSION [--file CHANGELOG.md] [--out FILE]
       changelog-notes.py --self-test

VERSION is "X.Y.Z" or "X.Y.Z-<suffix>" (a leading "v" is accepted), for example 0.2.0 or
0.2.0-rc1. The notes are the body of the matching "## [X.Y.Z] - <date>" section of
CHANGELOG.md (Keep a Changelog format), without the heading line and without the link
references at the bottom of the file, trimmed.

Release candidates and other pre-releases have no section of their own: "X.Y.Z-rcN" uses the
section [X.Y.Z], and the body starts with a one-line note
("> Release candidate N of X.Y.Z - for testing.") followed by a blank line.

Errors (printed as GitHub Actions "::error::" lines, exit code 1):
  - the section is missing or has no content;
  - a final version (no suffix) whose heading date is not a real date, i.e. still says
    "Unreleased": a final release cannot ship before its date is set.

The notes go to stdout, or to --out FILE. Python 3 standard library only. Nothing is read
from the environment, so the version can be passed from an env var as a plain argument
(never interpolated into a shell script).
"""
from __future__ import annotations

import argparse
import re
import sys
import tempfile
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
VERSION = re.compile(r"^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*))?$")
SECTION = re.compile(r"^##\s+\[([^\]]+)\](?:\s+-\s+(.*?))?\s*$")
LINK_REFERENCE = re.compile(r"^\[[^\]]+\]:\s+\S+.*$")
DATE = re.compile(r"^(\d{4})-(\d{2})-(\d{2})$")
# Pre-release kind -> how it is called in the note. Anything else is "Pre-release <suffix>".
KINDS = {"rc": "Release candidate", "beta": "Beta", "alpha": "Alpha"}
SUFFIX = re.compile(r"^([A-Za-z]+)[.-]?(\d+)?$")


class Failure(Exception):
    pass


def parse_version(text: str) -> tuple[str, str | None]:
    """Return (X.Y.Z, suffix or None)."""
    match = VERSION.match(text.strip())
    if not match:
        raise Failure(f"'{text}' is not a valid x.y.z[-prerelease] version")
    major, minor, patch, suffix = match.groups()
    return f"{major}.{minor}.{patch}", suffix


def prerelease_label(base: str, suffix: str) -> str:
    """Line that tells readers a candidate is not the final version."""
    match = SUFFIX.match(suffix)
    if match and match.group(1).lower() in KINDS:
        name = KINDS[match.group(1).lower()]
        number = f" {match.group(2)}" if match.group(2) else ""
        return f"> {name}{number} of {base} — for testing."
    return f"> Pre-release ({suffix}) of {base} — for testing."


def find_section(lines: list[str], base: str) -> tuple[str | None, list[str]]:
    """Return (heading date text, body lines) of the section [base]."""
    start = None
    date_text = None
    for index, line in enumerate(lines):
        match = SECTION.match(line)
        if not match:
            continue
        if start is not None:
            return date_text, lines[start:index]
        if match.group(1).strip() == base:
            start = index + 1
            date_text = (match.group(2) or "").strip()
    if start is None:
        raise Failure(f"CHANGELOG.md has no section '## [{base}]'")
    return date_text, lines[start:]


def has_content(body: list[str]) -> bool:
    # Headings alone ("### Added" with nothing under it) are not notes.
    return any(line.strip() and not line.lstrip().startswith("#") for line in body)


def valid_date(text: str) -> bool:
    match = DATE.match(text)
    if not match:
        return False
    try:
        date(int(match.group(1)), int(match.group(2)), int(match.group(3)))
    except ValueError:
        return False
    return True


def build_notes(changelog: str, version: str) -> str:
    base, suffix = parse_version(version)
    lines = changelog.replace("\r\n", "\n").replace("\r", "\n").split("\n")
    date_text, body = find_section(lines, base)

    # The link references sit after the last section.
    while body and (not body[-1].strip() or LINK_REFERENCE.match(body[-1])):
        body.pop()
    if not has_content(body):
        raise Failure(f"the section '## [{base}]' of CHANGELOG.md is empty")
    if suffix is None and not valid_date(date_text or ""):
        shown = date_text or "no date"
        raise Failure(
            f"the section '## [{base}]' of CHANGELOG.md says '{shown}': set its release date "
            "(YYYY-MM-DD) before tagging the final version"
        )

    text = "\n".join(body).strip("\n").strip()
    if suffix is not None:
        text = f"{prerelease_label(base, suffix)}\n\n{text}"
    return text + "\n"


# ---------------------------------------------------------------------------------------
# Self test
# ---------------------------------------------------------------------------------------
FIXTURE = """# Changelog

Intro.

## [Unreleased]

## [0.3.0] - Unreleased

### ✨ Added

- Something new.

## [0.2.0] - 2026-11-20

### 🐛 Fixed

- A fix. See [the page](https://example.org/x).

## [0.1.5] - Unreleased

### ✨ Added

## [0.1.0] - 2026-11-16

First public release.

### ✨ Added

- One thing.

[Unreleased]: https://example.org/compare/v0.2.0...HEAD
[0.1.0]: https://example.org/releases/tag/v0.1.0
"""


def self_test() -> int:
    folder = ROOT / ".tmp" / "changelog"
    folder.mkdir(parents=True, exist_ok=True)
    fixture = folder / "CHANGELOG.fixture.md"
    fixture.write_text(FIXTURE, encoding="utf-8")
    text = fixture.read_text(encoding="utf-8")

    failures: list[str] = []

    def expect_ok(version: str, expected: str) -> None:
        try:
            got = build_notes(text, version)
        except Failure as exc:
            failures.append(f"{version}: unexpected failure: {exc}")
            return
        if got != expected:
            failures.append(f"{version}: expected {expected!r}, got {got!r}")

    def expect_fail(version: str, fragment: str) -> None:
        try:
            got = build_notes(text, version)
        except Failure as exc:
            if fragment not in str(exc):
                failures.append(f"{version}: error {exc!r} lacks {fragment!r}")
            return
        failures.append(f"{version}: expected a failure, got {got!r}")

    fixed = "### 🐛 Fixed\n\n- A fix. See [the page](https://example.org/x).\n"
    expect_ok("0.2.0", fixed)
    expect_ok("v0.2.0", fixed)
    expect_ok("0.1.0", "First public release.\n\n### ✨ Added\n\n- One thing.\n")  # last section: no link references
    added = "### ✨ Added\n\n- Something new.\n"
    expect_ok("0.3.0-rc1", f"> Release candidate 1 of 0.3.0 — for testing.\n\n{added}")
    expect_ok("0.3.0-rc.2", f"> Release candidate 2 of 0.3.0 — for testing.\n\n{added}")
    expect_ok("0.3.0-beta.2", f"> Beta 2 of 0.3.0 — for testing.\n\n{added}")
    expect_ok("0.3.0-alpha", f"> Alpha of 0.3.0 — for testing.\n\n{added}")
    expect_ok("0.3.0-nightly.5", f"> Pre-release (nightly.5) of 0.3.0 — for testing.\n\n{added}")
    expect_ok("0.2.0-rc3", f"> Release candidate 3 of 0.2.0 — for testing.\n\n{fixed}")
    expect_fail("0.3.0", "release date")  # final version, still Unreleased
    expect_fail("0.9.0", "no section")
    expect_fail("0.9.0-rc1", "no section")
    expect_fail("0.1.5-rc1", "is empty")  # only a heading, no entries
    expect_fail("0.1.5", "is empty")
    expect_fail("1.0", "not a valid")
    expect_fail("0.2.0; rm -rf /", "not a valid")
    expect_fail("", "not a valid")

    # The real file: through the command line, as the workflow calls it.
    out = folder / "self-test-output.md"
    code = main(["changelog-notes.py", "0.1.0", "--out", str(out)])
    if code != 0 or "First public release." not in out.read_text(encoding="utf-8"):
        failures.append("the real CHANGELOG.md has no usable [0.1.0] section")

    with tempfile.TemporaryDirectory(dir=folder) as tmp:
        empty = Path(tmp) / "empty.md"
        empty.write_text("", encoding="utf-8")
        if main(["changelog-notes.py", "0.1.0", "--file", str(empty)]) != 1:
            failures.append("an empty file must fail")
        if main(["changelog-notes.py", "0.1.0", "--file", str(Path(tmp) / "missing.md")]) != 1:
            failures.append("a missing file must fail")

    for failure in failures:
        print(f"FAIL {failure}")
    if failures:
        return 1
    print("self-test: all checks passed")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog=Path(argv[0]).name, description=__doc__.splitlines()[0])
    parser.add_argument("version", nargs="?", default=None)
    parser.add_argument("--file", type=Path, default=ROOT / "CHANGELOG.md")
    parser.add_argument("--out", type=Path, default=None)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args(argv[1:])

    if args.self_test:
        return self_test()
    if args.version is None:
        parser.error("a version is required (or --self-test)")

    try:
        try:
            changelog = args.file.read_text(encoding="utf-8")
        except OSError as exc:
            raise Failure(f"cannot read {args.file.name}: {exc.strerror or exc}") from exc
        notes = build_notes(changelog, args.version)
        if args.out is not None:
            try:
                args.out.parent.mkdir(parents=True, exist_ok=True)
                args.out.write_text(notes, encoding="utf-8")
            except OSError as exc:
                raise Failure(f"cannot write {args.out}: {exc.strerror or exc}") from exc
        else:
            sys.stdout.write(notes)
    except Failure as exc:
        print(f"::error::{exc}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
