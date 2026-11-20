// The session toolbar as wired in the real shell (mock backend): buttons per kind, the facts it
// shows, Ctrl+Alt+Del, focus and hiding.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "../../App.svelte";
import type { Ipc } from "../ipc";
import { MOCK_IDS, createMockIpc } from "../ipc/mock";
import { createApp, type App as AppState } from "../state/app";
import { TOOLBAR_VISIBLE_KEY } from "./toolbar.svelte";

vi.mock("@xterm/xterm", async () => ({
  Terminal: (await import("../sessions/term/fakeXterm")).FakeTerminal,
}));

type Feed = (event: unknown) => void;

let store: Map<string, string>;

beforeEach(() => {
  store = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => {
      store.set(k, v);
    },
  });
  vi.stubGlobal("ImageData", class {});
  HTMLCanvasElement.prototype.toDataURL = () => "data:image/png;base64,AAAA";
  HTMLCanvasElement.prototype.getContext = function getContext() {
    return { putImageData() {} };
  } as unknown as typeof HTMLCanvasElement.prototype.getContext;
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function feedIpc() {
  const feeds: Feed[] = [];
  const keys: string[] = [];
  let counter = 0;
  const base = createMockIpc({ vault: "unlocked" });
  const ipc: Ipc = {
    ...base,
    termOpen: async (_id, _c, _r, onEvent) => {
      feeds.push(onEvent as Feed);
      return `s${++counter}`;
    },
    fbOpen: async (_id, _w, _h, onEvent) => {
      feeds.push(onEvent as Feed);
      return `s${++counter}`;
    },
    webOpen: async () => `s${++counter}`,
    fbKey: async (_sid, code, down) => void keys.push(`${code}${down ? "+" : "-"}`),
    sessionClose: async () => undefined,
  };
  return { ipc, feeds, keys };
}

async function mount(initial: { vaultKeyHidden?: boolean } = {}) {
  if (initial.vaultKeyHidden) store.set(TOOLBAR_VISIBLE_KEY, "false");
  const fx = feedIpc();
  const app: AppState = createApp(fx.ipc);
  render(App, { props: { app, demo: false } });
  await waitFor(() => expect(app.doc.status).toBe("ready"));
  return { ...fx, app };
}

const bar = () => screen.getByTestId("session-toolbar");
const buttons = () =>
  within(bar())
    .getAllByRole("button")
    .map((b) => b.getAttribute("aria-label"));

const security = (encrypted: boolean, auth?: "negotiate" | "ntlm" | "none") => ({
  type: "security",
  encrypted,
  ...(auth ? { auth } : {}),
});

describe("buttons per kind", () => {
  it("a local shell has Reconnect, Duplicate, Close and the default shell, no Ctrl+Alt+Del", async () => {
    const { app } = await mount();
    await app.tabs.openConnection(MOCK_IDS.localZsh);
    await waitFor(() => expect(bar()).toBeTruthy());
    expect(buttons()).toEqual(["Reconnect", "Duplicate tab", "Close tab"]);
    expect(within(bar()).getByTestId("session-info").textContent).toContain("Default shell");
  });

  it("SSH shows host:port and the connected state", async () => {
    const { app, feeds } = await mount();
    await app.tabs.openConnection(MOCK_IDS.localSshd);
    await waitFor(() => expect(feeds).toHaveLength(1));
    feeds[0]?.({ type: "data", bytes: new Uint8Array([1]) });
    await waitFor(() => expect(within(bar()).getByTestId("session-info").textContent).toContain("Connected"));
    expect(within(bar()).getByTestId("session-info").textContent).toContain("127.0.0.1:12222");
    expect(buttons()).not.toContain("Send Ctrl+Alt+Del");
    feeds[0]?.({ type: "exit", code: 0 });
    await waitFor(() => expect(within(bar()).getByTestId("session-info").textContent).toContain("Disconnected"));
  });

  it("RDP has Ctrl+Alt+Del, disabled until connected, with resolution and lock", async () => {
    const { app, feeds } = await mount();
    await app.tabs.openConnection(MOCK_IDS.rdpServer);
    await waitFor(() => expect(feeds).toHaveLength(1));
    const send = (): HTMLButtonElement => within(bar()).getByRole("button", { name: "Send Ctrl+Alt+Del" });
    expect(buttons()).toEqual([
      "Reconnect",
      "Duplicate tab",
      "Close tab",
      "Send Ctrl+Alt+Del",
    ]);
    expect(send().disabled).toBe(true);
    feeds[0]?.(security(true, "negotiate"));
    feeds[0]?.({ type: "resize", w: 1280, h: 720 });
    await waitFor(() => expect(send().disabled).toBe(false));
    const info = within(bar()).getByTestId("session-info");
    expect(info.textContent).toContain("fs01.lab.example:3389");
    expect(info.textContent).toContain("1280×720");
    expect(within(bar()).getByRole("img", { name: "NLA · Kerberos or NTLM" })).toBeTruthy();
  });

  it.each([
    ["ntlm", "NLA · NTLM", false],
    ["none", "TLS without NLA", true],
  ] as const)("RDP auth %s says %s", async (auth, text, warning) => {
    const { app, feeds } = await mount();
    await app.tabs.openConnection(MOCK_IDS.rdpServer);
    await waitFor(() => expect(feeds).toHaveLength(1));
    feeds[0]?.(security(true, auth));
    const lock = await within(bar()).findByRole("img", { name: text });
    expect(lock.classList.contains("warn")).toBe(warning);
  });

  it.each([
    [true, "Encrypted", false],
    [false, "Not encrypted", true],
  ] as const)("VNC encrypted=%s says %s", async (encrypted, text, warning) => {
    const { app, feeds } = await mount();
    await app.tabs.openConnection(MOCK_IDS.devVm);
    await waitFor(() => expect(feeds).toHaveLength(1));
    feeds[0]?.(security(encrypted));
    feeds[0]?.({ type: "resize", w: 800, h: 600 });
    const lock = await within(bar()).findByRole("img", { name: text });
    expect(lock.classList.contains("warn")).toBe(warning);
    expect(within(bar()).getByRole("button", { name: "Send Ctrl+Alt+Del" })).toBeTruthy();
  });

  it("web has the page controls and the address in this one row, and one toolbar only", async () => {
    const { app } = await mount();
    await app.tabs.openConnection(MOCK_IDS.intranet);
    await waitFor(() => expect(bar()).toBeTruthy());
    await waitFor(() => expect(buttons()).toContain("Back"));
    expect(buttons()).toEqual([
      "Reconnect",
      "Duplicate tab",
      "Close tab",
      "Back",
      "Forward",
      "Reload",
    ]);
    expect((within(bar()).getByLabelText("Address") as HTMLInputElement).value).toBe("https://intranet.example/");
    expect(screen.getAllByRole("toolbar", { name: "Web page controls" })).toHaveLength(1);
  });

  it("the Home tab has no toolbar", async () => {
    const { app } = await mount();
    app.tabs.openHome();
    await screen.findByTestId("home");
    expect(screen.queryByTestId("session-toolbar")).toBeNull();
  });
});

describe("actions", () => {
  it("Duplicate opens a second tab on the same connection", async () => {
    const { app } = await mount();
    await app.tabs.openConnection(MOCK_IDS.localZsh);
    await fireEvent.click(await within(await screen.findByTestId("session-toolbar")).findByRole("button", { name: "Duplicate tab" }));
    await waitFor(() => expect(app.tabs.connectionTabs).toHaveLength(2));
    expect(app.tabs.connectionTabs.map((x) => x.connectionId)).toEqual([MOCK_IDS.localZsh, MOCK_IDS.localZsh]);
  });

  it("Close on a connected session asks first", async () => {
    const { app, feeds } = await mount();
    await app.tabs.openConnection(MOCK_IDS.localSshd);
    await waitFor(() => expect(feeds).toHaveLength(1));
    feeds[0]?.({ type: "data", bytes: new Uint8Array([1]) });
    await waitFor(() => expect(app.tabs.liveCount()).toBe(1));
    await fireEvent.click(within(bar()).getByRole("button", { name: "Close tab" }));
    await screen.findByRole("dialog", { name: "Close 1 active session?" });
    expect(app.tabs.connectionTabs).toHaveLength(1);
  });

  it("Reconnect starts the session again", async () => {
    const { app, feeds } = await mount();
    await app.tabs.openConnection(MOCK_IDS.localZsh);
    await waitFor(() => expect(feeds).toHaveLength(1));
    await fireEvent.click(within(bar()).getByRole("button", { name: "Reconnect" }));
    await waitFor(() => expect(feeds).toHaveLength(2));
  });

  it("Ctrl+Alt+Del sends the exact key sequence, with the Mac shortcuts on and without focus", async () => {
    const { app, feeds, keys } = await mount();
    // The connection has the Mac shortcuts option, but the view is not focused at all.
    await app.tabs.openConnection(MOCK_IDS.rdpServer);
    await waitFor(() => expect(feeds).toHaveLength(1));
    feeds[0]?.(security(true, "negotiate"));
    feeds[0]?.({ type: "resize", w: 800, h: 600 });
    const send = within(bar()).getByRole("button", { name: "Send Ctrl+Alt+Del" });
    await waitFor(() => expect((send as HTMLButtonElement).disabled).toBe(false));
    expect(document.activeElement).not.toBe(screen.getByLabelText("Remote desktop"));
    await fireEvent.click(send);
    await waitFor(() => expect(keys).toHaveLength(6));
    expect(keys).toEqual(["ControlLeft+", "AltLeft+", "Delete+", "Delete-", "AltLeft-", "ControlLeft-"]);
    // The keyboard goes back to the desktop.
    expect(document.activeElement).toBe(screen.getByLabelText("Remote desktop"));
  });

  it("pressing a toolbar button does not take the focus from the session", async () => {
    const { app, feeds } = await mount();
    await app.tabs.openConnection(MOCK_IDS.rdpServer);
    await waitFor(() => expect(feeds).toHaveLength(1));
    const button = within(bar()).getByRole("button", { name: "Duplicate tab" });
    // fireEvent returns false when the default action (moving the focus) was prevented.
    expect(await fireEvent.mouseDown(button)).toBe(false);
  });
});

const tab = () => screen.getByTestId("session-toolbar-tab");
const rowButtons = () =>
  screen.queryAllByRole("button").filter((b) => bar().contains(b)).map((b) => b.getAttribute("aria-label"));

describe("the toolbar tab", () => {
  it.each([MOCK_IDS.localZsh, MOCK_IDS.localSshd, MOCK_IDS.rdpServer, MOCK_IDS.devVm, MOCK_IDS.intranet])(
    "%s has the tab with the toolbar open and hidden",
    async (id) => {
      const { app } = await mount();
      await app.tabs.openConnection(id);
      await waitFor(() => expect(tab()).toBeTruthy());
      expect(tab().getAttribute("aria-expanded")).toBe("true");
      expect(tab().getAttribute("aria-label")).toBe("Hide session toolbar");
      await fireEvent.click(tab());
      expect(tab().getAttribute("aria-expanded")).toBe("false");
      expect(tab().getAttribute("aria-label")).toBe("Show session toolbar");
    },
  );

  it("a click toggles the state and remembers it; the sidebar menu brings it back", async () => {
    const { app } = await mount();
    await app.tabs.openConnection(MOCK_IDS.localZsh);
    await fireEvent.click(await screen.findByTestId("session-toolbar-tab"));
    expect(app.toolbar.visible).toBe(false);
    expect(store.get(TOOLBAR_VISIBLE_KEY)).toBe("false");
    await fireEvent.click(tab());
    expect(app.toolbar.visible).toBe(true);
    expect(store.get(TOOLBAR_VISIBLE_KEY)).toBe("true");
    await fireEvent.click(tab());

    await fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    await fireEvent.click(await screen.findByRole("menuitem", { name: /Show session toolbar/ }));
    await waitFor(() => expect(tab().getAttribute("aria-expanded")).toBe("true"));
    expect(store.get(TOOLBAR_VISIBLE_KEY)).toBe("true");
  });

  it.each([MOCK_IDS.rdpServer, MOCK_IDS.localSshd])("hidden, %s has the tab and no reachable buttons", async (id) => {
    const { app } = await mount({ vaultKeyHidden: true });
    await app.tabs.openConnection(id);
    await waitFor(() => expect(app.tabs.connectionTabs).toHaveLength(1));
    expect(tab()).toBeTruthy();
    expect(bar().closest("[aria-hidden=true]")).not.toBeNull();
    expect(rowButtons()).toEqual([]);
    expect(screen.queryByRole("button", { name: "Close tab" })).toBeNull();
  });

  it("a web tab always has navigation and the address, the rest only when open", async () => {
    const hidden = await mount({ vaultKeyHidden: true });
    await hidden.app.tabs.openConnection(MOCK_IDS.intranet);
    await waitFor(() => expect(buttons()).toContain("Back"));
    expect(buttons()).toEqual(["Back", "Forward", "Reload"]);
    expect((within(bar()).getByLabelText("Address") as HTMLInputElement).value).toBe("https://intranet.example/");
    expect(bar().closest("[aria-hidden=true]")).toBeNull();
    expect(tab().getAttribute("aria-expanded")).toBe("false");
    await fireEvent.click(tab());
    await waitFor(() => expect(buttons()).toContain("Reconnect"));
    expect(buttons()).toEqual(["Reconnect", "Duplicate tab", "Close tab", "Back", "Forward", "Reload"]);
    expect(within(bar()).getByLabelText("Address")).toBeTruthy();
    await fireEvent.click(tab());
    await waitFor(() => expect(buttons()).toEqual(["Back", "Forward", "Reload"]));
  });

  it("a hidden web tab whose page failed to open has no empty row, and keeps its strip", async () => {
    const { app } = await mount({ vaultKeyHidden: true });
    (app.ipc as { webOpen: Ipc["webOpen"] }).webOpen = async () => {
      throw { code: "failed", message: "no page" };
    };
    await app.tabs.openConnection(MOCK_IDS.intranet);
    await screen.findByRole("alert");
    const dock = screen.getByTestId("session-dock");
    expect(dock.querySelector(".clip")?.classList.contains("collapsed")).toBe(true);
    expect(rowButtons()).toEqual([]);
    expect(dock.querySelector(".reserve")).not.toBeNull();
    expect(tab()).toBeTruthy();
  });

  it("clips only while the row moves, and not for a web tab", async () => {
    vi.stubGlobal("matchMedia", (query: string) => ({ matches: false, media: query, addEventListener() {}, removeEventListener() {} }));
    const { app } = await mount();
    await app.tabs.openConnection(MOCK_IDS.localZsh);
    await waitFor(() => expect(tab()).toBeTruthy());
    const inner = () => screen.getByTestId("session-dock").querySelector(".inner") as HTMLElement;
    expect(inner().classList.contains("moving")).toBe(false);
    // The sidebar-menu path: the store changes, not the tab click.
    app.toolbar.setVisible(false);
    await waitFor(() => expect(inner().classList.contains("moving")).toBe(true));
    const clip = screen.getByTestId("session-dock").querySelector(".clip") as HTMLElement;
    await fireEvent(clip, new Event("transitioncancel", { bubbles: true }));
    await waitFor(() => expect(inner().classList.contains("moving")).toBe(false));
    cleanup();
    store.set(TOOLBAR_VISIBLE_KEY, "true");

    const web = await mount();
    await web.app.tabs.openConnection(MOCK_IDS.intranet);
    await waitFor(() => expect(buttons()).toContain("Back"));
    await fireEvent.click(tab());
    await fireEvent.click(tab());
    expect(inner().classList.contains("moving")).toBe(false);
  });

  it("a hidden web tab's row appears at once when its view publishes, with no slide", async () => {
    vi.stubGlobal("matchMedia", (query: string) => ({ matches: false, media: query, addEventListener() {}, removeEventListener() {} }));
    const { app } = await mount({ vaultKeyHidden: true });
    const id = (await app.tabs.openConnection(MOCK_IDS.intranet)) as string;
    const dock = await screen.findByTestId("session-dock");
    const clip = () => dock.querySelector(".clip") as HTMLElement;
    await waitFor(() => expect(buttons()).toContain("Back"));
    expect(clip().classList.contains("moving")).toBe(false);
    // A reconnect withdraws and publishes again: still no slide.
    await app.tabs.reconnect(id);
    await waitFor(() => expect(buttons()).toContain("Back"));
    expect(clip().classList.contains("moving")).toBe(false);
    // Toggling a web row changes no height: nothing slides there either.
    app.toolbar.setVisible(true);
    expect(clip().classList.contains("moving")).toBe(false);
  });

  it("follows the reduced-motion preference while running", async () => {
    const listeners: Array<() => void> = [];
    const query = { matches: false, media: "", addEventListener: (_: string, fn: () => void) => void listeners.push(fn), removeEventListener() {} };
    vi.stubGlobal("matchMedia", () => query);
    const { app } = await mount();
    await app.tabs.openConnection(MOCK_IDS.localZsh);
    const dock = await screen.findByTestId("session-dock");
    await waitFor(() => expect(dock.classList.contains("animated")).toBe(true));
    query.matches = true;
    listeners.forEach((fn) => fn());
    await waitFor(() => expect(dock.classList.contains("animated")).toBe(false));
  });

  it("the Home tab has no toolbar and no tab", async () => {
    const { app } = await mount();
    app.tabs.openHome();
    await screen.findByTestId("home");
    expect(screen.queryByTestId("session-toolbar-tab")).toBeNull();
    expect(screen.queryByTestId("session-toolbar")).toBeNull();
  });

  it("has no reveal handle and no chevron inside the bar", async () => {
    const { app } = await mount({ vaultKeyHidden: true });
    await app.tabs.openConnection(MOCK_IDS.localZsh);
    await waitFor(() => expect(tab()).toBeTruthy());
    expect(screen.queryByTestId("session-toolbar-reveal")).toBeNull();
    await fireEvent.click(tab());
    expect(screen.queryByTestId("session-toolbar-reveal")).toBeNull();
    expect(buttons()).not.toContain("Hide session toolbar");
    expect(buttons()).not.toContain("Show session toolbar");
  });

  it("keeps the focus where it is on mousedown", async () => {
    const { app } = await mount();
    await app.tabs.openConnection(MOCK_IDS.localSshd);
    await waitFor(() => expect(tab()).toBeTruthy());
    expect(await fireEvent.mouseDown(tab())).toBe(false);
  });

  it("slides by default and not with reduced motion", async () => {
    const { app } = await mount();
    await app.tabs.openConnection(MOCK_IDS.localZsh);
    await waitFor(() => expect(tab()).toBeTruthy());
    expect(screen.getByTestId("session-dock").classList.contains("animated")).toBe(false);
    cleanup();

    vi.stubGlobal("matchMedia", (query: string) => ({ matches: false, media: query, addEventListener() {}, removeEventListener() {} }));
    const free = await mount();
    await free.app.tabs.openConnection(MOCK_IDS.localZsh);
    await waitFor(() => expect(tab()).toBeTruthy());
    expect(screen.getByTestId("session-dock").classList.contains("animated")).toBe(true);
    cleanup();

    vi.stubGlobal("matchMedia", (query: string) => ({ matches: true, media: query, addEventListener() {}, removeEventListener() {} }));
    const reduced = await mount();
    await reduced.app.tabs.openConnection(MOCK_IDS.localZsh);
    await waitFor(() => expect(tab()).toBeTruthy());
    expect(screen.getByTestId("session-dock").classList.contains("animated")).toBe(false);
  });
});

describe("publish lifecycle", () => {
  it("withdraws the model when the tab closes and replaces it on reconnect", async () => {
    const { app, feeds } = await mount();
    const id = (await app.tabs.openConnection(MOCK_IDS.rdpServer)) as string;
    await waitFor(() => expect(app.toolbar.model(id)).not.toBeNull());
    const first = app.toolbar.model(id);
    await app.tabs.reconnect(id);
    await waitFor(() => expect(feeds).toHaveLength(2));
    await waitFor(() => {
      const next = app.toolbar.model(id);
      expect(next).not.toBeNull();
      expect(next).not.toBe(first);
    });
    await app.tabs.close(id);
    await waitFor(() => expect(app.toolbar.model(id)).toBeNull());
  });

  it("disables Ctrl+Alt+Del once the session has closed", async () => {
    const { app, feeds } = await mount();
    await app.tabs.openConnection(MOCK_IDS.rdpServer);
    await waitFor(() => expect(feeds).toHaveLength(1));
    feeds[0]?.({ type: "resize", w: 800, h: 600 });
    const send = (): HTMLButtonElement => within(bar()).getByRole("button", { name: "Send Ctrl+Alt+Del" });
    await waitFor(() => expect(send().disabled).toBe(false));
    feeds[0]?.({ type: "closed", reason: "gone" });
    await waitFor(() => expect(send().disabled).toBe(true));
  });
});
