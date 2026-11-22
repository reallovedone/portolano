# Releasing Portolano

Releases are built by GitHub Actions (`.github/workflows/release.yml`); nothing has to be
built locally. A release is always created as a **draft**: you review it and publish it by
hand.

## Cutting a release

1. Pick the version `X.Y.Z`, make sure `CHANGELOG.md` has its section (see
   [The changelog](#the-changelog-is-the-source-of-the-release-notes) below), and set the
   version in these four places (they must be identical; the
   workflow fails in its first step otherwise):
   - `Cargo.toml` → `[workspace.package]` `version`
   - `src-tauri/tauri.conf.json` → `version`
   - `package.json` → `version`
   - `ui/package.json` → `version`

   Then run `cargo check --workspace` so that `Cargo.lock` picks up the new version (CI
   builds with `--locked` and would fail otherwise).
2. Run `bash dev/check.sh` and `python3 .github/scripts/changelog-notes.py X.Y.Z` (it must
   print the notes you expect), commit, and merge to `main` through the usual pull request.
3. If the release has screenshots for the release notes, publish them on the website under
   `public/whats-new/<version>/` (one light and one dark image per name, `<name>-light.webp` and
   `<name>-dark.webp`, WebP, 1600×1000) and add the version, with each name and its description, to
   `ui/src/lib/updates/screenshots.ts`. The files of a released version are never changed. An
   image that cannot be loaded is simply not shown, so a late upload does no harm.
4. Tag the merged commit and push the tag:

   ```bash
   git switch main && git pull
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```

5. Open the **Actions** tab and watch the *Release* run. It checks the tag against the four
   versions, extracts the release notes from `CHANGELOG.md` (the run stops here if the
   section is missing, empty, or a final version still says *Unreleased*), builds three installers in parallel (macOS Apple Silicon, macOS Intel,
   Windows x64) together with their signed update packages, writes `latest.json` and the
   checksums, and creates the draft release.
6. Open **Releases**, review the draft (the files listed below and the notes, which are the
   changelog section) and press **Publish release**.
7. After a final release, start the next cycle: keep `## [Unreleased]` at the top of
   `CHANGELOG.md` and update the link references at the bottom.

If the run fails, fix the cause and either re-run the failed jobs (same tag), or delete the
draft release and the tag (`git push origin :refs/tags/vX.Y.Z`), correct, and tag again.
Re-running only replaces files in a release that is still a draft; it refuses to touch a
published one.

A tag with a suffix (`v0.2.0-beta.1`) produces a release marked as a pre-release.

### The changelog is the source of the release notes

`CHANGELOG.md` follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/) and
Semantic Versioning, and it is the single source of the release notes: the workflow copies
the section of the version being released into the draft release (and into `latest.json`).
The app shows that text in the update dialog and, after an update, in "What's new", so
write the entries **for end users**: short, one line each, plain words, no crate names,
work-package numbers or file paths.

**While you work.** Add an entry under `## [Unreleased]` with every user-visible change, in
the same pull request. Group entries under these headings, in this order, only when they
have entries:

| Heading | Use for |
|---|---|
| `### ⚠️ Breaking changes` | Needs action, or something stops working as before (data format, removed options, changed defaults). An extension of the standard. |
| `### ✨ Added` | New features |
| `### 🔄 Changed` | Changes to existing behaviour |
| `### 🗑️ Deprecated` | Features that will go away |
| `### 🔥 Removed` | Features that went away |
| `### 🐛 Fixed` | Bug fixes |
| `### 🔒 Security` | Anything security related |

**At release time.** Rename or create the section of the version: `## [X.Y.Z] - YYYY-MM-DD`
(move the entries out of `[Unreleased]`, keep that heading, empty). Then update the link
references at the bottom (`[Unreleased]` compares the new tag with `HEAD`, `[X.Y.Z]` compares
the previous tag with the new one).

**Release candidates have no section of their own.** The tag `vX.Y.Z-rcN` uses the section
`[X.Y.Z]` and the notes start with the line "Release candidate N of X.Y.Z, for testing."
While X.Y.Z is not released, its heading is `## [X.Y.Z] - Unreleased`. The date is set when
the final version ships: a final tag `vX.Y.Z` **fails** if the heading still says
`Unreleased` (or the date is not `YYYY-MM-DD`), and any tag fails if the section is missing
or empty. A dry run (*Run workflow*) applies the same checks to the version in the project
files. (Until the final version exists, the `[X.Y.Z]` link reference may point to the latest
candidate tag; correct it when you release.)

Check the result locally with `python3 .github/scripts/changelog-notes.py X.Y.Z[-rcN]` and
`python3 .github/scripts/changelog-notes.py --self-test`.

**How the notes travel.** The `verify` job extracts them into a file (`release-notes`
artifact); `package` puts the same text in `latest.json` (`notes`); `publish` creates the
draft with that file as the body (`--notes-file`) plus a closing paragraph for first-time
installers (unsigned builds, checksums), which is not part of `latest.json`. The paragraph
is preceded by the line `<!-- portolano:install-note -->` (an HTML comment, invisible on
GitHub). The notes are only ever passed in files or as one quoted argument, never
interpolated into a shell script. Edit the draft's text before publishing only for small
corrections, and fix `CHANGELOG.md` too.

**Where the app reads them.** The update dialog and, after an update, "What's new" use the
`notes` of `latest.json` whenever it has any; that is the normal case and needs no request to
GitHub. Only if the manifest has no notes does the app fall back to the body of the GitHub
release (API request, with a size cap and a timeout), and it then cuts everything from the
marker on, so the install paragraph never shows inside the app. A body without the marker is
used as it is. The notes saved for the first start after an update follow the same rule. The
marker text lives in `release.yml` and in `INSTALL_NOTE_MARKER` (`src-tauri/src/update/notes.rs`);
a test fails if they drift apart.

## Dry run

*Actions → Release → Run workflow* builds everything from the selected branch and keeps the
installers, update packages, `latest.json` and checksums as workflow artifacts for 14 days.
It checks that the four versions agree with each other but publishes nothing and creates no
release. Like a tag run it needs the updater signing key secrets (see below).

## What the release contains

| File | For |
|---|---|
| `Portolano-X.Y.Z-macos-aarch64.dmg` | Macs with Apple silicon |
| `Portolano-X.Y.Z-macos-x86_64.dmg` | Intel Macs |
| `Portolano-X.Y.Z-windows-x64-setup.exe` | Windows 10/11, 64-bit (NSIS installer, per-user, no administrator rights) |
| `Portolano-X.Y.Z-macos-aarch64.app.tar.gz` and `.sig` | Auto-update package for Apple silicon and its signature |
| `Portolano-X.Y.Z-macos-x86_64.app.tar.gz` and `.sig` | Same for Intel Macs |
| `Portolano-X.Y.Z-windows-x64-setup.exe.sig` | Signature of the Windows installer, which is also the Windows update package (Tauri v2) |
| `latest.json` | Update manifest read by the app |
| `SHA256SUMS.txt` | SHA-256 of every other file above (`shasum -a 256 -c SHA256SUMS.txt`) |

Users only need the installers; the other files are for the updater.

## Automatic updates

### The updater signing key

Every update package is signed with a Tauri updater key pair, and the app refuses anything
that does not verify against the **public key** compiled into it
(`plugins.updater.pubkey` in `src-tauri/tauri.conf.json`). The **private key** lives only in
two GitHub repository secrets, `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (*Settings → Secrets and variables → Actions*); the
build step of `release.yml` is the only place that sees them, and it stops with an error if
either is empty. Dry runs need them too.

- This key is **unrelated to Apple or Windows code signing**. It does not remove the
  Gatekeeper or SmartScreen warnings, and it is needed even though the builds are unsigned.
- **If the private key or its password is lost, existing installs can never auto-update
  again**: they only trust the old public key, so users would have to install a new build by
  hand. Keep an offline backup of the key file and of the password, in a password manager or
  encrypted storage, separate from GitHub. Never commit it.
- If it leaks, anyone holding it could sign malicious updates that installed apps would
  accept, so treat it like a release credential and replace the key pair (a new public key
  can only reach existing installs through a release signed with the old key).

### How `latest.json` works

The app checks `https://github.com/reallovedone/portolano/releases/latest/download/latest.json`.
GitHub redirects `releases/latest` to the newest **published, non-pre-release** release, so
drafts and pre-releases are never offered to normal users. The file is generated by
`.github/scripts/make-latest-json.py`, which fails if a file or signature is missing or
unexpected. It looks like this:

```json
{
  "version": "0.2.0",
  "notes": "<the CHANGELOG.md section of 0.2.0>",
  "pub_date": "2026-11-16T12:00:00Z",
  "platforms": {
    "darwin-aarch64": {"signature": "<content of the .sig file>", "url": "https://github.com/reallovedone/portolano/releases/download/v0.2.0/Portolano-0.2.0-macos-aarch64.app.tar.gz"},
    "darwin-x86_64": {"signature": "...", "url": ".../Portolano-0.2.0-macos-x86_64.app.tar.gz"},
    "windows-x86_64": {"signature": "...", "url": ".../Portolano-0.2.0-windows-x64-setup.exe"}
  }
}
```

The `url`s point at the release's own assets, so do not rename or delete assets of a
published release: that would break updates.

### Testing an update with release candidates

The real flow cannot be simulated locally; use two pre-releases. GitHub's `releases/latest`
ignores pre-releases, so testers point the app at the candidate's own `latest.json` with the
`PORTOLANO_UPDATER_ENDPOINT` environment variable (https only).

1. Release `v0.2.0-rc1` (add the changelog section as `## [0.2.0] - Unreleased`, bump the four
   versions, tag, publish the draft; it is flagged as a pre-release automatically). Install it by hand from its installer.
2. Release `v0.2.0-rc2` the same way and publish it as a pre-release.
3. Start rc1 with the endpoint of rc2:
   `PORTOLANO_UPDATER_ENDPOINT=https://github.com/reallovedone/portolano/releases/download/v0.2.0-rc2/latest.json`
   - macOS, from a terminal:
     `PORTOLANO_UPDATER_ENDPOINT=<url> /Applications/Portolano.app/Contents/MacOS/portolano`
   - Windows, cmd: `set PORTOLANO_UPDATER_ENDPOINT=<url>` then start the installed
     `portolano.exe` from the same window; PowerShell:
     `$env:PORTOLANO_UPDATER_ENDPOINT = "<url>"` then start it from the same window.
4. Use *Check for updates…*, accept the update, and confirm that the app restarts as rc2 and
   shows the "What's new" dialog.

**The easy way for testers.** *Settings → Updates → Include pre-releases* (the
`includePrereleases` setting, off by default) makes the update check look up the newest
published release, pre-releases included, through the GitHub releases API
(`api.github.com/repos/reallovedone/portolano/releases`) and read the `latest.json` of that
release. The endpoint is built from the validated tag (`vX.Y.Z[-pre]`; drafts and other tags are
ignored); if the lookup fails the app falls back to the normal endpoint and logs a warning.
Signature checks and the no-downgrade rule are unchanged. `PORTOLANO_UPDATER_ENDPOINT` still
takes priority when set, so the procedure above keeps working for pinning an exact candidate.

Do this on both macOS and Windows before the final release.

### Moving from 0.1.0

0.1.0 has no updater. Users of 0.1.0 must install 0.2.0 (or later) by hand once; from then on
updates are offered inside the app. Updates downloaded by the app do not carry the macOS
quarantine flag, so the Gatekeeper warning does not come back, but the builds stay unsigned.

## Choices worth knowing

- **macOS 14 or later.** The isolated web profiles use WebKit data stores, which exist from
  macOS 14. This is `bundle.macOS.minimumSystemVersion`.
- **Ad-hoc signing on macOS.** `bundle.macOS.signingIdentity` is `"-"`, which signs the app
  ad hoc: required for the binary to run on Apple silicon, but it is not an identity, so
  Gatekeeper still warns (see the README for how users open the app).
- **WebView2 on Windows** uses `downloadBootstrapper`: the installer downloads the tiny
  Microsoft bootstrapper only if the runtime is missing (it is preinstalled on Windows 11 and
  on current Windows 10). For machines without internet access, switch to `offlineInstaller`
  in `tauri.conf.json` (adds about 127 MB to the installer).
- **No MSI.** The updater uses the NSIS installer on Windows.
- Third-party actions are pinned to full commit SHAs with the version in a comment;
  Dependabot proposes updates weekly. The build jobs have read-only access; only the last job
  (`publish`) can write, and only to create the draft release. The only other job that runs
  repository code besides the builds (`package`, which writes `latest.json`) is read-only too.

## Adding code signing later

Nothing here needs rewriting; the workflow has the placeholders.

**macOS (Apple Developer ID, notarisation).** Add the repository secrets
`APPLE_CERTIFICATE` (base64 of the `.p12`), `APPLE_CERTIFICATE_PASSWORD`,
`APPLE_SIGNING_IDENTITY`, and for notarisation `APPLE_ID`, `APPLE_PASSWORD` (app-specific
password) and `APPLE_TEAM_ID`; then uncomment the *Enable macOS signing and notarisation*
step in `release.yml`. The step exports the variables only when they are all present: Tauri
reads an *empty* `APPLE_*` variable as a real value and would fail, so never put the secrets
straight into the build step's `env:`. The environment variables override the `"-"` in
`tauri.conf.json`.

**Windows.** Set `bundle.windows.signCommand` (for a cloud signing service) or
`certificateThumbprint` in `tauri.conf.json`, add the credentials as secrets, and expose them
to the build step in the same conditional way. Until then the installer is unsigned and
SmartScreen warns.

## Known unknowns of the first run

The workflows were written without being able to run them. If the first run fails, the likely
places are: the Windows build (the `cfg(windows)` code had never been compiled), the
cross-compiled Intel Mac build, the Linux system packages in `ci.yml`, the NSIS license page
(`licenseFile`), and the notes flags of `gh release create`.
