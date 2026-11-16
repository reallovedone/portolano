// The reported defect: with the vault locked, "Unlock…" in a connection's editor opened the
// unlock dialog BEHIND the editor, out of reach. It must open on top, take the focus, and
// leave the half-filled editor untouched underneath.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "../../App.svelte";
import { MOCK_MASTER_PASSWORD, createMockIpc } from "../ipc/mock";
import { createApp } from "../state/app";

vi.mock("@xterm/xterm", async () => ({
  Terminal: (await import("../sessions/term/fakeXterm")).FakeTerminal,
}));

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

async function lockedApp() {
  const app = createApp(createMockIpc({ vault: "locked" }));
  render(App, { props: { app, demo: false } });
  await waitFor(() => expect(app.doc.status).toBe("ready"));
  await waitFor(() => expect(app.vault.status).toBe("locked"));
  return app;
}

const depthOf = (dialog: HTMLElement): number => Number(dialog.parentElement?.dataset.stackDepth);

describe("unlocking the vault from a connection editor", () => {
  it("opens the unlock dialog on top, focused, and keeps what was typed in the editor", async () => {
    const app = await lockedApp();
    app.ui.open({ type: "connection", id: null, parentId: null });
    const editor = await screen.findByRole("dialog", { name: "New connection" });

    await fireEvent.input(within(editor).getByLabelText("Name"), { target: { value: "Half filled" } });
    await fireEvent.input(within(editor).getByLabelText("Host"), { target: { value: "half.example" } });
    await fireEvent.click(within(editor).getByLabelText("Specific credential"));
    expect(within(editor).queryByRole("combobox", { name: "Choose a credential" })).toBeNull();

    const unlockOpener = within(editor).getByRole("button", { name: "Unlock…" });
    await fireEvent.click(unlockOpener);
    const unlock = await screen.findByRole("dialog", { name: "Unlock the vault" });

    // On top (stack order, not DOM order), focused, and the editor underneath is covered.
    expect(depthOf(unlock)).toBeGreaterThan(depthOf(editor));
    expect(editor.parentElement?.hasAttribute("inert")).toBe(true);
    expect(unlock.parentElement?.hasAttribute("inert")).toBe(false);
    const password = within(unlock).getByLabelText("Master password");
    await waitFor(() => expect(document.activeElement).toBe(password));

    // Typing works in the focused field and the unlock button is clickable.
    await fireEvent.input(password, { target: { value: MOCK_MASTER_PASSWORD } });
    expect((password as HTMLInputElement).value).toBe(MOCK_MASTER_PASSWORD);
    await fireEvent.click(within(unlock).getByRole("button", { name: "Unlock" }));

    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Unlock the vault" })).toBeNull());
    expect(app.vault.unlocked).toBe(true);

    // The editor is usable again, still holds the typed values, and lists the credentials.
    const back = screen.getByRole("dialog", { name: "New connection" });
    expect(back).toBe(editor);
    expect(editor.parentElement?.hasAttribute("inert")).toBe(false);
    expect((within(back).getByLabelText("Name") as HTMLInputElement).value).toBe("Half filled");
    expect((within(back).getByLabelText("Host") as HTMLInputElement).value).toBe("half.example");
    const picker = await within(back).findByRole("combobox", { name: "Choose a credential" });
    const labels = within(picker).getAllByRole("option").map((o) => o.textContent ?? "");
    expect(labels.some((l) => l.includes("Lab admin"))).toBe(true);
    expect(labels.length).toBeGreaterThan(1);
    // Focus went back into the editor (the opener button was replaced by the picker, so the
    // dialog falls back to its first field).
    expect(back.contains(document.activeElement)).toBe(true);
  });

  it("returns to the editor, unchanged, when the unlock is cancelled with Escape", async () => {
    const app = await lockedApp();
    app.ui.open({ type: "connection", id: null, parentId: null });
    const editor = await screen.findByRole("dialog", { name: "New connection" });
    await fireEvent.input(within(editor).getByLabelText("Name"), { target: { value: "Keep me" } });
    await fireEvent.click(within(editor).getByLabelText("Specific credential"));
    const opener = within(editor).getByRole("button", { name: "Unlock…" });
    opener.focus();
    await fireEvent.click(opener);
    const unlock = await screen.findByRole("dialog", { name: "Unlock the vault" });

    await fireEvent.keyDown(within(unlock).getByLabelText("Master password"), { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Unlock the vault" })).toBeNull());
    // The editor is still open (Escape went to the top dialog only) with its state and focus.
    expect(screen.getByRole("dialog", { name: "New connection" })).toBe(editor);
    expect((within(editor).getByLabelText("Name") as HTMLInputElement).value).toBe("Keep me");
    expect(document.activeElement).toBe(opener);
    expect(app.vault.unlocked).toBe(false);
  });

  it("shows a wrong password inside the unlock dialog and still lets the user retry", async () => {
    const app = await lockedApp();
    app.ui.open({ type: "connection", id: null, parentId: null });
    const editor = await screen.findByRole("dialog", { name: "New connection" });
    await fireEvent.click(within(editor).getByLabelText("Specific credential"));
    await fireEvent.click(within(editor).getByRole("button", { name: "Unlock…" }));
    const unlock = await screen.findByRole("dialog", { name: "Unlock the vault" });
    const field = within(unlock).getByLabelText("Master password");
    await fireEvent.input(field, { target: { value: "wrong" } });
    await fireEvent.click(within(unlock).getByRole("button", { name: "Unlock" }));
    await waitFor(() => expect(within(unlock).getByText("Wrong master password.")).toBeTruthy());
    expect(app.vault.unlocked).toBe(false);
    await fireEvent.input(field, { target: { value: MOCK_MASTER_PASSWORD } });
    await fireEvent.click(within(unlock).getByRole("button", { name: "Unlock" }));
    await waitFor(() => expect(app.vault.unlocked).toBe(true));
  });
});
