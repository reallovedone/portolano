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
  "vault_status",
  "vault_create",
  "vault_unlock",
  "vault_change_password",
  "vault_lock",
  "vault_list",
  "vault_upsert",
  "vault_delete",
  "vault_reveal_secret",
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
  "web_open",
  "web_set_bounds",
  "web_navigate",
  "session_close",
  "session_credential",
  "session_credential_cancel",
  "import_preview",
  "import_apply",
  "export_document",
  "ui_log",
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
      () => tauriIpc.vaultStatus(),
      () => tauriIpc.vaultCreate("p"),
      () => tauriIpc.vaultUnlock("p"),
      () => tauriIpc.vaultChangePassword("p", "q"),
      () => tauriIpc.vaultLock(),
      () => tauriIpc.vaultList(),
      () => tauriIpc.vaultUpsert({ id: null, name: "n", username: "u", domain: null, secret: { type: "password", password: "p" } }),
      () => tauriIpc.vaultDelete("a"),
      () => tauriIpc.vaultRevealSecret("a"),
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
      () => tauriIpc.uiLog("info", "m"),
    ];
    for (const run of calls) await run();
    const used = invokeMock.mock.calls.map((c) => c[0] as string);
    expect(used).toEqual(EXPECTED_COMMANDS);
    // Every command method of the interface is covered by one of the calls above (the `on…`
    // methods subscribe to events and are not commands).
    expect(Object.keys(tauriIpc).filter((name) => !name.startsWith("on")).length).toBe(calls.length);
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
    await vi.waitFor(() => expect(listenMock).toHaveBeenCalledTimes(2));
    expect(listenMock.mock.calls.map((c) => c[0])).toEqual(["vault-locked", "credential-save-failed"]);

    listenMock.mock.calls[0]?.[1]({ payload: null });
    expect(locked).toHaveBeenCalledTimes(1);
    const notice = { connectionId: "c", connectionName: "n", code: "vault_locked", message: "m" };
    listenMock.mock.calls[1]?.[1]({ payload: notice });
    expect(failed).toHaveBeenCalledWith(notice);

    stop();
    stopFailed();
    expect(off).toHaveBeenCalledTimes(2);
    vi.doUnmock("@tauri-apps/api/event");
  });

  it("matches the Ipc interface", () => {
    const ipc: Ipc = tauriIpc;
    expect(typeof ipc.docGet).toBe("function");
  });
});
