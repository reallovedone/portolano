// Editing a credential without its secret, and what the delete confirmations say.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "../../App.svelte";
import type { Ipc } from "../ipc";
import { MOCK_IDS, createMockIpc } from "../ipc/mock";
import { createApp, type App as AppState } from "../state/app";

vi.mock("@xterm/xterm", async () => ({
  Terminal: (await import("../sessions/term/fakeXterm")).FakeTerminal,
}));

async function mountApp(wrap?: (base: ReturnType<typeof createMockIpc>) => Ipc) {
  const mock = createMockIpc({ vault: "unlocked" });
  const ipc = wrap ? wrap(mock) : mock;
  const app: AppState = createApp(ipc);
  render(App, { props: { app, demo: false } });
  await waitFor(() => expect(app.doc.status).toBe("ready"));
  await waitFor(() => expect(app.vault.status).toBe("unlocked"));
  return { app, ipc: mock };
}

function treeRow(name: string): HTMLElement {
  const found = screen.getAllByRole("treeitem").find((r) => r.querySelector(".name")?.textContent === name);
  if (!found) throw new Error(`no tree row named ${name}`);
  return found;
}

async function openManager(): Promise<HTMLElement> {
  await fireEvent.click(screen.getByRole("button", { name: "Credentials…" }));
  return screen.findByRole("dialog", { name: "Credentials" });
}

/** Opens the row's ⋯ menu and picks an item (Edit… / Delete…). */
async function rowAction(item: HTMLElement, action: "Edit…" | "Delete…"): Promise<void> {
  await fireEvent.click(within(item).getByRole("button", { name: /^Actions for / }));
  await fireEvent.click(await screen.findByRole("menuitem", { name: action }));
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

describe("editing a credential keeps its secret without revealing it", () => {
  it("sends secret: null, never calls reveal, and the stored secret survives", async () => {
    const reveal = vi.fn();
    const upsert = vi.fn();
    const { ipc } = await mountApp((base) => ({
      ...base,
      vaultRevealSecret: (id) => {
        reveal(id);
        return base.vaultRevealSecret(id);
      },
      vaultUpsert: (input) => {
        upsert(input);
        return base.vaultUpsert(input);
      },
    }));
    const dialog = await openManager();
    const item = within(dialog).getByText("Lab admin").closest("li") as HTMLElement;
    await rowAction(item, "Edit…");
    const editor = await screen.findByRole("dialog", { name: "Edit credential" });
    await fireEvent.input(within(editor).getByLabelText("Name"), { target: { value: "Lab root" } });
    await fireEvent.click(within(editor).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Edit credential" })).toBeNull());

    expect(reveal).not.toHaveBeenCalled();
    expect(upsert).toHaveBeenCalledTimes(1);
    expect(upsert.mock.calls[0]?.[0]).toMatchObject({ id: MOCK_IDS.credLab, name: "Lab root", secret: null });
    expect(await ipc.vaultRevealSecret(MOCK_IDS.credLab)).toEqual({
      type: "password",
      password: "mock-lab-password",
    });
  });

  it("still sends a new secret when one is typed, and creating still needs one", async () => {
    const upsert = vi.fn();
    const { ipc } = await mountApp((base) => ({
      ...base,
      vaultUpsert: (input) => {
        upsert(input);
        return base.vaultUpsert(input);
      },
    }));
    const dialog = await openManager();
    const item = within(dialog).getByText("Lab admin").closest("li") as HTMLElement;
    await rowAction(item, "Edit…");
    const editor = await screen.findByRole("dialog", { name: "Edit credential" });
    await fireEvent.input(
      within(editor).getByLabelText("Password", { selector: "input[type=password]" }),
      { target: { value: "replacement" } },
    );
    await fireEvent.click(within(editor).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Edit credential" })).toBeNull());
    expect(upsert.mock.calls[0]?.[0]).toMatchObject({ secret: { type: "password", password: "replacement" } });
    expect(await ipc.vaultRevealSecret(MOCK_IDS.credLab)).toMatchObject({ password: "replacement" });

    // The contract refuses a new credential without a secret.
    await expect(
      ipc.vaultUpsert({ id: null, name: "x", username: "u", domain: null, secret: null }),
    ).rejects.toMatchObject({ code: "invalid_input" });
  });
});

describe("delete confirmations say how many connections are affected", () => {
  it("lists up to three users of a credential and counts the rest", async () => {
    const { app } = await mountApp();
    // Lab admin is used by the Lab folder; add three more direct users.
    for (const name of ["Alpha", "Beta", "Gamma"]) {
      const saved = await app.doc.upsert({
        type: "connection",
        id: crypto.randomUUID(),
        parentId: null,
        name,
        host: `${name}.example.org`,
        port: null,
        credential: { type: "id", id: MOCK_IDS.credLab },
        username: null,
        tags: [],
        notes: "",
        options: { type: "ssh", jumpHost: null, keepaliveSecs: null },
      });
      expect(saved.ok).toBe(true);
    }
    const dialog = await openManager();
    const item = within(dialog).getByText("Lab admin").closest("li") as HTMLElement;
    await rowAction(item, "Delete…");
    const confirm = await screen.findByRole("dialog", { name: "Delete credential" });
    expect(confirm.textContent).toContain("4 items use it directly");
    expect(confirm.textContent).toContain("“Lab”, “Alpha”, “Beta”, and 1 more");
    expect(confirm.textContent).toContain("They will have no credential");
  });

  it("says nothing extra for an unused credential, and the document no longer points at a deleted one", async () => {
    const { app } = await mountApp();
    const dialog = await openManager();
    const unused = within(dialog).getByText("Deploy key").closest("li") as HTMLElement;
    // The Dev VM connection uses the deploy key directly: one user.
    await rowAction(unused, "Delete…");
    const confirm = await screen.findByRole("dialog", { name: "Delete credential" });
    expect(confirm.textContent).toContain("1 item uses it directly (“Dev VM (VNC)”)");
    await fireEvent.click(within(confirm).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(app.vault.credentials.some((c) => c.id === MOCK_IDS.credKey)).toBe(false));
    // The UI's copy of the document already shows the cleared reference (no manual reload).
    await waitFor(() =>
      expect(app.doc.node(MOCK_IDS.devVm)).toMatchObject({ credential: { type: "none" } }),
    );

    // Now nothing refers to a credential nobody uses.
    await fireEvent.click(within(dialog).getByRole("button", { name: "Add credential" }));
    const editor = await screen.findByRole("dialog", { name: "New credential" });
    await fireEvent.input(within(editor).getByLabelText("Name"), { target: { value: "Spare" } });
    await fireEvent.input(within(editor).getByLabelText("Password", { selector: "input[type=password]" }), {
      target: { value: "p" },
    });
    await fireEvent.click(within(editor).getByRole("button", { name: "Save" }));
    const spare = await within(dialog).findByText("Spare");
    await rowAction(spare.closest("li") as HTMLElement, "Delete…");
    const second = await screen.findByRole("dialog", { name: "Delete credential" });
    expect(second.textContent).not.toContain("use it directly");
    expect(second.textContent).not.toContain("uses it directly");
  });

  it("counts the SSH connections that use a connection as jump host", async () => {
    const { app } = await mountApp();
    await app.doc.upsert({
      type: "connection",
      id: crypto.randomUUID(),
      parentId: null,
      name: "Second hop",
      host: "hop.example.org",
      port: null,
      credential: { type: "none" },
      username: null,
      tags: [],
      notes: "",
      options: { type: "ssh", jumpHost: MOCK_IDS.localSshd, keepaliveSecs: null },
    });
    // Build box already uses Local sshd: two connections depend on it.
    app.doc.expand(MOCK_IDS.lab);
    const sshdRow = await waitFor(() => treeRow("Local sshd"));
    sshdRow.focus();
    await fireEvent.keyDown(sshdRow, { key: "Delete" });
    const confirm = await screen.findByRole("dialog", { name: "Delete connection" });
    expect(confirm.textContent).toContain("2 SSH connections use it as a jump host and will connect directly instead.");
    await fireEvent.click(within(confirm).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(app.doc.node(MOCK_IDS.localSshd)).toBeUndefined());
    expect(app.doc.node(MOCK_IDS.buildBox)).toMatchObject({ options: { type: "ssh", jumpHost: null } });
  });

  it("counts jump-host users outside a folder that is deleted", async () => {
    const { app } = await mountApp();
    // Build box uses Local sshd but both live inside Lab; this one lives outside it.
    await app.doc.upsert({
      type: "connection",
      id: crypto.randomUUID(),
      parentId: MOCK_IDS.production,
      name: "Outside",
      host: "out.example.org",
      port: null,
      credential: { type: "none" },
      username: null,
      tags: [],
      notes: "",
      options: { type: "ssh", jumpHost: MOCK_IDS.localSshd, keepaliveSecs: null },
    });
    const labRow = treeRow("Lab");
    labRow.focus();
    await fireEvent.keyDown(labRow, { key: "Delete" });
    const confirm = await screen.findByRole("dialog", { name: "Delete folder" });
    expect(confirm.textContent).toContain(
      "1 SSH connection outside it uses a connection inside it as a jump host",
    );
  });
});
