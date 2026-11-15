// The import and export flows, end to end against the mock IPC and the mock file picker.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "../../App.svelte";
import { IpcError, type Ipc } from "../ipc";
import { MOCK_IDS, createMockIpc, type MockIpc } from "../ipc/mock";
import { createApp, type App as AppState } from "../state/app";
import { pickTreeValue, treeSelectOptions } from "../ui/treeSelectTesting";
import { createMockFilePicker, type MockFilePicker, type MockFilePickerOptions } from "./files";

vi.mock("@xterm/xterm", async () => ({
  Terminal: (await import("../sessions/term/fakeXterm")).FakeTerminal,
}));

async function mountApp(
  options: { seed?: boolean; picker?: MockFilePickerOptions; wrap?: (base: MockIpc) => Ipc } = {},
): Promise<{ app: AppState; ipc: MockIpc; picker: MockFilePicker }> {
  const ipc = createMockIpc({ vault: "unlocked", seed: options.seed });
  const picker = createMockFilePicker(options.picker);
  const app = createApp(options.wrap ? options.wrap(ipc) : ipc, picker);
  render(App, { props: { app, demo: false } });
  await waitFor(() => expect(app.doc.status).toBe("ready"));
  await waitFor(() => expect(app.vault.status).not.toBe("unknown"));
  return { app, ipc, picker };
}

const rowNames = (): string[] =>
  screen.queryAllByRole("treeitem").map((r) => r.querySelector(".name")?.textContent ?? "");
const row = (name: string): HTMLElement => {
  const el = screen.getAllByRole("treeitem").find((r) => r.querySelector(".name")?.textContent === name);
  if (!el) throw new Error(`no tree row named ${name}`);
  return el;
};

async function moreMenu(item: string): Promise<void> {
  await fireEvent.click(screen.getByRole("button", { name: "More actions" }));
  await fireEvent.click(await screen.findByRole("menuitem", { name: new RegExp(`^${item}`) }));
}

async function openImport(): Promise<HTMLElement> {
  await moreMenu("Import connections…");
  return screen.findByRole("dialog", { name: "Import connections" });
}

async function openNodeMenu(name: string): Promise<HTMLElement> {
  const target = row(name);
  target.focus();
  await fireEvent.keyDown(target, { key: "ContextMenu" });
  return screen.getByRole("menu");
}

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
});

describe("import flow", () => {
  it("asks for a file with the open dialog and previews it with the detected format", async () => {
    const { picker, ipc } = await mountApp();
    const spy = vi.spyOn(ipc, "importPreview");
    const dialog = await openImport();

    expect(picker.openRequests).toHaveLength(1);
    expect(picker.openRequests[0]?.filters?.at(-1)?.extensions).toEqual(["*"]);
    await within(dialog).findByRole("list", { name: "Contents" });
    expect(spy).toHaveBeenCalledWith("mobaxterm", "/Users/demo/Sessions.mxtsessions");
    expect(within(dialog).getByText("Sessions.mxtsessions")).toBeTruthy();
    expect((within(dialog).getByLabelText("Format") as HTMLSelectElement).value).toBe("mobaxterm");
  });

  it("shows counts per kind, the tree with hosts, and a target folder choice", async () => {
    await mountApp();
    const dialog = await openImport();
    const contents = await within(dialog).findByRole("list", { name: "Contents" });

    for (const badge of ["Folder · 3", "SSH · 3", "RDP · 3", "VNC · 1", "Web · 1"]) {
      expect(within(dialog).getByText(badge)).toBeTruthy();
    }
    expect(within(contents).getAllByRole("listitem")).toHaveLength(11);
    expect(within(contents).getByText("web01.example.org")).toBeTruthy();
    expect(within(contents).getByText("Web tier")).toBeTruthy();

    const options = await treeSelectOptions(within(dialog).getByLabelText("Import into"));
    expect(options.map((o) => [o.label, o.depth])).toEqual([
      ["(top level)", 0],
      ["Lab", 0],
      ["Linux", 1],
      ["Windows", 1],
      ["Production", 0],
    ]);
  });

  it("groups the ignored passwords into one sentence and lists the other warnings with their line", async () => {
    await mountApp();
    const dialog = await openImport();
    const banner = await within(dialog).findByText("Passwords are never imported");
    expect(banner.closest("[role=status]")?.textContent).toContain(
      "3 passwords or stored credentials in the file were ignored",
    );

    const list = within(dialog).getByRole("list", { name: "Warnings (3)" });
    const items = within(list).getAllByRole("listitem");
    expect(items).toHaveLength(3);
    expect(items[0]?.textContent).toContain("Line 21");
    expect(items[0]?.textContent).toContain("Telnet is not supported");
    expect(items[2]?.textContent).not.toContain("Line");
    expect(list.textContent).not.toContain("password ignored");
  });

  it("lists what the import neutralised, so nothing is silent", async () => {
    const neutralised = [
      "'jump-shell' was not imported: local shell entries with a custom program are not imported",
      "2 credential references ('a', 'b') were reset to none: credentials do not travel between vaults and an import never inherits one",
      "1 RDP connection ('prod-dc01') had \"do not verify the server certificate\": the option was turned off",
    ];
    await mountApp({
      wrap: (base) => ({
        ...base,
        importPreview: async (format, path) => {
          const preview = await base.importPreview(format, path);
          return { ...preview, warnings: neutralised.map((message) => ({ message, line: null })) };
        },
      }),
    });
    const dialog = await openImport();
    const list = await within(dialog).findByRole("list", { name: "Warnings (3)" });
    const text = within(list)
      .getAllByRole("listitem")
      .map((item) => item.textContent ?? "");
    expect(text[0]).toContain("'jump-shell' was not imported");
    expect(text[1]).toContain("2 credential references");
    expect(text[2]).toContain("the option was turned off");
  });

  it("imports at the top level, refreshes the tree and reports the number", async () => {
    const { app, ipc } = await mountApp();
    const apply = vi.spyOn(ipc, "importApply");
    const dialog = await openImport();
    await within(dialog).findByRole("list", { name: "Contents" });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Import" }));

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(apply).toHaveBeenCalledWith(expect.any(String), null);
    expect(await screen.findByText("11 items imported")).toBeTruthy();
    expect(rowNames()).toContain("Datacenter");
    expect(rowNames()).toContain("Jump box");
    expect(app.doc.nodes.length).toBe(12 + 11);
  });

  it("imports into the chosen folder and expands it", async () => {
    const { app, ipc } = await mountApp();
    const apply = vi.spyOn(ipc, "importApply");
    const dialog = await openImport();
    await within(dialog).findByRole("list", { name: "Contents" });
    await pickTreeValue(within(dialog).getByLabelText("Import into"), MOCK_IDS.production);
    await fireEvent.click(within(dialog).getByRole("button", { name: "Import" }));

    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(apply).toHaveBeenCalledWith(expect.any(String), MOCK_IDS.production);
    expect(app.doc.isExpanded(MOCK_IDS.production)).toBe(true);
    expect(app.doc.nodes.find((n) => n.name === "Jump box")?.parentId).toBe(MOCK_IDS.production);
    // The imported folders keep their own place below the target.
    const datacenter = app.doc.nodes.find((n) => n.name === "Datacenter");
    expect(datacenter?.parentId).toBe(MOCK_IDS.production);
    expect(rowNames()).toContain("Datacenter");
  });

  it("previews again when the format is changed by hand", async () => {
    const { ipc } = await mountApp();
    const spy = vi.spyOn(ipc, "importPreview");
    const dialog = await openImport();
    await within(dialog).findByRole("list", { name: "Contents" });

    await fireEvent.change(within(dialog).getByLabelText("Format"), { target: { value: "rdp" } });
    await waitFor(() => expect(spy).toHaveBeenLastCalledWith("rdp", "/Users/demo/Sessions.mxtsessions"));
    const contents = await within(dialog).findByRole("list", { name: "Contents" });
    await waitFor(() => expect(within(contents).getAllByRole("listitem")).toHaveLength(1));
    expect(within(contents).getByText("Reception desk")).toBeTruthy();
    expect(within(dialog).getByText("RDP · 1")).toBeTruthy();
  });

  it("asks for the format when the file is not recognised, and does not preview before", async () => {
    const { ipc } = await mountApp({ picker: { open: "/Users/demo/hosts.txt" } });
    const spy = vi.spyOn(ipc, "importPreview");
    const dialog = await openImport();
    expect(within(dialog).getByText(/format of this file was not recognised/)).toBeTruthy();
    expect(spy).not.toHaveBeenCalled();
    expect((within(dialog).getByRole("button", { name: "Import" }) as HTMLButtonElement).disabled).toBe(true);

    await fireEvent.change(within(dialog).getByLabelText("Format"), { target: { value: "csv" } });
    await within(dialog).findByRole("list", { name: "Contents" });
    expect(spy).toHaveBeenCalledWith("csv", "/Users/demo/hosts.txt");
    expect((within(dialog).getByRole("button", { name: "Import" }) as HTMLButtonElement).disabled).toBe(false);
  });

  it.each([
    ["/Users/demo/encrypted.xml", "This file is not supported", "the file is encrypted"],
    ["/Users/demo/missing.csv", "The file could not be read", "no such file"],
    ["/Users/demo/broken.json", "The file is not valid for this format", "not valid for this format"],
  ])("shows a preview error from the backend as a danger banner (%s)", async (path, title, detail) => {
    await mountApp({ picker: { open: path } });
    const dialog = await openImport();
    const alert = await within(dialog).findByRole("alert");
    expect(alert.textContent).toContain(title);
    expect(alert.textContent).toContain(detail);
    expect(within(dialog).queryByRole("list", { name: "Contents" })).toBeNull();
    expect((within(dialog).getByRole("button", { name: "Import" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("keeps the dialog open with an error when the import itself fails", async () => {
    const { app } = await mountApp({
      wrap: (base) => ({
        ...base,
        importApply: () => Promise.reject(new IpcError("io", "disk full")),
      }),
    });
    const before = app.doc.nodes.length;
    const dialog = await openImport();
    await within(dialog).findByRole("list", { name: "Contents" });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Import" }));
    const alert = await within(dialog).findByRole("alert");
    expect(alert.textContent).toContain("The import failed and nothing was changed");
    expect(alert.textContent).toContain("disk full");
    expect(screen.getByRole("dialog", { name: "Import connections" })).toBeTruthy();
    expect(app.doc.nodes.length).toBe(before);
  });

  it("does nothing when the file dialog is cancelled or Cancel is pressed", async () => {
    const { app } = await mountApp({ picker: { open: null } });
    await moreMenu("Import connections…");
    expect(screen.queryByRole("dialog")).toBeNull();

    cleanup();
    const second = await mountApp();
    const before = second.app.doc.nodes.length;
    const dialog = await openImport();
    await within(dialog).findByRole("list", { name: "Contents" });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(second.app.doc.nodes.length).toBe(before);
    expect(app.doc.nodes.length).toBe(before);
  });

  it("is reachable from the empty state of the tree", async () => {
    await mountApp({ seed: false });
    await fireEvent.click(await screen.findByRole("button", { name: "Import connections…" }));
    const dialog = await screen.findByRole("dialog", { name: "Import connections" });
    await within(dialog).findByRole("list", { name: "Contents" });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Import" }));
    await waitFor(() => expect(rowNames()).toContain("Datacenter"));
  });
});

describe("export flow", () => {
  it("exports the whole document through the save dialog and says passwords are not included", async () => {
    const { ipc, picker } = await mountApp();
    await moreMenu("Export all connections…");

    expect(picker.saveRequests).toHaveLength(1);
    expect(picker.saveRequests[0]?.defaultName).toBe("portolano-connections.json");
    expect(picker.saveRequests[0]?.title).toContain("passwords are never included");
    await waitFor(() => expect(ipc.exports).toHaveLength(1));
    expect(ipc.exports[0]).toEqual({
      format: "json",
      path: "/Users/demo/portolano-connections.json",
      nodeIds: null,
    });
    expect(await screen.findByText(/Exported to portolano-connections\.json/)).toBeTruthy();
    expect(screen.getByText(/Exports never contain passwords/)).toBeTruthy();
  });

  it("exports one node from its context menu with a name-based file name", async () => {
    const { ipc, picker } = await mountApp();
    const menu = await openNodeMenu("Local shell");
    await fireEvent.click(within(menu).getByRole("menuitem", { name: /^Export…/ }));
    await waitFor(() => expect(ipc.exports).toHaveLength(1));
    expect(picker.saveRequests[0]?.defaultName).toBe("Local shell.json");
    expect(ipc.exports[0]).toMatchObject({ format: "json", nodeIds: [MOCK_IDS.localZsh] });
  });

  it("offers .rdp only for RDP connections", async () => {
    const { ipc, picker } = await mountApp();
    await expand("Lab");
    await expand("Windows");

    const sshMenu = await openNodeMenu("Local sshd");
    expect(within(sshMenu).queryByRole("menuitem", { name: /Export as \.rdp/ })).toBeNull();
    await fireEvent.keyDown(sshMenu, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull());

    const rdpMenu = await openNodeMenu("File server");
    await fireEvent.click(within(rdpMenu).getByRole("menuitem", { name: /Export as \.rdp/ }));
    await waitFor(() => expect(ipc.exports).toHaveLength(1));
    expect(picker.saveRequests[0]?.defaultName).toBe("File server.rdp");
    expect(ipc.exports[0]).toMatchObject({ format: "rdp", nodeIds: [MOCK_IDS.rdpServer] });
  });

  it("does nothing when the save dialog is cancelled", async () => {
    const { ipc } = await mountApp({ picker: { save: null } });
    await moreMenu("Export all connections…");
    await Promise.resolve();
    expect(ipc.exports).toHaveLength(0);
    expect(screen.queryByText(/Exported to/)).toBeNull();
  });

  it("shows an export failure as an error toast", async () => {
    await mountApp({ picker: { save: "/Users/demo/readonly.json" } });
    await moreMenu("Export all connections…");
    const toast = await screen.findByText("The export failed");
    expect(toast.closest("[role=alert]")?.textContent).toContain("permission denied");
  });

  it("shows a file dialog failure as an error toast", async () => {
    const { app } = await mountApp();
    app.files.pickSave = () => Promise.reject(new Error("no window"));
    await moreMenu("Export all connections…");
    expect(await screen.findByText("The file dialog could not be opened")).toBeTruthy();
  });
});
