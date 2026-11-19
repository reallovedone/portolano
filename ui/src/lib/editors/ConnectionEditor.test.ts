import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import { t, type StringKey } from "../i18n/strings";
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
    const note = within(vnc).getByText(NOTE, { selector: ".short" });
    expect(note.closest("[role='note']")).toBeTruthy();
    expect(note.textContent).toMatch(/asks before it sends the password/);
    await fireEvent.click(within(vnc).getByRole("button", { name: "Learn more" }));
    expect(within(vnc).getByText(/once for each server/)).toBeTruthy();
    expect(within(vnc).getByText(/refused if it stops offering it/)).toBeTruthy();
    expect(within(vnc).getByText(/travels without encryption/)).toBeTruthy();
    cleanup();

    expect(within(await open(null, "rdp")).queryByText(NOTE)).toBeNull();
    cleanup();
    expect(within(await open(null, "ssh")).queryByText(NOTE)).toBeNull();
  });

  it("shows the note when an existing VNC connection is edited", async () => {
    expect(
      within(await open(MOCK_IDS.devVm)).getByText(NOTE, { selector: ".short" }),
    ).toBeTruthy();
  });
});

describe("ConnectionEditor RDP without NLA", () => {
  const LABEL = "Allow connections without NLA (less secure)";

  it("is off by default, only in the RDP options, and warns when it is turned on", async () => {
    const rdp = await open(null, "rdp");
    const box = within(rdp).getByLabelText(LABEL) as HTMLInputElement;
    expect(box.checked).toBe(false);
    expect(within(rdp).queryByText(/someone posing as the server/)).toBeNull();
    await fireEvent.click(box);
    const warning = within(rdp).getByText(/someone posing as the server/);
    expect(warning.closest("[role='note']")).toBeTruthy();
    // Turning it off again removes the warning.
    await fireEvent.click(box);
    expect(within(rdp).queryByText(/someone posing as the server/)).toBeNull();
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
    expect(within(rdp).getByText(/never shared/)).toBeTruthy();
    // The "Learn more" that belongs to the clipboard option, found through its own row.
    const row = box.closest<HTMLElement>(".option");
    expect(row).toBeTruthy();
    const more = within(row!).getByText(/can read and replace what you copy/) as HTMLElement;
    expect(more.hidden).toBe(true);
    await fireEvent.click(within(row!).getByRole("button", { name: "Learn more" }));
    expect(more.hidden).toBe(false);
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

describe("ConnectionEditor RDP Kerberos KDC", () => {
  const LABEL = "Kerberos KDC";
  const HINT = "Optional. Leave empty to find it automatically.";

  it("is offered for RDP only, with its hint", async () => {
    const rdp = await open(null, "rdp");
    expect((within(rdp).getByLabelText(LABEL) as HTMLInputElement).value).toBe("");
    expect(within(rdp).getByText(HINT)).toBeTruthy();
    cleanup();
    expect(within(await open(null, "ssh")).queryByLabelText(LABEL)).toBeNull();
    cleanup();
    expect(within(await open(null, "vnc")).queryByLabelText(LABEL)).toBeNull();
  });

  async function fillRdp(kdc: string) {
    const app = createApp(createMockIpc({ vault: "unlocked" }));
    await app.doc.load();
    render(EditorHarness, { props: { app, id: null, kind: "rdp" } });
    const dialog = await screen.findByRole("dialog");
    await fireEvent.input(within(dialog).getByLabelText("Name"), { target: { value: "Domain PC" } });
    await fireEvent.input(within(dialog).getByLabelText("Host"), { target: { value: "pc.corp.example.com" } });
    await fireEvent.input(within(dialog).getByLabelText(LABEL), { target: { value: kdc } });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    return { app, dialog };
  }

  it("is saved with the connection", async () => {
    const { app } = await fillRdp("dc.corp.example.com");
    await vi.waitFor(() => {
      const saved = app.doc.nodes.find((n) => n.type === "connection" && n.name === "Domain PC");
      expect(saved?.type === "connection" && saved.options).toMatchObject({
        type: "rdp",
        kerberosKdc: "dc.corp.example.com",
      });
    });
  });

  it("shows a validation message and does not save an invalid value", async () => {
    const { app, dialog } = await fillRdp("tcp://");
    expect(await within(dialog).findByText(/not a valid address/)).toBeTruthy();
    expect(app.doc.nodes.some((n) => n.type === "connection" && n.name === "Domain PC")).toBe(false);
  });
});

describe("ConnectionEditor RDP Mac shortcuts", () => {
  const LABEL = "Use Mac shortcuts";
  const HELP = "⌘C and ⌘V work as on your Mac.";

  /** Pretends to run on this platform (the editor reads it once when it opens). */
  function platform(name: string): void {
    vi.spyOn(globalThis.navigator, "platform", "get").mockReturnValue(name);
  }

  afterEach(() => vi.restoreAllMocks());

  it("is on by default for a new RDP connection on macOS, with its help text, and only for RDP", async () => {
    platform("MacIntel");
    const rdp = await open(null, "rdp");
    expect((within(rdp).getByLabelText(LABEL) as HTMLInputElement).checked).toBe(true);
    expect(within(rdp).getByText(HELP)).toBeTruthy();
    cleanup();
    expect(within(await open(null, "vnc")).queryByLabelText(LABEL)).toBeNull();
    cleanup();
    expect(within(await open(null, "ssh")).queryByLabelText(LABEL)).toBeNull();
  });

  it("is hidden on Windows", async () => {
    platform("Win32");
    const rdp = await open(null, "rdp");
    expect(within(rdp).queryByLabelText(LABEL)).toBeNull();
    expect(within(rdp).queryByText(HELP)).toBeNull();
  });

  it("an existing connection saved without the option has it off", async () => {
    platform("MacIntel");
    const dialog = await open(MOCK_IDS.rdpServer);
    expect((within(dialog).getByLabelText(LABEL) as HTMLInputElement).checked).toBe(false);
  });

  it("is saved with the connection, on or off", async () => {
    platform("MacIntel");
    for (const turnOff of [false, true]) {
      const app = createApp(createMockIpc({ vault: "unlocked" }));
      await app.doc.load();
      render(EditorHarness, { props: { app, id: null, kind: "rdp" } });
      const dialog = await screen.findByRole("dialog");
      const name = turnOff ? "Plain" : "Mac";
      await fireEvent.input(within(dialog).getByLabelText("Name"), { target: { value: name } });
      await fireEvent.input(within(dialog).getByLabelText("Host"), { target: { value: "h.example" } });
      if (turnOff) await fireEvent.click(within(dialog).getByLabelText(LABEL));
      await fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
      await vi.waitFor(() => {
        const saved = app.doc.nodes.find((n) => n.type === "connection" && n.name === name);
        expect(saved?.type === "connection" && saved.options).toMatchObject({
          type: "rdp",
          macShortcuts: !turnOff,
        });
      });
      cleanup();
    }
  });
});

describe("ConnectionEditor RDP sections", () => {
  const header = (root: HTMLElement, name: string) =>
    within(root).getByRole("button", { name: new RegExp(`^${name}`) });

  it("groups the RDP fields into titled sections, closed where the plan says", async () => {
    const rdp = await open(null, "rdp");
    const titles = [...rdp.querySelectorAll("section h3")].map(
      (h) => h.querySelector(".title")?.textContent ?? h.textContent,
    );
    expect(titles).toEqual([
      "General",
      "Connection",
      "Credentials",
      "Display",
      "Sign-in",
      "Clipboard & keyboard",
      "Security",
      "Notes",
    ]);
    expect(header(rdp, "Display").getAttribute("aria-expanded")).toBe("true");
    expect(header(rdp, "Clipboard & keyboard").getAttribute("aria-expanded")).toBe("true");
    expect(header(rdp, "Sign-in").getAttribute("aria-expanded")).toBe("false");
    expect(header(rdp, "Security").getAttribute("aria-expanded")).toBe("false");
  });

  it("toggles a section from its header and links it with aria-controls", async () => {
    const rdp = await open(null, "rdp");
    const signIn = header(rdp, "Sign-in");
    const body = rdp.querySelector<HTMLElement>(`#${signIn.getAttribute("aria-controls")}`);
    expect(body?.hidden).toBe(true);
    await fireEvent.click(signIn);
    expect(signIn.getAttribute("aria-expanded")).toBe("true");
    expect(body?.hidden).toBe(false);
  });

  async function openSaved(options: Record<string, unknown>) {
    const app = createApp(createMockIpc({ vault: "unlocked" }));
    await app.doc.load();
    const id = "00000000-0000-4000-8000-0000000000aa";
    await app.doc.upsert({
      type: "connection",
      id,
      parentId: null,
      name: "Saved",
      host: "h.example",
      port: null,
      username: null,
      tags: [],
      notes: "",
      credential: { type: "inherit" },
      options: {
        type: "rdp",
        ignoreCert: false,
        allowWithoutNla: false,
        shareClipboard: false,
        macShortcuts: false,
        ...options,
      },
    } as never);
    render(EditorHarness, { props: { app, id } });
    return screen.findByRole("dialog");
  }

  it("opens Sign-in and Security when they hold non-default values", async () => {
    const dialog = await openSaved({ ignoreCert: true, domain: "corp.example.com" });
    expect(header(dialog, "Sign-in").getAttribute("aria-expanded")).toBe("true");
    expect(header(dialog, "Security").getAttribute("aria-expanded")).toBe("true");
    // Closing it again shows what is relaxed, in the header.
    await fireEvent.click(header(dialog, "Security"));
    expect(header(dialog, "Security").textContent).toMatch(/Certificate not checked/);
  });

  it("opens Sign-in for a connection that only has a KDC", async () => {
    const dialog = await openSaved({ kerberosKdc: "dc.corp.example.com" });
    expect(header(dialog, "Sign-in").getAttribute("aria-expanded")).toBe("true");
    expect(header(dialog, "Security").getAttribute("aria-expanded")).toBe("false");
  });

  it("opens Security for a connection that only allows no NLA", async () => {
    const dialog = await openSaved({ allowWithoutNla: true });
    expect(header(dialog, "Security").getAttribute("aria-expanded")).toBe("true");
    expect(header(dialog, "Sign-in").getAttribute("aria-expanded")).toBe("false");
  });

  it("opens the section of a field with a validation error and focuses the field", async () => {
    const app = createApp(createMockIpc({ vault: "unlocked" }));
    await app.doc.load();
    render(EditorHarness, { props: { app, id: null, kind: "rdp" } });
    const dialog = await screen.findByRole("dialog");
    await fireEvent.input(within(dialog).getByLabelText("Name"), { target: { value: "X" } });
    await fireEvent.input(within(dialog).getByLabelText("Host"), { target: { value: "h.example" } });
    await fireEvent.input(within(dialog).getByLabelText("Kerberos KDC"), { target: { value: "tcp://" } });
    // The field is typed into while its section is still closed.
    expect(header(dialog, "Sign-in").getAttribute("aria-expanded")).toBe("false");
    await fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    await vi.waitFor(() =>
      expect(header(dialog, "Sign-in").getAttribute("aria-expanded")).toBe("true"),
    );
    await vi.waitFor(() =>
      expect(document.activeElement).toBe(within(dialog).getByLabelText("Kerberos KDC")),
    );
  });

  it("keeps the NLA warning visible once Security is open and the option is on", async () => {
    const rdp = await open(null, "rdp");
    await fireEvent.click(header(rdp, "Security"));
    await fireEvent.click(within(rdp).getByLabelText("Allow connections without NLA (less secure)"));
    expect(within(rdp).getByText(/someone posing as the server/)).toBeTruthy();
  });
});

describe("editor help texts", () => {
  const PAIRS: [StringKey, StringKey][] = [
    ["editor.vnc.securityNote", "editor.vnc.securityNoteMore"],
    ["editor.rdp.shareClipboardHelp", "editor.rdp.shareClipboardMore"],
    ["editor.rdp.macShortcutsHelp", "editor.rdp.macShortcutsMore"],
    ["editor.rdp.kerberosKdcHint", "editor.rdp.kerberosKdcMore"],
  ];

  it("the long text continues the short line instead of repeating its first sentence", () => {
    for (const [short, more] of PAIRS) {
      const first = t(short).split(/(?<=[.!?])\s/)[0]!.replace(/[.!?]$/, "");
      expect(t(more), `${more} repeats ${short}`).not.toContain(first);
    }
  });
});
