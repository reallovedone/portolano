# Changelog

All notable changes to Portolano are listed here. The format follows
[Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Sections marked ⚠️ need your
attention when you upgrade.

## [Unreleased]

### ✨ Added

- Folders and connections can be put in your own order: drag them between others (a line shows where they land), use *Move up* and *Move down* in the right-click menu, or go back to alphabetical with *Sort by name* in a folder's menu or the sidebar's more-actions menu.
- *Reconnect* in the tab menu, and ⌘⇧R (Ctrl+Shift+R on Windows), closes the session and opens it again, for every kind of connection, even while it is still connected.
- A *Home* tab, from the house button in the tab strip or ⌘⇧H (Ctrl+Shift+H on Windows), keeps the welcome screen with its connection tiles one click away next to your sessions.
- A slim toolbar above every session with Reconnect, Duplicate and Close, a *Send Ctrl+Alt+Del* button for remote desktops (RDP and VNC), and on the right the host, the remote screen size and a lock that tells how the connection is protected. Web pages keep their back, forward, reload and address controls in the same row.
- The session toolbar can be slid away and back with the small tab under its middle (web pages keep their navigation and address visible), or from *Show session toolbar* in the sidebar's more-actions menu.

### 🔄 Changed

- A connections file saved by 0.4.0 still opens in 0.3.x, but if you go back to 0.3.x and save there, your custom order, icons and colours are lost.
- After an update, the release notes of every version you skipped are shown; they are no longer shown before installing.
- The RDP connection editor is shorter: options are grouped into Display, Sign-in, Clipboard & keyboard and Security sections, and long explanations open with "Learn more".

### 🐛 Fixed

- RDP sessions left in a background tab no longer drop with "operation timed out": the connection is kept alive while idle. If it is lost anyway, the tab says "The connection to the server was lost." and offers Reconnect.

## [0.3.1] - 2026-11-18

### ✨ Added

- RDP connections to Active Directory servers can sign in with Kerberos, needed where NTLM is disabled; Portolano uses it automatically with a full server name and a domain account, and an optional KDC can be set per connection.

## [0.3.0] - 2026-11-18

### ✨ Added

- *Expand all* in the sidebar menu opens every folder at once, next to *Collapse all*.
- The status bar shows the running version and a dot with the update status (up to date, update available, or unknown); click it to open the update dialog.
- Choose when the vault locks itself after inactivity (never, 5, 15, 30 minutes or 1 hour) in the new *Security* section of the credentials window. The default stays 15 minutes.
- Portolano asks for confirmation before closing a tab, closing the window, quitting or installing an update when that would end a connected SSH, terminal, RDP or VNC session. Web tabs and disconnected sessions close without asking.
- With *Use Mac shortcuts* on an RDP connection, ⌘C, ⌘V, ⌘X, ⌘A, ⌘Z and every other ⌘ combination reach Windows as Ctrl+key. ⌘Q, ⌘H, ⌘M and ⌘W stay with Portolano. ⌘W closes the current tab (asking first when a session is connected).
- Optional unlock with Touch ID (macOS) or Windows Hello (Windows): turn it on in the *Security* section of the credentials window. After you unlock once with the master password it works for as long as Portolano stays open; the master password always works too.

### 🔄 Changed

- On Windows and Linux the terminal pastes with Ctrl+V and right-click (right-click copies the selection if there is one), like Windows Terminal.

## [0.2.0] - 2026-11-17

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

[Unreleased]: https://github.com/reallovedone/portolano/compare/v0.3.1...HEAD
[0.3.1]: https://github.com/reallovedone/portolano/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/reallovedone/portolano/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/reallovedone/portolano/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/reallovedone/portolano/releases/tag/v0.1.0
