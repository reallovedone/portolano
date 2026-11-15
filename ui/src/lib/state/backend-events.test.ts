import { cleanup, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import App from "../../App.svelte";
import { createMockIpc } from "../ipc/mock";
import { createApp } from "./app";
import { connectBackendEvents } from "./backend-events";

vi.mock("@xterm/xterm", async () => ({
  Terminal: (await import("../sessions/term/fakeXterm")).FakeTerminal,
}));

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("vault-locked event", () => {
  it("locks the store at once when the backend locks the vault by itself", async () => {
    const ipc = createMockIpc({ vault: "unlocked" });
    const app = createApp(ipc);
    await app.vault.refresh();
    expect(app.vault.credentials.length).toBeGreaterThan(0);
    const off = connectBackendEvents(app);

    ipc.simulate.autoLockVault();
    // No call to the backend in between: the event alone updates the state.
    expect(app.vault.status).toBe("locked");
    expect(app.vault.credentials).toEqual([]);
    off();
  });

  it("stops listening once unsubscribed", async () => {
    const ipc = createMockIpc({ vault: "unlocked" });
    const app = createApp(ipc);
    await app.vault.refresh();
    connectBackendEvents(app)();
    ipc.simulate.autoLockVault();
    expect(app.vault.status).toBe("unlocked");
  });

  it("is wired into the shell: the badge and the credential list follow the event", async () => {
    vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => undefined });
    const ipc = createMockIpc({ vault: "unlocked" });
    const app = createApp(ipc);
    render(App, { props: { app, demo: false } });
    await screen.findByRole("status", { name: "Vault status: Unlocked" });

    ipc.simulate.autoLockVault();
    await screen.findByRole("status", { name: "Vault status: Locked" });
    expect(app.vault.credentials).toEqual([]);
  });

  it("is emitted by an explicit lock too, and a lock that changes nothing is silent", async () => {
    const ipc = createMockIpc({ vault: "unlocked" });
    const seen = vi.fn();
    ipc.onVaultLocked(seen);
    await ipc.vaultLock();
    await ipc.vaultLock();
    expect(seen).toHaveBeenCalledTimes(1);
  });
});

describe("credential-save-failed event", () => {
  it("shows a non-blocking warning toast and nothing else", async () => {
    vi.stubGlobal("localStorage", { getItem: () => null, setItem: () => undefined });
    const ipc = createMockIpc({ vault: "unlocked" });
    const app = createApp(ipc);
    render(App, { props: { app, demo: false } });
    await waitFor(() => expect(app.doc.status).toBe("ready"));

    ipc.simulate.credentialSaveFailed({
      connectionId: "c1",
      connectionName: "Build box",
      code: "vault_locked",
      message: "The vault is locked: unlock it, or enter the credential by hand.",
    });
    const title = await screen.findByText("Could not save the credential to the vault");
    const toast = title.closest(".toast") as HTMLElement;
    expect(toast.classList.contains("warning")).toBe(true);
    expect(toast.getAttribute("role")).toBe("status");
    expect(toast.textContent).toContain("“Build box” is connected, but its credential was not saved");
    expect(toast.textContent).toContain("The vault is locked");
    expect(app.notices.items.map((n) => n.kind)).toEqual(["warning"]);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("fades by itself, later than an information", async () => {
    vi.useFakeTimers();
    try {
      const app = createApp(createMockIpc());
      const info = app.notices.push("info", "i");
      const warning = app.notices.push("warning", "w");
      const error = app.notices.push("error", "e");
      vi.advanceTimersByTime(5_100);
      expect(app.notices.items.map((n) => n.id)).toEqual([warning, error]);
      vi.advanceTimersByTime(7_000);
      expect(app.notices.items.map((n) => n.id)).toEqual([error]);
      expect(info).toBeLessThan(warning);
    } finally {
      vi.useRealTimers();
    }
  });
});
