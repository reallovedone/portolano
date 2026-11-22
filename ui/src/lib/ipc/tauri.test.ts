import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => {
  class Channel<T> {
    onmessage: (message: T) => void = () => undefined;
  }
  return { invoke: invokeMock, Channel };
});

import type { Ipc } from "./api";
import { IpcError } from "./errors";
import { tauriIpc } from "./tauri";

// Contract commands plus `session_close`, the credential-prompt commands
// of WP11 and the extra `ui_log`.
const EXPECTED_COMMANDS = [
  "doc_get",
  "doc_upsert_node",
  "doc_delete_node",
  "doc_move_node",
  "node_reorder",
  "node_sort_by_name",
  "vault_status",
  "vault_create",
  "vault_unlock",
  "vault_change_password",
  "vault_lock",
  "vault_list",
  "vault_upsert",
  "vault_delete",
  "vault_reveal_secret",
  "vault_get_auto_lock",
  "vault_set_auto_lock",
  "vault_quick_unlock_info",
  "vault_set_quick_unlock",
  "vault_quick_unlock",
  "term_open",
  "term_write",
  "term_ack",
  "term_resize",
  "ssh_host_key_decision",
  "known_hosts_list",
  "known_hosts_remove",
  "fb_open",
  "fb_pointer",
  "fb_key",
  "fb_resize",
  "fb_set_visible",
  "fb_cert_decision",
  "fb_unencrypted_decision",
  "fb_forget_certificate",
  "fb_clipboard_save",
  "fb_clipboard_cancel",
  "web_open",
  "web_set_bounds",
  "web_navigate",
  "session_close",
  "session_credential",
  "session_credential_cancel",
  "import_preview",
  "import_apply",
  "export_document",
  "read_private_key_file",
  "appearance_get",
  "appearance_set",
  "update_settings",
  "update_set_auto_check",
  "update_check",
  "update_skip",
  "update_install",
  "whats_new",
  "release_notes",
  "update_open_release_page",
  "update_open_changelog",
  "quit_reply",
  "menu_set_session_keys",
  "clipboard_read_text",
  "ui_log",
  "identity_get",
  "identity_create",
  "identity_export",
  "identity_import",
  "share_publish",
  "share_republish",
  "share_preview",
  "share_subscribe",
  "share_sync",
  "share_unsubscribe",
  "share_set_credential",
  "share_confirm_targets",
];

const noop = () => undefined;
const bounds = { x: 0, y: 0, width: 1, height: 1 };

describe("tauriIpc", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
  });

  it("invokes every contract command by its snake_case name", async () => {
    const calls: Array<() => Promise<unknown>> = [
      () => tauriIpc.docGet(),
      () => tauriIpc.docUpsertNode({ type: "folder", id: "a", parentId: null, name: "f", credential: { type: "inherit" } }),
      () => tauriIpc.docDeleteNode("a"),
      () => tauriIpc.docMoveNode("a", null),
      () => tauriIpc.nodeReorder("a", null, "b"),
      () => tauriIpc.nodeSortByName(null),
      () => tauriIpc.vaultStatus(),
      () => tauriIpc.vaultCreate("p"),
      () => tauriIpc.vaultUnlock("p"),
      () => tauriIpc.vaultChangePassword("p", "q"),
      () => tauriIpc.vaultLock(),
      () => tauriIpc.vaultList(),
      () => tauriIpc.vaultUpsert({ id: null, name: "n", username: "u", domain: null, secret: { type: "password", password: "p" } }),
      () => tauriIpc.vaultDelete("a"),
      () => tauriIpc.vaultRevealSecret("a"),
      () => tauriIpc.vaultGetAutoLock(),
      () => tauriIpc.vaultSetAutoLock(30),
      () => tauriIpc.vaultQuickUnlockInfo(),
      () => tauriIpc.vaultSetQuickUnlock(true),
      () => tauriIpc.vaultQuickUnlock(),
      () => tauriIpc.termOpen("a", 80, 24, noop),
      () => tauriIpc.termWrite("s", new Uint8Array([1])),
      () => tauriIpc.termAck("s", 1),
      () => tauriIpc.termResize("s", 80, 24),
      () => tauriIpc.sshHostKeyDecision("s", true),
      () => tauriIpc.knownHostsList(),
      () => tauriIpc.knownHostsRemove("h", 22),
      () => tauriIpc.fbOpen("a", 800, 600, noop),
      () => tauriIpc.fbPointer("s", 1, 2, 0, 0),
      () => tauriIpc.fbKey("s", "KeyA", true),
      () => tauriIpc.fbResize("s", 800, 600),
      () => tauriIpc.fbSetVisible("s", false),
      () => tauriIpc.fbCertDecision("s", true),
      () => tauriIpc.fbUnencryptedDecision("s", false),
      () => tauriIpc.fbForgetCertificate("s"),
      () => tauriIpc.fbClipboardSave("s"),
      () => tauriIpc.fbClipboardCancel("s"),
      () => tauriIpc.webOpen("a", bounds),
      () => tauriIpc.webSetBounds("s", bounds, true),
      () => tauriIpc.webNavigate("s", { type: "reload" }),
      () => tauriIpc.sessionClose("s"),
      () =>
        tauriIpc.sessionCredential(
          "s",
          { username: "u", password: "p", keyPath: null, passphrase: null, domain: null },
          false,
        ),
      () => tauriIpc.sessionCredentialCancel("s"),
      () => tauriIpc.importPreview("csv", "/p"),
      () => tauriIpc.importApply("a", null),
      () => tauriIpc.exportDocument("json", "/p", null),
      () => tauriIpc.readPrivateKeyFile("/p"),
      () => tauriIpc.appearanceGet(),
      () =>
        tauriIpc.appearanceSet({ theme: "dark", scheme: "nord", accent: "teal", density: "compact", terminalFontSize: 16 }),
      () => tauriIpc.updateSettings(),
      () => tauriIpc.updateSetAutoCheck(false),
      () => tauriIpc.updateCheck(true),
      () => tauriIpc.updateSkip("0.2.0"),
      () => tauriIpc.updateInstall(noop),
      () => tauriIpc.whatsNew(),
      () => tauriIpc.releaseNotes(),
      () => tauriIpc.updateOpenReleasePage("0.2.0"),
      () => tauriIpc.updateOpenChangelog(),
      () => tauriIpc.quitReply("asking"),
      () => tauriIpc.menuSetSessionKeys(true),
      () => tauriIpc.clipboardReadText(),
      () => tauriIpc.uiLog("info", "m"),
      () => tauriIpc.identityGet(),
      () => tauriIpc.identityCreate("Ada"),
      () => tauriIpc.identityExport("/p", "pw"),
      () => tauriIpc.identityImport("/p", "pw"),
      () => tauriIpc.sharePublish("f", "/p"),
      () => tauriIpc.shareRepublish("l"),
      () => tauriIpc.sharePreview({ type: "path", value: "/p" }),
      () => tauriIpc.shareSubscribe("p", null),
      () => tauriIpc.shareSync(null),
      () => tauriIpc.shareUnsubscribe("l"),
      () => tauriIpc.shareSetCredential("n", { type: "none" }),
      () => tauriIpc.shareConfirmTargets("l", 3),
    ];
    for (const run of calls) await run();
    const used = invokeMock.mock.calls.map((c) => c[0] as string);
    expect(used).toEqual(EXPECTED_COMMANDS);
    // Every command method of the interface is covered by one of the calls above (the `on…`
    // methods subscribe to events and are not commands).
    expect(Object.keys(tauriIpc).filter((name) => !name.startsWith("on")).length).toBe(calls.length);
  });

  it("tauri_maps_share_commands_and_read_only_error", async () => {
    await tauriIpc.sharePreview({ type: "url", value: "https://x.example/s" });
    expect(invokeMock).toHaveBeenLastCalledWith(
      "share_preview",
      { location: { type: "url", value: "https://x.example/s" } },
      undefined,
    );
    await tauriIpc.shareSubscribe("p", "f");
    expect(invokeMock).toHaveBeenLastCalledWith(
      "share_subscribe",
      { previewId: "p", parentId: "f" },
      undefined,
    );
    await tauriIpc.shareSync(null);
    expect(invokeMock).toHaveBeenLastCalledWith("share_sync", { linkId: null }, undefined);
    await tauriIpc.shareSetCredential("n", { type: "id", id: "c" });
    expect(invokeMock).toHaveBeenLastCalledWith(
      "share_set_credential",
      { nodeId: "n", credential: { type: "id", id: "c" } },
      undefined,
    );
    // The revision the user saw travels with the confirmation.
    await tauriIpc.shareConfirmTargets("l", 7);
    expect(invokeMock).toHaveBeenLastCalledWith(
      "share_confirm_targets",
      { linkId: "l", revision: 7 },
      undefined,
    );
    await tauriIpc.identityGet();
    expect(invokeMock).toHaveBeenLastCalledWith("identity_get", undefined, undefined);
    await tauriIpc.identityCreate("Ada");
    expect(invokeMock).toHaveBeenLastCalledWith("identity_create", { name: "Ada" }, undefined);
    await tauriIpc.identityExport("/tmp/id.bak", "pw");
    expect(invokeMock).toHaveBeenLastCalledWith(
      "identity_export",
      { path: "/tmp/id.bak", password: "pw" },
      undefined,
    );
    await tauriIpc.identityImport("/tmp/id.bak", "pw");
    expect(invokeMock).toHaveBeenLastCalledWith(
      "identity_import",
      { path: "/tmp/id.bak", password: "pw" },
      undefined,
    );
    await tauriIpc.sharePublish("f", "/tmp/s.json");
    expect(invokeMock).toHaveBeenLastCalledWith(
      "share_publish",
      { folderId: "f", path: "/tmp/s.json" },
      undefined,
    );
    await tauriIpc.shareRepublish("l");
    expect(invokeMock).toHaveBeenLastCalledWith("share_republish", { linkId: "l" }, undefined);
    // Errors of a share command keep their code.
    for (const code of ["read_only", "conflict"]) {
      invokeMock.mockRejectedValueOnce({ code, message: "m" });
      const error = await tauriIpc.shareSubscribe("p", "f").catch((e: unknown) => e);
      expect(error).toBeInstanceOf(IpcError);
      expect((error as IpcError).code).toBe(code);
      invokeMock.mockRejectedValueOnce({ code, message: "m" });
      const stale = await tauriIpc.shareConfirmTargets("l", 1).catch((e: unknown) => e);
      expect((stale as IpcError).code).toBe(code);
      invokeMock.mockRejectedValueOnce({ code, message: "m" });
      const doc = await tauriIpc.docDeleteNode("n").catch((e: unknown) => e);
      expect((doc as IpcError).code).toBe(code);
    }
  });

  it("sends the update arguments with camelCase names and streams progress", async () => {
    await tauriIpc.updateCheck(true);
    expect(invokeMock).toHaveBeenLastCalledWith("update_check", { manual: true }, undefined);
    await tauriIpc.updateSetAutoCheck(false);
    expect(invokeMock).toHaveBeenLastCalledWith(
      "update_set_auto_check",
      { enabled: false },
      undefined,
    );
    const events: unknown[] = [];
    invokeMock.mockImplementation(
      async (_cmd: string, args: { onEvent: { onmessage: (m: unknown) => void } }) => {
        args.onEvent.onmessage({ type: "progress", downloaded: 1, total: 2 });
        args.onEvent.onmessage({ type: "installing" });
      },
    );
    await tauriIpc.updateInstall((e) => events.push(e));
    expect(events).toEqual([{ type: "progress", downloaded: 1, total: 2 }, { type: "installing" }]);
  });

  it("sends the session id header and raw bytes for term_write", async () => {
    const data = new Uint8Array([1, 2, 3]);
    await tauriIpc.termWrite("sess-1", data);
    expect(invokeMock).toHaveBeenCalledWith("term_write", data, {
      headers: { "x-session-id": "sess-1" },
    });
  });

  it("adds the UI viewport height to the web commands that place a webview", async () => {
    const bounds = { x: 1, y: 2, width: 3, height: 4 };
    await tauriIpc.webOpen("c", bounds);
    await tauriIpc.webSetBounds("s", bounds, true);
    const viewportHeight = window.innerHeight;
    expect(invokeMock).toHaveBeenCalledWith(
      "web_open",
      { connectionId: "c", bounds, viewportHeight },
      undefined,
    );
    expect(invokeMock).toHaveBeenCalledWith(
      "web_set_bounds",
      { sessionId: "s", bounds, visible: true, viewportHeight },
      undefined,
    );
  });

  it("uses camelCase argument names", async () => {
    await tauriIpc.vaultChangePassword("p", "q");
    expect(invokeMock).toHaveBeenCalledWith(
      "vault_change_password",
      { currentPassword: "p", newPassword: "q" },
      undefined,
    );
  });

  it("sends the quick unlock arguments and keeps its three error codes", async () => {
    await tauriIpc.vaultSetQuickUnlock(true);
    expect(invokeMock).toHaveBeenLastCalledWith("vault_set_quick_unlock", { enabled: true }, undefined);
    for (const code of ["quick_unlock_cancelled", "quick_unlock_failed", "quick_unlock_not_ready"]) {
      invokeMock.mockRejectedValueOnce({ code, message: "m" });
      const error = await tauriIpc.vaultQuickUnlock().catch((e: unknown) => e);
      expect(error).toBeInstanceOf(IpcError);
      expect((error as IpcError).code).toBe(code);
    }
  });

  it("normalizes backend errors into IpcError", async () => {
    invokeMock.mockRejectedValue({ code: "unsupported", message: "nope" });
    const error = await tauriIpc.docGet().catch((e: unknown) => e);
    expect(error).toBeInstanceOf(IpcError);
    expect((error as IpcError).code).toBe("unsupported");

    invokeMock.mockRejectedValue("plain string");
    const other = await tauriIpc.docGet().catch((e: unknown) => e);
    expect((other as IpcError).code).toBe("internal");
  });

  it("decodes channel frames into events", async () => {
    const events: unknown[] = [];
    invokeMock.mockImplementation(async (_cmd: string, args: { onEvent: { onmessage: (m: ArrayBuffer) => void } }) => {
      args.onEvent.onmessage(new Uint8Array([0, 65, 66]).buffer);
      args.onEvent.onmessage(new Uint8Array([42]).buffer);
      return "sid";
    });
    const sid = await tauriIpc.termOpen("a", 80, 24, (e) => events.push(e));
    expect(sid).toBe("sid");
    expect(events[0]).toEqual({ type: "data", bytes: new Uint8Array([65, 66]) });
    expect(events[1]).toMatchObject({ type: "error", error: { code: "internal" } });
  });

  it("sends the credential answer and the ask flag with camelCase names", async () => {
    const answer = { username: "u", password: "p", keyPath: null, passphrase: null, domain: "D" };
    await tauriIpc.sessionCredential("sess-1", answer, true);
    expect(invokeMock).toHaveBeenCalledWith(
      "session_credential",
      { sessionId: "sess-1", answer, saveToVault: true },
      undefined,
    );
    await tauriIpc.sessionCredentialCancel("sess-1");
    expect(invokeMock).toHaveBeenLastCalledWith(
      "session_credential_cancel",
      { sessionId: "sess-1" },
      undefined,
    );
    // Opening a session: the flag defaults to false and "enter manually" sets it.
    await tauriIpc.termOpen("a", 80, 24, noop);
    expect(invokeMock.mock.lastCall?.[1]).toMatchObject({ askCredentials: false });
    await tauriIpc.fbOpen("a", 800, 600, noop, { askCredentials: true });
    expect(invokeMock.mock.lastCall?.[1]).toMatchObject({ askCredentials: true });
  });

  it("subscribes to backend events lazily and stops listening on unsubscribe", async () => {
    const off = vi.fn();
    const listenMock = vi.fn(async (_event: string, _handler: (e: { payload: unknown }) => void) => off);
    vi.doMock("@tauri-apps/api/event", () => ({ listen: listenMock }));
    vi.resetModules();
    const { tauriIpc: fresh } = await import("./tauri");

    const locked = vi.fn();
    const stop = fresh.onVaultLocked(locked);
    const failed = vi.fn();
    const stopFailed = fresh.onCredentialSaveFailed(failed);
    const menu = vi.fn();
    const stopMenu = fresh.onCheckForUpdatesMenu(menu);
    const quit = vi.fn();
    const stopQuit = fresh.onQuitRequested(quit);
    const closeTab = vi.fn();
    const stopCloseTab = fresh.onCloseTabMenu(closeTab);
    const settings = vi.fn();
    const stopSettings = fresh.onSettingsMenu(settings);
    const releaseNotes = vi.fn();
    const stopReleaseNotes = fresh.onReleaseNotesMenu(releaseNotes);
    const sharesChanged = vi.fn();
    const stopShares = fresh.onSharesChanged(sharesChanged);
    await vi.waitFor(() => expect(listenMock).toHaveBeenCalledTimes(8));
    expect(listenMock.mock.calls.map((c) => c[0])).toEqual([
      "vault-locked",
      "credential-save-failed",
      "menu-check-for-updates",
      "quit-requested",
      "menu-close-tab",
      "menu-settings",
      "menu-release-notes",
      "shares-changed",
    ]);
    listenMock.mock.calls[7]?.[1]({ payload: null });
    expect(sharesChanged).toHaveBeenCalledTimes(1);
    listenMock.mock.calls[6]?.[1]({ payload: null });
    expect(releaseNotes).toHaveBeenCalledTimes(1);
    listenMock.mock.calls[5]?.[1]({ payload: null });
    expect(settings).toHaveBeenCalledTimes(1);
    listenMock.mock.calls[4]?.[1]({ payload: null });
    expect(closeTab).toHaveBeenCalledTimes(1);
    listenMock.mock.calls[3]?.[1]({ payload: null });
    expect(quit).toHaveBeenCalledTimes(1);
    listenMock.mock.calls[2]?.[1]({ payload: null });
    expect(menu).toHaveBeenCalledTimes(1);

    listenMock.mock.calls[0]?.[1]({ payload: null });
    expect(locked).toHaveBeenCalledTimes(1);
    const notice = { connectionId: "c", connectionName: "n", code: "vault_locked", message: "m" };
    listenMock.mock.calls[1]?.[1]({ payload: notice });
    expect(failed).toHaveBeenCalledWith(notice);

    stop();
    stopFailed();
    stopMenu();
    stopQuit();
    stopCloseTab();
    stopSettings();
    stopReleaseNotes();
    stopShares();
    expect(off).toHaveBeenCalledTimes(8);
    vi.doUnmock("@tauri-apps/api/event");
  });

  it("matches the Ipc interface", () => {
    const ipc: Ipc = tauriIpc;
    expect(typeof ipc.docGet).toBe("function");
  });
});
