// The credentials window: search, type filter, order, count, row and bar menus.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "../../App.svelte";
import { MOCK_IDS, createMockIpc } from "../ipc/mock";
import { createApp } from "../state/app";

vi.mock("@xterm/xterm", async () => ({
  Terminal: (await import("../sessions/term/fakeXterm")).FakeTerminal,
}));

async function openManager() {
  const mock = createMockIpc({ vault: "unlocked" });
  const app = createApp(mock);
  render(App, { props: { app, demo: false } });
  await waitFor(() => expect(app.doc.status).toBe("ready"));
  await waitFor(() => expect(app.vault.status).toBe("unlocked"));
  await fireEvent.click(screen.getByRole("button", { name: "Credentials…" }));
  const dialog = await screen.findByRole("dialog", { name: "Credentials" });
  return { app, mock, dialog };
}

function rowTexts(dialog: HTMLElement): string[] {
  return within(dialog)
    .queryAllByRole("listitem")
    .map((li) => li.textContent ?? "");
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

describe("credentials window list", () => {
  it("sorts by name and shows the count", async () => {
    const { dialog } = await openManager();
    const rows = rowTexts(dialog);
    expect(rows).toHaveLength(3);
    expect(rows[0]).toContain("Deploy key");
    expect(rows[1]).toContain("Lab admin");
    expect(rows[2]).toContain("Windows domain user");
    expect(within(dialog).getByTestId("cred-count").textContent?.trim()).toBe("3 credentials");
  });

  it("shows the user, the domain and how many connections use each credential", async () => {
    const { dialog } = await openManager();
    const rows = rowTexts(dialog);
    expect(rows[2]).toContain("LAB\\svc-portolano");
    expect(rows[0]).toContain("deploy · Used by 1 connection");
    expect(rows[0]).not.toContain("Used by 1 connections");
    // The kind is the icon's name, not text.
    expect(within(dialog).getAllByRole("img", { name: "SSH key" })).toHaveLength(1);
    expect(within(dialog).getAllByRole("img", { name: "Password" })).toHaveLength(2);
  });

  it("filters by name, user and domain, ignoring case, and updates the count", async () => {
    const { dialog } = await openManager();
    const search = within(dialog).getByRole("searchbox", { name: "Search credentials" });
    await fireEvent.input(search, { target: { value: "LAB ADMIN" } });
    expect(rowTexts(dialog)).toHaveLength(1);
    expect(within(dialog).getByTestId("cred-count").textContent?.trim()).toBe("1 of 3 credentials");

    await fireEvent.input(search, { target: { value: "deploy" } }); // user name and name
    expect(rowTexts(dialog)[0]).toContain("Deploy key");

    await fireEvent.input(search, { target: { value: "lab" } }); // domain LAB and "Lab admin"
    expect(rowTexts(dialog)).toHaveLength(2);
  });

  it("filters by type, offered only when both types exist", async () => {
    const { dialog } = await openManager();
    await fireEvent.click(within(dialog).getByRole("radio", { name: "SSH keys" }));
    expect(rowTexts(dialog)).toHaveLength(1);
    expect(within(dialog).getByTestId("cred-count").textContent?.trim()).toBe("1 of 3 credentials");
    await fireEvent.click(within(dialog).getByRole("radio", { name: "Passwords" }));
    expect(rowTexts(dialog)).toHaveLength(2);
    await fireEvent.click(within(dialog).getByRole("radio", { name: "All" }));
    expect(rowTexts(dialog)).toHaveLength(3);
  });

  it("hides the type filter when only one type exists", async () => {
    const { app, dialog } = await openManager();
    await app.vault.remove(MOCK_IDS.credKey);
    await waitFor(() => expect(within(dialog).queryByRole("radiogroup")).toBeNull());
    expect(within(dialog).getByTestId("cred-count").textContent?.trim()).toBe("2 credentials");
  });

  it("explains an empty search and clears it", async () => {
    const { dialog } = await openManager();
    const search = within(dialog).getByRole("searchbox", { name: "Search credentials" });
    await fireEvent.input(search, { target: { value: "zzz" } });
    expect(within(dialog).getByText("No credentials match “zzz”")).toBeTruthy();
    await fireEvent.click(within(dialog).getByRole("button", { name: "Clear search" }));
    expect(rowTexts(dialog)).toHaveLength(3);
    expect((search as HTMLInputElement).value).toBe("");
  });

  it("opens Edit and Delete from the row menu, and Edit on double click", async () => {
    const { dialog } = await openManager();
    const row = within(dialog).getByText("Lab admin").closest("li") as HTMLElement;
    await fireEvent.click(within(row).getByRole("button", { name: "Actions for Lab admin" }));
    const menu = await screen.findByRole("menu");
    expect(within(menu).getByRole("menuitem", { name: "Edit…" })).toBeTruthy();
    await fireEvent.click(within(menu).getByRole("menuitem", { name: "Delete…" }));
    expect(await screen.findByRole("dialog", { name: "Delete credential" })).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    await fireEvent.doubleClick(within(dialog).getByText("Deploy key"));
    expect(await screen.findByRole("dialog", { name: "Edit credential" })).toBeTruthy();
  });

  it("keeps reveal and copy visible on every row", async () => {
    const { dialog } = await openManager();
    expect(within(dialog).getAllByRole("button", { name: "Reveal" })).toHaveLength(3);
    expect(within(dialog).getAllByRole("button", { name: "Copy secret" })).toHaveLength(3);
  });

  it("locks and changes the password from the bar's More actions menu", async () => {
    const { app, dialog } = await openManager();
    await fireEvent.click(within(dialog).getByRole("button", { name: "More actions" }));
    await fireEvent.click(await screen.findByRole("menuitem", { name: "Change master password…" }));
    expect(await screen.findByRole("dialog", { name: "Change master password" })).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Change master password" })).toBeNull());

    await fireEvent.click(within(dialog).getByRole("button", { name: "More actions" }));
    await fireEvent.click(await screen.findByRole("menuitem", { name: "Lock" }));
    await waitFor(() => expect(app.vault.status).toBe("locked"));
  });
});
