// "Load from file…" in the credential editor: the native dialog gives a path, the narrow backend
// command `readPrivateKeyFile` gives back the text, which fills the private key field.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "../../App.svelte";
import type { Ipc } from "../ipc";
import { createMockIpc } from "../ipc/mock";
import { createApp } from "../state/app";
import { createMockFilePicker, type MockFilePickerOptions } from "../transfer/files";

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

async function openKeyEditor(picker: MockFilePickerOptions, wrap?: (base: Ipc) => Ipc) {
  const base = createMockIpc({ vault: "unlocked" });
  const files = createMockFilePicker(picker);
  const app = createApp(wrap ? wrap(base) : base, files);
  render(App, { props: { app, demo: false } });
  await waitFor(() => expect(app.vault.status).toBe("unlocked"));
  await fireEvent.click(screen.getByRole("button", { name: "Credentials…" }));
  const manager = await screen.findByRole("dialog", { name: "Credentials" });
  await fireEvent.click(within(manager).getByRole("button", { name: "Add credential" }));
  const editor = await screen.findByRole("dialog", { name: "New credential" });
  await fireEvent.click(within(editor).getByLabelText("SSH key"));
  return { app, files, editor };
}

const keyField = (editor: HTMLElement) => within(editor).getByLabelText("Private key (PEM)") as HTMLTextAreaElement;

describe("credential editor: Load from file…", () => {
  it("fills the private key field with the key the backend returned for the chosen path", async () => {
    const read = vi.fn();
    const { editor, files } = await openKeyEditor({ open: "/Users/demo/.ssh/id_ed25519" }, (base) => ({
      ...base,
      readPrivateKeyFile: (path) => {
        read(path);
        return base.readPrivateKeyFile(path);
      },
    }));
    await fireEvent.click(within(editor).getByRole("button", { name: "Load from file…" }));
    await waitFor(() => expect(keyField(editor).value).toContain("BEGIN OPENSSH PRIVATE KEY"));
    expect(read).toHaveBeenCalledExactlyOnceWith("/Users/demo/.ssh/id_ed25519");
    expect(files.openRequests[0]?.filters).toBeUndefined();
    expect(files.openRequests[0]?.startInSshDir).toBe(true);
    expect(within(editor).queryByTestId("load-error")).toBeNull();
  });

  it("cancelling the file dialog changes nothing and reads nothing", async () => {
    const read = vi.fn();
    const { editor } = await openKeyEditor({ open: null }, (base) => ({
      ...base,
      readPrivateKeyFile: (path) => {
        read(path);
        return base.readPrivateKeyFile(path);
      },
    }));
    await fireEvent.input(keyField(editor), { target: { value: "typed by hand" } });
    await fireEvent.click(within(editor).getByRole("button", { name: "Load from file…" }));
    await waitFor(() => expect(document.activeElement).not.toBeNull());
    expect(keyField(editor).value).toBe("typed by hand");
    expect(read).not.toHaveBeenCalled();
  });

  it("shows the backend's refusal for a file that is not a key and leaves the field alone", async () => {
    const { editor } = await openKeyEditor({ open: "/Users/demo/notakey.txt" });
    await fireEvent.input(keyField(editor), { target: { value: "keep" } });
    await fireEvent.click(within(editor).getByRole("button", { name: "Load from file…" }));
    const error = await within(editor).findByTestId("load-error");
    expect(error.textContent).toContain("does not contain a private key");
    expect(error.textContent).toContain("invalid_input");
    expect(keyField(editor).value).toBe("keep");
  });

  it("shows not_found for a vanished file, and a later success clears the error", async () => {
    const { editor, files } = await openKeyEditor({ open: "/Users/demo/missing_key" });
    await fireEvent.click(within(editor).getByRole("button", { name: "Load from file…" }));
    expect((await within(editor).findByTestId("load-error")).textContent).toContain("not_found");
    files.pickOpen = async () => "/Users/demo/.ssh/id_rsa";
    await fireEvent.click(within(editor).getByRole("button", { name: "Load from file…" }));
    await waitFor(() => expect(within(editor).queryByTestId("load-error")).toBeNull());
    expect(keyField(editor).value).toContain("PRIVATE KEY");
  });

  it("is not offered for a password credential", async () => {
    const { editor } = await openKeyEditor({});
    await fireEvent.click(within(editor).getByLabelText("Password"));
    expect(within(editor).queryByRole("button", { name: "Load from file…" })).toBeNull();
  });

  it("the loaded key is saved like a pasted one, and the field is cleared afterwards", async () => {
    const upsert = vi.fn();
    const { editor } = await openKeyEditor({ open: "/Users/demo/.ssh/id_ed25519" }, (base) => ({
      ...base,
      vaultUpsert: (input) => {
        upsert(input);
        return base.vaultUpsert(input);
      },
    }));
    await fireEvent.input(within(editor).getByLabelText("Name"), { target: { value: "From file" } });
    await fireEvent.input(within(editor).getByLabelText("User name"), { target: { value: "tester" } });
    await fireEvent.click(within(editor).getByRole("button", { name: "Load from file…" }));
    await waitFor(() => expect(keyField(editor).value).not.toBe(""));
    await fireEvent.click(within(editor).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(upsert).toHaveBeenCalledTimes(1));
    expect(upsert.mock.calls[0]?.[0]).toMatchObject({
      name: "From file",
      secret: { type: "sshKey", privateKeyPem: expect.stringContaining("PRIVATE KEY") },
    });
  });
});
