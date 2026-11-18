// Keyboard policy of the terminal: which key combinations copy/paste (platform conventions)
// and which ones the shell's global shortcuts must not steal from the remote program.

import { matchShortcut, type KeyInfo, type ShortcutAction } from "../../shortcuts";

export type ClipboardAction = "copy" | "paste";

/**
 * Copy / paste combinations.
 * - macOS: Cmd+C copies when there is a selection (otherwise the key is not ours), Cmd+V pastes.
 * - Windows/Linux (Windows Terminal conventions): Ctrl+Shift+C / Ctrl+Insert copy, Ctrl+C copies
 *   only when there is a selection (otherwise it stays SIGINT), Ctrl+V / Ctrl+Shift+V /
 *   Shift+Insert paste. Ctrl+V is therefore not sent to the remote program (no vim block mode
 *   or shell literal-next with it). Right-click is handled by the view, not here.
 * Paste is performed by the browser's native `paste` event, which xterm turns into input
 * (bracketed paste included): the terminal must only not send the key itself.
 */
export function clipboardAction(
  e: KeyInfo,
  isMac: boolean,
  hasSelection: boolean,
): ClipboardAction | null {
  const key = e.key.toLowerCase();
  if (isMac) {
    if (!e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return null;
    if (key === "c") return hasSelection ? "copy" : null;
    if (key === "v") return "paste";
    return null;
  }
  if (e.metaKey || e.altKey) return null;
  if (e.ctrlKey && e.shiftKey) {
    if (key === "c") return "copy";
    if (key === "v") return "paste";
    return null;
  }
  if (e.ctrlKey && key === "v") return "paste";
  if (e.ctrlKey && key === "c") return hasSelection ? "copy" : null;
  if (e.ctrlKey && key === "insert") return hasSelection ? "copy" : null;
  if (e.shiftKey && key === "insert") return "paste";
  return null;
}

const STOLEN_ON_NON_MAC: ReadonlySet<ShortcutAction> = new Set(["closeTab", "search", "newConnection"]);

/**
 * True when the key combination is a global shortcut the remote program needs: on Windows and
 * Linux the shell's Ctrl+W / Ctrl+K / Ctrl+N are readline and editor keys. On macOS the shell
 * uses Cmd, which terminals do not. Tab switching (Ctrl+Tab, Ctrl+PageUp/Down) stays global.
 */
export function keepForTerminal(e: KeyInfo, isMac: boolean): boolean {
  if (isMac) return false;
  const action = matchShortcut(e, false);
  return action !== null && STOLEN_ON_NON_MAC.has(action);
}
