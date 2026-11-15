import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { CredentialAnswer, CredentialPrompt } from "../ipc";
import { createMockIpc } from "../ipc/mock";
import { createApp } from "../state/app";
import PromptHarness from "./PromptHarness.svelte";

afterEach(cleanup);

const base: CredentialPrompt = {
  kind: "usernamePassword",
  usernameHint: null,
  domainHint: null,
  target: "host.example:3389",
  error: null,
  canSaveToVault: false,
};

async function show(
  prompt: Partial<CredentialPrompt> = {},
  options: { vault?: "notCreated" | "locked" | "unlocked"; name?: string | null } = {},
) {
  const app = createApp(createMockIpc({ vault: options.vault ?? "notCreated" }));
  await app.vault.refresh();
  const onsubmit = vi.fn<(answer: CredentialAnswer, save: boolean) => void>();
  const oncancel = vi.fn<() => void>();
  render(PromptHarness, {
    props: {
      app,
      prompt: { ...base, ...prompt },
      connectionName: options.name ?? null,
      onsubmit,
      oncancel,
    },
  });
  const dialog = await screen.findByRole("dialog", { name: "Credentials needed" });
  return { app, onsubmit, oncancel, dialog };
}

const type = (label: string, value: string) =>
  fireEvent.input(screen.getByLabelText(label), { target: { value } });

describe("CredentialPromptDialog", () => {
  it("focuses the user name while it is empty, the password once the hint filled it", async () => {
    await show();
    expect(document.activeElement).toBe(screen.getByLabelText("Username"));
    cleanup();
    await show({ usernameHint: "ann" });
    expect(document.activeElement).toBe(screen.getByLabelText("Password"));
  });

  it("names the connection and the target (the hop of a jump chain)", async () => {
    const { dialog } = await show({ target: "jump.example:22", kind: "sshKey" }, { name: "Build box" });
    expect(dialog.textContent).toContain("“Build box” has no credential");
    expect(dialog.textContent).toContain("jump.example:22");
  });

  it("shows why the previous answer failed", async () => {
    await show({ error: "The server rejected the credentials." });
    expect(screen.getByTestId("prompt-error").textContent).toBe("The server rejected the credentials.");
  });

  it("submits user name, password and domain, trimmed, and clears the secret", async () => {
    const { onsubmit, dialog } = await show({ domainHint: "LAB" });
    await type("Username", "  ann ");
    await type("Password", "p4ss word");
    await type("Domain (optional)", " CORP ");
    await fireEvent.submit(dialog.querySelector("form") as HTMLFormElement);
    expect(onsubmit).toHaveBeenCalledExactlyOnceWith(
      { username: "ann", password: "p4ss word", keyPath: null, passphrase: null, domain: "CORP" },
      false,
    );
    expect((screen.getByLabelText("Password") as HTMLInputElement).value).toBe("");
  });

  it("the Connect button submits the same form (so Enter in a field does too)", async () => {
    const { dialog, onsubmit } = await show({ usernameHint: "ann" });
    const connect = screen.getByRole("button", { name: "Connect" }) as HTMLButtonElement;
    expect(connect.type).toBe("submit");
    expect(connect.getAttribute("form")).toBe(dialog.querySelector("form")?.id);
    await type("Password", "x");
    await fireEvent.click(connect);
    expect(onsubmit).toHaveBeenCalledTimes(1);
  });

  it("does not submit without a user name or a password, and says why", async () => {
    const { onsubmit, dialog } = await show();
    await fireEvent.submit(dialog.querySelector("form") as HTMLFormElement);
    expect(screen.getByText("Enter a user name.")).toBeTruthy();
    await type("Username", "ann");
    await fireEvent.submit(dialog.querySelector("form") as HTMLFormElement);
    expect(screen.getByText("Enter a password.")).toBeTruthy();
    expect(onsubmit).not.toHaveBeenCalled();
  });

  it("Escape and the Cancel button cancel, and clear what was typed", async () => {
    const { oncancel, dialog } = await show({ usernameHint: "ann" });
    await type("Password", "typed-secret");
    await fireEvent.keyDown(dialog, { key: "Escape" });
    expect(oncancel).toHaveBeenCalledTimes(1);
    expect((screen.getByLabelText("Password") as HTMLInputElement).value).toBe("");
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(oncancel).toHaveBeenCalledTimes(2);
  });

  it("asks only for a password when the kind is `password` (VNC)", async () => {
    const { onsubmit, dialog } = await show({ kind: "password" });
    expect(screen.queryByLabelText("Username")).toBeNull();
    expect(screen.queryByLabelText("Domain (optional)")).toBeNull();
    expect(document.activeElement).toBe(screen.getByLabelText("Password"));
    await type("Password", "vnc");
    await fireEvent.submit(dialog.querySelector("form") as HTMLFormElement);
    expect(onsubmit).toHaveBeenCalledWith(
      { username: "", password: "vnc", keyPath: null, passphrase: null, domain: null },
      false,
    );
  });

  it("SSH: a password or a private key file with an optional passphrase", async () => {
    const { onsubmit, dialog } = await show({ kind: "sshKey", usernameHint: "tester" });
    expect(screen.queryByLabelText("Domain (optional)")).toBeNull();
    // Key file mode swaps the secret fields; the file's content is never asked for.
    await fireEvent.click(screen.getByLabelText("Use a private key file"));
    expect(screen.queryByLabelText("Password")).toBeNull();
    await fireEvent.submit(dialog.querySelector("form") as HTMLFormElement);
    expect(screen.getByText("Enter the path of the key file.")).toBeTruthy();
    await type("Private key file path", " ~/.ssh/id_ed25519 ");
    await type("Key passphrase (optional)", "pp");
    await fireEvent.submit(dialog.querySelector("form") as HTMLFormElement);
    expect(onsubmit).toHaveBeenCalledExactlyOnceWith(
      { username: "tester", password: null, keyPath: "~/.ssh/id_ed25519", passphrase: "pp", domain: null },
      false,
    );
    expect((screen.getByLabelText("Key passphrase (optional)") as HTMLInputElement).value).toBe("");
  });

  it("SSH password mode sends no key path", async () => {
    const { onsubmit, dialog } = await show({ kind: "sshKey", usernameHint: "tester" });
    await type("Password", "pw");
    await fireEvent.submit(dialog.querySelector("form") as HTMLFormElement);
    expect(onsubmit).toHaveBeenCalledWith(
      { username: "tester", password: "pw", keyPath: null, passphrase: null, domain: null },
      false,
    );
  });

  it("offers 'save to the vault' only when the backend allows it AND the vault is unlocked", async () => {
    await show({ canSaveToVault: true }, { vault: "unlocked" });
    expect(screen.getByLabelText("Save to the vault")).toBeTruthy();
    cleanup();
    await show({ canSaveToVault: false }, { vault: "unlocked" });
    expect(screen.queryByLabelText("Save to the vault")).toBeNull();
    cleanup();
    await show({ canSaveToVault: true }, { vault: "locked" });
    expect(screen.queryByLabelText("Save to the vault")).toBeNull();
    cleanup();
    // No vault at all: never offered, and never a request to create one.
    await show({ canSaveToVault: true }, { vault: "notCreated" });
    expect(screen.queryByLabelText("Save to the vault")).toBeNull();
    expect(screen.queryByText(/create/i)).toBeNull();
  });

  it("passes the save choice along", async () => {
    const { onsubmit, dialog } = await show({ canSaveToVault: true, usernameHint: "ann" }, { vault: "unlocked" });
    await fireEvent.click(screen.getByLabelText("Save to the vault"));
    await type("Password", "pw");
    await fireEvent.submit(dialog.querySelector("form") as HTMLFormElement);
    expect(onsubmit.mock.calls[0]?.[1]).toBe(true);
  });

  it("raises the overlay flag while it is open and lowers it when it goes away", async () => {
    const { app } = await show();
    expect(app.overlay.open).toBe(true);
    cleanup();
    expect(app.overlay.open).toBe(false);
  });
});
