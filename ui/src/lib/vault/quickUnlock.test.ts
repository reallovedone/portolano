// Quick unlock in the UI: the biometric button/auto attempt in the unlock dialog and the switch
// in the Security section.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "../../App.svelte";
import { MOCK_MASTER_PASSWORD, createMockIpc, type MockOptions } from "../ipc/mock";
import { createApp } from "../state/app";

vi.mock("@xterm/xterm", async () => ({
  Terminal: (await import("../sessions/term/fakeXterm")).FakeTerminal,
}));

function setPlatform(platform: "mac" | "windows"): void {
  vi.stubGlobal("navigator", {
    platform: platform === "mac" ? "MacIntel" : "Win32",
    userAgent: platform === "mac" ? "Mac" : "Windows",
  });
}

beforeEach(() => {
  const data = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => data.get(k) ?? null,
    setItem: (k: string, v: string) => {
      data.set(k, v);
    },
  });
  setPlatform("mac");
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

/** A locked vault that holds a quick-unlock key (option on, one password unlock done). */
async function readyApp(options: MockOptions = {}) {
  const mock = createMockIpc({ vault: "locked", ...options });
  await mock.vaultSetQuickUnlock(true);
  await mock.vaultUnlock(MOCK_MASTER_PASSWORD);
  await mock.vaultLock();
  const app = createApp(mock);
  render(App, { props: { app, demo: false } });
  await waitFor(() => expect(app.vault.status).toBe("locked"));
  return { mock, app };
}

async function openUnlock(app: ReturnType<typeof createApp>) {
  const pending = app.vault.ensureUnlocked();
  const dialog = await screen.findByRole("dialog", { name: "Unlock the vault" });
  return { dialog, pending };
}

describe("unlock dialog with quick unlock", () => {
  it("starts the biometric check by itself once and unlocks", async () => {
    const { mock, app } = await readyApp();
    const { pending } = await openUnlock(app);
    expect(await pending).toBe(true);
    expect(mock.quickUnlock.prompts).toBe(1);
    expect(app.vault.status).toBe("unlocked");
  });

  it("cancelled: keeps the password field focused, no error, a button to retry", async () => {
    const { mock, app } = await readyApp();
    mock.quickUnlock.outcome = "cancelled";
    const { dialog } = await openUnlock(app);
    const retry = await within(dialog).findByRole("button", { name: "Use Touch ID" });
    await waitFor(() => expect(mock.quickUnlock.prompts).toBe(1));
    expect(within(dialog).queryByText(/did not work/)).toBeNull();
    expect(within(dialog).queryByRole("status")).toBeNull();
    expect(within(dialog).queryByRole("alert")).toBeNull();
    const password = within(dialog).getByLabelText("Master password");
    await waitFor(() => expect(document.activeElement).toBe(password));
    // No loop: still one prompt after a moment.
    await new Promise((r) => setTimeout(r, 30));
    expect(mock.quickUnlock.prompts).toBe(1);
    mock.quickUnlock.outcome = "ok";
    await fireEvent.click(retry);
    await waitFor(() => expect(app.vault.status).toBe("unlocked"));
    expect(mock.quickUnlock.prompts).toBe(2);
  });

  it("failed: a short neutral message, the password field and the retry button stay", async () => {
    const { mock, app } = await readyApp();
    mock.quickUnlock.outcome = "failed";
    const { dialog } = await openUnlock(app);
    expect(await within(dialog).findByText("Touch ID did not work. Use your master password.")).toBeTruthy();
    expect(within(dialog).getByLabelText("Master password")).toBeTruthy();
    expect(mock.quickUnlock.prompts).toBe(1);
  });

  it("names Windows Hello on Windows", async () => {
    setPlatform("windows");
    const { mock, app } = await readyApp();
    mock.quickUnlock.outcome = "cancelled";
    const { dialog } = await openUnlock(app);
    expect(await within(dialog).findByRole("button", { name: "Use Windows Hello" })).toBeTruthy();
  });

  it("not ready: no button, no prompt, the password works", async () => {
    const mock = createMockIpc({ vault: "locked" });
    const app = createApp(mock);
    render(App, { props: { app, demo: false } });
    await waitFor(() => expect(app.vault.status).toBe("locked"));
    const { dialog, pending } = await openUnlock(app);
    expect(within(dialog).queryByRole("button", { name: /Use (Touch ID|Windows Hello)/ })).toBeNull();
    expect(mock.quickUnlock.prompts).toBe(0);
    await fireEvent.input(within(dialog).getByLabelText("Master password"), {
      target: { value: MOCK_MASTER_PASSWORD },
    });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Unlock" }));
    expect(await pending).toBe(true);
  });

  it("not ready answer hides the retry button", async () => {
    const { mock, app } = await readyApp();
    mock.quickUnlock.outcome = "cancelled";
    const { dialog } = await openUnlock(app);
    const retry = await within(dialog).findByRole("button", { name: "Use Touch ID" });
    await waitFor(() => expect((retry as HTMLButtonElement).disabled).toBe(false));
    mock.quickUnlock.outcome = "unlock-failed"; // drops the key: next ask is not ready
    await fireEvent.click(retry); // stale key: fails and is dropped
    await within(dialog).findByText("Touch ID did not work. Use your master password.");
    await fireEvent.click(within(dialog).getByRole("button", { name: "Use Touch ID" }));
    await waitFor(() =>
      expect(within(dialog).queryByRole("button", { name: "Use Touch ID" })).toBeNull(),
    );
  });

  it("a late biometric result is ignored once the password has unlocked", async () => {
    const { mock, app } = await readyApp();
    let release: () => void = () => {};
    const gate = new Promise<void>((r) => (release = r));
    const original = mock.vaultQuickUnlock.bind(mock);
    mock.vaultQuickUnlock = async () => {
      await gate;
      return original();
    };
    const { dialog, pending } = await openUnlock(app);
    await fireEvent.input(within(dialog).getByLabelText("Master password"), {
      target: { value: MOCK_MASTER_PASSWORD },
    });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Unlock" }));
    expect(await pending).toBe(true);
    const finish = vi.spyOn(app.vault, "finishRequest");
    release();
    await new Promise((r) => setTimeout(r, 20));
    expect(finish).not.toHaveBeenCalled();
    expect(app.vault.status).toBe("unlocked");
  });
});

describe("unlock dialog races", () => {
  it("a password submit in flight does not close twice when the biometric wins", async () => {
    const { mock, app } = await readyApp();
    let release: () => void = () => {};
    const gate = new Promise<void>((r) => (release = r));
    const originalUnlock = mock.vaultUnlock.bind(mock);
    const finish = vi.spyOn(app.vault, "finishRequest");
    let first = true;
    mock.vaultUnlock = async (pw: string) => {
      if (first && pw === "typed") {
        first = false;
        await gate;
        return originalUnlock(MOCK_MASTER_PASSWORD);
      }
      return originalUnlock(pw);
    };
    mock.quickUnlock.outcome = "cancelled";
    const { dialog } = await openUnlock(app);
    const retry = await within(dialog).findByRole("button", { name: "Use Touch ID" });
    await waitFor(() => expect((retry as HTMLButtonElement).disabled).toBe(false));
    mock.vaultQuickUnlock = async () => {
      mock.quickUnlock.prompts += 1;
      return originalUnlock(MOCK_MASTER_PASSWORD).then(() => undefined);
    };
    await fireEvent.input(within(dialog).getByLabelText("Master password"), {
      target: { value: "typed" },
    });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Unlock" }));
    await fireEvent.click(retry);
    await waitFor(() => expect(finish).toHaveBeenCalledTimes(1));
    release();
    await new Promise((r) => setTimeout(r, 30));
    expect(finish).toHaveBeenCalledTimes(1);
  });

  it("a refusal because an earlier prompt is still open shows no failure message", async () => {
    const { mock, app } = await readyApp();
    mock.vaultQuickUnlock = async () => {
      const { IpcError } = await import("../ipc");
      throw new IpcError("quick_unlock_cancelled", "a quick unlock request is already open");
    };
    const { dialog } = await openUnlock(app);
    await within(dialog).findByRole("button", { name: "Use Touch ID" });
    await new Promise((r) => setTimeout(r, 30));
    expect(within(dialog).queryByRole("status")).toBeNull();
  });
});

describe("Security: quick unlock switch", () => {
  async function openSecurity(options: MockOptions = {}) {
    const mock = createMockIpc({ vault: "unlocked", ...options });
    const app = createApp(mock);
    render(App, { props: { app, demo: false } });
    await waitFor(() => expect(app.vault.status).toBe("unlocked"));
    await fireEvent.click(screen.getByRole("button", { name: "Credentials…" }));
    const dialog = await screen.findByRole("dialog", { name: "Credentials" });
    await within(dialog).findByRole("combobox", { name: "Lock the vault after inactivity" });
    return { mock, dialog };
  }

  it("is absent when quick unlock is not available", async () => {
    const { dialog } = await openSecurity({ quickUnlockAvailable: false });
    await new Promise((r) => setTimeout(r, 20));
    expect(within(dialog).queryByRole("switch")).toBeNull();
  });

  it("turning it on says it works from the next password unlock", async () => {
    const { mock, dialog } = await openSecurity();
    const sw = (await within(dialog).findByRole("switch", {
      name: "Unlock with Touch ID",
    })) as HTMLInputElement;
    expect(sw.checked).toBe(false);
    await fireEvent.click(sw);
    await waitFor(() => expect(sw.checked).toBe(true));
    expect((await mock.vaultQuickUnlockInfo()).enabled).toBe(true);
    expect(
      within(dialog).getByText(/from the next time you unlock with the master password/),
    ).toBeTruthy();
  });

  it("shows the hint when enabled but not ready, on reopening", async () => {
    const mock = createMockIpc({ vault: "unlocked" });
    await mock.vaultSetQuickUnlock(true);
    const app = createApp(mock);
    render(App, { props: { app, demo: false } });
    await waitFor(() => expect(app.vault.status).toBe("unlocked"));
    await fireEvent.click(screen.getByRole("button", { name: "Credentials…" }));
    const dialog = await screen.findByRole("dialog", { name: "Credentials" });
    expect(
      await within(dialog).findByText(/from the next time you unlock with the master password/),
    ).toBeTruthy();
  });

  it("the switch is disabled while its call is in flight", async () => {
    const { mock, dialog } = await openSecurity();
    let release: () => void = () => {};
    const gate = new Promise<void>((r) => (release = r));
    mock.vaultSetQuickUnlock = async () => gate;
    const sw = (await within(dialog).findByRole("switch")) as HTMLInputElement;
    await fireEvent.click(sw);
    await waitFor(() => expect(sw.disabled).toBe(true));
    release();
    await waitFor(() => expect(sw.disabled).toBe(false));
  });

  it("a failing info call shows a notice", async () => {
    const mock = createMockIpc({ vault: "unlocked" });
    mock.vaultQuickUnlockInfo = async () => {
      throw new (await import("../ipc")).IpcError("io", "boom");
    };
    const app = createApp(mock);
    render(App, { props: { app, demo: false } });
    await waitFor(() => expect(app.vault.status).toBe("unlocked"));
    await fireEvent.click(screen.getByRole("button", { name: "Credentials…" }));
    expect(await screen.findByText("Quick unlock status could not be read")).toBeTruthy();
  });

  it("turning it off takes effect at once", async () => {
    const mock = createMockIpc({ vault: "unlocked" });
    await mock.vaultSetQuickUnlock(true);
    const app = createApp(mock);
    render(App, { props: { app, demo: false } });
    await waitFor(() => expect(app.vault.status).toBe("unlocked"));
    await fireEvent.click(screen.getByRole("button", { name: "Credentials…" }));
    const dialog = await screen.findByRole("dialog", { name: "Credentials" });
    const sw = (await within(dialog).findByRole("switch")) as HTMLInputElement;
    await waitFor(() => expect(sw.checked).toBe(true));
    await fireEvent.click(sw);
    await waitFor(() => expect(sw.checked).toBe(false));
    expect((await mock.vaultQuickUnlockInfo()).enabled).toBe(false);
  });

  it("an error is shown and the switch goes back", async () => {
    const { mock, dialog } = await openSecurity();
    mock.vaultSetQuickUnlock = async () => {
      throw new (await import("../ipc")).IpcError("io", "disk full");
    };
    const sw = (await within(dialog).findByRole("switch")) as HTMLInputElement;
    await fireEvent.click(sw);
    expect(await screen.findByText("Quick unlock could not be changed")).toBeTruthy();
    await waitFor(() => expect(sw.checked).toBe(false));
  });

  it("uses the Windows Hello name on Windows", async () => {
    setPlatform("windows");
    const { dialog } = await openSecurity();
    expect(await within(dialog).findByRole("switch", { name: "Unlock with Windows Hello" })).toBeTruthy();
  });
});
