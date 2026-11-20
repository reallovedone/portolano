// Choosing an icon and a colour for a connection, and seeing them in the tree and on the tab.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "../../App.svelte";
import { MOCK_IDS, createMockIpc } from "../ipc/mock";
import { createApp } from "../state/app";
import EditorHarness from "./EditorHarness.svelte";

vi.mock("@xterm/xterm", async () => ({
  Terminal: (await import("../sessions/term/fakeXterm")).FakeTerminal,
}));

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

async function openEditor(id: string | null) {
  const app = createApp(createMockIpc({ vault: "unlocked" }));
  await app.doc.load();
  render(EditorHarness, { props: { app, id } });
  const dialog = await screen.findByRole("dialog");
  // Without an icon or a colour the section starts closed: open it.
  await fireEvent.click(sectionHeader(dialog));
  return { app, dialog };
}

const sectionHeader = (root: HTMLElement) => within(root).getByRole("button", { name: /^Icon & colour/ });

const checked = (group: HTMLElement): string =>
  (within(group).getAllByRole("radio").find((r) => (r as HTMLInputElement).checked) as HTMLInputElement | undefined)
    ?.value ?? "none";

describe("appearance picker in the editors", () => {
  it("keeps icon and colour in their own section, closed until one is set", async () => {
    const app = createApp(createMockIpc({ vault: "unlocked" }));
    await app.doc.load();
    render(EditorHarness, { props: { app, id: MOCK_IDS.localSshd } });
    const dialog = await screen.findByRole("dialog");
    expect(sectionHeader(dialog).getAttribute("aria-expanded")).toBe("false");
    expect(within(dialog).queryByRole("radiogroup", { name: "Icon" })).toBeNull();
    cleanup();

    await app.doc.upsert({ ...app.doc.node(MOCK_IDS.localSshd)!, color: "teal" });
    render(EditorHarness, { props: { app, id: MOCK_IDS.localSshd } });
    const coloured = await screen.findByRole("dialog");
    expect(sectionHeader(coloured).getAttribute("aria-expanded")).toBe("true");
  });

  it("starts on the default icon and no colour for a new connection", async () => {
    const { dialog } = await openEditor(null);
    expect(checked(within(dialog).getByRole("radiogroup", { name: "Icon" }))).toBe("");
    expect(checked(within(dialog).getByRole("radiogroup", { name: "Colour" }))).toBe("");
  });

  it("offers the 40 icons plus the default, as radios, and the nine colours plus None", async () => {
    const { dialog } = await openEditor(null);
    expect(within(within(dialog).getByRole("radiogroup", { name: "Icon" })).getAllByRole("radio")).toHaveLength(41);
    expect(within(within(dialog).getByRole("radiogroup", { name: "Colour" })).getAllByRole("radio")).toHaveLength(10);
    // The colour names are not written under the dots: they are the tooltips and accessible names.
    const purple = within(dialog).getByRole("radio", { name: "Purple" }).closest("label")!;
    expect(purple.getAttribute("title")).toBe("Purple");
    expect(purple.querySelector(".caption")?.classList.contains("sr-only")).toBe(true);
  });

  it("filters the icons by name and says when nothing matches", async () => {
    const { dialog } = await openEditor(null);
    const grid = within(dialog).getByRole("radiogroup", { name: "Icon" });
    const search = within(dialog).getByRole("searchbox", { name: "Search icons" });
    await fireEvent.input(search, { target: { value: "hard drive" } });
    expect(within(grid).getAllByRole("radio").map((r) => (r as HTMLInputElement).value)).toEqual(["", "hard-drive"]);
    await fireEvent.input(search, { target: { value: "zzz" } });
    expect(within(dialog).getByRole("status").textContent).toContain("No icon matches “zzz”");
  });

  it("does not submit the form when Enter is pressed in the icon search", async () => {
    const { dialog } = await openEditor(null);
    const search = within(dialog).getByRole("searchbox", { name: "Search icons" });
    expect(await fireEvent.keyDown(search, { key: "Enter" })).toBe(false);
  });

  it("saves the chosen icon and colour with the connection, and clears them again", async () => {
    const { app, dialog } = await openEditor(MOCK_IDS.localSshd);
    await fireEvent.click(within(dialog).getByRole("radio", { name: "Rocket" }));
    await fireEvent.click(within(dialog).getByRole("radio", { name: "Purple" }));
    await fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(app.doc.node(MOCK_IDS.localSshd)).toMatchObject({ icon: "rocket", color: "purple" }),
    );
    cleanup();

    render(EditorHarness, { props: { app, id: MOCK_IDS.localSshd } });
    const again = await screen.findByRole("dialog");
    expect(checked(within(again).getByRole("radiogroup", { name: "Icon" }))).toBe("rocket");
    expect(checked(within(again).getByRole("radiogroup", { name: "Colour" }))).toBe("purple");
    await fireEvent.click(within(again).getByRole("radio", { name: "Default" }));
    await fireEvent.click(within(again).getByRole("radio", { name: "None" }));
    await fireEvent.click(within(again).getByRole("button", { name: "Save" }));
    await waitFor(() => {
      const node = app.doc.node(MOCK_IDS.localSshd);
      expect(node?.icon).toBeUndefined();
      expect(node?.color).toBeUndefined();
    });
  });

  it("puts every icon in one native radio group, so the arrow keys move between them", async () => {
    const { dialog } = await openEditor(null);
    const server = within(dialog).getByRole("radio", { name: "Server" });
    expect((server as HTMLInputElement).name).toBe((within(dialog).getByRole("radio", { name: "Database" }) as HTMLInputElement).name);
  });
});

describe("icon and colour in the tree and on the tab", () => {
  async function mount() {
    const app = createApp(createMockIpc({ vault: "unlocked" }));
    render(App, { props: { app, demo: false } });
    await waitFor(() => expect(app.doc.status).toBe("ready"));
    return app;
  }

  it("shows the chosen icon instead of the kind's and a dot for the colour, only where set", async () => {
    const app = await mount();
    const node = app.doc.node(MOCK_IDS.localSshd);
    if (!node || node.type !== "connection") throw new Error("seed changed");
    await app.doc.upsert({ ...node, icon: "rocket", color: "teal" });
    app.doc.setExpanded(MOCK_IDS.lab, true);
    const tree = await screen.findByRole("tree");
    const row = await waitFor(() => {
      const el = tree.querySelector<HTMLElement>(`[data-node-id="${MOCK_IDS.localSshd}"]`);
      expect(el).toBeTruthy();
      return el as HTMLElement;
    });
    expect(row.querySelector('[data-node-icon="rocket"]')).not.toBeNull();
    expect(row.querySelector("svg.kind")).toBeNull();
    expect(row.querySelector('[data-node-color="teal"]')).not.toBeNull();
    // A folder's colour does not reach the rows inside it.
    const other = tree.querySelector<HTMLElement>(`[data-node-id="${MOCK_IDS.lab}"]`);
    expect(other?.querySelector("[data-node-color]")).toBeNull();
    const folder = app.doc.node(MOCK_IDS.lab);
    if (!folder || folder.type !== "folder") throw new Error("seed changed");
    await app.doc.upsert({ ...folder, color: "red" });
    await waitFor(() =>
      expect(tree.querySelector(`[data-node-id="${MOCK_IDS.lab}"] [data-node-color="red"]`)).not.toBeNull(),
    );
    expect(tree.querySelectorAll('[data-node-color="red"]')).toHaveLength(1);
  });

  it("puts the colour of an open connection on its tab, and nothing on an uncoloured one", async () => {
    const app = await mount();
    const node = app.doc.node(MOCK_IDS.localSshd);
    if (!node || node.type !== "connection") throw new Error("seed changed");
    await app.doc.upsert({ ...node, icon: "rocket", color: "orange" });
    const tabId = await app.tabs.openConnection(MOCK_IDS.localSshd);
    const strip = await screen.findByTestId("tabstrip");
    const tab = await waitFor(() => {
      const el = strip.querySelector<HTMLElement>(`[data-tab-id="${tabId}"]`)?.closest<HTMLElement>(".tab");
      expect(el).toBeTruthy();
      return el as HTMLElement;
    });
    expect(tab.classList.contains("tinted")).toBe(true);
    expect(tab.style.getPropertyValue("--node-color")).toBe("var(--node-orange)");
    expect(tab.querySelector('[data-node-icon="rocket"]')).not.toBeNull();
    await app.tabs.openHome();
    await waitFor(() => expect(strip.querySelectorAll(".tab")).toHaveLength(2));
    const plain = [...strip.querySelectorAll<HTMLElement>(".tab")].filter((el) => !el.classList.contains("tinted"));
    expect(plain).toHaveLength(1);
    expect(plain[0]?.querySelector('[data-icon="house"]')).not.toBeNull();
  });
});
