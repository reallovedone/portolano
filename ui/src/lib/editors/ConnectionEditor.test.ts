import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import { createMockIpc, MOCK_IDS } from "../ipc/mock";
import { createApp } from "../state/app";
import type { ConnectionKind } from "../tree/model";
import EditorHarness from "./EditorHarness.svelte";

afterEach(cleanup);

async function open(id: string | null, kind?: ConnectionKind) {
  const app = createApp(createMockIpc({ vault: "unlocked" }));
  await app.doc.load();
  await app.vault.refresh();
  render(EditorHarness, { props: { app, id, kind } });
  return screen.findByRole("dialog");
}

describe("ConnectionEditor structure", () => {
  it("groups the fields into titled sections, in reading order", async () => {
    const dialog = await open(null);
    const titles = [...dialog.querySelectorAll("section h3")].map((h) => h.textContent);
    // SSH is the default kind: it has options, so every section is present.
    expect(titles).toEqual(["General", "Connection", "Credentials", "Options", "Notes"]);
  });

  it("drops the Options section for a local shell", async () => {
    const dialog = await open(null);
    const group = within(dialog).getByRole("radiogroup", { name: "Connection type" });
    await fireEvent.click(within(group).getByLabelText("Local shell"));
    const titles = [...dialog.querySelectorAll("section h3")].map((h) => h.textContent);
    expect(titles).toEqual(["General", "Connection", "Credentials", "Notes"]);
  });

  it("offers the kind as a radio group that is fixed once the connection exists", async () => {
    const creating = await open(null);
    const radios = within(creating).getAllByRole("radio", { name: /^(Local shell|SSH|RDP|VNC|Web)$/ });
    expect(radios).toHaveLength(5);
    expect(radios.every((r) => !(r as HTMLInputElement).disabled)).toBe(true);
    cleanup();

    const editing = await open(MOCK_IDS.localSshd);
    const group = within(editing).getByRole("radiogroup", { name: "Connection type" });
    const kinds = within(group).getAllByRole("radio") as HTMLInputElement[];
    expect(kinds.every((r) => r.disabled)).toBe(true);
    expect(kinds.find((r) => r.checked)?.value).toBe("ssh");
  });

  it("describes each credential choice and keeps the effective-credential line", async () => {
    const dialog = await open(null);
    const inherit = within(dialog).getByLabelText("Inherit from parent folder");
    expect(inherit.getAttribute("aria-describedby")).toBeTruthy();
    expect(within(dialog).getByTestId("effective-credential").textContent).toMatch(/^Effective:/);
  });
});

describe("ConnectionEditor starting kind", () => {
  const checkedKind = (dialog: HTMLElement): string =>
    (
      within(within(dialog).getByRole("radiogroup", { name: "Connection type" }))
        .getAllByRole("radio")
        .find((r) => (r as HTMLInputElement).checked) as HTMLInputElement
    ).value;

  it("starts on SSH by default and on the requested kind when creating", async () => {
    expect(checkedKind(await open(null))).toBe("ssh");
    cleanup();
    const rdp = await open(null, "rdp");
    expect(checkedKind(rdp)).toBe("rdp");
    expect(within(rdp).getByLabelText("Width")).toBeTruthy();
    cleanup();
    expect(checkedKind(await open(null, "localShell"))).toBe("localShell");
  });

  it("ignores the requested kind when editing an existing connection", async () => {
    expect(checkedKind(await open(MOCK_IDS.localSshd, "web"))).toBe("ssh");
  });
});

describe("ConnectionEditor VNC security note", () => {
  const NOTE = /VNC is encrypted only when the server offers TLS \(VeNCrypt\)/;

  it("tells honestly that VNC is encrypted only through TLS, in the VNC options only", async () => {
    const vnc = await open(null, "vnc");
    const note = within(vnc).getByText(NOTE);
    expect(note.closest("[role='note']")).toBeTruthy();
    expect(note.textContent).toMatch(/asks before it sends the password/);
    expect(note.textContent).toMatch(/refused if it stops offering it/);
    cleanup();

    expect(within(await open(null, "rdp")).queryByText(NOTE)).toBeNull();
    cleanup();
    expect(within(await open(null, "ssh")).queryByText(NOTE)).toBeNull();
  });

  it("shows the note when an existing VNC connection is edited", async () => {
    expect(within(await open(MOCK_IDS.devVm)).getByText(NOTE)).toBeTruthy();
  });
});

describe("ConnectionEditor RDP without NLA", () => {
  const LABEL = "Allow connections without NLA (less secure)";

  it("is off by default, only in the RDP options, and warns when it is turned on", async () => {
    const rdp = await open(null, "rdp");
    const box = within(rdp).getByLabelText(LABEL) as HTMLInputElement;
    expect(box.checked).toBe(false);
    expect(within(rdp).queryByText(/someone who can pose as the server/)).toBeNull();
    await fireEvent.click(box);
    const warning = within(rdp).getByText(/someone who can pose as the server/);
    expect(warning.closest("[role='note']")).toBeTruthy();
    // Turning it off again removes the warning.
    await fireEvent.click(box);
    expect(within(rdp).queryByText(/someone who can pose as the server/)).toBeNull();
    cleanup();

    expect(within(await open(null, "vnc")).queryByLabelText(LABEL)).toBeNull();
    cleanup();
    expect(within(await open(null, "ssh")).queryByLabelText(LABEL)).toBeNull();
  });

  it("an existing connection keeps the choice it was saved with", async () => {
    // The mock "File server" connection is saved without the option.
    const dialog = await open(MOCK_IDS.rdpServer);
    expect((within(dialog).getByLabelText(LABEL) as HTMLInputElement).checked).toBe(false);
  });
});

describe("ConnectionEditor RDP clipboard sharing", () => {
  const LABEL = "Share the clipboard with the remote desktop";

  it("is off by default, explained, and only offered for RDP", async () => {
    const rdp = await open(null, "rdp");
    const box = within(rdp).getByLabelText(LABEL) as HTMLInputElement;
    expect(box.checked).toBe(false);
    // The explanation says who sees what, and that vault passwords are not shared.
    expect(within(rdp).getByText(/can read what you copy/)).toBeTruthy();
    expect(within(rdp).getByText(/never shared/)).toBeTruthy();
    cleanup();
    expect(within(await open(null, "vnc")).queryByLabelText(LABEL)).toBeNull();
    cleanup();
    expect(within(await open(null, "ssh")).queryByLabelText(LABEL)).toBeNull();
  });

  it("an existing connection keeps the choice it was saved with", async () => {
    const dialog = await open(MOCK_IDS.rdpServer);
    expect((within(dialog).getByLabelText(LABEL) as HTMLInputElement).checked).toBe(false);
  });

  it("is saved with the connection", async () => {
    const app = createApp(createMockIpc({ vault: "unlocked" }));
    await app.doc.load();
    render(EditorHarness, { props: { app, id: null, kind: "rdp" } });
    const dialog = await screen.findByRole("dialog");
    await fireEvent.input(within(dialog).getByLabelText("Name"), { target: { value: "Shared" } });
    await fireEvent.input(within(dialog).getByLabelText("Host"), { target: { value: "h.example" } });
    await fireEvent.click(within(dialog).getByLabelText(LABEL));
    await fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    await vi.waitFor(() => {
      const saved = app.doc.nodes.find((n) => n.type === "connection" && n.name === "Shared");
      expect(saved?.type === "connection" && saved.options).toMatchObject({
        type: "rdp",
        shareClipboard: true,
      });
    });
  });
});
