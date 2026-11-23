import { describe, expect, it, vi } from "vitest";

import type { FbEvent } from "../../ipc";
import { FbController, type FbViewState } from "./controller";
import { ManualClock, fakeKey, flushPromises, recordingApi } from "./testing";

function setup() {
  const clock = new ManualClock();
  const { api, calls } = recordingApi();
  const drawn: Array<{ x: number; y: number }> = [];
  const sizes: Array<[number, number]> = [];
  const states: FbViewState[] = [];
  const onFatal = vi.fn();
  const onError = vi.fn();
  const onReconnect = vi.fn();
  const controller: FbController = new FbController({
    api,
    surface: { putImageData: (_i, x, y) => void drawn.push({ x, y }) },
    setCanvasSize: (w, h) => void sizes.push([w, h]),
    onState: (s) => void states.push(s),
    onFatal,
    onError,
    onReconnect,
    geometry: () => ({
      rect: { left: 0, top: 0, width: 800, height: 600 },
      remote: controller.state.remote,
    }),
    renderer: { ...clock, makeImage: () => ({}) as ImageData },
    input: clock,
    setTimer: clock.setTimer,
    clearTimer: clock.clearTimer,
    toDataUrl: () => "data:image/png;base64,AAAA",
  });
  return { controller, clock, calls, drawn, sizes, states, onFatal, onError, onReconnect };
}

const rect = (x: number, y: number): FbEvent => ({
  type: "rect",
  x,
  y,
  w: 2,
  h: 2,
  rgba: new Uint8Array(16),
});

describe("events", () => {
  it("remembers whether the transport is encrypted; sessions that never say stay unknown", () => {
    const { controller } = setup();
    expect(controller.state.transport).toBe("unknown");
    controller.handleEvent({ type: "security", encrypted: false });
    expect(controller.state.transport).toBe("clear");
    controller.handleEvent({ type: "security", encrypted: true });
    expect(controller.state.transport).toBe("encrypted");
    // The report does not change the phase: the desktop's size does.
    expect(controller.state.phase).toBe("connecting");
    controller.handleEvent({ type: "resize", w: 800, h: 600 });
    expect(controller.state).toMatchObject({ phase: "live", transport: "encrypted" });
  });

  it("remembers how an RDP login was authenticated; VNC never says", () => {
    const { controller } = setup();
    expect(controller.state.auth).toBeNull();
    controller.handleEvent({ type: "security", encrypted: true, auth: "ntlm" });
    expect(controller.state.auth).toBe("ntlm");
    controller.handleEvent({ type: "security", encrypted: true });
    expect(controller.state.auth).toBeNull();
  });

  it("resizes the canvas, goes live and draws rectangles per frame", () => {
    const { controller, clock, drawn, sizes } = setup();
    expect(controller.state.phase).toBe("connecting");
    controller.handleEvent({ type: "resize", w: 800, h: 600 });
    expect(sizes).toEqual([[800, 600]]);
    expect(controller.state).toMatchObject({ phase: "live", remote: { w: 800, h: 600 } });

    controller.handleEvent(rect(0, 0));
    controller.handleEvent(rect(5, 5));
    expect(drawn).toHaveLength(0);
    clock.frame();
    expect(drawn).toEqual([
      { x: 0, y: 0 },
      { x: 5, y: 5 },
    ]);
  });

  it("drops queued rectangles of the old size on a resize", () => {
    const { controller, clock, drawn } = setup();
    controller.handleEvent({ type: "resize", w: 800, h: 600 });
    controller.handleEvent(rect(1, 1));
    controller.handleEvent({ type: "resize", w: 640, h: 480 });
    clock.frame();
    expect(drawn).toHaveLength(0);
  });

  it("turns cursor events into a CSS cursor, the empty one into the default", () => {
    const { controller } = setup();
    controller.handleEvent({
      type: "cursor",
      hotspotX: 3,
      hotspotY: 4,
      w: 1,
      h: 1,
      rgba: new Uint8Array(4),
    });
    expect(controller.state.cursor).toBe('url("data:image/png;base64,AAAA") 0 0, default');
    controller.handleEvent({
      type: "cursor",
      hotspotX: 0,
      hotspotY: 0,
      w: 0,
      h: 0,
      rgba: new Uint8Array(0),
    });
    expect(controller.state.cursor).toBe("default");
  });

  it("shows the closed state with the reason and releases the keyboard", async () => {
    const { controller, calls } = setup();
    controller.handleEvent({ type: "resize", w: 800, h: 600 });
    controller.input.keyDown(fakeKey("KeyA"));
    controller.handleEvent({ type: "closed", reason: "server initiated disconnect" });
    await flushPromises();
    expect(controller.state).toMatchObject({
      phase: "closed",
      closedReason: "server initiated disconnect",
    });
    expect(calls.at(-1)).toEqual({ fn: "key", code: "KeyA", down: false });
  });

  it("hands backend errors to the shell", () => {
    const { controller, onFatal } = setup();
    controller.handleEvent({ type: "error", error: { code: "auth_failed", message: "no" } });
    expect(onFatal).toHaveBeenCalledWith({ code: "auth_failed", message: "no" });
  });

  it("ignores events after dispose", () => {
    const { controller, sizes } = setup();
    controller.dispose();
    controller.handleEvent({ type: "resize", w: 10, h: 10 });
    expect(sizes).toHaveLength(0);
  });
});

describe("visibility", () => {
  it("calls fbSetVisible only on changes", async () => {
    const { controller, calls } = setup();
    controller.setVisible(true);
    controller.setVisible(true);
    controller.setVisible(false);
    controller.setVisible(false);
    controller.setVisible(true);
    await flushPromises();
    expect(calls.filter((c) => c.fn === "setVisible").map((c) => c.fn === "setVisible" && c.visible)).toEqual([
      true,
      false,
      true,
    ]);
  });

  it("releases held keys when the tab is hidden", async () => {
    const { controller, calls } = setup();
    controller.handleEvent({ type: "resize", w: 800, h: 600 });
    controller.setVisible(true);
    controller.input.keyDown(fakeKey("AltLeft"));
    controller.setVisible(false);
    await flushPromises();
    const keysSent = calls.filter((c) => c.fn === "key").map((c) => (c.fn === "key" ? `${c.code}:${c.down}` : ""));
    expect(keysSent).toEqual(["AltLeft:true", "AltLeft:false"]);
    expect(calls.filter((c) => c.fn === "setVisible")).toHaveLength(2);
  });

  it("reports a failing call without throwing", async () => {
    const { controller, onError } = setup();
    // Replace the api behaviour through a failing visibility call.
    const failing = new FbController({
      api: {
        pointer: async () => undefined,
        key: async () => undefined,
        resize: async () => undefined,
        certDecision: async () => undefined,
        unencryptedDecision: async () => undefined,
        forgetCertificate: async () => undefined,
        credential: async () => undefined,
        credentialCancel: async () => undefined,
        clipboardSave: async () => true,
        clipboardCancel: async () => undefined,
        setVisible: async () => {
          throw new Error("gone");
        },
      },
      surface: { putImageData: () => undefined },
      setCanvasSize: () => undefined,
      onState: () => undefined,
      onFatal: () => undefined,
      onError,
      geometry: () => null,
    });
    failing.setVisible(true);
    await flushPromises();
    expect(onError).toHaveBeenCalledWith("fb_set_visible", expect.any(Error));
    expect(controller.state.phase).toBe("connecting");
  });
});

describe("certificate prompt", () => {
  it("shows the prompt, and a trust decision is sent once and clears it", async () => {
    const { controller, calls } = setup();
    controller.handleEvent({ type: "certPrompt", fingerprint: "AA:BB", changed: false, stored: null });
    expect(controller.state.cert).toEqual({ fingerprint: "AA:BB", changed: false, stored: null });
    controller.decideCert(true);
    controller.decideCert(true);
    await flushPromises();
    expect(controller.state.cert).toBeNull();
    expect(calls.filter((c) => c.fn === "certDecision")).toEqual([{ fn: "certDecision", accept: true }]);
  });

  it("a changed certificate carries both fingerprints and offers no decision", async () => {
    const { controller, calls } = setup();
    controller.handleEvent({ type: "certPrompt", fingerprint: "CC:DD", changed: true, stored: "11:22" });
    expect(controller.state.cert).toEqual({ fingerprint: "CC:DD", changed: true, stored: "11:22" });
    // Neither "accept" nor "reject" is sent: the backend is not waiting for an answer.
    controller.decideCert(true);
    controller.decideCert(false);
    await flushPromises();
    expect(calls).toEqual([]);
    expect(controller.state.cert?.changed).toBe(true);
  });

  it("the only way forward is removing the stored certificate, then reconnecting", async () => {
    const { controller, calls, onReconnect } = setup();
    controller.handleEvent({ type: "certPrompt", fingerprint: "CC:DD", changed: true, stored: "11:22" });
    controller.forgetCertificateAndReconnect();
    expect(onReconnect).not.toHaveBeenCalled();
    await flushPromises();
    expect(calls).toEqual([{ fn: "forgetCertificate" }]);
    expect(onReconnect).toHaveBeenCalledOnce();
  });

  it("does not reconnect when the stored certificate could not be removed", async () => {
    const { controller, onReconnect, onError } = setup();
    controller.handleEvent({ type: "certPrompt", fingerprint: "CC:DD", changed: true, stored: "11:22" });
    const failing = new FbController({
      api: {
        pointer: async () => undefined,
        key: async () => undefined,
        resize: async () => undefined,
        certDecision: async () => undefined,
        unencryptedDecision: async () => undefined,
        forgetCertificate: async () => {
          throw new Error("disk full");
        },
        credential: async () => undefined,
        credentialCancel: async () => undefined,
        clipboardSave: async () => true,
        clipboardCancel: async () => undefined,
        setVisible: async () => undefined,
      },
      surface: { putImageData: () => undefined },
      setCanvasSize: () => undefined,
      onState: () => undefined,
      onFatal: () => undefined,
      onError,
      onReconnect,
      geometry: () => null,
    });
    failing.handleEvent({ type: "certPrompt", fingerprint: "CC:DD", changed: true, stored: "11:22" });
    failing.forgetCertificateAndReconnect();
    await flushPromises();
    expect(onError).toHaveBeenCalledWith("fb_forget_certificate", expect.any(Error));
    expect(onReconnect).not.toHaveBeenCalled();
  });

  it("clears the prompt when the session proceeds or closes", () => {
    const { controller } = setup();
    controller.handleEvent({ type: "certPrompt", fingerprint: "AA", changed: false, stored: null });
    controller.handleEvent({ type: "closed", reason: "the server certificate was not accepted" });
    expect(controller.state.cert).toBeNull();
  });
});

describe("unencrypted connection prompt", () => {
  it("is shown, answered once, and cleared", async () => {
    const { controller, calls } = setup();
    controller.handleEvent({ type: "unencryptedPrompt" });
    expect(controller.state.unencryptedPrompt).toBe(true);
    controller.decideUnencrypted(true);
    controller.decideUnencrypted(true);
    await flushPromises();
    expect(controller.state.unencryptedPrompt).toBe(false);
    expect(calls).toEqual([{ fn: "unencryptedDecision", allow: true }]);
  });

  it("sends a refusal as it is", async () => {
    const { controller, calls } = setup();
    controller.handleEvent({ type: "unencryptedPrompt" });
    controller.decideUnencrypted(false);
    await flushPromises();
    expect(calls).toEqual([{ fn: "unencryptedDecision", allow: false }]);
  });

  it("sends nothing when no prompt is pending", async () => {
    const { controller, calls } = setup();
    controller.decideUnencrypted(true);
    await flushPromises();
    expect(calls).toEqual([]);
  });
});

describe("security downgrade", () => {
  it("is its own state, not a fatal error, and offers the same deliberate way forward", async () => {
    const { controller, calls, onFatal, onReconnect } = setup();
    controller.handleEvent({
      type: "error",
      error: { code: "security_downgrade", message: "security downgrade refused: h:5900 ..." },
    });
    expect(controller.state.downgrade).toContain("security downgrade refused");
    expect(onFatal).not.toHaveBeenCalled();
    controller.forgetCertificateAndReconnect();
    await flushPromises();
    expect(calls).toEqual([{ fn: "forgetCertificate" }]);
    expect(onReconnect).toHaveBeenCalledOnce();
  });
});

describe("resize requests", () => {
  it("debounces and asks for the final size only", async () => {
    const { controller, clock, calls } = setup();
    controller.handleEvent({ type: "resize", w: 800, h: 600 });
    controller.requestResize(900, 700);
    clock.advance(100);
    controller.requestResize(1000.7, 720.2);
    clock.advance(299);
    await flushPromises();
    expect(calls.filter((c) => c.fn === "resize")).toHaveLength(0);
    clock.advance(1);
    await flushPromises();
    expect(calls.filter((c) => c.fn === "resize")).toEqual([{ fn: "resize", width: 1000, height: 720 }]);
  });

  it("does not ask before connecting, for the current size, or twice for the same size", async () => {
    const { controller, clock, calls } = setup();
    controller.requestResize(800, 600);
    clock.advance(500);
    controller.handleEvent({ type: "resize", w: 800, h: 600 });
    controller.requestResize(800, 600);
    clock.advance(500);
    controller.requestResize(1024, 768);
    clock.advance(500);
    controller.requestResize(1024, 768);
    clock.advance(500);
    controller.requestResize(0, 0);
    clock.advance(500);
    await flushPromises();
    expect(calls.filter((c) => c.fn === "resize")).toEqual([{ fn: "resize", width: 1024, height: 768 }]);
  });
});

describe("credential prompt", () => {
  const prompt = {
    type: "credentialPrompt" as const,
    kind: "usernamePassword" as const,
    usernameHint: "ann",
    domainHint: null,
    target: "rdp.example:3389",
    error: null,
    canSaveToVault: false,
  };
  const answer = { username: "ann", password: "pw", keyPath: null, passphrase: null, domain: null };

  function withSaved() {
    const saved = vi.fn();
    const { api, calls } = recordingApi();
    const onFatal = vi.fn();
    const controller: FbController = new FbController({
      api,
      surface: { putImageData: () => undefined },
      setCanvasSize: () => undefined,
      onState: () => undefined,
      onFatal,
      onError: vi.fn(),
      onCredentialSaved: saved,
      geometry: () => null,
    });
    return { controller, calls, saved, onFatal };
  }

  it("holds the prompt, sends the answer once and drops it", async () => {
    const t = withSaved();
    t.controller.handleEvent(prompt);
    expect(t.controller.state.credential).toMatchObject({ kind: "usernamePassword", usernameHint: "ann" });
    t.controller.submitCredential(answer, false);
    await flushPromises();
    expect(t.calls).toEqual([{ fn: "credential", username: "ann", saveToVault: false }]);
    expect(t.controller.state.credential).toBeNull();
    // Nothing pending any more: a second submit is ignored.
    t.controller.submitCredential(answer, false);
    await flushPromises();
    expect(t.calls).toHaveLength(1);
  });

  it("a kind without user name can be expressed (VNC)", () => {
    const t = withSaved();
    t.controller.handleEvent({ ...prompt, kind: "password", usernameHint: null });
    expect(t.controller.state.credential?.kind).toBe("password");
  });

  it("cancelling shows the cancelled phase and is not a fatal error", async () => {
    const t = withSaved();
    t.controller.handleEvent(prompt);
    t.controller.cancelCredential();
    await flushPromises();
    expect(t.calls).toEqual([{ fn: "credentialCancel" }]);
    expect(t.controller.state.phase).toBe("cancelled");
    t.controller.handleEvent({ type: "error", error: { code: "cancelled", message: "x" } });
    expect(t.onFatal).not.toHaveBeenCalled();
    // Any other error is still fatal.
    t.controller.handleEvent({ type: "error", error: { code: "io", message: "boom" } });
    expect(t.onFatal).toHaveBeenCalledTimes(1);
  });

  it("reports a saved credential once the desktop arrived, not before", async () => {
    const t = withSaved();
    t.controller.handleEvent(prompt);
    t.controller.submitCredential(answer, true);
    await flushPromises();
    expect(t.saved).not.toHaveBeenCalled();
    t.controller.handleEvent({ type: "resize", w: 800, h: 600 });
    t.controller.handleEvent({ type: "resize", w: 640, h: 480 });
    expect(t.saved).toHaveBeenCalledTimes(1);
  });
});

describe("shared clipboard", () => {
  const offer: FbEvent = {
    type: "clipboard",
    event: { type: "remoteFiles", count: 2, bytes: 300, skipped: 0 },
  };

  it("follows the backend's clipboard events", () => {
    const { controller } = setup();
    expect(controller.state.clipboard.offer).toBeNull();
    controller.handleEvent(offer);
    expect(controller.state.clipboard.offer).toEqual({ count: 2, bytes: 300, skipped: 0 });
    controller.handleEvent({
      type: "clipboard",
      event: { type: "progress", filesDone: 1, filesTotal: 2, bytesDone: 100, bytesTotal: 300 },
    });
    expect(controller.state.clipboard.phase.kind).toBe("saving");
    controller.handleEvent({ type: "clipboard", event: { type: "done", files: 2, bytes: 300, skipped: 0 } });
    expect(controller.state.clipboard.phase).toEqual({ kind: "done", files: 2, bytes: 300, skipped: 0 });
    controller.dismissClipboardNotice();
    expect(controller.state.clipboard.phase.kind).toBe("idle");
  });

  it("saves only when there is an offer, once at a time", async () => {
    const { controller, calls } = setup();
    controller.saveClipboardFiles();
    await flushPromises();
    expect(calls.filter((c) => c.fn === "clipboardSave")).toHaveLength(0);

    controller.handleEvent(offer);
    controller.saveClipboardFiles();
    await flushPromises();
    expect(controller.state.clipboard.phase.kind).toBe("saving");
    // A second request while saving does nothing.
    controller.saveClipboardFiles();
    await flushPromises();
    expect(calls.filter((c) => c.fn === "clipboardSave")).toHaveLength(1);
  });

  it("keeps a transfer that finished before the save command answered", async () => {
    let controller: FbController | null = null;
    const done: FbEvent = { type: "clipboard", event: { type: "done", files: 1, bytes: 11, skipped: 0 } };
    controller = new FbController({
      api: {
        pointer: async () => undefined,
        key: async () => undefined,
        resize: async () => undefined,
        certDecision: async () => undefined,
        unencryptedDecision: async () => undefined,
        forgetCertificate: async () => undefined,
        credential: async () => undefined,
        credentialCancel: async () => undefined,
        clipboardSave: async () => {
          // A small file is saved before the command returns.
          controller?.handleEvent(done);
          return true;
        },
        clipboardCancel: async () => undefined,
        setVisible: async () => undefined,
      },
      surface: { putImageData: () => undefined },
      setCanvasSize: () => undefined,
      onState: () => undefined,
      onFatal: () => undefined,
      onError: () => undefined,
      geometry: () => null,
    });
    controller.handleEvent(offer);
    controller.saveClipboardFiles();
    await flushPromises();
    expect(controller.state.clipboard.phase).toEqual({ kind: "done", files: 1, bytes: 11, skipped: 0 });
    controller.dismissClipboardNotice();
    expect(controller.state.clipboard.phase.kind).toBe("idle");
    controller.dispose();
  });

  it("cancels only while saving and reports a refused command without throwing", async () => {
    const { controller, calls, onError } = setup();
    controller.cancelClipboardSave();
    expect(calls.filter((c) => c.fn === "clipboardCancel")).toHaveLength(0);

    const failing = new FbController({
      api: {
        pointer: async () => undefined,
        key: async () => undefined,
        resize: async () => undefined,
        certDecision: async () => undefined,
        unencryptedDecision: async () => undefined,
        forgetCertificate: async () => undefined,
        credential: async () => undefined,
        credentialCancel: async () => undefined,
        clipboardSave: async () => {
          throw new Error("clipboard sharing is off for this connection");
        },
        clipboardCancel: async () => undefined,
        setVisible: async () => undefined,
      },
      surface: { putImageData: () => undefined },
      setCanvasSize: () => undefined,
      onState: () => undefined,
      onFatal: () => undefined,
      onError,
      geometry: () => null,
    });
    failing.handleEvent(offer);
    failing.saveClipboardFiles();
    await flushPromises();
    expect(onError).toHaveBeenCalledWith("fb_clipboard_save", expect.any(Error));
    expect(failing.state.clipboard.phase).toEqual({ kind: "failed", reason: "unavailable" });
    failing.dispose();
  });
});
