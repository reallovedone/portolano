// The tab context menu and the Reconnect shortcut, as wired in the real shell (mock backend).

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "../../App.svelte";
import type { Ipc } from "../ipc";
import { detectMac } from "../shortcuts";
import { MOCK_IDS, createMockIpc } from "../ipc/mock";
import { createApp } from "../state/app";

vi.mock("@xterm/xterm", async () => ({
  Terminal: (await import("../sessions/term/fakeXterm")).FakeTerminal,
}));

async function mount() {
  const closed: string[] = [];
  const opened: string[] = [];
  let counter = 0;
  const base = createMockIpc({ vault: "unlocked" });
  const ipc: Ipc = {
    ...base,
    termOpen: async () => {
      opened.push("term");
      return `s${++counter}`;
    },
    sessionClose: async (sid) => {
      closed.push(sid);
    },
  };
  const app = createApp(ipc);
  render(App, { props: { app, demo: false } });
  await waitFor(() => expect(app.doc.status).toBe("ready"));
  await app.tabs.openConnection(MOCK_IDS.localSshd);
  return { app, closed, opened };
}

beforeEach(() => {
  const store = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => {
      store.set(k, v);
    },
  });
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("tab menu", () => {
  it("lists Reconnect for a connection tab, right before Close tab", async () => {
    await mount();
    await fireEvent.contextMenu(screen.getByRole("tab"));
    const menu = await screen.findByRole("menu");
    const labels = within(menu)
      .getAllByRole("menuitem")
      .map((i) => i.textContent?.replace(/\s+/g, " ").trim() ?? "");
    const reconnect = labels.findIndex((l) => l.startsWith("Reconnect"));
    const close = labels.findIndex((l) => l.startsWith("Close tab"));
    expect(reconnect).toBeGreaterThanOrEqual(0);
    expect(reconnect + 1).toBe(close);
    expect(labels[reconnect]).toMatch(/(⌘⇧R|Ctrl\+Shift\+R)$/);
  });

  it("reconnects the tab when the item is chosen, even with a live session", async () => {
    const { app, closed } = await mount();
    const oldSession = app.tabs.activeConnection?.sessionId;
    await fireEvent.contextMenu(screen.getByRole("tab"));
    await fireEvent.click(await screen.findByRole("menuitem", { name: /Reconnect/ }));
    await waitFor(() => expect(app.tabs.activeTab).toMatchObject({ status: "open", attempt: 1 }));
    expect(closed).toEqual([oldSession]);
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

describe("Reconnect shortcut", () => {
  it("reconnects the active tab", async () => {
    const { app, closed } = await mount();
    const oldSession = app.tabs.activeConnection?.sessionId;
    const mac = detectMac();
    await fireEvent.keyDown(window, { key: "R", code: "KeyR", shiftKey: true, metaKey: mac, ctrlKey: !mac });
    await waitFor(() => expect(app.tabs.activeTab).toMatchObject({ status: "open", attempt: 1 }));
    expect(closed).toEqual([oldSession]);
  });

  it("does nothing without tabs", async () => {
    const { app, closed, opened } = await mount();
    await app.tabs.closeAll();
    const closedBefore = closed.length;
    const openedBefore = opened.length;
    const mac = detectMac();
    await fireEvent.keyDown(window, { key: "R", code: "KeyR", shiftKey: true, metaKey: mac, ctrlKey: !mac });
    expect(app.tabs.tabs).toHaveLength(0);
    expect(closed).toHaveLength(closedBefore);
    expect(opened).toHaveLength(openedBefore);
    expect(app.notices.items).toEqual([]);
  });
});

describe("Home tab", () => {
  const mac = detectMac();
  const homeKeys = { key: "H", code: "KeyH", shiftKey: true, metaKey: mac, ctrlKey: !mac };

  it("opens from the house button in the strip and activates the existing one the second time", async () => {
    const { app } = await mount();
    const strip = screen.getByTestId("tabstrip");
    await fireEvent.click(within(strip).getByRole("button", { name: "Open Home" }));
    expect(app.tabs.tabs.map((t) => t.type)).toEqual(["connection", "home"]);
    expect(app.tabs.activeTab?.type).toBe("home");
    await fireEvent.click(within(strip).getAllByRole("tab")[0] as HTMLElement);
    await fireEvent.click(within(strip).getByRole("button", { name: "Open Home" }));
    expect(app.tabs.tabs).toHaveLength(2);
    expect(app.tabs.activeTab?.type).toBe("home");
  });

  it("opens with the shortcut and is titled Home", async () => {
    const { app } = await mount();
    await fireEvent.keyDown(window, homeKeys);
    expect(app.tabs.activeTab?.type).toBe("home");
    const tab = screen.getByRole("tab", { name: /Home/ });
    expect(tab.querySelector("svg")).toBeTruthy();
  });

  it("has no Reconnect in its menu, and reconnect does nothing to it", async () => {
    const { app, closed, opened } = await mount();
    await fireEvent.keyDown(window, homeKeys);
    const id = app.tabs.activeTab?.id ?? "";
    await fireEvent.contextMenu(screen.getByRole("tab", { name: /Home/ }));
    const menu = await screen.findByRole("menu");
    const labels = within(menu).getAllByRole("menuitem").map((i) => i.textContent?.replace(/\s+/g, " ").trim() ?? "");
    expect(labels.some((l) => l.startsWith("Reconnect"))).toBe(false);
    expect(labels.some((l) => l.startsWith("Close tab"))).toBe(true);
    await fireEvent.keyDown(window, { key: "Escape" });
    const before = { closed: closed.length, opened: opened.length };
    await app.tabs.reconnect(id);
    await app.tabs.retry(id);
    expect(app.tabs.tabs.find((t) => t.id === id)).toEqual({ id, type: "home" });
    expect({ closed: closed.length, opened: opened.length }).toEqual(before);
  });

  it("is not counted as a live session and closes without confirmation", async () => {
    const { app } = await mount();
    await app.tabs.closeAll();
    await fireEvent.keyDown(window, homeKeys);
    expect(app.tabs.liveCount()).toBe(0);
    await fireEvent.keyDown(window, { key: "W", code: "KeyW", metaKey: mac, ctrlKey: !mac });
    await waitFor(() => expect(app.tabs.tabs).toHaveLength(0));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("is not counted in the status bar sessions", async () => {
    await mount();
    expect(screen.getByTestId("statusbar").textContent).toMatch(/1 open/);
    await fireEvent.keyDown(window, homeKeys);
    expect(screen.getByTestId("statusbar").textContent).toMatch(/1 open/);
  });

  it("shows the welcome content, wired like the welcome screen", async () => {
    const { app } = await mount();
    await fireEvent.keyDown(window, homeKeys);
    const home = screen.getByTestId("home");
    expect(within(home).getByRole("heading", { level: 1, name: "Chart a course" })).toBeTruthy();
    const tiles = within(within(home).getByRole("group", { name: "Start a new connection" })).getAllByRole("button");
    expect(tiles).toHaveLength(5);
    await fireEvent.click(within(home).getByRole("button", { name: /^VNC/ }));
    expect(app.ui.dialog).toMatchObject({ type: "connection", id: null, kind: "vnc" });
    expect(home.textContent).toContain("new connection");
    expect(home.textContent).toContain("search");
  });
});
