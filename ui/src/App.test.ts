import { cleanup, createEvent, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "./App.svelte";
import { IpcError, type Ipc, type VaultStatus } from "./lib/ipc";
import { MOCK_IDS, MOCK_MASTER_PASSWORD, createMockIpc } from "./lib/ipc/mock";
import { createApp, type App as AppState } from "./lib/state/app";
import { pickTreeValue, treeSelectOptions } from "./lib/ui/treeSelectTesting";

// The real xterm.js does not run in jsdom (WP5).
vi.mock("@xterm/xterm", async () => ({
  Terminal: (await import("./lib/sessions/term/fakeXterm")).FakeTerminal,
}));

/** Mock whose session commands succeed. */
function withSessions(base: Ipc): Ipc {
  let n = 0;
  return {
    ...base,
    termOpen: async () => `sess-${++n}`,
    fbOpen: async () => `sess-${++n}`,
    webOpen: async () => `sess-${++n}`,
    sessionClose: async () => undefined,
  };
}

async function mountApp(
  options: { vault?: VaultStatus; sessions?: boolean; seed?: boolean } = {},
): Promise<{ app: AppState; ipc: Ipc }> {
  const base = createMockIpc({ vault: options.vault ?? "unlocked", seed: options.seed });
  const ipc = options.sessions ? withSessions(base) : base;
  const app = createApp(ipc);
  render(App, { props: { app, demo: false } });
  await waitFor(() => expect(app.doc.status).toBe("ready"));
  await waitFor(() => expect(app.vault.status).not.toBe("unknown"));
  return { app, ipc };
}

const row = (name: string): HTMLElement => {
  const el = screen.getAllByRole("treeitem").find((r) => r.querySelector(".name")?.textContent === name);
  if (!el) throw new Error(`no tree row named ${name}`);
  return el;
};
const rowNames = (): string[] =>
  screen.queryAllByRole("treeitem").map((r) => r.querySelector(".name")?.textContent ?? "");

async function expand(name: string): Promise<void> {
  const target = row(name);
  target.focus();
  await fireEvent.keyDown(target, { key: "ArrowRight" });
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
  vi.useRealTimers();
});

describe("connection tree", () => {
  it("shows folders first, collapsed, and expands with the keyboard", async () => {
    await mountApp();
    expect(rowNames()).toEqual(["Lab", "Production", "Local shell"]);
    expect(row("Lab").getAttribute("aria-expanded")).toBe("false");

    await expand("Lab");
    expect(rowNames()).toEqual(["Lab", "Linux", "Windows", "Local sshd", "Production", "Local shell"]);
    expect(row("Linux").getAttribute("aria-level")).toBe("2");

    await fireEvent.keyDown(row("Lab"), { key: "ArrowLeft" });
    expect(rowNames()).toEqual(["Lab", "Production", "Local shell"]);
  });

  it("moves focus with arrows, Home and End and goes to the parent with ArrowLeft", async () => {
    await mountApp();
    await expand("Lab");
    const lab = row("Lab");
    lab.focus();
    await fireEvent.keyDown(lab, { key: "ArrowDown" });
    await waitFor(() => expect(document.activeElement).toBe(row("Linux")));
    await fireEvent.keyDown(row("Linux"), { key: "ArrowLeft" });
    await waitFor(() => expect(document.activeElement).toBe(row("Lab")));
    await fireEvent.keyDown(row("Lab"), { key: "End" });
    await waitFor(() => expect(document.activeElement).toBe(row("Local shell")));
    await fireEvent.keyDown(row("Local shell"), { key: "Home" });
    await waitFor(() => expect(document.activeElement).toBe(row("Lab")));
  });

  it("remembers expansion in local storage", async () => {
    const { app } = await mountApp();
    await expand("Lab");
    expect(app.doc.isExpanded(MOCK_IDS.lab)).toBe(true);
    expect(localStorage.getItem("portolano.tree.expanded")).toContain(MOCK_IDS.lab);
  });

  it("filters by name, host and tag and expands the path to matches", async () => {
    await mountApp();
    const search = screen.getByRole("searchbox", { name: "Filter connections" });
    await fireEvent.input(search, { target: { value: "ci" } }); // tag of Build box
    expect(rowNames()).toEqual(["Lab", "Linux", "Build box"]);

    await fireEvent.input(search, { target: { value: "fs01" } }); // host of File server
    expect(rowNames()).toEqual(["Lab", "Windows", "File server"]);

    await fireEvent.input(search, { target: { value: "zzz" } });
    expect(screen.getByText("No connection matches the filter.")).toBeTruthy();

    await fireEvent.keyDown(search, { key: "Escape" });
    expect(rowNames()).toEqual(["Lab", "Production", "Local shell"]);
  });

  it("focuses the search box with Mod+K", async () => {
    await mountApp();
    const search = screen.getByRole("searchbox", { name: "Filter connections" });
    await fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    await waitFor(() => expect(document.activeElement).toBe(search));
  });

  it("renames with F2 and shows the error when the backend refuses", async () => {
    const { app } = await mountApp();
    const target = row("Local shell");
    target.focus();
    await fireEvent.keyDown(target, { key: "F2" });
    const dialog = screen.getByRole("dialog", { name: "Rename" });
    const input = within(dialog).getByLabelText("New name") as HTMLInputElement;
    expect(input.value).toBe("Local shell");

    await fireEvent.input(input, { target: { value: "   " } });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(within(dialog).getByRole("alert").textContent).toContain("A name is required");

    await fireEvent.input(input, { target: { value: "My shell" } });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(app.doc.node(MOCK_IDS.localZsh)?.name).toBe("My shell");
    expect(rowNames()).toContain("My shell");
  });

  it("deletes a folder with its subtree after confirmation, closing its tabs", async () => {
    const { app } = await mountApp({ sessions: true });
    await expand("Lab");
    await fireEvent.dblClick(row("Local sshd"));
    await waitFor(() => expect(app.tabs.tabs).toHaveLength(1));

    const lab = row("Lab");
    lab.focus();
    await fireEvent.keyDown(lab, { key: "Delete" });
    const dialog = screen.getByRole("dialog", { name: "Delete folder" });
    expect(dialog.textContent).toContain("“Lab”");
    expect(dialog.textContent).toContain("7 items"); // everything below Lab in the seed

    await fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(rowNames()).toEqual(["Production", "Local shell"]);
    expect(app.doc.node(MOCK_IDS.buildBox)).toBeUndefined();
    expect(app.tabs.tabs).toHaveLength(0);
  });

  it("cancelling the confirmation deletes nothing", async () => {
    const { app } = await mountApp();
    const target = row("Production");
    target.focus();
    await fireEvent.keyDown(target, { key: "Delete" });
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(app.doc.node(MOCK_IDS.production)).toBeDefined();
  });

  it("shows a backend error when deleting fails", async () => {
    const base = createMockIpc({ vault: "unlocked" });
    const ipc: Ipc = { ...base, docDeleteNode: () => Promise.reject(new IpcError("io", "disk full")) };
    const app = createApp(ipc);
    render(App, { props: { app, demo: false } });
    await waitFor(() => expect(app.doc.status).toBe("ready"));
    const target = row("Local shell");
    target.focus();
    await fireEvent.keyDown(target, { key: "Delete" });
    await fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("disk full (io)"));
    expect(app.doc.node(MOCK_IDS.localZsh)).toBeDefined();
  });

  it("opens the context menu with the keyboard and moves a node with 'Move to'", async () => {
    const { app } = await mountApp();
    const target = row("Local shell");
    target.focus();
    await fireEvent.keyDown(target, { key: "ContextMenu" });
    const menu = screen.getByRole("menu");
    // Items also carry a shortcut hint, so compare their labels only.
    expect(within(menu).getAllByRole("menuitem").map((i) => i.querySelector(".label")?.textContent?.trim())).toEqual([
      "Open",
      "Edit…",
      "Rename",
      "Move to…",
      "Move up",
      "Move down",
      "Export…",
      "Delete",
    ]);
    await fireEvent.click(within(menu).getByRole("menuitem", { name: "Move to…" }));

    const dialog = screen.getByRole("dialog", { name: "Move “Local shell”" });
    await pickTreeValue(within(dialog).getByLabelText("Location"), MOCK_IDS.production);
    await fireEvent.click(within(dialog).getByRole("button", { name: "Move to" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(app.doc.node(MOCK_IDS.localZsh)?.parentId).toBe(MOCK_IDS.production);
  });

  it("does not offer a folder's own subtree as a move target", async () => {
    await mountApp();
    const target = row("Lab");
    target.focus();
    await fireEvent.keyDown(target, { key: "ContextMenu" });
    await fireEvent.click(screen.getByRole("menuitem", { name: "Move to…" }));
    const options = await treeSelectOptions(screen.getByLabelText("Location"));
    // Already top level: no "(top level)" entry. The folder itself and everything inside it
    // stay visible, so the tree reads correctly, but cannot be chosen.
    expect(options.map((o) => o.label)).not.toContain("(top level)");
    expect(options.filter((o) => !o.disabled).map((o) => o.label)).toEqual(["Production"]);
    expect(options.find((o) => o.label === "Lab")?.disabled).toBe(true);
  });

  describe("drag and drop", () => {
    const dataTransfer = () => ({ effectAllowed: "", dropEffect: "", setData: vi.fn() });

    it("moves a connection into a folder and refuses a folder into its descendant", async () => {
      const { app } = await mountApp();
      await expand("Lab");
      const dt = dataTransfer();

      await fireEvent.dragStart(row("Local shell"), { dataTransfer: dt });
      const allowed = await fireEvent.dragOver(row("Production"), { dataTransfer: dt });
      expect(allowed).toBe(false); // default prevented = drop allowed
      await fireEvent.drop(row("Production"), { dataTransfer: dt });
      await waitFor(() => expect(app.doc.node(MOCK_IDS.localZsh)?.parentId).toBe(MOCK_IDS.production));

      await fireEvent.dragStart(row("Lab"), { dataTransfer: dt });
      const refused = await fireEvent.dragOver(row("Linux"), { dataTransfer: dt });
      expect(refused).toBe(true); // not prevented: drop not allowed
      await fireEvent.drop(row("Linux"), { dataTransfer: dt });
      expect(app.doc.node(MOCK_IDS.lab)?.parentId).toBeNull();
    });

    it("drops on the empty tree area to move to the top level", async () => {
      const { app } = await mountApp();
      await expand("Lab");
      const dt = dataTransfer();
      await fireEvent.dragStart(row("Local sshd"), { dataTransfer: dt });
      const tree = screen.getByRole("tree");
      expect(await fireEvent.dragOver(tree, { dataTransfer: dt })).toBe(false);
      await fireEvent.drop(tree, { dataTransfer: dt });
      await waitFor(() => expect(app.doc.node(MOCK_IDS.localSshd)?.parentId).toBeNull());
    });

    describe("reordering zones", () => {
      /** Drags over `target` at `fraction` of its height (rows have no layout in jsdom). */
      async function dragOver(
        dragged: string,
        target: string,
        fraction: number,
      ): Promise<{ prevented: boolean; el: HTMLElement; dt: ReturnType<typeof dataTransfer> }> {
        const dt = dataTransfer();
        await fireEvent.dragStart(row(dragged), { dataTransfer: dt });
        const el = row(target);
        el.getBoundingClientRect = () => ({ top: 0, height: 100 }) as DOMRect;
        // jsdom's drag events carry no coordinates: add clientY by hand.
        const event = createEvent.dragOver(el, { dataTransfer: dt });
        Object.defineProperty(event, "clientY", { value: fraction * 100 });
        const notPrevented = await fireEvent(el, event);
        return { prevented: !notPrevented, el, dt };
      }
      async function dropAt(el: HTMLElement, dt: ReturnType<typeof dataTransfer>, fraction: number): Promise<void> {
        const event = createEvent.drop(el, { dataTransfer: dt });
        Object.defineProperty(event, "clientY", { value: fraction * 100 });
        await fireEvent(el, event);
      }
      const topLevel = (): string[] => rowNames().filter((n) => ["Lab", "Production"].includes(n));

      it("drops a folder before and after another folder", async () => {
        const { app } = await mountApp();
        const first = await dragOver("Production", "Lab", 0.1);
        expect(first.prevented).toBe(true);
        expect(first.el.classList.contains("edge-before")).toBe(true);
        await dropAt(first.el, first.dt, 0.1);
        await waitFor(() => expect(topLevel()).toEqual(["Production", "Lab"]));
        expect(app.doc.node(MOCK_IDS.production)?.sortIndex).toBeDefined();

        const second = await dragOver("Production", "Lab", 0.9);
        expect(second.el.classList.contains("edge-after")).toBe(true);
        await dropAt(second.el, second.dt, 0.9);
        await waitFor(() => expect(topLevel()).toEqual(["Lab", "Production"]));
      });

      it("drops into the middle of a folder, as before", async () => {
        const { app } = await mountApp();
        const over = await dragOver("Local shell", "Production", 0.5);
        expect(over.prevented).toBe(true);
        expect(over.el.classList.contains("edge-before")).toBe(false);
        expect(over.el.classList.contains("edge-after")).toBe(false);
        await dropAt(over.el, over.dt, 0.5);
        await waitFor(() => expect(app.doc.node(MOCK_IDS.localZsh)?.parentId).toBe(MOCK_IDS.production));
      });

      it("drops a connection before and after another connection (top and bottom half)", async () => {
        const { app } = await mountApp();
        await expand("Production");
        const first = await dragOver("Public site (isolated)", "Intranet", 0.4);
        expect(first.el.classList.contains("edge-before")).toBe(true);
        await dropAt(first.el, first.dt, 0.4);
        await waitFor(() => expect(app.doc.node(MOCK_IDS.publicSite)?.sortIndex).toBe(0));
        expect(rowNames().indexOf("Public site (isolated)")).toBeLessThan(rowNames().indexOf("Intranet"));

        const second = await dragOver("Public site (isolated)", "Intranet", 0.6);
        expect(second.el.classList.contains("edge-after")).toBe(true);
        await dropAt(second.el, second.dt, 0.6);
        await waitFor(() =>
          expect(rowNames().indexOf("Intranet")).toBeLessThan(rowNames().indexOf("Public site (isolated)")),
        );
      });

      it("shows no line and refuses a drop that changes nothing", async () => {
        const { app } = await mountApp();
        await expand("Production");
        const over = await dragOver("Intranet", "Intranet", 0.9);
        expect(over.prevented).toBe(false);
        expect(over.el.classList.contains("edge-after")).toBe(false);
        await dropAt(over.el, over.dt, 0.9);
        expect(app.doc.node(MOCK_IDS.intranet)?.parentId).toBe(MOCK_IDS.production);
      });

      it("never puts a connection among folders, nor a folder among connections", async () => {
        const { app } = await mountApp();
        const connection = await dragOver("Local shell", "Lab", 0.05);
        expect(connection.el.classList.contains("edge-before")).toBe(false);
        expect(connection.el.classList.contains("edge-after")).toBe(false);
        await fireEvent.dragEnd(row("Local shell"));

        const folder = await dragOver("Production", "Local shell", 0.1);
        expect(folder.el.classList.contains("edge-before")).toBe(false);
        expect(app.doc.node(MOCK_IDS.production)?.sortIndex).toBeUndefined();
      });

      it("treats the lower part of an expanded folder as into, not as a line under it", async () => {
        await mountApp();
        await expand("Lab");
        const over = await dragOver("Production", "Lab", 0.9);
        expect(over.el.classList.contains("edge-after")).toBe(false);
        expect(over.prevented).toBe(true);
      });

      it("disables Move up / Move down and drag lines while a filter is active", async () => {
        await mountApp();
        await fireEvent.input(screen.getByRole("searchbox", { name: "Filter connections" }), {
          target: { value: "Intranet" },
        });
        const target = row("Intranet");
        target.focus();
        await fireEvent.keyDown(target, { key: "ContextMenu" });
        const menu = screen.getByRole("menu");
        expect((within(menu).getByRole("menuitem", { name: "Move up" }) as HTMLButtonElement).disabled).toBe(true);
        expect((within(menu).getByRole("menuitem", { name: "Move down" }) as HTMLButtonElement).disabled).toBe(true);
        await fireEvent.keyDown(menu, { key: "Escape" });

        const over = await dragOver("Intranet", "Intranet", 0.9);
        expect(over.el.classList.contains("edge-after")).toBe(false);
      });
    });
  });
});

describe("tabs and sessions", () => {
  it("opens a tab on Enter and on double click, with one tab per open", async () => {
    const { app } = await mountApp({ sessions: true });
    const shell = row("Local shell");
    shell.focus();
    await fireEvent.keyDown(shell, { key: "Enter" });
    await waitFor(() => expect(app.tabs.tabs).toHaveLength(1));
    await fireEvent.dblClick(row("Local shell"));
    await waitFor(() => expect(app.tabs.tabs).toHaveLength(2));
    expect(screen.getAllByRole("tab")).toHaveLength(2);
    const panel = document.querySelector(`#tabpanel-${app.tabs.state.activeId}`);
    expect(panel?.querySelector("[data-session]")?.getAttribute("data-session")).toBe("sess-2");
  });

  it("renders the right placeholder per kind and passes the sessionId", async () => {
    const { app } = await mountApp({ sessions: true });
    await expand("Lab");
    await expand("Windows");
    await expand("Production");
    await fireEvent.dblClick(row("Local sshd"));
    await fireEvent.dblClick(row("File server"));
    await fireEvent.dblClick(row("Intranet"));
    await waitFor(() => expect(app.tabs.connectionTabs.every((t) => t.status === "open")).toBe(true));
    const text = (id: string): string => document.querySelector(`#tabpanel-${id}`)?.textContent ?? "";
    const [ssh, rdp, web] = app.tabs.tabs;
    expect(document.querySelector(`#tabpanel-${ssh?.id ?? ""} .term`)).not.toBeNull();
    expect(text(rdp?.id ?? "")).toContain("Connecting");
    expect(text(web?.id ?? "")).toContain("Web page area");
  });

  it("shows the unsupported state of the mock gracefully, with retry and close", async () => {
    const { app } = await mountApp({ sessions: false });
    await fireEvent.dblClick(row("Local shell"));
    const alert = await screen.findByText("The session could not be opened");
    const panel = alert.closest("[role=alert]") as HTMLElement;
    expect(panel.textContent).toContain("Error code: unsupported");
    expect(panel.textContent).toContain("not available in this build");
    expect(app.tabs.tabs).toHaveLength(1);

    await fireEvent.click(within(panel).getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(app.tabs.connectionTabs[0]?.attempt).toBe(1));
    await waitFor(() => expect(app.tabs.connectionTabs[0]?.status).toBe("failed"));

    const failed = (await screen.findByText("The session could not be opened")).closest("[role=alert]") as HTMLElement;
    await fireEvent.click(within(failed).getByRole("button", { name: "Close tab" }));
    await waitFor(() => expect(app.tabs.tabs).toHaveLength(0));
    expect(screen.getByText("Chart a course")).toBeTruthy();
  });

  it("switches, closes and cycles tabs with the keyboard", async () => {
    const { app } = await mountApp({ sessions: true });
    await fireEvent.dblClick(row("Local shell"));
    await waitFor(() => expect(app.tabs.tabs).toHaveLength(1));
    await fireEvent.dblClick(row("Local shell"));
    await waitFor(() => expect(app.tabs.tabs).toHaveLength(2));
    const [first, second] = app.tabs.tabs;

    await fireEvent.keyDown(window, { key: "Tab", ctrlKey: true });
    expect(app.tabs.state.activeId).toBe(first?.id);
    await fireEvent.keyDown(window, { key: "Tab", ctrlKey: true, shiftKey: true });
    expect(app.tabs.state.activeId).toBe(second?.id);

    await fireEvent.keyDown(window, { key: "w", ctrlKey: true });
    await waitFor(() => expect(app.tabs.tabs).toHaveLength(1));
    expect(app.tabs.state.activeId).toBe(first?.id);
  });

  it("reorders tabs with Alt+Arrow and drag and drop, and closes others from the menu", async () => {
    const { app } = await mountApp({ sessions: true });
    await expand("Production");
    await fireEvent.dblClick(row("Local shell"));
    await fireEvent.dblClick(row("Intranet"));
    await fireEvent.dblClick(row("Public site (isolated)"));
    await waitFor(() => expect(app.tabs.tabs).toHaveLength(3));
    const titles = (): string[] => screen.getAllByRole("tab").map((t) => t.textContent?.replace(/[!\s]+$/, "").trim() ?? "");
    expect(titles()).toEqual(["Local shell", "Intranet", "Public site (isolated)"]); // new tabs open next to the active one

    const tabs = screen.getAllByRole("tab");
    tabs[0]?.focus();
    await fireEvent.keyDown(tabs[0] as HTMLElement, { key: "ArrowRight", altKey: true });
    expect(titles()).toEqual(["Intranet", "Local shell", "Public site (isolated)"]);

    const dt = { effectAllowed: "", dropEffect: "", setData: vi.fn() };
    const wrappers = screen.getAllByRole("tab").map((t) => t.parentElement as HTMLElement);
    await fireEvent.dragStart(wrappers[2] as HTMLElement, { dataTransfer: dt });
    await fireEvent.dragOver(wrappers[0] as HTMLElement, { dataTransfer: dt });
    await fireEvent.drop(wrappers[0] as HTMLElement, { dataTransfer: dt });
    expect(titles()).toEqual(["Public site (isolated)", "Intranet", "Local shell"]);

    await fireEvent.contextMenu(screen.getAllByRole("tab")[1] as HTMLElement);
    await fireEvent.click(screen.getByRole("menuitem", { name: "Close other tabs" }));
    await waitFor(() => expect(titles()).toEqual(["Intranet"]));
  });
});

describe("overlay flag", () => {
  it("is set while a dialog or a menu is open and cleared afterwards", async () => {
    const { app } = await mountApp();
    expect(app.overlay.open).toBe(false);

    await fireEvent.keyDown(window, { key: "n", ctrlKey: true });
    await screen.findByRole("dialog", { name: "New connection" });
    expect(app.overlay.open).toBe(true);
    await fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    await waitFor(() => expect(app.overlay.open).toBe(false));

    const target = row("Local shell");
    target.focus();
    await fireEvent.keyDown(target, { key: "ContextMenu" });
    expect(app.overlay.open).toBe(true);
    await fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
    await waitFor(() => expect(app.overlay.open).toBe(false));
  });

  it("ignores global shortcuts while a dialog is open", async () => {
    const { app } = await mountApp();
    await fireEvent.keyDown(window, { key: "n", ctrlKey: true });
    await screen.findByRole("dialog");
    await fireEvent.keyDown(window, { key: "n", ctrlKey: true });
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    expect(app.ui.dialog?.type).toBe("connection");
  });

  it("restores focus to the opener when a dialog closes", async () => {
    await mountApp();
    const target = row("Local shell");
    target.focus();
    await fireEvent.keyDown(target, { key: "F2" });
    const dialog = screen.getByRole("dialog");
    expect(dialog.contains(document.activeElement)).toBe(true);
    await fireEvent.keyDown(dialog, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(document.activeElement).toBe(row("Local shell"));
  });
});

describe("connection editor", () => {
  // The kind picker is a segmented control (radio group): pick a kind by clicking its segment.
  const pickKind = async (dialog: HTMLElement, label: string): Promise<void> => {
    const group = within(dialog).getByRole("radiogroup", { name: "Connection type" });
    await fireEvent.click(within(group).getByLabelText(label));
  };

  const openNew = async (): Promise<HTMLElement> => {
    await fireEvent.keyDown(window, { key: "n", ctrlKey: true });
    return screen.findByRole("dialog", { name: "New connection" });
  };

  it("validates before saving and focuses the first invalid field", async () => {
    const { app } = await mountApp();
    const dialog = await openNew();
    await fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(within(dialog).getByText("A name is required.")).toBeTruthy();
    expect(within(dialog).getByText("A host is required.")).toBeTruthy();
    await waitFor(() => expect(document.activeElement).toBe(within(dialog).getByLabelText("Name")));
    expect(app.doc.nodes.some((n) => n.name === "")).toBe(false);

    await fireEvent.input(within(dialog).getByLabelText("Name"), { target: { value: "x" } });
    await fireEvent.input(within(dialog).getByLabelText("Host"), { target: { value: "h" } });
    await fireEvent.input(within(dialog).getByLabelText("Port"), { target: { value: "70000" } });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(within(dialog).getByText(/port must be a whole number/)).toBeTruthy();
  });

  it("shows the per-kind default port and the right fields for each kind", async () => {
    await mountApp();
    const dialog = await openNew();
    const port = (): HTMLInputElement => within(dialog).getByLabelText("Port") as HTMLInputElement;

    expect(port().placeholder).toBe("default: 22");
    expect(within(dialog).getByLabelText("Keepalive (seconds)")).toBeTruthy();

    await pickKind(dialog, "RDP");
    expect(port().placeholder).toBe("default: 3389");
    expect(within(dialog).getByLabelText("Domain")).toBeTruthy();
    expect(within(dialog).getByLabelText("Do not verify the server certificate")).toBeTruthy();

    await pickKind(dialog, "VNC");
    expect(port().placeholder).toBe("default: 5900");
    expect(within(dialog).getByLabelText(/View only/)).toBeTruthy();

    await pickKind(dialog, "Web");
    expect(within(dialog).queryByLabelText("Port")).toBeNull();
    expect(within(dialog).getByLabelText("URL")).toBeTruthy();

    await pickKind(dialog, "Local shell");
    expect(within(dialog).getByLabelText("Program")).toBeTruthy();
    expect(within(dialog).getByLabelText("Arguments")).toBeTruthy();
    expect(within(dialog).getByLabelText("Working directory")).toBeTruthy();
  });

  it("creates a connection in the selected folder and selects it", async () => {
    const { app } = await mountApp();
    await fireEvent.click(row("Production")); // selection = default location
    const dialog = await openNew();
    expect(within(dialog).getByLabelText("Location").dataset.value).toBe(MOCK_IDS.production);

    await pickKind(dialog, "Web");
    await fireEvent.input(within(dialog).getByLabelText("Name"), { target: { value: "Wiki" } });
    await fireEvent.input(within(dialog).getByLabelText("URL"), { target: { value: "https://wiki.example/" } });
    await fireEvent.input(within(dialog).getByLabelText("Tags"), { target: { value: "docs, wiki" } });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

    const created = app.doc.nodes.find((n) => n.name === "Wiki");
    expect(created).toMatchObject({
      type: "connection",
      parentId: MOCK_IDS.production,
      host: "wiki.example",
      tags: ["docs", "wiki"],
      options: { type: "web", url: "https://wiki.example/", isolatedProfile: false },
    });
    expect(rowNames()).toContain("Wiki");
    expect(row("Wiki").getAttribute("aria-selected")).toBe("true");
  });

  it("has an optional Username field for SSH, RDP and VNC only, and saves it in the connection", async () => {
    const { app } = await mountApp({ vault: "notCreated" });
    const dialog = await openNew();
    for (const withUser of ["SSH", "RDP", "VNC"]) {
      await pickKind(dialog, withUser);
      expect(within(dialog).getByLabelText("Username")).toBeTruthy();
    }
    for (const without of ["Web", "Local shell"]) {
      await pickKind(dialog, without);
      expect(within(dialog).queryByLabelText("Username")).toBeNull();
    }

    // Saving: the user name is stored (trimmed); no credential and no vault are involved.
    await pickKind(dialog, "SSH");
    await fireEvent.input(within(dialog).getByLabelText("Name"), { target: { value: "Prompted" } });
    await fireEvent.input(within(dialog).getByLabelText("Host"), { target: { value: "h.example" } });
    await fireEvent.input(within(dialog).getByLabelText("Username"), { target: { value: " deploy " } });
    await fireEvent.click(within(dialog).getByLabelText("None (ask when connecting)"));
    expect(within(dialog).getByTestId("effective-credential").textContent).toContain("will ask when you connect");
    await fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(app.doc.nodes.find((n) => n.name === "Prompted")).toMatchObject({
      type: "connection",
      username: "deploy",
      credential: { type: "none" },
    });
    expect(app.vault.status).toBe("notCreated");
  });

  it("an empty Username is stored as none, and an existing one is shown when editing", async () => {
    const { app } = await mountApp({ vault: "notCreated" });
    let dialog = await openNew();
    await fireEvent.input(within(dialog).getByLabelText("Name"), { target: { value: "Plain" } });
    await fireEvent.input(within(dialog).getByLabelText("Host"), { target: { value: "p.example" } });
    await fireEvent.input(within(dialog).getByLabelText("Username"), { target: { value: "  " } });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    const plain = app.doc.nodes.find((n) => n.name === "Plain");
    expect(plain?.type === "connection" && plain.username).toBeNull();

    // Editing a connection that has one shows it.
    const target = app.doc.node(MOCK_IDS.localSshd);
    if (!target || target.type !== "connection") throw new Error("seed changed");
    await app.doc.upsert({ ...target, username: "tester" });
    await expand("Lab");
    const item = row("Local sshd");
    item.focus();
    await fireEvent.keyDown(item, { key: "ContextMenu" });
    await fireEvent.click(screen.getByRole("menuitem", { name: "Edit…" }));
    dialog = await screen.findByRole("dialog", { name: "Edit connection" });
    expect((within(dialog).getByLabelText("Username") as HTMLInputElement).value).toBe("tester");
  });

  it("shows which credential is effectively inherited and from which folder", async () => {
    await mountApp({ vault: "unlocked" });
    await expand("Lab");
    const target = row("Local sshd");
    target.focus();
    await fireEvent.keyDown(target, { key: "ContextMenu" });
    await fireEvent.click(screen.getByRole("menuitem", { name: "Edit…" }));
    const dialog = await screen.findByRole("dialog", { name: "Edit connection" });
    const effective = within(dialog).getByTestId("effective-credential");
    expect(effective.textContent).toBe("Effective: Lab admin, inherited from “Lab”");

    await fireEvent.click(within(dialog).getByLabelText("None (ask when connecting)"));
    expect(effective.textContent).toBe(
      "Effective: no credential, the app will ask when you connect",
    );

    await fireEvent.click(within(dialog).getByLabelText("Specific credential"));
    await fireEvent.change(within(dialog).getByLabelText("Choose a credential"), {
      target: { value: MOCK_IDS.credKey },
    });
    expect(effective.textContent).toBe("Effective: Deploy key");

    await fireEvent.click(within(dialog).getByLabelText("Inherit from parent folder"));
    await pickTreeValue(within(dialog).getByLabelText("Location"), MOCK_IDS.production);
    expect(effective.textContent).toBe(
      "Effective: no credential (set to None in “Production”), the app will ask when you connect",
    );
  });

  it("requires choosing a credential when 'specific' is selected", async () => {
    await mountApp();
    const dialog = await openNew();
    await fireEvent.input(within(dialog).getByLabelText("Name"), { target: { value: "x" } });
    await fireEvent.input(within(dialog).getByLabelText("Host"), { target: { value: "h" } });
    await fireEvent.click(within(dialog).getByLabelText("Specific credential"));
    await fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(within(dialog).getByText("Choose a credential.")).toBeTruthy();
  });

  it("offers an unlock button instead of the credential list while the vault is locked", async () => {
    const { app } = await mountApp({ vault: "locked" });
    const dialog = await openNew();
    await fireEvent.click(within(dialog).getByLabelText("Specific credential"));
    expect(within(dialog).getByText("Unlock the vault to choose a credential.")).toBeTruthy();
    await fireEvent.click(within(dialog).getByRole("button", { name: "Unlock…" }));
    await waitFor(() => expect(app.vault.request).not.toBeNull());
  });

  it("shows a save error from the backend inside the dialog", async () => {
    const base = createMockIpc({ vault: "unlocked" });
    const ipc: Ipc = { ...base, docUpsertNode: () => Promise.reject(new IpcError("io", "read-only")) };
    const app = createApp(ipc);
    render(App, { props: { app, demo: false } });
    await waitFor(() => expect(app.doc.status).toBe("ready"));
    const dialog = await openNew();
    await fireEvent.input(within(dialog).getByLabelText("Name"), { target: { value: "x" } });
    await fireEvent.input(within(dialog).getByLabelText("Host"), { target: { value: "h" } });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(within(dialog).getByRole("alert").textContent).toContain("read-only (io)"));
    expect(screen.getByRole("dialog", { name: "New connection" })).toBeTruthy();
  });

  it("edits a folder (name and credential)", async () => {
    const { app } = await mountApp();
    const target = row("Production");
    target.focus();
    await fireEvent.keyDown(target, { key: "ContextMenu" });
    await fireEvent.click(screen.getByRole("menuitem", { name: "Edit…" }));
    const dialog = await screen.findByRole("dialog", { name: "Edit folder" });
    await fireEvent.input(within(dialog).getByLabelText("Name"), { target: { value: "Prod" } });
    await fireEvent.click(within(dialog).getByLabelText("Inherit from parent folder"));
    expect(within(dialog).getByTestId("effective-credential").textContent).toContain("nothing set");
    await fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(app.doc.node(MOCK_IDS.production)).toMatchObject({ name: "Prod", credential: { type: "inherit" } });
  });
});

describe("vault UI", () => {
  const typePassword = async (dialog: HTMLElement, label: string, value: string): Promise<void> => {
    await fireEvent.input(within(dialog).getByLabelText(label), { target: { value } });
  };

  it("lets the user browse while locked, and unlocks with the right password only", async () => {
    const { app } = await mountApp({ vault: "locked" });
    expect(rowNames().length).toBeGreaterThan(0);
    expect(screen.getByRole("status", { name: "Vault status: Locked" })).toBeTruthy();

    await fireEvent.click(screen.getByRole("button", { name: "Unlock…" }));
    const dialog = await screen.findByRole("dialog", { name: "Unlock the vault" });
    const input = within(dialog).getByLabelText("Master password") as HTMLInputElement;
    expect(document.activeElement).toBe(input);

    await typePassword(dialog, "Master password", "wrong");
    await fireEvent.click(within(dialog).getByRole("button", { name: "Unlock" }));
    await waitFor(() => expect(within(dialog).getByRole("alert").textContent).toBe("Wrong master password."));
    expect(input.value).toBe(""); // cleared after use
    expect(app.vault.status).toBe("locked");

    await typePassword(dialog, "Master password", MOCK_MASTER_PASSWORD);
    await fireEvent.click(within(dialog).getByRole("button", { name: "Unlock" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(app.vault.status).toBe("unlocked");
    expect(screen.getByRole("status", { name: "Vault status: Unlocked" })).toBeTruthy();
    expect(app.vault.credentials).toHaveLength(3);
  });

  it("prompts for the unlock when opening a connection that needs a credential", async () => {
    const { app } = await mountApp({ vault: "locked", sessions: true });
    await expand("Lab");
    await fireEvent.dblClick(row("Local sshd"));
    const dialog = await screen.findByRole("dialog", { name: "Unlock the vault" });
    expect(dialog.textContent).toContain("A credential is needed to open “Local sshd”.");
    expect(app.tabs.tabs).toHaveLength(0);

    await typePassword(dialog, "Master password", MOCK_MASTER_PASSWORD);
    await fireEvent.click(within(dialog).getByRole("button", { name: "Unlock" }));
    await waitFor(() => expect(app.tabs.tabs).toHaveLength(1));
    expect(app.tabs.connectionTabs[0]?.status).toBe("open");
  });

  it("cancelling that unlock prompt opens nothing", async () => {
    const { app } = await mountApp({ vault: "locked", sessions: true });
    await expand("Lab");
    await fireEvent.dblClick(row("Local sshd"));
    const dialog = await screen.findByRole("dialog", { name: "Unlock the vault" });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(app.tabs.tabs).toHaveLength(0);
  });

  it("with a locked vault the unlock screen also offers 'Enter manually for this session'", async () => {
    const seen: Array<boolean | undefined> = [];
    const base = createMockIpc({ vault: "locked" });
    const ipc: Ipc = {
      ...withSessions(base),
      termOpen: async (_id, _c, _r, _e, options) => {
        seen.push(options?.askCredentials);
        return "sess-1";
      },
    };
    const app = createApp(ipc);
    render(App, { props: { app, demo: false } });
    await waitFor(() => expect(app.doc.status).toBe("ready"));
    await waitFor(() => expect(app.vault.status).toBe("locked"));
    await expand("Lab");
    await fireEvent.dblClick(row("Local sshd"));
    const dialog = await screen.findByRole("dialog", { name: "Unlock the vault" });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Enter manually for this session" }));
    await waitFor(() => expect(app.tabs.tabs).toHaveLength(1));
    expect(seen).toEqual([true]);
    // The vault stayed locked and the unlock screen is gone.
    expect(app.vault.status).toBe("locked");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("the plain unlock screen (from the badge) has no manual alternative", async () => {
    const { app } = await mountApp({ vault: "locked" });
    void app.vault.ensureUnlocked(null);
    const dialog = await screen.findByRole("dialog", { name: "Unlock the vault" });
    expect(within(dialog).queryByRole("button", { name: "Enter manually for this session" })).toBeNull();
  });

  it("a user who never created a vault connects without ever seeing a vault dialog", async () => {
    const { app } = await mountApp({ vault: "notCreated", sessions: true });
    await expand("Lab");
    await fireEvent.dblClick(row("Local sshd"));
    await waitFor(() => expect(app.tabs.tabs).toHaveLength(1));
    expect(app.tabs.connectionTabs[0]?.status).toBe("open");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(app.vault.status).toBe("notCreated");
  });

  it("creates a vault: strength hint, mismatch, acknowledgement and the unrecoverable warning", async () => {
    const { app } = await mountApp({ vault: "notCreated" });
    expect(screen.getByRole("status", { name: "Vault status: No vault" })).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Create vault…" }));
    const dialog = await screen.findByRole("dialog", { name: "Create the vault" });
    expect(dialog.textContent).toContain("cannot be recovered");

    await typePassword(dialog, "Master password", "abc");
    expect(within(dialog).getByText("Strength: weak")).toBeTruthy();
    await fireEvent.click(within(dialog).getByRole("button", { name: "Create" }));
    expect(within(dialog).getByRole("alert").textContent).toContain("at least 8");

    await typePassword(dialog, "Master password", "correct-Horse-battery-9");
    expect(within(dialog).getByText("Strength: strong")).toBeTruthy();
    await typePassword(dialog, "Confirm master password", "different");
    await fireEvent.click(within(dialog).getByRole("button", { name: "Create" }));
    expect(within(dialog).getByRole("alert").textContent?.trim()).toBe("The two passwords do not match.");

    await typePassword(dialog, "Confirm master password", "correct-Horse-battery-9");
    await fireEvent.click(within(dialog).getByRole("button", { name: "Create" }));
    expect(within(dialog).getByRole("alert").textContent).toContain("understand");

    await fireEvent.click(within(dialog).getByRole("checkbox"));
    await fireEvent.click(within(dialog).getByRole("button", { name: "Create" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(app.vault.status).toBe("unlocked");
  });

  it("locks from the badge and with Mod+Shift+L", async () => {
    const { app } = await mountApp({ vault: "unlocked" });
    await fireEvent.click(screen.getByRole("button", { name: "Lock" }));
    await waitFor(() => expect(app.vault.status).toBe("locked"));

    await app.vault.unlock(MOCK_MASTER_PASSWORD);
    await fireEvent.keyDown(window, { key: "L", ctrlKey: true, shiftKey: true });
    await waitFor(() => expect(app.vault.status).toBe("locked"));
    expect(app.vault.credentials).toEqual([]);
  });

  describe("credential manager", () => {
    const openManager = async (): Promise<HTMLElement> => {
      await fireEvent.click(screen.getByRole("button", { name: "Credentials…" }));
      return screen.findByRole("dialog", { name: "Credentials" });
    };

    it("lists credentials without secrets", async () => {
      await mountApp({ vault: "unlocked" });
      const dialog = await openManager();
      expect(within(dialog).getByText("Lab admin")).toBeTruthy();
      expect(within(dialog).getByText("Deploy key")).toBeTruthy();
      expect(dialog.textContent).not.toContain("mock-lab-password");
      expect(screen.queryByTestId("revealed-secret")).toBeNull();
    });

    it("reveals only on explicit action and hides again, automatically after the delay", async () => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
      await mountApp({ vault: "unlocked" });
      const dialog = await openManager();
      const list = within(dialog).getByRole("list", { name: "Credentials" });
      const item = within(list).getByText("Lab admin").closest("li") as HTMLElement;

      await fireEvent.click(within(item).getByRole("button", { name: "Reveal" }));
      expect((await within(item).findByTestId("revealed-secret")).textContent).toContain("mock-lab-password");

      await fireEvent.click(within(item).getByRole("button", { name: "Hide" }));
      expect(screen.queryByTestId("revealed-secret")).toBeNull();
      expect(dialog.textContent).not.toContain("mock-lab-password");

      await fireEvent.click(within(item).getByRole("button", { name: "Reveal" }));
      await within(item).findByTestId("revealed-secret");
      vi.advanceTimersByTime(15_100);
      await waitFor(() => expect(screen.queryByTestId("revealed-secret")).toBeNull());
    });

    it("hides a revealed secret when the vault locks", async () => {
      const { app } = await mountApp({ vault: "unlocked" });
      const dialog = await openManager();
      await fireEvent.click(within(dialog).getAllByRole("button", { name: "Reveal" })[0] as HTMLElement);
      await screen.findByTestId("revealed-secret");
      await app.vault.lock();
      await waitFor(() => expect(screen.queryByTestId("revealed-secret")).toBeNull());
      expect(dialog.textContent).toContain("Unlock the vault to manage credentials.");
    });

    it("copies without displaying the secret", async () => {
      const writeText = vi.fn().mockResolvedValue(undefined);
      vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText }, platform: "Linux" });
      await mountApp({ vault: "unlocked" });
      const dialog = await openManager();
      await fireEvent.click(within(dialog).getAllByRole("button", { name: "Copy secret" })[0] as HTMLElement);
      await waitFor(() => expect(writeText).toHaveBeenCalledOnce());
      expect(screen.queryByTestId("revealed-secret")).toBeNull();
    });

    it("adds, edits (keeping the secret) and deletes credentials", async () => {
      const { app, ipc } = await mountApp({ vault: "unlocked" });
      const dialog = await openManager();

      await fireEvent.click(within(dialog).getByRole("button", { name: "Add credential" }));
      const editor = await screen.findByRole("dialog", { name: "New credential" });
      await fireEvent.click(within(editor).getByRole("button", { name: "Save" }));
      expect(within(editor).getByText("A name is required.")).toBeTruthy();
      expect(within(editor).getByText("A password is required.")).toBeTruthy();

      await fireEvent.input(within(editor).getByLabelText("Name"), { target: { value: "New one" } });
      await fireEvent.input(within(editor).getByLabelText("User name"), { target: { value: "bob" } });
      const pw = within(editor).getByLabelText("Password", { selector: "input[type=password]" }) as HTMLInputElement;
      await fireEvent.input(pw, { target: { value: "s3cret-value" } });
      await fireEvent.click(within(editor).getByRole("button", { name: "Save" }));
      await waitFor(() => expect(screen.queryByRole("dialog", { name: "New credential" })).toBeNull());
      const created = app.vault.credentials.find((c) => c.name === "New one");
      expect(created).toMatchObject({ username: "bob", kind: "password" });
      expect(JSON.stringify(app.vault.credentials)).not.toContain("s3cret-value");

      // edit without retyping the secret
      const item = within(dialog).getByText("New one").closest("li") as HTMLElement;
      await fireEvent.click(within(item).getByRole("button", { name: "Edit" }));
      const edit = await screen.findByRole("dialog", { name: "Edit credential" });
      expect(
        (within(edit).getByLabelText("Password", { selector: "input[type=password]" }) as HTMLInputElement).value,
      ).toBe("");
      await fireEvent.input(within(edit).getByLabelText("Name"), { target: { value: "Renamed one" } });
      await fireEvent.click(within(edit).getByRole("button", { name: "Save" }));
      await waitFor(() => expect(screen.queryByRole("dialog", { name: "Edit credential" })).toBeNull());
      expect(await ipc.vaultRevealSecret(created?.id ?? "")).toEqual({
        type: "password",
        password: "s3cret-value",
      });

      // delete with confirmation
      const renamed = within(dialog).getByText("Renamed one").closest("li") as HTMLElement;
      await fireEvent.click(within(renamed).getByRole("button", { name: "Delete" }));
      const confirm = await screen.findByRole("dialog", { name: "Delete credential" });
      await fireEvent.click(within(confirm).getByRole("button", { name: "Delete" }));
      await waitFor(() => expect(within(dialog).queryByText("Renamed one")).toBeNull());
    });

    it("adds an SSH key credential", async () => {
      const { app } = await mountApp({ vault: "unlocked" });
      const dialog = await openManager();
      await fireEvent.click(within(dialog).getByRole("button", { name: "Add credential" }));
      const editor = await screen.findByRole("dialog", { name: "New credential" });
      await fireEvent.click(within(editor).getByLabelText("SSH key"));
      await fireEvent.input(within(editor).getByLabelText("Name"), { target: { value: "Key 2" } });
      await fireEvent.click(within(editor).getByRole("button", { name: "Save" }));
      expect(within(editor).getByText("A private key is required.")).toBeTruthy();
      await fireEvent.input(within(editor).getByLabelText("Private key (PEM)"), {
        target: { value: "-----BEGIN KEY-----\nx\n-----END KEY-----" },
      });
      await fireEvent.click(within(editor).getByRole("button", { name: "Save" }));
      await waitFor(() => expect(app.vault.credentials.some((c) => c.name === "Key 2" && c.kind === "sshKey")).toBe(true));
    });

    it("changes the master password (wrong current password is reported)", async () => {
      const { app } = await mountApp({ vault: "unlocked" });
      const dialog = await openManager();
      await fireEvent.click(within(dialog).getByRole("button", { name: "Change master password…" }));
      const change = await screen.findByRole("dialog", { name: "Change master password" });

      await typePassword(change, "Current master password", "wrong");
      await typePassword(change, "New master password", "another-long-pass");
      await typePassword(change, "Confirm new master password", "another-long-pass");
      await fireEvent.click(within(change).getByRole("button", { name: "Save" }));
      await waitFor(() => expect(within(change).getByRole("alert").textContent?.trim()).toBe("Wrong master password."));
      expect((within(change).getByLabelText("Current master password") as HTMLInputElement).value).toBe("");

      await typePassword(change, "Current master password", MOCK_MASTER_PASSWORD);
      await typePassword(change, "New master password", "another-long-pass");
      await typePassword(change, "Confirm new master password", "another-long-pass");
      await fireEvent.click(within(change).getByRole("button", { name: "Save" }));
      await waitFor(() => expect(screen.queryByRole("dialog", { name: "Change master password" })).toBeNull());
      await app.vault.lock();
      expect((await app.vault.unlock("another-long-pass")).ok).toBe(true);
    });
  });
});

describe("empty and error states", () => {
  it("shows the empty hint for an empty document", async () => {
    await mountApp({ seed: false });
    expect(screen.getByText(/No connections yet/)).toBeTruthy();
  });

  it("shows the load error with a retry", async () => {
    let fail = true;
    const base = createMockIpc();
    const ipc: Ipc = {
      ...base,
      docGet: () => (fail ? Promise.reject(new IpcError("io", "cannot read")) : base.docGet()),
    };
    const app = createApp(ipc);
    render(App, { props: { app, demo: false } });
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("cannot read (io)");
    fail = false;
    await fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(rowNames()).toContain("Lab"));
  });

  it("keeps the sidebar splitter usable by keyboard", async () => {
    await mountApp();
    const splitter = screen.getByRole("separator", { name: "Resize sidebar" });
    const before = Number(splitter.getAttribute("aria-valuenow"));
    await fireEvent.keyDown(splitter, { key: "ArrowRight" });
    expect(Number(splitter.getAttribute("aria-valuenow"))).toBe(before + 16);
    await fireEvent.keyDown(splitter, { key: "Home" });
    expect(Number(splitter.getAttribute("aria-valuenow"))).toBe(180);
  });
});
