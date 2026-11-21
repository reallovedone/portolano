// Update flows through the whole shell: start-up order, the offer dialog and its three choices,
// installation progress and failure, the manual check, the automatic-check setting, and
// "What's new". The backend is the mock (no network, nothing is installed).

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "../../App.svelte";
import { MOCK_IDS, createMockIpc, type MockIpc, type MockOptions } from "../ipc/mock";
import type { MockUpdateOptions } from "../ipc/mock-updates";
import { createApp, type App as AppState } from "../state/app";
import { NoticeStore } from "../state/notices.svelte";
import { UpdateStore } from "../state/updates.svelte";

vi.mock("@xterm/xterm", async () => ({
  Terminal: (await import("../sessions/term/fakeXterm")).FakeTerminal,
}));

const NOTES = "## What's Changed\n- Automatic updates\n- **Faster** start";
const FAST = { startDelayMs: 0, intervalMs: 60_000 };

async function mountApp(options: MockOptions = {}, timing = FAST) {
  const mock: MockIpc = createMockIpc({ vault: "unlocked", ...options });
  const app: AppState = createApp(mock, undefined, timing);
  render(App, { props: { app, demo: false } });
  await waitFor(() => expect(app.doc.status).toBe("ready"));
  return { app, mock };
}

const offered = { version: "0.2.0" };

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
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

async function openCheckDialog(): Promise<void> {
  await fireEvent.click(screen.getByRole("button", { name: "More actions" }));
  await fireEvent.click(await screen.findByRole("menuitem", { name: "Check for updates…" }));
}

describe("automatic update offer", () => {
  it("shows current to new version, a release notes link and the three choices", async () => {
    const { mock } = await mountApp({ updates: { available: offered } });
    const dialog = await screen.findByRole("dialog", { name: "Update available" });
    expect(within(dialog).getByTestId("update-versions").textContent).toBe(
      "Portolano 0.1.0 → 0.2.0",
    );
    expect(within(dialog).getByRole("button", { name: "Release notes" })).toBeTruthy();
    expect(within(dialog).queryByTestId("release-notes")).toBeNull();
    expect(within(dialog).getByRole("button", { name: "Update now" })).toBeTruthy();
    expect(within(dialog).getByRole("button", { name: "Later" })).toBeTruthy();
    expect(within(dialog).getByRole("button", { name: "Skip this version" })).toBeTruthy();
    expect(mock.updates.checks).toEqual([false]);
    expect(mock.updates.installs).toBe(0);
  });

  it("does not offer anything when up to date", async () => {
    const { mock } = await mountApp();
    await waitFor(() => expect(mock.updates.checks).toEqual([false]));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("'Later' closes the dialog and the next scheduled check offers it again", async () => {
    const { mock } = await mountApp(
      { updates: { available: offered } },
      { startDelayMs: 0, intervalMs: 50 },
    );
    await fireEvent.click(await screen.findByRole("button", { name: "Later" }));
    expect(screen.queryByRole("dialog", { name: "Update available" })).toBeNull();
    expect(mock.updates.skipped).toEqual([]);
    await screen.findByRole("dialog", { name: "Update available" });
    expect(mock.updates.checks.length).toBeGreaterThanOrEqual(2);
  });

  it("'Skip this version' is remembered and not proposed again automatically", async () => {
    const { app, mock } = await mountApp(
      { updates: { available: offered } },
      { startDelayMs: 0, intervalMs: 50 },
    );
    await fireEvent.click(await screen.findByRole("button", { name: "Skip this version" }));
    expect(mock.updates.skipped).toEqual(["0.2.0"]);
    expect(screen.queryByRole("dialog", { name: "Update available" })).toBeNull();
    await waitFor(() => expect(mock.updates.checks.length).toBeGreaterThanOrEqual(3));
    expect(screen.queryByRole("dialog", { name: "Update available" })).toBeNull();
    expect(app.updates.offer).toBeNull();
  });

  it("a manual check still shows a skipped version", async () => {
    const { mock } = await mountApp({ updates: { available: offered } });
    await fireEvent.click(await screen.findByRole("button", { name: "Skip this version" }));
    await openCheckDialog();
    const dialog = await screen.findByRole("dialog", { name: "Update available" });
    expect(within(dialog).getByTestId("update-versions").textContent).toContain("0.2.0");
    expect(mock.updates.checks.at(-1)).toBe(true);
  });

  it("does not check when 'Automatically check for updates' is off", async () => {
    const { mock } = await mountApp(
      { updates: { available: offered, autoCheck: false } },
      { startDelayMs: 0, intervalMs: 20 },
    );
    await new Promise((resolve) => setTimeout(resolve, 120));
    expect(mock.updates.checks).toEqual([]);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("waits for another open dialog before interrupting", async () => {
    const { app } = await mountApp({ updates: { available: offered } }, {
      startDelayMs: 60,
      intervalMs: 60_000,
    });
    app.ui.open({ type: "folder", id: null, parentId: null });
    await screen.findByRole("dialog");
    await new Promise((resolve) => setTimeout(resolve, 150));
    await waitFor(() => expect(app.updates.offer).not.toBeNull());
    expect(screen.queryByRole("dialog", { name: "Update available" })).toBeNull();
    app.ui.closeDialog();
    await screen.findByRole("dialog", { name: "Update available" });
  });
});

describe("installation", () => {
  it("downloads with progress, installs and restarts", async () => {
    const { mock } = await mountApp({ updates: { available: offered } });
    mock.updates.holdInstall = true;
    await fireEvent.click(await screen.findByRole("button", { name: "Update now" }));
    const bar = await screen.findByRole("progressbar", { name: "Download progress" });
    expect((bar as HTMLProgressElement).value).toBe(512);
    expect((bar as HTMLProgressElement).max).toBe(2048);
    expect(screen.getByText("Downloading the update… 25%")).toBeTruthy();
    // No choices while it runs.
    expect(screen.queryByRole("button", { name: "Later" })).toBeNull();
    mock.updates.releaseInstall();
    await screen.findByText("Restarting Portolano…");
    expect(mock.updates.installs).toBe(1);
  });

  it("shows a failure clearly and allows trying again", async () => {
    const { mock } = await mountApp({
      updates: { available: offered, installError: "the update could not be verified" },
    });
    await fireEvent.click(await screen.findByRole("button", { name: "Update now" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("The update was not installed");
    expect(alert.textContent).toContain("the update could not be verified (internal)");
    mock.updates.options.installError = undefined;
    await fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    await screen.findByText("Restarting Portolano…");
    expect(mock.updates.installs).toBe(2);
  });

  it("warns that open sessions are closed", async () => {
    const { app } = await mountApp(
      { updates: { available: offered }, terminal: true },
      { startDelayMs: 100, intervalMs: 60_000 },
    );
    await app.tabs.openConnection(MOCK_IDS.localZsh);
    expect(app.tabs.tabs.length).toBe(1);
    const dialog = await screen.findByRole("dialog", { name: "Update available" });
    expect(within(dialog).getByText(/Open sessions will be closed/)).toBeTruthy();
  });

  it("does not warn about sessions when only the Home tab is open", async () => {
    const { app } = await mountApp(
      { updates: { available: offered } },
      { startDelayMs: 100, intervalMs: 60_000 },
    );
    app.tabs.openHome();
    const dialog = await screen.findByRole("dialog", { name: "Update available" });
    expect(within(dialog).queryByText(/Open sessions will be closed/)).toBeNull();
  });
});

describe("release notes before installing", () => {
  it("are not shown or fetched: only a link to the release page", async () => {
    const { mock } = await mountApp({ updates: { available: offered } });
    const dialog = await screen.findByRole("dialog", { name: "Update available" });
    expect(dialog.querySelector("[data-testid='release-notes']")).toBeNull();
    await fireEvent.click(within(dialog).getByRole("button", { name: "Release notes" }));
    await waitFor(() => expect(mock.updates.openedPages).toEqual(["0.2.0"]));
  });
});

describe("Check for updates…", () => {
  it("says the application is up to date", async () => {
    const { mock } = await mountApp();
    await openCheckDialog();
    const dialog = await screen.findByRole("dialog", { name: "Software update" });
    await within(dialog).findByText("You're up to date.");
    expect(within(dialog).getByText("Portolano 0.1.0 is the latest version.")).toBeTruthy();
    expect(within(dialog).getByText("Portolano 0.1.0")).toBeTruthy();
    expect(mock.updates.checks.at(-1)).toBe(true);
  });

  it("shows network errors and can check again", async () => {
    const { mock } = await mountApp({ updates: { checkError: "could not reach the update server" } });
    await openCheckDialog();
    const dialog = await screen.findByRole("dialog", { name: "Software update" });
    const alert = await within(dialog).findByRole("alert");
    expect(alert.textContent).toContain("Could not check for updates");
    expect(alert.textContent).toContain("could not reach the update server");
    mock.updates.options.checkError = undefined;
    await fireEvent.click(within(dialog).getByRole("button", { name: "Check for updates" }));
    await within(dialog).findByText("You're up to date.");
  });

  it("hands over to the offer when an update exists", async () => {
    await mountApp({ updates: { available: offered, autoCheck: false } });
    await openCheckDialog();
    await screen.findByRole("dialog", { name: "Update available" });
    expect(screen.queryByRole("dialog", { name: "Software update" })).toBeNull();
  });

  it("turns the automatic check off and on and keeps manual checks working", async () => {
    const { mock } = await mountApp();
    await openCheckDialog();
    const dialog = await screen.findByRole("dialog", { name: "Software update" });
    const toggle = within(dialog).getByRole("switch", { name: "Automatically check for updates" });
    expect((toggle as HTMLInputElement).checked).toBe(true);
    await fireEvent.click(toggle);
    await waitFor(() => expect(mock.updates.settings.autoCheck).toBe(false));
    await fireEvent.click(within(dialog).getByRole("button", { name: "Check for updates" }));
    await within(dialog).findByText("You're up to date.");
    expect(mock.updates.checks.at(-1)).toBe(true);
    await fireEvent.click(toggle);
    await waitFor(() => expect(mock.updates.settings.autoCheck).toBe(true));
  });
});

describe("What's new", () => {
  const whatsNew = {
    version: "0.3.2",
    truncated: false,
    sections: [
      { version: "0.3.2", date: "2026-12-01", notes: NOTES },
      { version: "0.3.1", date: "2026-11-18", notes: "### Added\n\n- Kerberos sign-in" },
      { version: "0.3.0-rc", date: null, notes: "- Release candidate item" },
    ],
  };

  it("shows every version with its title and notes, and closes", async () => {
    await mountApp({ updates: { whatsNew } });
    const dialog = await screen.findByRole("dialog", { name: "What's new in 0.3.2" });
    const sections = within(dialog).getAllByTestId("release-section");
    expect(sections.map((x) => x.getAttribute("data-version"))).toEqual([
      "0.3.2",
      "0.3.1",
      "0.3.0-rc",
    ]);
    expect(within(dialog).getByRole("heading", { name: "0.3.2 — 1 Dec 2026" })).toBeTruthy();
    expect(within(dialog).getByRole("heading", { name: "0.3.1 — 18 Nov 2026" })).toBeTruthy();
    expect(within(dialog).getByRole("heading", { name: "0.3.0-rc" })).toBeTruthy();
    expect(within(dialog).getByText("Automatic updates")).toBeTruthy();
    expect(within(dialog).getByText("Kerberos sign-in")).toBeTruthy();
    expect(within(dialog).queryByTestId("whats-new-more")).toBeNull();
    const close = within(dialog).getAllByRole("button", { name: "Close" }).at(-1);
    if (!close) throw new Error("no Close button");
    await fireEvent.click(close);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("never turns notes into markup", async () => {
    await mountApp({
      updates: {
        whatsNew: {
          version: "0.3.2",
          truncated: false,
          sections: [
            {
              version: "0.3.2",
              date: null,
              notes: '<img src=x onerror="alert(1)">\n\n<script>alert(2)</script>\n\n[a](javascript:alert(3))',
            },
          ],
        },
      },
    });
    const dialog = await screen.findByRole("dialog", { name: "What's new in 0.3.2" });
    expect(dialog.querySelector("img")).toBeNull();
    expect(dialog.querySelector("script")).toBeNull();
    expect(dialog.querySelector("a")).toBeNull();
    expect(within(dialog).getByTestId("release-notes").textContent).toContain(
      '<img src=x onerror="alert(1)">',
    );
  });

  it("links to the Release notes window when older versions were left out", async () => {
    await mountApp({ updates: { whatsNew: { ...whatsNew, truncated: true } } });
    const dialog = await screen.findByRole("dialog", { name: "What's new in 0.3.2" });
    expect(within(dialog).getByText(/Older versions are not shown here/)).toBeTruthy();
    await fireEvent.click(
      within(dialog).getByRole("button", { name: "See all release notes" }),
    );
    await screen.findByRole("dialog", { name: "Release notes" });
    expect(screen.queryByRole("dialog", { name: "What's new in 0.3.2" })).toBeNull();
  });

  it("comes before the update dialog, which follows once it is closed", async () => {
    const { app } = await mountApp({ updates: { whatsNew, available: { version: "0.4.0" } } });
    await screen.findByRole("dialog", { name: "What's new in 0.3.2" });
    await waitFor(() => expect(app.updates.offer).not.toBeNull());
    expect(screen.queryByRole("dialog", { name: "Update available" })).toBeNull();
    app.updates.dismissWhatsNew();
    await screen.findByRole("dialog", { name: "Update available" });
  });

  it("a failure is silent: nothing is shown", async () => {
    const mock = createMockIpc({ vault: "unlocked" });
    const failing = {
      ...mock,
      whatsNew: () => Promise.reject(new Error("no network")),
    };
    const app = createApp(failing, undefined, FAST);
    render(App, { props: { app, demo: false } });
    await waitFor(() => expect(app.doc.status).toBe("ready"));
    await waitFor(() => expect(mock.updates.checks).toEqual([false]));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("freshness of the update status", () => {
  function store(updates: MockUpdateOptions = {}) {
    const mock = createMockIpc({ vault: "unlocked", updates });
    return { mock, store: new UpdateStore(mock, new NoticeStore(), FAST) };
  }

  it("starts unknown, before any check", () => {
    const { store: updates } = store();
    expect(updates.freshness).toBe("unknown");
  });

  it("is upToDate after a scheduled check that finds nothing new", async () => {
    const { store: updates } = store();
    await updates.checkInBackground();
    expect(updates.freshness).toBe("upToDate");
  });

  it("is available when a scheduled check finds a version", async () => {
    const { store: updates } = store({ available: offered });
    await updates.checkInBackground();
    expect(updates.freshness).toBe("available");
  });

  it("stays available for a skipped version", async () => {
    const { mock, store: updates } = store();
    await updates.checkInBackground();
    expect(updates.freshness).toBe("upToDate");
    mock.updates.options.available = offered;
    mock.updates.skipped.push(offered.version);
    await updates.checkInBackground();
    expect(updates.freshness).toBe("available");
  });

  it("becomes unknown when the backend answers disabled", async () => {
    const { mock, store: updates } = store();
    await updates.checkInBackground();
    expect(updates.freshness).toBe("upToDate");
    mock.updates.settings = { ...mock.updates.settings, autoCheck: false };
    await updates.checkInBackground();
    expect(updates.freshness).toBe("unknown");
  });

  it("keeps the previous value when a scheduled check fails", async () => {
    const { mock, store: updates } = store({ available: offered });
    await updates.checkInBackground();
    mock.updates.options.checkError = "could not reach the update server";
    await updates.checkInBackground();
    expect(updates.freshness).toBe("available");
  });

  it("keeps the previous value when a manual check fails", async () => {
    const { mock, store: updates } = store();
    await updates.checkNow();
    expect(updates.freshness).toBe("upToDate");
    mock.updates.options.checkError = "could not reach the update server";
    await expect(updates.checkNow()).rejects.toMatchObject({ code: "host_unreachable" });
    expect(updates.freshness).toBe("upToDate");
  });

  it("follows the answer of a manual check", async () => {
    const { mock, store: updates } = store();
    await updates.checkNow();
    expect(updates.freshness).toBe("upToDate");
    mock.updates.options.available = offered;
    await updates.checkNow();
    expect(updates.freshness).toBe("available");
  });

  it("is unknown after automatic checks are turned off, and a manual check sets it again", async () => {
    const { store: updates } = store();
    await updates.checkInBackground();
    expect(updates.freshness).toBe("upToDate");
    expect(await updates.setAutoCheck(false)).toBe(true);
    expect(updates.freshness).toBe("unknown");
    await updates.checkNow();
    expect(updates.freshness).toBe("upToDate");
  });
});
