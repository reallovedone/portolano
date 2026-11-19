// Global keyboard shortcuts. "Mod" is Cmd on macOS and Ctrl elsewhere.
//
// The handler is attached in the bubble phase on `window`, so a focused session component
// (terminal, remote desktop) can `stopPropagation()` for keys it needs (for example Ctrl+W
// in a remote shell) before they reach the shell.

export type ShortcutAction =
  | "newConnection"
  | "search"
  | "closeTab"
  | "nextTab"
  | "prevTab"
  | "lockVault"
  | "reconnectTab"
  | "openHome";

export interface KeyInfo {
  key: string;
  code: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
}

export function detectMac(): boolean {
  const nav = globalThis.navigator;
  if (!nav) return false;
  return /mac|iphone|ipad/i.test(nav.platform || nav.userAgent);
}

export function matchShortcut(e: KeyInfo, isMac: boolean): ShortcutAction | null {
  const mod = isMac ? e.metaKey : e.ctrlKey;
  const key = e.key.toLowerCase();

  // Ctrl+Tab / Ctrl+PageDown / Ctrl+PageUp work on both platforms.
  if (e.ctrlKey && !e.metaKey && !e.altKey) {
    if (key === "tab") return e.shiftKey ? "prevTab" : "nextTab";
    if (!e.shiftKey && key === "pagedown") return "nextTab";
    if (!e.shiftKey && key === "pageup") return "prevTab";
  }

  if (!mod || e.altKey) return null;
  if (isMac && e.ctrlKey) return null;
  if (!isMac && e.metaKey) return null;

  if (e.shiftKey) {
    if (isMac && e.code === "BracketRight") return "nextTab";
    if (isMac && e.code === "BracketLeft") return "prevTab";
    if (key === "l") return "lockVault";
    if (key === "r") return "reconnectTab";
    if (key === "h") return "openHome";
    return null;
  }
  switch (key) {
    case "n":
      return "newConnection";
    case "k":
      return "search";
    case "w":
      return "closeTab";
    default:
      return null;
  }
}

/** Human-readable shortcut for hints. */
export function shortcutLabel(action: ShortcutAction, isMac: boolean): string {
  const mod = isMac ? "⌘" : "Ctrl+";
  const shift = isMac ? "⇧" : "Shift+";
  switch (action) {
    case "newConnection":
      return `${mod}N`;
    case "search":
      return `${mod}K`;
    case "closeTab":
      return `${mod}W`;
    case "nextTab":
      return isMac ? `${mod}${shift}]` : "Ctrl+Tab";
    case "prevTab":
      return isMac ? `${mod}${shift}[` : "Ctrl+Shift+Tab";
    case "lockVault":
      return `${mod}${shift}L`;
    case "reconnectTab":
      return `${mod}${shift}R`;
    case "openHome":
      return `${mod}${shift}H`;
  }
}
