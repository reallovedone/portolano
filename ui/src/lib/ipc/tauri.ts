import { Channel, invoke } from "@tauri-apps/api/core";

import type { Ipc } from "./api";
import { IpcError, toIpcError } from "./errors";
import { decodeFbEvent, decodeTermEvent, toBytes } from "./frames";
import type { CredentialSaveFailed, FbEvent, TermEvent, UpdateEvent } from "./types";

/** Subscribes to a backend event (lazy import: the event API does not exist in a browser). */
function subscribe<P>(event: string, handler: (payload: P) => void): () => void {
  let unlisten: (() => void) | null = null;
  let stopped = false;
  void import("@tauri-apps/api/event")
    .then(({ listen }) => listen<P>(event, (e) => handler(e.payload)))
    .then((off) => {
      if (stopped) off();
      else unlisten = off;
    })
    // Without the listener the UI falls back to discovering the change on its next call.
    .catch(() => undefined);
  return () => {
    stopped = true;
    unlisten?.();
  };
}

async function call<T>(
  command: string,
  args?: Record<string, unknown> | Uint8Array,
  options?: { headers: Record<string, string> },
): Promise<T> {
  try {
    return await invoke<T>(command, args, options);
  } catch (e) {
    throw toIpcError(e);
  }
}

/** Binary channel: every message is a raw frame that `decode` turns into an event. */
function binaryChannel<E>(
  decode: (frame: Uint8Array) => E,
  onEvent: (event: E) => void,
  onDecodeError: (error: IpcError) => E,
): Channel<ArrayBuffer> {
  const channel = new Channel<ArrayBuffer>();
  channel.onmessage = (message) => {
    try {
      onEvent(decode(toBytes(message)));
    } catch (e) {
      onEvent(onDecodeError(toIpcError(e)));
    }
  };
  return channel;
}

const termDecodeError = (e: IpcError): TermEvent => ({
  type: "error",
  error: { code: e.code, message: e.message },
});
const fbDecodeError = (e: IpcError): FbEvent => ({
  type: "error",
  error: { code: e.code, message: e.message },
});

/** Real backend. Command names are the snake_case names of the Rust commands. */
export const tauriIpc: Ipc = {
  docGet: () => call("doc_get"),
  docUpsertNode: (node) => call("doc_upsert_node", { node }),
  docDeleteNode: (id) => call("doc_delete_node", { id }),
  docMoveNode: (id, newParentId) => call("doc_move_node", { id, newParentId }),
  nodeReorder: (id, newParentId, before, visibleOrder) =>
    call("node_reorder", { id, newParentId, before, visibleOrder: visibleOrder ?? null }),
  nodeSortByName: (parentId) => call("node_sort_by_name", { parentId }),

  vaultStatus: () => call("vault_status"),
  vaultCreate: (masterPassword) => call("vault_create", { masterPassword }),
  vaultUnlock: (masterPassword) => call("vault_unlock", { masterPassword }),
  vaultChangePassword: (currentPassword, newPassword) =>
    call("vault_change_password", { currentPassword, newPassword }),
  vaultLock: () => call("vault_lock"),
  vaultList: () => call("vault_list"),
  vaultUpsert: (input) => call("vault_upsert", { input }),
  vaultDelete: (id) => call("vault_delete", { id }),
  vaultRevealSecret: (id) => call("vault_reveal_secret", { id }),
  vaultGetAutoLock: () => call("vault_get_auto_lock"),
  vaultSetAutoLock: (minutes) => call("vault_set_auto_lock", { minutes }),
  vaultQuickUnlockInfo: () => call("vault_quick_unlock_info"),
  vaultSetQuickUnlock: (enabled) => call("vault_set_quick_unlock", { enabled }),
  vaultQuickUnlock: () => call("vault_quick_unlock"),

  termOpen: (connectionId, cols, rows, onEvent, options) =>
    call("term_open", {
      connectionId,
      cols,
      rows,
      onEvent: binaryChannel(decodeTermEvent, onEvent, termDecodeError),
      askCredentials: options?.askCredentials ?? false,
    }),
  termWrite: (sessionId, data) =>
    call("term_write", data, { headers: { "x-session-id": sessionId } }),
  termAck: (sessionId, bytes) => call("term_ack", { sessionId, bytes }),
  termResize: (sessionId, cols, rows) => call("term_resize", { sessionId, cols, rows }),
  sshHostKeyDecision: (sessionId, accept) => call("ssh_host_key_decision", { sessionId, accept }),
  knownHostsList: () => call("known_hosts_list"),
  knownHostsRemove: (host, port) => call("known_hosts_remove", { host, port }),

  fbOpen: (connectionId, width, height, onEvent, options) =>
    call("fb_open", {
      connectionId,
      width,
      height,
      onEvent: binaryChannel(decodeFbEvent, onEvent, fbDecodeError),
      askCredentials: options?.askCredentials ?? false,
    }),
  fbPointer: (sessionId, x, y, buttons, wheel) =>
    call("fb_pointer", { sessionId, x, y, buttons, wheel }),
  fbKey: (sessionId, code, down) => call("fb_key", { sessionId, code, down }),
  fbResize: (sessionId, width, height) => call("fb_resize", { sessionId, width, height }),
  fbSetVisible: (sessionId, visible) => call("fb_set_visible", { sessionId, visible }),
  fbCertDecision: (sessionId, accept) => call("fb_cert_decision", { sessionId, accept }),
  fbUnencryptedDecision: (sessionId, allow) =>
    call("fb_unencrypted_decision", { sessionId, allow }),
  fbForgetCertificate: (sessionId) => call("fb_forget_certificate", { sessionId }),
  fbClipboardSave: (sessionId) => call("fb_clipboard_save", { sessionId }),
  fbClipboardCancel: (sessionId) => call("fb_clipboard_cancel", { sessionId }),

  // `viewportHeight` is the UI's `innerHeight` at the time the bounds were measured: the
  // backend needs it for the title bar correction of child webviews (commands/web/geometry.rs).
  webOpen: (connectionId, bounds) =>
    call("web_open", { connectionId, bounds, viewportHeight: window.innerHeight }),
  webSetBounds: (sessionId, bounds, visible) =>
    call("web_set_bounds", { sessionId, bounds, visible, viewportHeight: window.innerHeight }),
  webNavigate: (sessionId, action) => call("web_navigate", { sessionId, action }),

  sessionClose: (sessionId) => call("session_close", { sessionId }),
  sessionCredential: (sessionId, answer, saveToVault) =>
    call("session_credential", { sessionId, answer, saveToVault }),
  sessionCredentialCancel: (sessionId) => call("session_credential_cancel", { sessionId }),

  importPreview: (format, path) => call("import_preview", { format, path }),
  importApply: (previewId, targetFolderId) => call("import_apply", { previewId, targetFolderId }),
  exportDocument: (format, path, nodeIds) => call("export_document", { format, path, nodeIds }),
  readPrivateKeyFile: (path) => call("read_private_key_file", { path }),

  updateSettings: () => call("update_settings"),
  updateSetAutoCheck: (enabled) => call("update_set_auto_check", { enabled }),
  updateCheck: (manual) => call("update_check", { manual }),
  updateSkip: (version) => call("update_skip", { version }),
  updateInstall: (onEvent) => {
    const channel = new Channel<UpdateEvent>();
    channel.onmessage = onEvent;
    return call("update_install", { onEvent: channel });
  },
  whatsNew: () => call("whats_new"),
  updateOpenReleasePage: (version) => call("update_open_release_page", { version }),
  updateOpenChangelog: () => call("update_open_changelog"),

  onVaultLocked: (handler) => subscribe<null>("vault-locked", () => handler()),
  onCheckForUpdatesMenu: (handler) => subscribe<null>("menu-check-for-updates", () => handler()),
  onCloseTabMenu: (handler) => subscribe<null>("menu-close-tab", () => handler()),
  onQuitRequested: (handler) => subscribe<null>("quit-requested", () => handler()),
  onCredentialSaveFailed: (handler) =>
    subscribe<CredentialSaveFailed>("credential-save-failed", handler),

  quitReply: (reply) => call("quit_reply", { reply }),

  menuSetSessionKeys: (captured) => call("menu_set_session_keys", { captured }),

  clipboardReadText: () => call("clipboard_read_text"),
  uiLog: (level, message) => call("ui_log", { level, message }),
};
