// The status bar shows the running version and a dot with the state of the update check. The dot
// opens the update dialog. Mounted through the whole shell, as the app is.

import { fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import App from "../../App.svelte";
import { createMockIpc, type MockOptions } from "../ipc/mock";
import { createApp, type App as AppState } from "../state/app";

vi.mock("@xterm/xterm", async () => ({
  Terminal: (await import("../sessions/term/fakeXterm")).FakeTerminal,
}));

const FAST = { startDelayMs: 0, intervalMs: 60_000 };
const NEVER = { startDelayMs: 60_000, intervalMs: 60_000 };

async function mountApp(options: MockOptions = {}, timing = FAST): Promise<AppState> {
  const app = createApp(createMockIpc({ vault: "unlocked", ...options }), undefined, timing);
  render(App, { props: { app, demo: false } });
  await waitFor(() => expect(app.doc.status).toBe("ready"));
  return app;
}

function bar(): HTMLElement {
  return screen.getByTestId("statusbar");
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("status bar version", () => {
  it("shows the running version with a v prefix", async () => {
    await mountApp({ updates: { currentVersion: "0.3.0" } });
    await within(bar()).findByText("v0.3.0");
  });

  it("shows no version while the settings are not loaded", async () => {
    const mock = createMockIpc({ vault: "unlocked" });
    const app = createApp(
      { ...mock, updateSettings: () => Promise.reject(new Error("no settings")) },
      undefined,
      FAST,
    );
    render(App, { props: { app, demo: false } });
    await waitFor(() => expect(app.doc.status).toBe("ready"));
    expect(bar().textContent).not.toMatch(/\bv\d/);
  });
});

describe("status bar update dot", () => {
  it("is unknown before a check has answered", async () => {
    await mountApp({}, NEVER);
    expect(within(bar()).getByRole("button", { name: "Update status unknown" })).toBeTruthy();
  });

  it("is up to date when the last check found nothing new", async () => {
    await mountApp();
    await within(bar()).findByRole("button", { name: "Up to date" });
  });

  it("is available when an update exists", async () => {
    await mountApp({ updates: { available: { version: "0.2.0" } } });
    await within(bar()).findByRole("button", { name: "Update available" });
  });

  it("opens the update dialog when clicked", async () => {
    const app = await mountApp({ updates: { autoCheck: false } }, NEVER);
    const open = vi.spyOn(app.ui, "open");
    await fireEvent.click(within(bar()).getByRole("button", { name: "Update status unknown" }));
    expect(open).toHaveBeenCalledWith({ type: "updates" });
    await screen.findByRole("dialog", { name: "Software update" });
  });
});
