import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { type AppError, type Ipc } from "../ipc";
import { MOCK_IDS, createMockIpc } from "../ipc/mock";
import { createApp, type App } from "../state/app";
import { SessionBridge } from "../tabs/session";
import TerminalHarness, { type Hooks } from "./term/TerminalHarness.svelte";

// A scriptable stand-in for xterm.js: jsdom has no canvas or layout.
const xterm = vi.hoisted(() => {
  type Listener<T> = (value: T) => void;
  class FakeTerminal {
    static instances: FakeTerminal[] = [];
    cols = 100;
    rows = 30;
    written: Uint8Array[] = [];
    doneCallbacks: Array<() => void> = [];
    selection = "";
    focus = vi.fn();
    dispose = vi.fn();
    open = vi.fn();
    keyHandler: ((e: KeyboardEvent) => boolean) | null = null;
    #data: Array<Listener<string>> = [];
    #binary: Array<Listener<string>> = [];
    #resize: Array<Listener<{ cols: number; rows: number }>> = [];
    options: { theme?: Record<string, string> };
    constructor(options: { theme?: Record<string, string> } = {}) {
      this.options = { ...options };
      FakeTerminal.instances.push(this);
    }
    loadAddon(addon: { activate?: (t: FakeTerminal) => void }): void {
      addon.activate?.(this);
    }
    onData(fn: Listener<string>) {
      this.#data.push(fn);
      return { dispose: () => undefined };
    }
    onBinary(fn: Listener<string>) {
      this.#binary.push(fn);
      return { dispose: () => undefined };
    }
    onResize(fn: Listener<{ cols: number; rows: number }>) {
      this.#resize.push(fn);
      return { dispose: () => undefined };
    }
    attachCustomKeyEventHandler(fn: (e: KeyboardEvent) => boolean): void {
      this.keyHandler = fn;
    }
    write(data: Uint8Array, done?: () => void): void {
      this.written.push(data);
      if (done) this.doneCallbacks.push(done);
    }
    hasSelection(): boolean {
      return this.selection !== "";
    }
    getSelection(): string {
      return this.selection;
    }
    // helpers for the tests
    type(text: string): void {
      this.#data.forEach((fn) => fn(text));
    }
    resizeTo(cols: number, rows: number): void {
      this.cols = cols;
      this.rows = rows;
      this.#resize.forEach((fn) => fn({ cols, rows }));
    }
  }
  class FakeFit {
    static fits = 0;
    static next: { cols: number; rows: number } | null = null;
    term: FakeTerminal | null = null;
    activate(t: FakeTerminal): void {
      this.term = t;
    }
    fit(): void {
      FakeFit.fits++;
      if (FakeFit.next && this.term) this.term.resizeTo(FakeFit.next.cols, FakeFit.next.rows);
    }
  }
  return { FakeTerminal, FakeFit };
});

vi.mock("@xterm/xterm", () => ({ Terminal: xterm.FakeTerminal }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: xterm.FakeFit }));
vi.mock("@xterm/xterm/css/xterm.css", () => ({}));

const { FakeTerminal, FakeFit } = xterm;

interface Mounted {
  app: App;
  ipc: Ipc;
  bridge: SessionBridge;
  hooks: Hooks;
  fails: AppError[];
  retries: number[];
  term: () => InstanceType<typeof FakeTerminal>;
  spies: {
    termWrite: ReturnType<typeof vi.fn>;
    termAck: ReturnType<typeof vi.fn>;
    termResize: ReturnType<typeof vi.fn>;
    sshHostKeyDecision: ReturnType<typeof vi.fn>;
    knownHostsRemove: ReturnType<typeof vi.fn>;
    sessionCredential: ReturnType<typeof vi.fn>;
    sessionCredentialCancel: ReturnType<typeof vi.fn>;
    docGet: ReturnType<typeof vi.fn>;
    vaultList: ReturnType<typeof vi.fn>;
  };
}

async function mount(connectionId: string = MOCK_IDS.localSshd): Promise<Mounted> {
  const base = createMockIpc({ vault: "unlocked", terminal: true });
  const spies = {
    termWrite: vi.fn(async () => undefined),
    termAck: vi.fn(async () => undefined),
    termResize: vi.fn(async () => undefined),
    sshHostKeyDecision: vi.fn(async () => undefined),
    knownHostsRemove: vi.fn((host: string, port: number) => base.knownHostsRemove(host, port)),
    sessionCredential: vi.fn(async () => undefined),
    sessionCredentialCancel: vi.fn(async () => undefined),
    docGet: vi.fn(() => base.docGet()),
    vaultList: vi.fn(() => base.vaultList()),
  };
  const ipc: Ipc = { ...base, ...spies };
  const app = createApp(ipc);
  await app.doc.load();
  const bridge = new SessionBridge();
  const fails: AppError[] = [];
  const retries: number[] = [];
  const hooks: Hooks = {
    onFail: (e) => fails.push(e),
    onRetry: () => retries.push(1),
  };
  render(TerminalHarness, { props: { app, bridge, hooks, connectionId } });
  return { app, ipc, bridge, hooks, fails, retries, spies, term: () => FakeTerminal.instances.at(-1) as never };
}

beforeEach(() => {
  FakeTerminal.instances = [];
  FakeFit.fits = 0;
  FakeFit.next = null;
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
    cb(0);
    return 1;
  });
  vi.stubGlobal("cancelAnimationFrame", () => undefined);
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, value: 800 });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, value: 600 });
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("TerminalView", () => {
  it("opens xterm, fits it and reports the size to the backend", async () => {
    FakeFit.next = { cols: 120, rows: 40 };
    const m = await mount();
    expect(m.term().open).toHaveBeenCalledTimes(1);
    expect(FakeFit.fits).toBeGreaterThanOrEqual(1);
    expect(m.spies.termResize).toHaveBeenCalledWith("sess-1", 120, 40);
  });

  it("writes session data to the terminal and acknowledges it after it was consumed", async () => {
    const m = await mount();
    m.bridge.emit({ type: "data", bytes: new Uint8Array([104, 105]) });
    const t = m.term();
    expect(t.written).toHaveLength(1);
    expect(m.spies.termAck).not.toHaveBeenCalled();
    t.doneCallbacks.splice(0).forEach((cb) => cb());
    await waitFor(() => expect(m.spies.termAck).toHaveBeenCalledWith("sess-1", 2));
  });

  it("replays events that arrived before the view subscribed", async () => {
    const base = createMockIpc({ vault: "unlocked", terminal: true });
    const app = createApp(base);
    await app.doc.load();
    const bridge = new SessionBridge();
    bridge.emit({ type: "data", bytes: new Uint8Array([65]) });
    render(TerminalHarness, {
      props: { app, bridge, hooks: { onFail: () => undefined, onRetry: () => undefined }, connectionId: MOCK_IDS.localSshd },
    });
    expect(FakeTerminal.instances.at(-1)?.written).toHaveLength(1);
  });

  it("sends typed input as raw bytes, one write at a time and in order", async () => {
    let release: () => void = () => undefined;
    const m = await mount();
    let first = true;
    m.spies.termWrite.mockImplementation(async () => {
      if (first) {
        first = false;
        await new Promise<void>((resolve) => (release = resolve));
      }
    });
    m.term().type("l");
    m.term().type("s");
    m.term().type("\r");
    await waitFor(() => expect(m.spies.termWrite).toHaveBeenCalledTimes(1));
    release();
    await waitFor(() => expect(m.spies.termWrite).toHaveBeenCalledTimes(2));
    const sent = m.spies.termWrite.mock.calls.map((c) => new TextDecoder().decode(c[1] as Uint8Array));
    expect(sent).toEqual(["l", "s\r"]);
  });

  it("shows the host key prompt and sends accept", async () => {
    const m = await mount();
    m.bridge.emit({ type: "hostKeyPrompt", fingerprint: "SHA256:abcdef", algorithm: "ssh-ed25519", host: "127.0.0.1", port: 12222 });
    expect(await screen.findByText("Unknown host key")).toBeTruthy();
    expect(screen.getByText("SHA256:abcdef")).toBeTruthy();
    expect(screen.getByText("ssh-ed25519")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Trust and connect" }));
    await waitFor(() => expect(m.spies.sshHostKeyDecision).toHaveBeenCalledWith("sess-1", true));
    await waitFor(() => expect(screen.queryByText("Unknown host key")).toBeNull());
  });

  it("names the hop the key belongs to: a jump host is not the target", async () => {
    const m = await mount();
    m.bridge.emit({ type: "hostKeyPrompt", fingerprint: "SHA256:jump", algorithm: "ssh-ed25519", host: "jump.example", port: 2222 });
    expect(await screen.findByText("jump.example:2222")).toBeTruthy();
    // The hint says it is not the key of the final target (127.0.0.1:12222 here).
    expect(screen.getByText(/jump host on the way to 127\.0\.0\.1:12222/)).toBeTruthy();
    expect(screen.getByText("SHA256:jump")).toBeTruthy();
  });

  it("does not add the jump host hint when the key is the target's own", async () => {
    const m = await mount();
    m.bridge.emit({ type: "hostKeyPrompt", fingerprint: "SHA256:own", algorithm: "ssh-ed25519", host: "127.0.0.1", port: 12222 });
    expect(await screen.findByText("127.0.0.1:12222")).toBeTruthy();
    expect(screen.queryByText(/jump host/)).toBeNull();
  });

  it("announces the host key prompt as an alert dialog and focuses the safe action", async () => {
    const m = await mount();
    m.bridge.emit({ type: "hostKeyPrompt", fingerprint: "SHA256:abcdef", algorithm: "ssh-ed25519", host: "127.0.0.1", port: 12222 });
    const dialog = await screen.findByRole("alertdialog", { name: "Unknown host key" });
    expect(dialog.getAttribute("aria-describedby")).toBeTruthy();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Reject" }));
    expect(m.spies.sshHostKeyDecision).not.toHaveBeenCalled();
  });

  it("shows a connecting state with the target while an SSH session opens", async () => {
    await mount();
    const status = await screen.findByRole("status");
    expect(status.textContent).toContain("Connecting to host…");
    expect(status.textContent).toContain("127.0.0.1");
  });

  it("recolours the terminal when the theme changes and refits", async () => {
    document.documentElement.style.setProperty("--terminal-bg", "#ffffff");
    document.documentElement.style.setProperty("--text-primary", "#1c1c1e");
    const m = await mount();
    expect(m.term().options.theme?.background).toBe("#ffffff");
    const fits = FakeFit.fits;
    document.documentElement.style.setProperty("--terminal-bg", "#1e1f22");
    document.documentElement.style.setProperty("--text-primary", "#f2f2f5");
    document.documentElement.setAttribute("data-theme", "dark");
    await waitFor(() => expect(m.term().options.theme?.background).toBe("#1e1f22"));
    expect(m.term().options.theme?.foreground).toBe("#f2f2f5");
    await waitFor(() => expect(FakeFit.fits).toBeGreaterThan(fits));
    document.documentElement.removeAttribute("style");
    document.documentElement.removeAttribute("data-theme");
  });

  it("sends reject from the prompt", async () => {
    const m = await mount();
    m.bridge.emit({ type: "hostKeyPrompt", fingerprint: "SHA256:x", algorithm: "ssh-ed25519", host: "127.0.0.1", port: 12222 });
    await fireEvent.click(await screen.findByRole("button", { name: "Reject" }));
    await waitFor(() => expect(m.spies.sshHostKeyDecision).toHaveBeenCalledWith("sess-1", false));
  });

  it("shows a changed host key clearly, with the way to remove the stored key", async () => {
    // A connection to `changed.example`, whose (mock) stored key is stale.
    const base = createMockIpc({ vault: "unlocked", terminal: true });
    const doc = await base.docGet();
    const seed = doc.nodes.find((n) => n.id === MOCK_IDS.localSshd);
    if (!seed || seed.type !== "connection") throw new Error("seed changed");
    const changed = await base.docUpsertNode({ ...seed, id: crypto.randomUUID(), name: "Changed", host: "changed.example", port: null });
    const knownHostsRemove = vi.fn((h: string, p: number) => base.knownHostsRemove(h, p));
    const app = createApp({ ...base, knownHostsRemove });
    await app.doc.load();
    const bridge = new SessionBridge();
    const retries: number[] = [];
    render(TerminalHarness, {
      props: { app, bridge, connectionId: changed.id, hooks: { onFail: () => undefined, onRetry: () => retries.push(1) } },
    });
    bridge.emit({
      type: "error",
      error: { code: "host_key_mismatch", message: "The host key of changed.example:22 has CHANGED. Stored: A. Presented: B." },
    });
    expect(await screen.findByText("The host key has changed")).toBeTruthy();
    expect(screen.getByRole("alertdialog", { name: "The host key has changed" })).toBeTruthy();
    // The safe choice is the default; accepting the new key is not offered at all.
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Close tab" }));
    expect(screen.queryByRole("button", { name: /trust/i })).toBeNull();
    expect(screen.getByText("A")).toBeTruthy();
    expect(screen.getByText("B")).toBeTruthy();
    expect(screen.getByText(/has CHANGED/)).toBeTruthy();
    expect(screen.getByText(/remove the stored key first/)).toBeTruthy();

    await fireEvent.click(screen.getByRole("button", { name: "Known hosts" }));
    const rows = await screen.findAllByTestId("known-host");
    expect(rows[0]?.textContent).toContain("changed.example:22");
    expect(rows[0]?.textContent).toContain("this connection");

    await fireEvent.click(screen.getByRole("button", { name: "Remove stored key and reconnect" }));
    await waitFor(() => expect(knownHostsRemove).toHaveBeenCalledWith("changed.example", 22));
    await waitFor(() => expect(retries).toHaveLength(1));
    expect(await base.knownHostsList()).not.toEqual(expect.arrayContaining([expect.objectContaining({ host: "changed.example" })]));
  });

  it("a changed key of a jump host names that hop and removes that hop's key, not the target's", async () => {
    const base = createMockIpc({ vault: "unlocked", terminal: true });
    const knownHostsRemove = vi.fn(async () => undefined);
    const app = createApp({ ...base, knownHostsRemove });
    await app.doc.load();
    const bridge = new SessionBridge();
    const retries: number[] = [];
    render(TerminalHarness, {
      props: { app, bridge, connectionId: MOCK_IDS.localSshd, hooks: { onFail: () => undefined, onRetry: () => retries.push(1) } },
    });
    bridge.emit({
      type: "error",
      error: { code: "host_key_mismatch", message: "The host key of jump.example:2222 has CHANGED (possible man-in-the-middle attack). Stored: A. Presented: B." },
    });
    expect(await screen.findByText("The host key has changed")).toBeTruthy();
    // The panel says which host the key is about.
    expect(screen.getByText("jump.example:2222")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Remove stored key and reconnect" }));
    await waitFor(() => expect(knownHostsRemove).toHaveBeenCalledWith("jump.example", 2222));
    expect(knownHostsRemove).not.toHaveBeenCalledWith("127.0.0.1", 12222);
    await waitFor(() => expect(retries).toHaveLength(1));
  });

  it("hands every other error to the shell's error view", async () => {
    const m = await mount();
    const error: AppError = { code: "host_unreachable", message: "no route" };
    m.bridge.emit({ type: "error", error });
    await waitFor(() => expect(m.fails).toEqual([error]));
  });

  it("shows the exit code and offers to reconnect", async () => {
    const m = await mount();
    m.bridge.emit({ type: "exit", code: 3 });
    expect(await screen.findByText("The session ended (exit code 3).")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Reconnect" }));
    expect(m.retries).toHaveLength(1);
    // Nothing is typed into a finished session.
    m.term().type("x");
    expect(m.spies.termWrite).not.toHaveBeenCalled();
  });

  it("refits and focuses when the tab becomes visible again", async () => {
    const m = await mount();
    m.bridge.emit({ type: "data", bytes: new Uint8Array([1]) });
    await waitFor(() => expect(m.term().focus).toHaveBeenCalled());
    const fitsBefore = FakeFit.fits;
    const focusBefore = m.term().focus.mock.calls.length;
    m.hooks.setVisible?.(false);
    await Promise.resolve();
    FakeFit.next = { cols: 90, rows: 25 };
    m.hooks.setVisible?.(true);
    await waitFor(() => expect(FakeFit.fits).toBeGreaterThan(fitsBefore));
    await waitFor(() => expect(m.spies.termResize).toHaveBeenCalledWith("sess-1", 90, 25));
    await waitFor(() => expect(m.term().focus.mock.calls.length).toBeGreaterThan(focusBefore));
  });

  it("does not take the keyboard focus while a dialog is open", async () => {
    const m = await mount();
    m.hooks.setOverlay?.(true);
    await Promise.resolve();
    const before = m.term().focus.mock.calls.length;
    m.hooks.setVisible?.(false);
    m.hooks.setVisible?.(true);
    await Promise.resolve();
    expect(m.term().focus.mock.calls.length).toBe(before);
    m.hooks.setOverlay?.(false);
    await waitFor(() => expect(m.term().focus.mock.calls.length).toBeGreaterThan(before));
  });

  it("keeps Ctrl+W, Ctrl+K and Ctrl+N away from the shell's global shortcuts", async () => {
    const m = await mount();
    const seen: string[] = [];
    const onWindow = (e: KeyboardEvent) => seen.push(e.key);
    window.addEventListener("keydown", onWindow);
    const target = document.querySelector(".term") as HTMLElement;
    for (const key of ["w", "k", "n"]) {
      target.dispatchEvent(new KeyboardEvent("keydown", { key, ctrlKey: true, bubbles: true }));
    }
    // Tab switching stays global.
    target.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", ctrlKey: true, bubbles: true }));
    window.removeEventListener("keydown", onWindow);
    expect(seen).toEqual(["Tab"]);
    expect(m.term()).toBeTruthy();
  });

  it("copies the selection with Ctrl+Shift+C and does not send the key to the remote", async () => {
    const writeText = vi.fn(async () => undefined);
    vi.stubGlobal("navigator", { ...navigator, platform: "Win32", clipboard: { writeText } });
    const m = await mount();
    m.term().selection = "selected text";
    const handler = m.term().keyHandler;
    expect(handler).not.toBeNull();
    const down = new KeyboardEvent("keydown", { key: "C", ctrlKey: true, shiftKey: true });
    expect(handler?.(down)).toBe(false);
    expect(writeText).toHaveBeenCalledWith("selected text");
    // Plain keys go to xterm.
    expect(handler?.(new KeyboardEvent("keydown", { key: "a" }))).toBe(true);
    // Ctrl+C without a selection is SIGINT: xterm handles it.
    m.term().selection = "";
    expect(handler?.(new KeyboardEvent("keydown", { key: "c", ctrlKey: true }))).toBe(true);
  });

  it("tells the user when copying fails", async () => {
    const writeText = vi.fn(async () => {
      throw new Error("denied");
    });
    vi.stubGlobal("navigator", { ...navigator, platform: "Win32", clipboard: { writeText } });
    const m = await mount();
    m.term().selection = "x";
    m.term().keyHandler?.(new KeyboardEvent("keydown", { key: "C", ctrlKey: true, shiftKey: true }));
    await waitFor(() => expect(m.app.notices.items.some((n) => n.title.includes("denied"))).toBe(true));
  });

  it("disposes the terminal when the view goes away, without closing the session itself", async () => {
    const m = await mount();
    const t = m.term();
    cleanup();
    expect(t.dispose).toHaveBeenCalledTimes(1);
  });

  describe("credential prompt", () => {
    const prompt = {
      type: "credentialPrompt" as const,
      kind: "sshKey" as const,
      usernameHint: "tester",
      domainHint: null,
      target: "127.0.0.1:12222",
      error: null,
      canSaveToVault: false,
    };

    it("asks for the credential, sends the answer to the backend only and leaves no secret on screen", async () => {
      const m = await mount();
      m.bridge.emit(prompt);
      const dialog = await screen.findByRole("dialog", { name: "Credentials needed" });
      expect(dialog.textContent).toContain("127.0.0.1:12222");
      expect(document.activeElement).toBe(screen.getByLabelText("Password"));
      await fireEvent.input(screen.getByLabelText("Password"), { target: { value: "hunter2" } });
      await fireEvent.submit(dialog.querySelector("form") as HTMLFormElement);
      await waitFor(() =>
        expect(m.spies.sessionCredential).toHaveBeenCalledWith(
          "sess-1",
          { username: "tester", password: "hunter2", keyPath: null, passphrase: null, domain: null },
          false,
        ),
      );
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
      expect(document.body.innerHTML).not.toContain("hunter2");
      // Nothing of it was written to the terminal either.
      expect(m.term().written).toHaveLength(0);
      expect(m.fails).toEqual([]);
    });

    it("shows the prompt again with the error after wrong credentials, without failing the tab", async () => {
      const m = await mount();
      m.bridge.emit(prompt);
      await screen.findByRole("dialog", { name: "Credentials needed" });
      await fireEvent.input(screen.getByLabelText("Password"), { target: { value: "nope" } });
      await fireEvent.click(screen.getByRole("button", { name: "Connect" }));
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
      m.bridge.emit({ ...prompt, error: "127.0.0.1:12222 rejected the credentials of user tester." });
      expect((await screen.findByTestId("prompt-error")).textContent).toContain("rejected the credentials");
      expect((screen.getByLabelText("Password") as HTMLInputElement).value).toBe("");
      expect(m.fails).toEqual([]);
    });

    it("Escape cancels: a cancelled state with retry and close, no error toast", async () => {
      const m = await mount();
      m.bridge.emit(prompt);
      const dialog = await screen.findByRole("dialog", { name: "Credentials needed" });
      await fireEvent.keyDown(dialog, { key: "Escape" });
      await waitFor(() => expect(m.spies.sessionCredentialCancel).toHaveBeenCalledWith("sess-1"));
      expect(await screen.findByTestId("term-cancelled")).toBeTruthy();
      expect(screen.getByText(/Connection cancelled/)).toBeTruthy();
      m.bridge.emit({ type: "error", error: { code: "cancelled", message: "No credential was provided." } });
      await Promise.resolve();
      expect(m.fails).toEqual([]);
      expect(m.app.notices.items).toHaveLength(0);
      await fireEvent.click(screen.getByRole("button", { name: "Try again" }));
      expect(m.retries).toHaveLength(1);
    });

    it("waits for the tab to be shown before it opens the dialog", async () => {
      const m = await mount();
      m.hooks.setVisible?.(false);
      await Promise.resolve();
      m.bridge.emit(prompt);
      await Promise.resolve();
      expect(screen.queryByRole("dialog")).toBeNull();
      m.hooks.setVisible?.(true);
      expect(await screen.findByRole("dialog", { name: "Credentials needed" })).toBeTruthy();
    });

    it("after a save request, re-reads the document and the vault once the shell is there", async () => {
      const m = await mount();
      await m.app.vault.refresh();
      m.bridge.emit({ ...prompt, canSaveToVault: true });
      await screen.findByRole("dialog", { name: "Credentials needed" });
      await fireEvent.click(screen.getByLabelText("Save to the vault"));
      await fireEvent.input(screen.getByLabelText("Password"), { target: { value: "pw" } });
      await fireEvent.click(screen.getByRole("button", { name: "Connect" }));
      await waitFor(() => expect(m.spies.sessionCredential).toHaveBeenCalled());
      expect(m.spies.sessionCredential.mock.calls[0]?.[2]).toBe(true);
      const docReads = m.spies.docGet.mock.calls.length;
      const listReads = m.spies.vaultList.mock.calls.length;
      m.bridge.emit({ type: "data", bytes: new Uint8Array([36, 32]) });
      await waitFor(() => expect(m.spies.docGet.mock.calls.length).toBeGreaterThan(docReads));
      await waitFor(() => expect(m.spies.vaultList.mock.calls.length).toBeGreaterThan(listReads));
    });
  });
});
