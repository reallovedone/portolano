import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "../../App.svelte";
import type { Ipc } from "../ipc";
import { MOCK_IDS, createMockIpc } from "../ipc/mock";
import { createApp } from "./app";

vi.mock("@xterm/xterm", async () => ({
  Terminal: (await import("../sessions/term/fakeXterm")).FakeTerminal,
}));

/** Sessions open fine and keep their event callbacks, so tests can make them connected. */
function feedIpc() {
  const feeds: Array<(e: unknown) => void> = [];
  let counter = 0;
  const base = createMockIpc({ vault: "unlocked" });
  const ipc = {
    ...base,
    simulate: base.simulate,
    quitReplies: base.quitReplies,
    termOpen: async (_id, _c, _r, onEvent) => {
      feeds.push(onEvent as (e: unknown) => void);
      return `s${++counter}`;
    },
    sessionClose: async () => undefined,
  } as Ipc & Pick<typeof base, "simulate" | "quitReplies">;
  return { ipc, feeds };
}

const data = { type: "data", bytes: new Uint8Array([1]) };

async function mount() {
  const fx = feedIpc();
  const app = createApp(fx.ipc);
  render(App, { props: { app, demo: false } });
  await waitFor(() => expect(app.doc.status).toBe("ready"));
  return { ...fx, app };
}

const confirmButton = (): HTMLElement => {
  const top = screen.getAllByRole("dialog").at(-1) as HTMLElement;
  const buttons = within(top).getAllByRole("button", { name: "Close" });
  return buttons[buttons.length - 1] as HTMLElement;
};

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

describe("quit-requested", () => {
  it("replies quit at once when no session is connected, without a dialog", async () => {
    const { ipc, app } = await mount();
    await app.tabs.openConnection(MOCK_IDS.intranet); // a web tab never counts
    ipc.simulate.quitRequested();
    await waitFor(() => expect(ipc.quitReplies).toEqual(["quit"]));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("replies asking first, then Close answers quit", async () => {
    const { ipc, app, feeds } = await mount();
    await app.tabs.openConnection(MOCK_IDS.localSshd);
    feeds[0]?.(data);
    ipc.simulate.quitRequested();
    await screen.findByRole("dialog", { name: "Close 1 active session?" });
    // "asking" went out before the dialog was shown; nothing else yet.
    expect(ipc.quitReplies).toEqual(["asking"]);
    await fireEvent.click(confirmButton());
    await waitFor(() => expect(ipc.quitReplies).toEqual(["asking", "quit"]));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("replies cancel when the user cancels, and the app stays usable", async () => {
    const { ipc, app, feeds } = await mount();
    await app.tabs.openConnection(MOCK_IDS.localSshd);
    feeds[0]?.(data);
    ipc.simulate.quitRequested();
    await screen.findByRole("dialog", { name: "Close 1 active session?" });
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(ipc.quitReplies).toEqual(["asking", "cancel"]));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(app.tabs.tabs).toHaveLength(1);
  });

  it("while a tab-close confirmation is open: one dialog only, the quit is cancelled", async () => {
    const { ipc, app, feeds } = await mount();
    const tab = (await app.tabs.openConnection(MOCK_IDS.localSshd)) as string;
    feeds[0]?.(data);
    const closing = app.closeConfirm.closeTab(tab);
    await screen.findByRole("dialog", { name: "Close 1 active session?" });

    ipc.simulate.quitRequested();
    await waitFor(() => expect(ipc.quitReplies).toEqual(["asking", "cancel"]));
    expect(screen.getAllByRole("dialog")).toHaveLength(1);

    // The open dialog still gets the user's answer, and it is about the tab, not the quit.
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await closing;
    expect(app.tabs.tabs).toHaveLength(1);
    expect(ipc.quitReplies).toEqual(["asking", "cancel"]);
  });

  it("quit pressed twice while the quit dialog is open: one dialog, then Close sends quit", async () => {
    const { ipc, app, feeds } = await mount();
    await app.tabs.openConnection(MOCK_IDS.localSshd);
    feeds[0]?.(data);
    ipc.simulate.quitRequested();
    await screen.findByRole("dialog", { name: "Close 1 active session?" });
    ipc.simulate.quitRequested();
    await waitFor(() => expect(ipc.quitReplies).toEqual(["asking", "asking"]));
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    await fireEvent.click(confirmButton());
    await waitFor(() => expect(ipc.quitReplies).toEqual(["asking", "asking", "quit"]));
  });

  it("stops listening when the shell goes away", async () => {
    const { ipc, app, feeds } = await mount();
    await app.tabs.openConnection(MOCK_IDS.localSshd);
    feeds[0]?.(data);
    // Proof that the request would be answered: a mounted shell replies.
    ipc.simulate.quitRequested();
    await waitFor(() => expect(ipc.quitReplies).toEqual(["asking"]));
    cleanup();
    ipc.simulate.quitRequested();
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(ipc.quitReplies).toEqual(["asking"]);
  });
});
