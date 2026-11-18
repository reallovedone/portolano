// Subscriptions to the notices the backend sends on its own (Tauri events), wired to the
// stores. Called once by the shell; the returned function unsubscribes everything.

import { t } from "../i18n/strings";
import type { App } from "./app";

// Holds the app while the confirmation this module opened for a quit is on screen. A confirmation opened
// by something else (closing a tab, installing an update) does not set it.
const quitDialogOpen = new WeakSet<App>();

/**
 * Answers `quit-requested`. With nothing connected: `quit` at once. Otherwise `asking` first (it
 * stops the backend's timer, so it must go out before the dialog opens), then the one
 * confirmation. A second request while the quit dialog is open (quit pressed twice, or the
 * backend asking again) only answers `asking`: still one dialog, and the user's choice is sent
 * when made. If a confirmation for something else is open (closing a tab, installing an update)
 * the request is cancelled: never two dialogs, and a dialog about something else must not
 * approve a quit.
 */
async function answerQuitRequest(app: App): Promise<void> {
  try {
    if (quitDialogOpen.has(app)) {
      await app.ipc.quitReply("asking");
      return;
    }
    if (app.tabs.liveCount() === 0) {
      await app.ipc.quitReply("quit");
      return;
    }
    await app.ipc.quitReply("asking");
    quitDialogOpen.add(app);
    let ok: boolean;
    try {
      ok = await app.closeConfirm.confirmClose(app.tabs.tabs.map((tab) => tab.id));
    } finally {
      quitDialogOpen.delete(app);
    }
    await app.ipc.quitReply(ok ? "quit" : "cancel");
  } catch (e) {
    // Nothing more to do here: without an answer the backend quits by itself after 2 seconds.
    app.ipc
      .uiLog("warn", `quit reply failed: ${e instanceof Error ? e.message : String(e)}`)
      // Safe to drop: the line is only a diagnostic and the backend's timer does not depend on it.
      .catch(() => undefined);
  }
}

export function connectBackendEvents(app: App): () => void {
  const subscriptions = [
    // The vault locked itself or was locked: show it now instead of on the next call.
    app.vault.listen(),
    // macOS menu bar "Check for Updates…": the same dialog as the sidebar entry.
    app.ipc.onCheckForUpdatesMenu(() => {
      const { ui, updates, notices } = app;
      // Already on screen (the check, an offer, an installation): never stack a second one.
      if (ui.dialog?.type === "updates" || updates.offer || updates.installing) return;
      // Another dialog may hold unsaved input: do not replace it.
      if (ui.dialog) {
        notices.push("info", t("update.menuBusy"));
        return;
      }
      ui.open({ type: "updates" });
    }),
    // macOS menu bar "Close Tab" (⌘W): closes the active tab, asking first if it is connected.
    app.ipc.onCloseTabMenu(() => {
      const id = app.tabs.state.activeId;
      if (id) void app.closeConfirm.closeTab(id);
    }),
    // ⌘Q, menu Quit, Dock → Quit, the window's close button: the backend stopped it and waits
    // for us (2 seconds, then it quits anyway).
    app.ipc.onQuitRequested(() => void answerQuitRequest(app)),
    // Not blocking: the session is connected, only the "save to the vault" step failed.
    app.ipc.onCredentialSaveFailed((notice) => {
      app.notices.push(
        "warning",
        t("vault.saveFailedTitle"),
        t("vault.saveFailedDetail", { name: notice.connectionName, message: notice.message }),
      );
    }),
  ];
  return () => {
    for (const unsubscribe of subscriptions) unsubscribe();
  };
}
