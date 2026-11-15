import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Bounds, Ipc, WebNavAction } from "../ipc";
import { createMockIpc, MOCK_IDS } from "../ipc/mock";
import { createApp } from "../state/app";
import { createTestSession } from "./fb/test-context.svelte";
import WebHarness from "./web/WebHarness.svelte";
import type { UrlChange, UrlSubscribe } from "./web/urlEvents";

let rect = { left: 260, top: 78, width: 900, height: 600 };
let resizeCallbacks: (() => void)[] = [];

beforeEach(() => {
  rect = { left: 260, top: 78, width: 900, height: 600 };
  resizeCallbacks = [];
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    const r = this.dataset.testid === "web-page" ? rect : { left: 0, top: 0, width: 0, height: 0 };
    return { ...r, x: r.left, y: r.top, right: r.left + r.width, bottom: r.top + r.height } as DOMRect;
  });
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(cb: () => void) {
        resizeCallbacks.push(cb);
      }
      observe(): void {}
      unobserve(): void {}
      disconnect(): void {}
    },
  );
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

interface Calls {
  bounds: { bounds: Bounds; visible: boolean }[];
  nav: WebNavAction[];
}

function recordingIpc(): { ipc: Ipc; calls: Calls } {
  const base = createMockIpc({ web: true });
  const calls: Calls = { bounds: [], nav: [] };
  const ipc: Ipc = {
    ...base,
    webSetBounds: async (_id, bounds, visible) => void calls.bounds.push({ bounds, visible }),
    webNavigate: async (_id, action) => void calls.nav.push(action),
  };
  return { ipc, calls };
}

async function mount(ipc: Ipc, options: { standIn?: boolean; urlEvents?: UrlSubscribe } = {}) {
  const session = createTestSession("web-sess");
  const app = createApp(ipc);
  // The shell loads the document before any tab opens.
  await app.doc.load();
  // Not spread: `visible` and `overlayOpen` are reactive getters.
  const context = Object.assign(session.context, {
    connectionId: MOCK_IDS.publicSite,
    kind: "web" as const,
  });
  render(WebHarness, { props: { app, context, ...options } });
  return { ...session, app };
}

const lastBounds = (calls: Calls) => calls.bounds.at(-1);
const frame = (): Promise<void> => new Promise((resolve) => requestAnimationFrame(() => resolve()));

describe("WebView", () => {
  it("syncs the placeholder rect on mount", async () => {
    const { ipc, calls } = recordingIpc();
    await mount(ipc);
    await waitFor(() => expect(calls.bounds.length).toBeGreaterThan(0));
    expect(lastBounds(calls)).toEqual({
      bounds: { x: 260, y: 78, width: 900, height: 600 },
      visible: true,
    });
  });

  it("syncs again when the placeholder is resized or the window resizes", async () => {
    const { ipc, calls } = recordingIpc();
    await mount(ipc);
    await waitFor(() => expect(calls.bounds).toHaveLength(1));
    rect = { left: 300, top: 78, width: 860, height: 600 };
    resizeCallbacks.forEach((cb) => cb());
    await waitFor(() => expect(calls.bounds).toHaveLength(2));
    expect(lastBounds(calls)?.bounds).toEqual({ x: 300, y: 78, width: 860, height: 600 });
    rect = { left: 300, top: 78, width: 860, height: 500 };
    window.dispatchEvent(new Event("resize"));
    await waitFor(() => expect(calls.bounds).toHaveLength(3));
    expect(lastBounds(calls)?.bounds.height).toBe(500);
  });

  it("coalesces a burst of resizes into one call", async () => {
    const { ipc, calls } = recordingIpc();
    await mount(ipc);
    await waitFor(() => expect(calls.bounds).toHaveLength(1));
    rect = { left: 300, top: 78, width: 800, height: 600 };
    for (let i = 0; i < 20; i++) window.dispatchEvent(new Event("resize"));
    await frame();
    await frame();
    await waitFor(() => expect(calls.bounds).toHaveLength(2));
    await frame();
    expect(calls.bounds).toHaveLength(2);
  });

  it("hides when the tab is deactivated or an overlay opens, and shows again", async () => {
    const { ipc, calls } = recordingIpc();
    const session = await mount(ipc);
    await waitFor(() => expect(calls.bounds).toHaveLength(1));

    session.state.overlayOpen = true;
    await waitFor(() => expect(lastBounds(calls)?.visible).toBe(false));
    session.state.overlayOpen = false;
    await waitFor(() => expect(lastBounds(calls)?.visible).toBe(true));

    session.state.visible = false;
    await waitFor(() => expect(lastBounds(calls)?.visible).toBe(false));
    session.state.visible = true;
    await waitFor(() => expect(lastBounds(calls)?.visible).toBe(true));
  });

  it("keeps the last rect, hidden, when the inactive tab has no layout", async () => {
    const { ipc, calls } = recordingIpc();
    const session = await mount(ipc);
    await waitFor(() => expect(calls.bounds).toHaveLength(1));
    rect = { left: 0, top: 0, width: 0, height: 0 };
    session.state.visible = false;
    await waitFor(() => expect(lastBounds(calls)?.visible).toBe(false));
    expect(lastBounds(calls)?.bounds).toEqual({ x: 260, y: 78, width: 900, height: 600 });
  });

  it("toolbar buttons send back, forward and reload", async () => {
    const { ipc, calls } = recordingIpc();
    await mount(ipc);
    await fireEvent.click(screen.getByRole("button", { name: "Back" }));
    await fireEvent.click(screen.getByRole("button", { name: "Forward" }));
    await fireEvent.click(screen.getByRole("button", { name: "Reload" }));
    expect(calls.nav).toEqual([{ type: "back" }, { type: "forward" }, { type: "reload" }]);
  });

  it("shows the connection's URL and navigates on Enter", async () => {
    const { ipc, calls } = recordingIpc();
    await mount(ipc);
    const field = (await screen.findByLabelText("Address")) as HTMLInputElement;
    await waitFor(() => expect(field.value).toBe("https://www.example/admin"));
    await fireEvent.input(field, { target: { value: "example.org/docs" } });
    await fireEvent.keyDown(field, { key: "Enter" });
    expect(calls.nav).toEqual([{ type: "url", url: "https://example.org/docs" }]);
    expect(field.value).toBe("https://example.org/docs");
  });

  it("rejects an invalid address without calling the backend", async () => {
    const { ipc, calls } = recordingIpc();
    await mount(ipc);
    const field = (await screen.findByLabelText("Address")) as HTMLInputElement;
    await fireEvent.input(field, { target: { value: "file:///etc/passwd" } });
    await fireEvent.keyDown(field, { key: "Enter" });
    expect(await screen.findByText(/starting with http/)).toBeTruthy();
    expect(calls.nav).toEqual([]);
    await fireEvent.input(field, { target: { value: "https://ok.example" } });
    expect(screen.queryByText(/starting with http/)).toBeNull();
  });

  it("shows a backend navigation failure next to the field", async () => {
    const { ipc } = recordingIpc();
    ipc.webNavigate = async () => {
      throw Object.assign(new Error("boom"), { code: "internal" });
    };
    await mount(ipc);
    await fireEvent.click(screen.getByRole("button", { name: "Reload" }));
    expect(await screen.findByText(/could not be opened/)).toBeTruthy();
  });

  it("follows live URL changes unless the user is typing", async () => {
    const { ipc } = recordingIpc();
    let push: (change: UrlChange) => void = () => undefined;
    const urlEvents: UrlSubscribe = (handler) => {
      push = handler;
      return () => undefined;
    };
    await mount(ipc, { urlEvents });
    const field = (await screen.findByLabelText("Address")) as HTMLInputElement;
    push({ sessionId: "other", url: "https://nope.example/", phase: "finished" });
    expect(field.value).not.toBe("https://nope.example/");
    push({ sessionId: "web-sess", url: "https://next.example/", phase: "started" });
    await waitFor(() => expect(field.value).toBe("https://next.example/"));
    expect(await screen.findByRole("status", { name: "Loading the page…" })).toBeTruthy();
    await fireEvent.focus(field);
    push({ sessionId: "web-sess", url: "https://later.example/", phase: "finished" });
    expect(field.value).toBe("https://next.example/");
    await waitFor(() => expect(screen.queryByRole("status", { name: "Loading the page…" })).toBeNull());
  });

  it("reports a vanished session through the session context", async () => {
    const { ipc } = recordingIpc();
    ipc.webSetBounds = async () => {
      throw Object.assign(new Error("gone"), { code: "not_found" });
    };
    const session = await mount(ipc);
    await waitFor(() => expect(session.fail).toHaveBeenCalled());
    expect(session.fail.mock.calls[0]?.[0].code).toBe("not_found");
  });

  it("shows a labelled stand-in only when there is no real webview", async () => {
    const { ipc } = recordingIpc();
    await mount(ipc, { standIn: true });
    expect(await screen.findByText("Web page area")).toBeTruthy();
    cleanup();
    await mount(ipc, { standIn: false });
    expect(screen.queryByText("Web page area")).toBeNull();
  });

  it("stops syncing after it is destroyed", async () => {
    const { ipc, calls } = recordingIpc();
    await mount(ipc);
    await waitFor(() => expect(calls.bounds).toHaveLength(1));
    cleanup();
    window.dispatchEvent(new Event("resize"));
    await frame();
    await frame();
    expect(calls.bounds).toHaveLength(1);
  });
});
