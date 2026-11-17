# Changelog

All notable changes to Portolano are listed here. The format follows
[Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Sections marked ⚠️ need your
attention when you upgrade.

## [Unreleased]

## [0.2.0] - Unreleased

### ⚠️ Breaking changes

- Version 0.1.0 has no updater: install 0.2.0 by hand once. From then on, updates are offered inside the app.
- Building from source now needs Rust 1.90 or later.

### ✨ Added

- Automatic updates: Portolano checks for a new version at start-up and once a day, shows what is new, and asks before installing. It never installs anything silently.
- *Check for Updates…* in the macOS app menu and in the sidebar menu, and a setting to turn the automatic check off.
- Skip a version you do not want, or choose *Later* to be asked again.
- After an update, a "What's new" window shows the changes once.
- Shared clipboard in RDP, as a per-connection option (off by default): copy and paste text in both directions.
- Copy files and folders from your computer and paste them on the remote desktop.
- Save files copied on the remote desktop to a folder you choose.
- *Use Mac shortcuts* for RDP on macOS, on by default for new connections: ⌘C, ⌘V and other ⌘ shortcuts work as Ctrl shortcuts on the remote desktop, and ⌘ alone opens the Start menu. Existing connections can turn it on in the editor.

### 🔄 Changed

- Portolano now contacts GitHub to look for updates. No personal data is sent, and you can turn the check off.
- Updates are verified with the project's signing key before they are installed.

### 🐛 Fixed

- "Check for updates" no longer reports an error when the latest release has no update information. It now says you are up to date.

### 🔒 Security

- RDP clipboard sharing is off by default, and imported connections never turn it on.
- Passwords revealed from the vault are never shared with the remote desktop.

## [0.1.0] - 2026-11-16

First public release.

### ✨ Added

- SSH connections and local terminals, with password or key login, jump hosts and host-key checks.
- RDP with Network Level Authentication.
- VNC, including encrypted connections (VeNCrypt/TLS).
- Web consoles in tabs, optionally with their own isolated cookies.
- A tree of folders and connections, with search, tags and drag and drop.
- An optional encrypted vault for credentials.
- Import from MobaXterm, OpenSSH config, `.rdp` files, mRemoteNG and CSV; export to JSON and `.rdp`.
- Light and dark themes that follow the system.
- Apple silicon and Intel Macs, and Windows 10/11 (64-bit).

[Unreleased]: https://github.com/reallovedone/portolano/compare/v0.2.0-rc2...HEAD
[0.2.0]: https://github.com/reallovedone/portolano/compare/v0.1.0...v0.2.0-rc2
[0.1.0]: https://github.com/reallovedone/portolano/releases/tag/v0.1.0
