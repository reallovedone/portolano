# Changelog

All notable changes to Portolano are listed here. The format follows
[Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Sections marked ⚠️ need your
attention when you upgrade.

## [Unreleased]

## [0.4.1] - Unreleased

### 🔄 Changed

- The credentials window is easier to use with many credentials: search, a filter by type, a count, and how many connections use each one; edit and delete moved into a ⋯ menu on each row.
- The vault *auto-lock* time and the *Unlock with Touch ID* / *Windows Hello* option moved from the credentials window to a new *Security* section in Settings.

### 🐛 Fixed

- Compact list density no longer squeezes a connection's host against the rows around it, the *System* theme preview in Settings is as tall as *Light* and *Dark*, and only one separator line sits above *Updates* in Settings.
- Saving files copied on a remote desktop no longer leaves the progress notice stuck at 0 after small files have already been saved.
- Shared folders: a web connection is listed by its address only, a shared folder that no longer exists disappears from the list after you unsubscribe, and deleting a folder that still holds shared folders explains why it asked again.
- Shared folders show their state with one clear icon next to the name (up to date, offline, update refused, error, or hosts to confirm), with the reason and the last update in its tooltip and in *Share info…*, which now also has *Sync now*.
- A shared folder whose source has never been reachable on this computer stays listed with *Sync now*, *Share info…* and *Unsubscribe…* instead of disappearing, and one slow or unreachable source no longer holds up the others.
- Buttons inside text fields, such as *Browse…* and the eye that shows a password, now fit the field's edges, and the list in the credentials window has a frame of its own.
- Before you add a shared folder, its preview now also names the connections whose unsafe options (skipping the certificate check, connecting without NLA, clipboard sharing, a custom Kerberos KDC) were turned off, and a folder edited while it is being published is no longer published with its old content.

## [0.4.0] - 2026-11-22

### ⚠️ Breaking changes

- The connections file moves to a newer format: once 0.4.0 has opened it, earlier versions of Portolano can no longer open it. If you may go back to 0.3.x, keep a copy of the file first.

### ✨ Added

- Connections and folders can have their own icon (40 to choose from, with search) and colour, picked in the editor under *General*: the icon replaces the default one in the list, and the colour shows as a dot after the name and as a thin bar on the tab of an open connection.
- A new Settings window (⋯ menu, or *Settings…* with ⌘, on macOS) lets you choose the theme (system, light or dark), a colour theme (Portolano, Nord, Solarized or Rosé Pine, each with a light and a dark look), one of five accent colours, a compact list density and the terminal font size; changes apply immediately, including to terminals that are already open, and the window opens in the right theme without a white flash.
- Folders and connections can be put in your own order: drag them between others (a line shows where they land), use *Move up* and *Move down* in the right-click menu, or go back to alphabetical with *Sort by name* in a folder's menu or the sidebar's more-actions menu.
- *Reconnect* in the tab menu, and ⌘⇧R (Ctrl+Shift+R on Windows), closes the session and opens it again, for every kind of connection, even while it is still connected.
- A *Home* tab, from the house button in the tab strip or ⌘⇧H (Ctrl+Shift+H on Windows), keeps the welcome screen with its connection tiles one click away next to your sessions.
- A slim toolbar above every session with Reconnect, Duplicate and Close, a *Send Ctrl+Alt+Del* button for remote desktops (RDP and VNC), and on the right the host, the remote screen size and a lock that tells how the connection is protected. Web pages keep their back, forward, reload and address controls in the same row.
- The session toolbar can be slid away and back with the small tab under its middle (web pages keep their navigation and address visible), or from *Show session toolbar* in the sidebar's more-actions menu.
- A new *Release notes* window, from *Help → Release Notes…* on macOS or from Settings, lists what changed in every version; release notes show screenshots of the new features in the look you use, and after an update the *What's new* window links to it when older versions do not fit.
- Shared folders: publish a folder as a signed file on a network drive, OneDrive, Nextcloud or an internal web server, and subscribe to folders shared by colleagues. Shared folders are read-only; you choose the credentials.
- A new *Updates* section in Settings groups the automatic check, the version, *Check for updates…* and the release notes, and adds *Include pre-releases*: turn it on to also be offered test versions (release candidates) before they are final.

### 🔄 Changed

- After an update, the release notes of every version you skipped are shown; they are no longer shown before installing.
- The RDP connection editor is shorter: options are grouped into Display, Sign-in, Clipboard & keyboard and Security sections, and long explanations open with "Learn more".
- Tabs are wider, so more of the connection name is visible.

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

[Unreleased]: https://github.com/reallovedone/portolano/compare/v0.4.1-rc1...HEAD
[0.4.1]: https://github.com/reallovedone/portolano/compare/v0.4.0...v0.4.1-rc1
[0.4.0]: https://github.com/reallovedone/portolano/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/reallovedone/portolano/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/reallovedone/portolano/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/reallovedone/portolano/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/reallovedone/portolano/releases/tag/v0.1.0
