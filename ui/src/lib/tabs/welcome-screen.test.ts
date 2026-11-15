// Welcome screen, sidebar header and tab strip as wired in the real shell (mock backend).

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "../../App.svelte";
import { createMockIpc } from "../ipc/mock";
import { createApp, type App as AppState } from "../state/app";

vi.mock("@xterm/xterm", async () => ({
  Terminal: (await import("../sessions/term/fakeXterm")).FakeTerminal,
}));

async function mountApp(seed = true): Promise<AppState> {
  const base = createMockIpc({ vault: "unlocked", seed });
  const ipc = { ...base, termOpen: async () => "sess-1", sessionClose: async () => undefined };
  const app = createApp(ipc);
  render(App, { props: { app, demo: false } });
  await waitFor(() => expect(app.doc.status).toBe("ready"));
  return app;
}

beforeEach(() => {
  const data = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => data.get(k) ?? null,
    setItem: (k: string, v: string) => {
      data.set(k, v);
    },
  });
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("welcome screen", () => {
  it("shows the hero, one tile per kind with its default port, and the shortcut hints", async () => {
    await mountApp();
    const welcome = screen.getByTestId("welcome");
    expect(within(welcome).getByRole("heading", { level: 1, name: "Chart a course" })).toBeTruthy();
    const tiles = within(within(welcome).getByRole("group", { name: "Start a new connection" })).getAllByRole("button");
    expect(tiles.map((b) => b.textContent?.replace(/\s+/g, " ").trim())).toEqual([
      "SSH 22",
      "RDP 3389",
      "VNC 5900",
      "Web 443",
      "Local shell local",
    ]);
    expect(welcome.textContent).toContain("new connection");
    expect(welcome.querySelectorAll("kbd").length).toBeGreaterThanOrEqual(4);
  });

  it("opens the new-connection flow from a tile, in the selected folder", async () => {
    const app = await mountApp();
    const tile = within(screen.getByTestId("welcome")).getByRole("button", { name: /^RDP/ });
    await fireEvent.click(tile);
    expect(app.ui.dialog?.type).toBe("connection");
    expect(app.ui.dialog && "id" in app.ui.dialog ? app.ui.dialog.id : "x").toBeNull();
  });

  it("preselects the kind of the tile in the new-connection dialog", async () => {
    const app = await mountApp();
    const welcome = screen.getByTestId("welcome");
    await fireEvent.click(within(welcome).getByRole("button", { name: /^VNC/ }));
    expect(app.ui.dialog).toMatchObject({ type: "connection", id: null, kind: "vnc" });
    const dialog = await screen.findByRole("dialog");
    const checked = within(dialog)
      .getAllByRole("radio", { name: /^(Local shell|SSH|RDP|VNC|Web)$/ })
      .find((r) => (r as HTMLInputElement).checked) as HTMLInputElement;
    expect(checked.value).toBe("vnc");
  });

  it("disappears once a tab is open and returns when the last one closes", async () => {
    const app = await mountApp();
    const shell = screen.getAllByRole("treeitem").find((r) => r.getAttribute("aria-label")?.startsWith("Local shell"));
    if (!shell) throw new Error("no Local shell row");
    await fireEvent.dblClick(shell);
    await waitFor(() => expect(app.tabs.tabs).toHaveLength(1));
    expect(screen.queryByTestId("welcome")).toBeNull();
    await fireEvent.click(screen.getByRole("button", { name: /^Close tab: / }));
    await waitFor(() => expect(screen.getByTestId("welcome")).toBeTruthy());
  });
});

describe("sidebar header and tab strip", () => {
  it("has a toolbar whose buttons start a connection, a folder and the more menu", async () => {
    const app = await mountApp();
    const toolbar = screen.getByRole("toolbar", { name: "Connection actions" });
    await fireEvent.click(within(toolbar).getByRole("button", { name: "New connection" }));
    expect(app.ui.dialog?.type).toBe("connection");
    app.ui.closeDialog();

    await fireEvent.click(within(toolbar).getByRole("button", { name: "New folder" }));
    expect(await screen.findByRole("dialog", { name: "New folder" })).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    await fireEvent.click(within(toolbar).getByRole("button", { name: "More actions" }));
    const menu = screen.getByRole("menu");
    await fireEvent.click(within(menu).getByRole("menuitem", { name: /Collapse all/ }));
    expect(app.doc.expanded.size).toBe(0);
  });

  it("marks the title-bar strips as window drag handles", async () => {
    await mountApp();
    expect(screen.getByTestId("sidebar-header").hasAttribute("data-tauri-drag-region")).toBe(true);
    expect(screen.getByTestId("tabstrip").hasAttribute("data-tauri-drag-region")).toBe(true);
  });

  it("offers a plus button at the end of the strip", async () => {
    const app = await mountApp();
    const strip = screen.getByTestId("tabstrip");
    await fireEvent.click(within(strip).getByRole("button", { name: "New connection" }));
    expect(app.ui.dialog?.type).toBe("connection");
  });
});

describe("empty tree", () => {
  it("shows an empty state whose action opens the new-connection dialog", async () => {
    const app = await mountApp(false);
    const tree = screen.getByRole("tree");
    expect(within(tree).queryAllByRole("treeitem")).toHaveLength(0);
    await fireEvent.click(screen.getByRole("heading", { name: "No connections yet" }).closest(".empty")?.querySelector("button") as HTMLElement);
    expect(app.ui.dialog?.type).toBe("connection");
  });
});
