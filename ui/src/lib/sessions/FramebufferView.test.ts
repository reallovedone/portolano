import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Ipc } from "../ipc";
import { createMockIpc } from "../ipc/mock";
import { createApp } from "../state/app";
import FbHarness from "./fb/FbHarness.svelte";
import { createTestSession } from "./fb/test-context.svelte";

interface Drawn {
  x: number;
  y: number;
  w: number;
  h: number;
}

let drawn: Drawn[] = [];
/** What `fbClipboardSave` answers: `false` is a dismissed folder dialog. */
let saveAnswer = true;

beforeEach(() => {
  drawn = [];
  saveAnswer = true;
  vi.stubGlobal(
    "ImageData",
    class {
      constructor(
        readonly data: Uint8ClampedArray,
        readonly width: number,
        readonly height: number,
      ) {}
    },
  );
  HTMLCanvasElement.prototype.toDataURL = () => "data:image/png;base64,AAAA";
  HTMLCanvasElement.prototype.getContext = function getContext() {
    return {
      putImageData(image: { width: number; height: number }, x: number, y: number) {
        drawn.push({ x, y, w: image.width, h: image.height });
      },
    };
  } as unknown as typeof HTMLCanvasElement.prototype.getContext;
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function recordingIpc(base: Ipc = createMockIpc()) {
  const calls: string[] = [];
  const ipc: Ipc = {
    ...base,
    fbPointer: async (id, x, y, b, w) => void calls.push(`pointer:${x},${y},${b},${w}`),
    fbKey: async (id, code, down) => void calls.push(`key:${code}:${down ? "down" : "up"}`),
    fbResize: async (id, w, h) => void calls.push(`resize:${w}x${h}`),
    fbSetVisible: async (id, v) => void calls.push(`visible:${v}`),
    fbCertDecision: async (id, a) => void calls.push(`cert:${a}`),
    // The secret is deliberately not recorded.
    sessionCredential: async (id, answer, save) =>
      void calls.push(`credential:${answer.username}:${answer.domain ?? "-"}:save=${save}`),
    sessionCredentialCancel: async () => void calls.push("credentialCancel"),
    fbClipboardSave: async () => {
      calls.push("clipboardSave");
      return saveAnswer;
    },
    fbClipboardCancel: async () => void calls.push("clipboardCancel"),
  };
  return { ipc, calls };
}

function mount(ipc: Ipc) {
  const session = createTestSession();
  const app = createApp(ipc);
  render(FbHarness, { props: { app, context: session.context } });
  return { ...session, app };
}

const canvasOf = (): HTMLCanvasElement => screen.getByLabelText("Remote desktop") as HTMLCanvasElement;

function layoutCanvas(canvas: HTMLCanvasElement, width: number, height: number): void {
  canvas.getBoundingClientRect = () =>
    ({ left: 0, top: 0, width, height, right: width, bottom: height, x: 0, y: 0 }) as DOMRect;
}

describe("FramebufferView", () => {
  it("shows the connecting state, then draws rectangles at the remote size", async () => {
    const { ipc } = recordingIpc();
    const session = mount(ipc);
    expect(screen.getByText("Connecting to desktop…")).toBeTruthy();

    session.bridge.emit({ type: "resize", w: 800, h: 600 });
    session.bridge.emit({ type: "rect", x: 4, y: 8, w: 2, h: 2, rgba: new Uint8Array(16) });
    await waitFor(() => expect(drawn).toEqual([{ x: 4, y: 8, w: 2, h: 2 }]));
    expect(canvasOf().width).toBe(800);
    expect(canvasOf().height).toBe(600);
    expect(screen.queryByText("Connecting to desktop…")).toBeNull();
  });

  it("replays events that arrived before it was mounted", async () => {
    const { ipc } = recordingIpc();
    const session = createTestSession();
    session.bridge.emit({ type: "resize", w: 640, h: 480 });
    render(FbHarness, { props: { app: createApp(ipc), context: session.context } });
    await waitFor(() => expect(canvasOf().width).toBe(640));
  });

  it("asks before trusting an unknown certificate and sends the answer", async () => {
    const { ipc, calls } = recordingIpc();
    const session = mount(ipc);
    session.bridge.emit({ type: "certPrompt", fingerprint: "AA:BB:CC", changed: false, stored: null });
    expect(await screen.findByText("Unknown server certificate")).toBeTruthy();
    expect(screen.getByText("AA:BB:CC")).toBeTruthy();
    expect(screen.queryByText("The server certificate has changed")).toBeNull();
    await fireEvent.click(screen.getByRole("button", { name: "Trust and connect" }));
    expect(calls).toContain("cert:true");
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
  });

  it("announces the first-time certificate prompt and focuses the safe action", async () => {
    const { ipc, calls } = recordingIpc();
    const session = mount(ipc);
    session.bridge.emit({ type: "certPrompt", fingerprint: "AA:BB:CC", changed: false, stored: null });
    const dialog = await screen.findByRole("alertdialog", { name: "Unknown server certificate" });
    expect(dialog).toBeTruthy();
    await waitFor(() => expect(document.activeElement).toBe(screen.getByRole("button", { name: "Cancel" })));
    expect(calls).not.toContain("cert:true");
  });

  it("a changed certificate is a danger panel with both fingerprints and no one-click accept", async () => {
    const { ipc, calls } = recordingIpc();
    const session = mount(ipc);
    session.bridge.emit({ type: "certPrompt", fingerprint: "11:22", changed: true, stored: "33:44" });
    expect(await screen.findByText("The server certificate has changed")).toBeTruthy();
    expect(screen.getByRole("alertdialog", { name: "The server certificate has changed" })).toBeTruthy();
    expect(screen.getByText(/someone may be intercepting/)).toBeTruthy();
    // Both fingerprints, labelled.
    expect(screen.getByText("Stored certificate")).toBeTruthy();
    expect(screen.getByText("33:44")).toBeTruthy();
    expect(screen.getByText("11:22")).toBeTruthy();
    // The safe choice is the default; there is nothing like "accept the new certificate".
    const close = screen.getByRole("button", { name: "Close tab" });
    await waitFor(() => expect(document.activeElement).toBe(close));
    expect(screen.queryByRole("button", { name: /trust|accept/i })).toBeNull();
    expect(calls).not.toContain("cert:true");
    expect(calls).not.toContain("cert:false");
  });

  it("removing the stored certificate is a deliberate action that then reconnects", async () => {
    const { ipc, calls } = recordingIpc();
    const forget = vi.fn(async () => undefined);
    const session = mount({ ...ipc, fbForgetCertificate: forget });
    session.bridge.emit({ type: "certPrompt", fingerprint: "11:22", changed: true, stored: "33:44" });
    const remove = await screen.findByRole("button", { name: "Remove stored certificate and reconnect" });
    expect(forget).not.toHaveBeenCalled();
    await fireEvent.click(remove);
    await waitFor(() => expect(forget).toHaveBeenCalledWith("sess-1"));
    await waitFor(() => expect(session.retry).toHaveBeenCalledOnce());
    expect(calls).not.toContain("cert:true");
  });

  it("a refused security downgrade explains itself and offers the same deliberate step", async () => {
    const { ipc } = recordingIpc();
    const forget = vi.fn(async () => undefined);
    const session = mount({ ...ipc, fbForgetCertificate: forget });
    session.bridge.emit({
      type: "error",
      error: { code: "security_downgrade", message: "security downgrade refused: h:5900 used an encrypted connection before" },
    });
    expect(await screen.findByText("The server stopped offering encryption")).toBeTruthy();
    expect(screen.getByText(/security downgrade refused/)).toBeTruthy();
    // It is not handed to the shell's generic error view.
    expect(session.fail).not.toHaveBeenCalled();
    await fireEvent.click(screen.getByRole("button", { name: "Remove stored certificate and reconnect" }));
    await waitFor(() => expect(forget).toHaveBeenCalledWith("sess-1"));
    await waitFor(() => expect(session.retry).toHaveBeenCalledOnce());
  });

  it("asks before sending a password without encryption; the safe choice is the default", async () => {
    const { ipc } = recordingIpc();
    const decision = vi.fn(async () => undefined);
    const session = mount({ ...ipc, fbUnencryptedDecision: decision });
    session.bridge.emit({ type: "unencryptedPrompt" });
    const dialog = await screen.findByRole("alertdialog", { name: "This server does not encrypt the connection" });
    expect(dialog).toBeTruthy();
    expect(screen.getByText(/Nothing has been sent yet/)).toBeTruthy();
    const cancel = screen.getByRole("button", { name: "Cancel" });
    await waitFor(() => expect(document.activeElement).toBe(cancel));
    expect(decision).not.toHaveBeenCalled();
    await fireEvent.click(cancel);
    expect(decision).toHaveBeenCalledWith("sess-1", false);
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
  });

  it("agreeing to the unencrypted exchange is the explicit, less prominent choice", async () => {
    const { ipc } = recordingIpc();
    const decision = vi.fn(async () => undefined);
    const session = mount({ ...ipc, fbUnencryptedDecision: decision });
    session.bridge.emit({ type: "unencryptedPrompt" });
    await fireEvent.click(await screen.findByRole("button", { name: "Send the password without encryption" }));
    expect(decision).toHaveBeenCalledWith("sess-1", true);
  });

  it("follows the tab visibility with fb_set_visible, including overlays", async () => {
    const { ipc, calls } = recordingIpc();
    const session = mount(ipc);
    await waitFor(() => expect(calls).toContain("visible:true"));
    session.state.visible = false;
    await waitFor(() => expect(calls).toContain("visible:false"));
    session.state.visible = true;
    await waitFor(() => expect(calls.filter((c) => c.startsWith("visible:"))).toEqual([
      "visible:true",
      "visible:false",
      "visible:true",
    ]));
    session.state.overlayOpen = true;
    await waitFor(() => expect(calls.at(-1)).toBe("visible:false"));
  });

  it("captures the keyboard while focused, without leaking keys to the shell", async () => {
    const { ipc, calls } = recordingIpc();
    const session = mount(ipc);
    session.bridge.emit({ type: "resize", w: 800, h: 600 });
    const outer = screen.getByTestId("outer");
    const leaked = vi.fn();
    outer.addEventListener("keydown", leaked);
    const canvas = canvasOf();
    canvas.focus();
    await fireEvent.keyDown(canvas, { code: "KeyW", ctrlKey: true });
    await fireEvent.keyUp(canvas, { code: "KeyW" });
    await waitFor(() => expect(calls).toEqual(expect.arrayContaining(["key:KeyW:down", "key:KeyW:up"])));
    expect(leaked).not.toHaveBeenCalled();
    expect(screen.getByText(/Ctrl\+Alt\+Esc/)).toBeTruthy();
  });

  it("fades the keyboard hint after a few seconds and shows it again on capture", async () => {
    vi.useFakeTimers();
    try {
      const { ipc } = recordingIpc();
      const session = mount(ipc);
      session.bridge.emit({ type: "resize", w: 800, h: 600 });
      await vi.advanceTimersByTimeAsync(0);
      const chip = screen.getByRole("status");
      expect(chip.classList.contains("faded")).toBe(false);
      await vi.advanceTimersByTimeAsync(5000);
      expect(chip.classList.contains("faded")).toBe(true);
      canvasOf().focus();
      await vi.advanceTimersByTimeAsync(0);
      expect(chip.classList.contains("faded")).toBe(false);
      expect(chip.textContent).toContain("Ctrl+Alt+Esc");
    } finally {
      vi.useRealTimers();
    }
  });

  it("releases held keys on blur and on the escape chord", async () => {
    const { ipc, calls } = recordingIpc();
    const session = mount(ipc);
    session.bridge.emit({ type: "resize", w: 800, h: 600 });
    const canvas = canvasOf();
    canvas.focus();
    await fireEvent.keyDown(canvas, { code: "ShiftLeft" });
    await fireEvent.keyDown(canvas, { code: "KeyA" });
    await fireEvent.blur(canvas);
    await waitFor(() =>
      expect(calls.filter((c) => c.endsWith(":up")).sort()).toEqual(["key:KeyA:up", "key:ShiftLeft:up"]),
    );

    calls.length = 0;
    canvas.focus();
    await fireEvent.keyDown(canvas, { code: "ControlLeft" });
    await fireEvent.keyDown(canvas, { code: "Escape", ctrlKey: true, altKey: true });
    await waitFor(() => expect(calls).toContain("key:ControlLeft:up"));
    expect(calls).not.toContain("key:Escape:down");
    expect(document.activeElement).not.toBe(canvas);
  });

  it("maps pointer positions through the on-screen scale", async () => {
    const { ipc, calls } = recordingIpc();
    const session = mount(ipc);
    session.bridge.emit({ type: "resize", w: 1000, h: 500 });
    const canvas = canvasOf();
    layoutCanvas(canvas, 500, 250); // displayed at half size
    await fireEvent.pointerDown(canvas, { clientX: 250, clientY: 125, buttons: 1, pointerId: 1 });
    await waitFor(() => expect(calls).toContain("pointer:500,250,1,0"));
    await fireEvent.pointerUp(canvas, { clientX: 250, clientY: 125, buttons: 0, pointerId: 1 });
    await waitFor(() => expect(calls).toContain("pointer:500,250,0,0"));
  });

  it("shows the closed state and reconnects through the shell", async () => {
    const { ipc } = recordingIpc();
    const session = mount(ipc);
    session.bridge.emit({ type: "resize", w: 800, h: 600 });
    session.bridge.emit({ type: "closed", reason: "server initiated disconnect" });
    expect(await screen.findByText("Disconnected")).toBeTruthy();
    expect(screen.getByRole("alert")).toBeTruthy();
    expect(screen.getByText("server initiated disconnect")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Reconnect" }));
    expect(session.retry).toHaveBeenCalledTimes(1);
  });

  it("hands backend errors to the shell (failed tab with retry)", async () => {
    const { ipc } = recordingIpc();
    const session = mount(ipc);
    session.bridge.emit({ type: "error", error: { code: "auth_failed", message: "bad" } });
    await waitFor(() => expect(session.fail).toHaveBeenCalledWith({ code: "auth_failed", message: "bad" }));
  });

  it("works end to end with the mock framebuffer: credentials, then certificate, then desktop", async () => {
    // Unlocked vault: the seeded RDP connection gets its credential from it (no prompt).
    const base = createMockIpc({ framebuffer: true, vault: "unlocked" });
    const { ipc, calls } = recordingIpc(base);
    const session = createTestSession("will-be-replaced");
    const doc = await base.docGet();
    const rdp = doc.nodes.find((n) => n.type === "connection" && n.options.type === "rdp");
    if (!rdp) throw new Error("no rdp connection in the demo document");
    // Open through the mock, wiring its events into the bridge like the shell does.
    const sessionId = await base.fbOpen(rdp.id, 640, 480, (e) => session.bridge.emit(e));
    const context = { ...session.context, sessionId };
    render(FbHarness, { props: { app: createApp({ ...ipc }), context } });

    expect(await screen.findByText("Unknown server certificate")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Trust and connect" }));
    // The recording wrapper swallowed the decision: forward it to the real mock.
    await base.fbCertDecision(sessionId, true);
    await waitFor(() => expect(canvasOf().width).toBe(640));
    await waitFor(() => expect(drawn.some((d) => d.w === 640 && d.h === 480)).toBe(true));
    expect(calls).toContain("cert:true");
  });

  describe("transport security", () => {
    it("tells once, without blocking, that the connection is not encrypted", async () => {
      const { ipc } = recordingIpc();
      const session = mount(ipc);
      session.bridge.emit({ type: "security", encrypted: false });
      session.bridge.emit({ type: "resize", w: 800, h: 600 });
      const notice = await screen.findByText("This connection is not encrypted");
      expect(notice.closest(".banner")?.className).toContain("warning");
      // It is not a dialog: the desktop stays usable.
      expect(screen.queryByRole("dialog")).toBeNull();
      expect(screen.queryByTestId("fb-encrypted")).toBeNull();

      await fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
      await waitFor(() => expect(screen.queryByText("This connection is not encrypted")).toBeNull());
    });

    it("shows a lock instead when the connection is encrypted", async () => {
      const { ipc } = recordingIpc();
      const session = mount(ipc);
      session.bridge.emit({ type: "security", encrypted: true });
      session.bridge.emit({ type: "resize", w: 800, h: 600 });
      expect(await screen.findByTestId("fb-encrypted")).toBeTruthy();
      expect(screen.getByLabelText("Encrypted connection (TLS)")).toBeTruthy();
      expect(screen.queryByText("This connection is not encrypted")).toBeNull();
    });

    it("says nothing for sessions that never report (RDP)", async () => {
      const { ipc } = recordingIpc();
      const session = mount(ipc);
      session.bridge.emit({ type: "resize", w: 800, h: 600 });
      await waitFor(() => expect(canvasOf().width).toBe(800));
      expect(screen.queryByTestId("fb-encrypted")).toBeNull();
      expect(screen.queryByText("This connection is not encrypted")).toBeNull();
    });
  });

  describe("credential prompt", () => {
    const prompt = {
      type: "credentialPrompt" as const,
      kind: "usernamePassword" as const,
      usernameHint: "ann",
      domainHint: "LAB",
      target: "fs01.lab.example:3389",
      error: null,
      canSaveToVault: false,
    };

    it("asks for user name, password and domain, then sends the answer and clears the dialog", async () => {
      const { ipc, calls } = recordingIpc();
      const session = mount(ipc);
      session.bridge.emit(prompt);
      const dialog = await screen.findByRole("dialog", { name: "Credentials needed" });
      expect((screen.getByLabelText("Username") as HTMLInputElement).value).toBe("ann");
      expect((screen.getByLabelText("Domain (optional)") as HTMLInputElement).value).toBe("LAB");
      // The user name is known: the password gets the focus.
      expect(document.activeElement).toBe(screen.getByLabelText("Password"));
      expect(dialog.textContent).toContain("fs01.lab.example:3389");
      // While it is open the app is told (child webviews hide).
      expect(session.app.overlay.open).toBe(true);

      await fireEvent.input(screen.getByLabelText("Password"), { target: { value: "s3cret" } });
      await fireEvent.submit(dialog.querySelector("form") as HTMLFormElement);
      await waitFor(() => expect(calls).toContain("credential:ann:LAB:save=false"));
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
      expect(session.app.overlay.open).toBe(false);
      // The secret is nowhere in the rendered page.
      expect(document.body.innerHTML).not.toContain("s3cret");
    });

    it("comes back with the reason when the credentials were wrong", async () => {
      const { ipc } = recordingIpc();
      const session = mount(ipc);
      session.bridge.emit(prompt);
      await screen.findByRole("dialog", { name: "Credentials needed" });
      await fireEvent.input(screen.getByLabelText("Password"), { target: { value: "x" } });
      await fireEvent.click(screen.getByRole("button", { name: "Connect" }));
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
      session.bridge.emit({ ...prompt, error: "authentication failed (user name, password or domain)" });
      expect((await screen.findByTestId("prompt-error")).textContent).toContain("authentication failed");
      // The new dialog starts empty again: the previous password is gone.
      expect((screen.getByLabelText("Password") as HTMLInputElement).value).toBe("");
      expect(session.fail).not.toHaveBeenCalled();
    });

    it("Escape cancels: the backend is told and the tab shows a cancelled state with retry", async () => {
      const { ipc, calls } = recordingIpc();
      const session = mount(ipc);
      session.bridge.emit(prompt);
      const dialog = await screen.findByRole("dialog", { name: "Credentials needed" });
      await fireEvent.keyDown(dialog, { key: "Escape" });
      await waitFor(() => expect(calls).toContain("credentialCancel"));
      expect(await screen.findByTestId("fb-cancelled")).toBeTruthy();
      expect(screen.getByText("Connection cancelled")).toBeTruthy();
      // The backend's own `cancelled` error afterwards is not an error to report.
      session.bridge.emit({ type: "error", error: { code: "cancelled", message: "No credential was provided." } });
      await Promise.resolve();
      expect(session.fail).not.toHaveBeenCalled();
      await fireEvent.click(screen.getByRole("button", { name: "Try again" }));
      expect(session.retry).toHaveBeenCalledTimes(1);
    });

    it("a cancelled error alone (cancelled elsewhere) shows the same calm state", async () => {
      const { ipc } = recordingIpc();
      const session = mount(ipc);
      session.bridge.emit({ type: "error", error: { code: "cancelled", message: "x" } });
      expect(await screen.findByTestId("fb-cancelled")).toBeTruthy();
      expect(session.fail).not.toHaveBeenCalled();
    });

    it("can ask for a password only (VNC): no user name, no domain", async () => {
      const { ipc, calls } = recordingIpc();
      const session = mount(ipc);
      session.bridge.emit({ ...prompt, kind: "password", usernameHint: null, domainHint: null });
      await screen.findByRole("dialog", { name: "Credentials needed" });
      expect(screen.queryByLabelText("Username")).toBeNull();
      expect(screen.queryByLabelText("Domain (optional)")).toBeNull();
      await fireEvent.input(screen.getByLabelText("Password"), { target: { value: "vnc-pass" } });
      await fireEvent.click(screen.getByRole("button", { name: "Connect" }));
      await waitFor(() => expect(calls).toContain("credential::-:save=false"));
    });

    it("does not pop a dialog over another tab: it waits until this tab is shown", async () => {
      const { ipc } = recordingIpc();
      const session = mount(ipc);
      session.state.visible = false;
      session.bridge.emit(prompt);
      await Promise.resolve();
      expect(screen.queryByRole("dialog")).toBeNull();
      session.state.visible = true;
      expect(await screen.findByRole("dialog", { name: "Credentials needed" })).toBeTruthy();
    });

    it("re-reads the document and the vault once a saved credential got the session going", async () => {
      const base = createMockIpc({ vault: "unlocked", framebuffer: true });
      const docGet = vi.fn(() => base.docGet());
      const vaultList = vi.fn(() => base.vaultList());
      const { ipc, calls } = recordingIpc({ ...base, docGet, vaultList });
      const session = mount(ipc);
      await session.app.vault.refresh();
      session.bridge.emit({ ...prompt, canSaveToVault: true });
      await screen.findByRole("dialog", { name: "Credentials needed" });
      await fireEvent.click(screen.getByLabelText("Save to the vault"));
      await fireEvent.input(screen.getByLabelText("Password"), { target: { value: "keep-me" } });
      await fireEvent.click(screen.getByRole("button", { name: "Connect" }));
      await waitFor(() => expect(calls).toContain("credential:ann:LAB:save=true"));
      const docReads = docGet.mock.calls.length;
      const listReads = vaultList.mock.calls.length;
      // The session proves itself (the desktop arrives): the backend has saved by now.
      session.bridge.emit({ type: "resize", w: 800, h: 600 });
      await waitFor(() => expect(docGet.mock.calls.length).toBeGreaterThan(docReads));
      await waitFor(() => expect(vaultList.mock.calls.length).toBeGreaterThan(listReads));
    });
  });
});

describe("FramebufferView shared clipboard", () => {
  const live = (session: ReturnType<typeof mount>): void => {
    session.bridge.emit({ type: "resize", w: 800, h: 600 });
  };
  const offer = (count: number, bytes: number, skipped = 0) =>
    ({ type: "clipboard", event: { type: "remoteFiles", count, bytes, skipped } }) as const;

  it("shows nothing until the remote clipboard holds files", async () => {
    const { ipc } = recordingIpc();
    const session = mount(ipc);
    live(session);
    await waitFor(() => expect(canvasOf().width).toBe(800));
    expect(screen.queryByTestId("fb-clipboard")).toBeNull();
    expect(screen.queryByRole("button", { name: "Save…" })).toBeNull();
  });

  it("offers to save the files, asks the backend for the folder dialog and shows progress", async () => {
    const { ipc, calls } = recordingIpc();
    const session = mount(ipc);
    live(session);
    session.bridge.emit(offer(3, 2_500_000));
    expect(await screen.findByText("3 items on the remote clipboard (2.5 MB)")).toBeTruthy();
    // Nothing happens until the user asks.
    expect(calls).not.toContain("clipboardSave");
    await fireEvent.click(screen.getByRole("button", { name: "Save…" }));
    await waitFor(() => expect(calls).toContain("clipboardSave"));
    expect(await screen.findByText("Saving files from the remote clipboard")).toBeTruthy();

    session.bridge.emit({
      type: "clipboard",
      event: { type: "progress", filesDone: 1, filesTotal: 3, bytesDone: 1_000_000, bytesTotal: 2_500_000 },
    });
    expect(await screen.findByText("1 of 3 files, 1.0 MB of 2.5 MB")).toBeTruthy();
    const bar = screen.getByLabelText("Save progress") as HTMLProgressElement;
    expect(bar.value).toBe(40);

    session.bridge.emit({ type: "clipboard", event: { type: "done", files: 3, bytes: 2_500_000, skipped: 1 } });
    expect(await screen.findByText("Files saved")).toBeTruthy();
    expect(screen.getByText(/3 files saved \(2\.5 MB\)\./)).toBeTruthy();
    expect(screen.getByText(/1 could not be saved because of their names/)).toBeTruthy();
  });

  it("cancels a save in progress and says that written files are kept", async () => {
    const { ipc, calls } = recordingIpc();
    const session = mount(ipc);
    live(session);
    session.bridge.emit(offer(1, 100));
    expect(await screen.findByText("1 item on the remote clipboard (100 B)")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Save…" }));
    await screen.findByText("Saving files from the remote clipboard");
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(calls).toContain("clipboardCancel"));
    session.bridge.emit({ type: "clipboard", event: { type: "failed", reason: "cancelled" } });
    expect(await screen.findByText(/The save was cancelled\. Files already written are kept\./)).toBeTruthy();
  });

  it("a dismissed folder dialog starts nothing", async () => {
    saveAnswer = false;
    const { ipc, calls } = recordingIpc();
    const session = mount(ipc);
    live(session);
    session.bridge.emit(offer(2, 10));
    await fireEvent.click(await screen.findByRole("button", { name: "Save…" }));
    await waitFor(() => expect(calls).toContain("clipboardSave"));
    expect(screen.queryByText("Saving files from the remote clipboard")).toBeNull();
    expect(screen.getByRole("button", { name: "Save…" })).toBeTruthy();
  });

  it("reports a failure in words and lets the user close it", async () => {
    const { ipc } = recordingIpc();
    const session = mount(ipc);
    live(session);
    session.bridge.emit(offer(2, 10));
    session.bridge.emit({ type: "clipboard", event: { type: "failed", reason: "destination" } });
    expect(await screen.findByText("The files were not saved")).toBeTruthy();
    expect(screen.getByText(/refused a file/)).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    await waitFor(() => expect(screen.queryByText("The files were not saved")).toBeNull());
    // The offer is still there for another try.
    expect(screen.getByRole("button", { name: "Save…" })).toBeTruthy();
  });

  it("the offer goes away with the files and can be closed by the user", async () => {
    const { ipc } = recordingIpc();
    const session = mount(ipc);
    live(session);
    session.bridge.emit(offer(2, 10));
    await screen.findByRole("button", { name: "Save…" });
    session.bridge.emit(offer(0, 0));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Save…" })).toBeNull());
    session.bridge.emit(offer(2, 10));
    await fireEvent.click(await screen.findByRole("button", { name: "Dismiss" }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Save…" })).toBeNull());
    // A different offer shows again.
    session.bridge.emit(offer(5, 99));
    expect(await screen.findByText("5 items on the remote clipboard (99 B)")).toBeTruthy();
  });
});
