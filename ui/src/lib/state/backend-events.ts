// Subscriptions to the notices the backend sends on its own (Tauri events), wired to the
// stores. Called once by the shell; the returned function unsubscribes everything.

import { t } from "../i18n/strings";
import type { App } from "./app";

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
