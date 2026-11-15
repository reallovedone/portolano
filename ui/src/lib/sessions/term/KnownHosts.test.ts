import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import { createMockIpc } from "../../ipc/mock";
import { createApp } from "../../state/app";
import KnownHostsHarness from "./KnownHostsHarness.svelte";

afterEach(() => cleanup());

describe("KnownHosts", () => {
  it("lists entries with monospace fingerprints and removes one only after confirming", async () => {
    const base = createMockIpc({ vault: "unlocked", terminal: true });
    const knownHostsRemove = vi.fn((h: string, p: number) => base.knownHostsRemove(h, p));
    const app = createApp({ ...base, knownHostsRemove });
    render(KnownHostsHarness, { props: { app } });
    const rows = await screen.findAllByTestId("known-host");
    expect(rows.length).toBeGreaterThan(0);
    expect(rows[0]?.querySelector("code")?.textContent).toBeTruthy();

    const first = rows[0]?.querySelector("button");
    if (!first) throw new Error("no remove button");
    await fireEvent.click(first);
    // Nothing is removed yet: the confirmation comes first, with cancel as the default.
    const dialog = await screen.findByRole("dialog", { name: "Remove stored key?" });
    expect(knownHostsRemove).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Cancel" }));
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(dialog.isConnected).toBe(false));
    expect(knownHostsRemove).not.toHaveBeenCalled();

    await fireEvent.click(first);
    await fireEvent.click(await screen.findByRole("button", { name: "Remove" }));
    await waitFor(() => expect(knownHostsRemove).toHaveBeenCalledTimes(1));
  });
});
