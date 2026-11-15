# Releasing Portolano

Releases are built by GitHub Actions (`.github/workflows/release.yml`); nothing has to be
built locally. A release is always created as a **draft**: you review it and publish it by
hand.

## Cutting a release

1. Pick the version `X.Y.Z` and set it in these four places (they must be identical; the
   workflow fails in its first step otherwise):
   - `Cargo.toml` → `[workspace.package]` `version`
   - `src-tauri/tauri.conf.json` → `version`
   - `package.json` → `version`
   - `ui/package.json` → `version`

   Then run `cargo check --workspace` so that `Cargo.lock` picks up the new version (CI
   builds with `--locked` and would fail otherwise).
2. Run `bash dev/check.sh`, commit, and merge to `main` through the usual pull request.
3. Tag the merged commit and push the tag:

   ```bash
   git switch main && git pull
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```

4. Open the **Actions** tab and watch the *Release* run. It checks the tag against the four
   versions, builds three installers in parallel (macOS Apple Silicon, macOS Intel,
   Windows x64), computes the checksums, and creates the draft release.
5. Open **Releases**, review the draft (generated notes, the three installers and
   `SHA256SUMS.txt`), edit the notes if you like, and press **Publish release**.

If the run fails, fix the cause and either re-run the failed jobs (same tag), or delete the
draft release and the tag (`git push origin :refs/tags/vX.Y.Z`), correct, and tag again.
Re-running only replaces files in a release that is still a draft; it refuses to touch a
published one.

A tag with a suffix (`v0.2.0-beta.1`) produces a release marked as a pre-release.

## Dry run

*Actions → Release → Run workflow* builds everything from the selected branch and keeps the
installers (and the checksums) as workflow artifacts for 14 days. It checks that the four
versions agree with each other but publishes nothing and creates no release.

## What the release contains

| File | For |
|---|---|
| `Portolano-X.Y.Z-macos-aarch64.dmg` | Macs with Apple silicon |
| `Portolano-X.Y.Z-macos-x86_64.dmg` | Intel Macs |
| `Portolano-X.Y.Z-windows-x64-setup.exe` | Windows 10/11, 64-bit (NSIS installer, per-user, no administrator rights) |
| `SHA256SUMS.txt` | SHA-256 of the files above (`shasum -a 256 -c SHA256SUMS.txt`) |

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
- **No MSI, no auto-updater.**
- Third-party actions are pinned to full commit SHAs with the version in a comment;
  Dependabot proposes updates weekly. The build jobs have read-only access; only the last job
  (`publish`) can write, and only to create the draft release.

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
