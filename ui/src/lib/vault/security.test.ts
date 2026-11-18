// The "Security" section of the credential manager: the vault auto-lock time.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "../../App.svelte";
import { IpcError } from "../ipc";
import { createMockIpc } from "../ipc/mock";
import { createApp } from "../state/app";

vi.mock("@xterm/xterm", async () => ({
  Terminal: (await import("../sessions/term/fakeXterm")).FakeTerminal,
}));

async function openSecurity(mock = createMockIpc({ vault: "unlocked", autoLockMinutes: 30 })) {
  const app = createApp(mock);
  render(App, { props: { app, demo: false } });
  await waitFor(() => expect(app.vault.status).toBe("unlocked"));
  await fireEvent.click(screen.getByRole("button", { name: "Credentials…" }));
  const dialog = await screen.findByRole("dialog", { name: "Credentials" });
  const select = (await within(dialog).findByRole("combobox", {
    name: "Lock the vault after inactivity",
  })) as HTMLSelectElement;
  return { mock, dialog, select };
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

describe("Security: auto-lock time", () => {
  it("offers the five choices with the saved one selected", async () => {
    const { dialog, select } = await openSecurity();
    expect(within(dialog).getByRole("heading", { name: "Security" })).toBeTruthy();
    const labels = Array.from(select.options).map((o) => o.textContent);
    expect(labels).toEqual(["Never", "5 minutes", "15 minutes", "30 minutes", "1 hour"]);
    await waitFor(() => expect(select.value).toBe("30"));
  });

  it("saves a new choice through vaultSetAutoLock", async () => {
    const { mock, select } = await openSecurity();
    const spy = vi.spyOn(mock, "vaultSetAutoLock");
    await waitFor(() => expect(select.value).toBe("30"));
    await fireEvent.change(select, { target: { value: "0" } });
    await waitFor(() => expect(spy).toHaveBeenCalledWith(0));
    expect(await mock.vaultGetAutoLock()).toBe(0);
    expect(select.value).toBe("0");
  });

  it("shows an error and goes back to the previous choice when saving fails", async () => {
    const { mock, select } = await openSecurity();
    mock.vaultSetAutoLock = async () => {
      throw new IpcError("io", "disk full");
    };
    await waitFor(() => expect(select.value).toBe("30"));
    await fireEvent.change(select, { target: { value: "5" } });
    await waitFor(() => expect(select.value).toBe("30"));
    expect(await screen.findByText("The auto-lock time could not be changed")).toBeTruthy();
    expect(await mock.vaultGetAutoLock()).toBe(30);
  });
});
