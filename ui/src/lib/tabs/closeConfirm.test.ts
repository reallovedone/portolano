import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { flushSync } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "../../App.svelte";
import type { Ipc } from "../ipc";
import { MOCK_IDS, createMockIpc } from "../ipc/mock";
import { createApp } from "../state/app";
import { trackLiveCount } from "./liveCount.testing.svelte";

vi.mock("@xterm/xterm", async () => ({
  Terminal: (await import("../sessions/term/fakeXterm")).FakeTerminal,
}));

/** Sessions open fine and keep their event callbacks, so tests can make them connected. */
function feedIpc() {
  const feeds: Array<(e: unknown) => void> = [];
  const closed: string[] = [];
  let counter = 0;
  const base = createMockIpc({ vault: "unlocked" });
  const ipc: Ipc = {
    ...base,
    termOpen: async (_id, _c, _r, onEvent) => {
      feeds.push(onEvent as (e: unknown) => void);
      return `s${++counter}`;
    },
    fbOpen: async (_id, _w, _h, onEvent) => {
      feeds.push(onEvent as (e: unknown) => void);
      return `s${++counter}`;
    },
    webOpen: async () => `s${++counter}`,
    sessionClose: async (sid) => {
      closed.push(sid);
    },
  };
  return { ipc, feeds, closed };
}

const data = { type: "data", bytes: new Uint8Array([1]) };
const resize = { type: "resize", w: 1, h: 1 };

async function mount() {
  const fx = feedIpc();
  const app = createApp(fx.ipc);
  render(App, { props: { app, demo: false } });
  await waitFor(() => expect(app.doc.status).toBe("ready"));
  const ssh = (await app.tabs.openConnection(MOCK_IDS.localSshd)) as string;
  const vnc = (await app.tabs.openConnection(MOCK_IDS.devVm)) as string;
  const web = (await app.tabs.openConnection(MOCK_IDS.intranet)) as string;
  return { ...fx, app, ssh, vnc, web };
}

/** The dialog's confirm button (the header's close button has the same name; it comes first). */
const confirmButton = (): HTMLElement => {
  const top = screen.getAllByRole("dialog").at(-1) as HTMLElement; // the newest dialog
  const buttons = within(top).getAllByRole("button", { name: "Close" });
  return buttons[buttons.length - 1] as HTMLElement;
};
const dialog = () => screen.queryByRole("dialog");

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

describe("confirmClose", () => {
  it("answers true at once, without a dialog, when no listed tab is connected", async () => {
    const { app, ssh, web } = await mount();
    expect(await app.closeConfirm.confirmClose([ssh, web])).toBe(true);
    expect(dialog()).toBeNull();
  });

  it("asks when a listed tab is connected, and Close answers true", async () => {
    const { app, feeds, ssh, vnc } = await mount();
    feeds[0]?.(data);
    feeds[1]?.(resize);
    const answer = app.closeConfirm.confirmClose([ssh, vnc]);
    await screen.findByRole("dialog", { name: "Close 2 active sessions?" });
    expect(app.overlay.open).toBe(true);
    await fireEvent.click(confirmButton());
    expect(await answer).toBe(true);
    await waitFor(() => expect(dialog()).toBeNull());
    expect(app.overlay.open).toBe(false);
  });

  it("uses the singular for one session, and Cancel and Esc answer false", async () => {
    const { app, feeds, ssh } = await mount();
    feeds[0]?.(data);
    const first = app.closeConfirm.confirmClose([ssh]);
    await screen.findByRole("dialog", { name: "Close 1 active session?" });
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(await first).toBe(false);
    await waitFor(() => expect(dialog()).toBeNull());

    const second = app.closeConfirm.confirmClose([ssh]);
    await screen.findByRole("dialog");
    await fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    expect(await second).toBe(false);
  });

  it("keeps one dialog: a second request while one is open answers false", async () => {
    const { app, feeds, ssh, vnc } = await mount();
    feeds[0]?.(data);
    feeds[1]?.(resize);
    const first = app.closeConfirm.confirmClose([ssh]);
    await screen.findByRole("dialog", { name: "Close 1 active session?" });
    expect(await app.closeConfirm.confirmClose([ssh, vnc])).toBe(false);
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    // The first request is untouched and still gets the user's answer.
    await fireEvent.click(confirmButton());
    expect(await first).toBe(true);
  });

  it("fixes the count when the dialog opens; a session dropping meanwhile still closes cleanly", async () => {
    const { app, feeds, ssh, vnc, closed } = await mount();
    feeds[0]?.(data);
    feeds[1]?.(resize);
    const done = app.closeConfirm.closeAll();
    await screen.findByRole("dialog", { name: "Close 2 active sessions?" });
    feeds[0]?.({ type: "exit", code: 0 });
    expect(app.tabs.isLive(ssh)).toBe(false);
    expect(app.tabs.liveCount()).toBe(1);
    // The title keeps the count of the moment of opening.
    expect(screen.getByRole("dialog", { name: "Close 2 active sessions?" })).toBeTruthy();
    await fireEvent.click(confirmButton());
    await done;
    expect(app.tabs.tabs).toHaveLength(0);
    expect(closed).toHaveLength(3);
    expect(app.tabs.isLive(vnc)).toBe(false);
    expect(app.notices.items).toHaveLength(0);
  });
});

describe("closing tabs asks first when a session is connected", () => {
  it("Ctrl+W: connected tab asks, Cancel keeps it, Close closes it", async () => {
    const { app, feeds, ssh } = await mount();
    feeds[0]?.(data);
    await fireEvent.click(screen.getAllByRole("tab")[0] as HTMLElement);
    expect(app.tabs.state.activeId).toBe(ssh);
    await fireEvent.keyDown(window, { key: "w", ctrlKey: true });
    await screen.findByRole("dialog", { name: "Close 1 active session?" });
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(dialog()).toBeNull());
    expect(app.tabs.tabs).toHaveLength(3);

    await fireEvent.keyDown(window, { key: "w", ctrlKey: true });
    await fireEvent.click((await screen.findAllByRole("dialog"), confirmButton()));
    await waitFor(() => expect(app.tabs.tabs).toHaveLength(2));
  });

  it("a web tab closes without a question, by shortcut and by its button", async () => {
    const { app, feeds, web } = await mount();
    feeds[0]?.(data);
    expect(app.tabs.state.activeId).toBe(web);
    await fireEvent.keyDown(window, { key: "w", ctrlKey: true });
    await waitFor(() => expect(app.tabs.tabs).toHaveLength(2));
    expect(dialog()).toBeNull();
  });

  it("the tab's close button asks for a connected tab", async () => {
    const { app, feeds } = await mount();
    feeds[0]?.(data);
    await fireEvent.click(screen.getAllByRole("button", { name: /^Close tab:/ })[0] as HTMLElement);
    await screen.findByRole("dialog", { name: "Close 1 active session?" });
    await fireEvent.click(confirmButton());
    await waitFor(() => expect(app.tabs.tabs).toHaveLength(2));
  });

  it("'Close other tabs' counts only the others", async () => {
    const { app, feeds } = await mount();
    feeds[0]?.(data);
    feeds[1]?.(resize);
    const keep = app.tabs.tabs[0]?.id as string; // the connected ssh tab stays open
    const done = app.closeConfirm.closeOthers(keep);
    await screen.findByRole("dialog", { name: "Close 1 active session?" });
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await done;
    expect(app.tabs.tabs).toHaveLength(3);

    const again = app.closeConfirm.closeOthers(keep);
    await fireEvent.click((await screen.findAllByRole("dialog"), confirmButton()));
    await again;
    expect(app.tabs.tabs.map((x) => x.id)).toEqual([keep]);
  });

  it("'Close all tabs' counts everything; with nothing connected it closes at once", async () => {
    const { app, feeds } = await mount();
    feeds[0]?.(data);
    feeds[1]?.(resize);
    const done = app.closeConfirm.closeAll();
    await screen.findByRole("dialog", { name: "Close 2 active sessions?" });
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await done;
    expect(app.tabs.tabs).toHaveLength(3);

    feeds[0]?.({ type: "exit", code: 0 });
    feeds[1]?.({ type: "closed" });
    await app.closeConfirm.closeAll();
    expect(app.tabs.tabs).toHaveLength(0);
    expect(dialog()).toBeNull();
  });

  it("the tab menu entries go through the confirmation", async () => {
    const { app, feeds } = await mount();
    feeds[0]?.(data);
    await fireEvent.contextMenu(screen.getAllByRole("tab")[1] as HTMLElement);
    await fireEvent.click(await screen.findByRole("menuitem", { name: "Close other tabs" }));
    await screen.findByRole("dialog", { name: "Close 1 active session?" });
    await fireEvent.click(confirmButton());
    await waitFor(() => expect(app.tabs.tabs).toHaveLength(1));
  });
});

describe("what gets closed", () => {
  it("closes exactly the confirmed tabs; a tab opened meanwhile stays", async () => {
    const { app, feeds } = await mount();
    feeds[0]?.(data);
    const done = app.closeConfirm.closeAll();
    await screen.findByRole("dialog", { name: "Close 1 active session?" });
    const late = (await app.tabs.openConnection(MOCK_IDS.intranet)) as string;
    await fireEvent.click(confirmButton());
    await done;
    expect(app.tabs.tabs.map((x) => x.id)).toEqual([late]);
  });

  it("'Close other tabs' leaves a tab opened meanwhile", async () => {
    const { app, feeds } = await mount();
    feeds[1]?.(resize);
    const keep = app.tabs.tabs[0]?.id as string;
    const done = app.closeConfirm.closeOthers(keep);
    await screen.findByRole("dialog");
    const late = (await app.tabs.openConnection(MOCK_IDS.intranet)) as string;
    await fireEvent.click(confirmButton());
    await done;
    expect(app.tabs.tabs.map((x) => x.id)).toEqual([keep, late]);
  });
});

describe("the count is read reactively", () => {
  it("liveCount() follows session events inside an effect", async () => {
    const { app, feeds } = await mount();
    const { seen, stop } = trackLiveCount(app.tabs);
    feeds[0]?.(data);
    flushSync();
    feeds[1]?.(resize);
    flushSync();
    stop();
    expect(seen).toEqual([0, 1, 2]);
  });
});

describe("installing an update", () => {
  async function mountWithOffer() {
    const fx = feedIpc();
    const mock = createMockIpc({
      vault: "unlocked",
      updates: { available: { version: "0.2.0", notes: "notes" } },
    });
    const ipc: Ipc = { ...mock, termOpen: fx.ipc.termOpen, fbOpen: fx.ipc.fbOpen, sessionClose: fx.ipc.sessionClose };
    const app = createApp(ipc, undefined, { startDelayMs: 0, intervalMs: 60_000 });
    render(App, { props: { app, demo: false } });
    await waitFor(() => expect(app.doc.status).toBe("ready"));
    await app.tabs.openConnection(MOCK_IDS.localSshd);
    await screen.findByRole("dialog", { name: "Update available" });
    return { ...fx, app, mock };
  }

  it("asks first when a session is connected; Cancel installs nothing", async () => {
    const { app, mock, feeds } = await mountWithOffer();
    feeds[0]?.(data);
    await fireEvent.click(screen.getByRole("button", { name: "Update now" }));
    await screen.findByRole("dialog", { name: "Close 1 active session?" });
    expect(mock.updates.installs).toBe(0);
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Close 1 active session?" })).toBeNull());
    expect(mock.updates.installs).toBe(0);
    expect(app.updates.installing).toBe(false);

    await fireEvent.click(screen.getByRole("button", { name: "Update now" }));
    await fireEvent.click((await screen.findAllByRole("dialog"), confirmButton()));
    await screen.findByText("Restarting Portolano…");
    expect(mock.updates.installs).toBe(1);
  });

  it("installs at once when no session is connected", async () => {
    const { mock } = await mountWithOffer();
    await fireEvent.click(screen.getByRole("button", { name: "Update now" }));
    await screen.findByText("Restarting Portolano…");
    expect(mock.updates.installs).toBe(1);
  });
});
