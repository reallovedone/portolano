// Screenshots next to the notes, and the "Release notes" window: what it lists, how its images
// follow the look and disappear when they cannot load, and the three ways to open it.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "../../App.svelte";
import { createMockIpc, type MockIpc, type MockOptions } from "../ipc/mock";
import type { Appearance } from "../ipc";
import { createApp, type App as AppState } from "../state/app";

vi.mock("@xterm/xterm", async () => ({
  Terminal: (await import("../sessions/term/fakeXterm")).FakeTerminal,
}));

const FAST = { startDelayMs: 0, intervalMs: 60_000 };

const LIST = {
  truncated: false,
  sections: [
    { version: "0.4.0", date: "2026-11-30", notes: "### ✨ Added\n\n- A big thing" },
    { version: "0.3.1", date: "2026-11-18", notes: "### ✨ Added\n\n- Kerberos" },
    { version: "0.1.0", date: null, notes: "- First" },
  ],
};

async function mountApp(options: MockOptions = {}) {
  const mock: MockIpc = createMockIpc({ vault: "unlocked", updates: { releaseNotes: LIST }, ...options });
  const app: AppState = createApp(mock, undefined, FAST);
  render(App, { props: { app, demo: false } });
  await waitFor(() => expect(app.doc.status).toBe("ready"));
  return { app, mock };
}

function theme(choice: Appearance["theme"]): Partial<Appearance> {
  return { theme: choice };
}

function mediaQuery(dark: boolean) {
  const listeners = new Set<(event: { matches: boolean }) => void>();
  const query = {
    matches: dark,
    media: "(prefers-color-scheme: dark)",
    addEventListener: (_: string, fn: (event: { matches: boolean }) => void) => listeners.add(fn),
    removeEventListener: (_: string, fn: (event: { matches: boolean }) => void) => listeners.delete(fn),
  };
  return {
    query,
    set(matches: boolean) {
      query.matches = matches;
      for (const fn of [...listeners]) fn({ matches });
    },
    get listening() {
      return listeners.size;
    },
  };
}

async function openReleaseNotes(options: MockOptions = {}) {
  const ctx = await mountApp(options);
  ctx.mock.simulate.releaseNotesMenu();
  const dialog = await screen.findByRole("dialog", { name: "Release notes" });
  await within(dialog).findByTestId("release-notes-list");
  return { ...ctx, dialog };
}

const srcOf = (img: HTMLElement): string | null => img.getAttribute("src");

beforeEach(() => {
  const data = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (k: string) => data.get(k) ?? null,
    setItem: (k: string, v: string) => {
      data.set(k, v);
    },
  });
  vi.stubGlobal("matchMedia", () => mediaQuery(false).query);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  for (const name of ["data-theme", "data-scheme", "data-accent", "data-density"]) document.documentElement.removeAttribute(name);
});

describe("Release notes window", () => {
  it("lists every version, newest first, with the screenshots of the versions that have them", async () => {
    const { dialog } = await openReleaseNotes();
    const sections = within(dialog).getAllByTestId("release-section");
    expect(sections.map((s) => s.getAttribute("data-version"))).toEqual(["0.4.0", "0.3.1", "0.1.0"]);
    expect(within(dialog).getByRole("heading", { name: "0.4.0 — 30 Nov 2026" })).toBeTruthy();
    expect(within(dialog).getByRole("heading", { name: "0.1.0" })).toBeTruthy();
    const first = sections[0] as HTMLElement;
    const images = within(first).getAllByRole("img");
    expect(images.map(srcOf)).toEqual([
      "https://portolano.cloud/whats-new/0.4.0/settings-light.webp",
      "https://portolano.cloud/whats-new/0.4.0/connections-light.webp",
    ]);
    expect(images.map((i) => i.getAttribute("alt"))).toEqual([
      "Settings: colour schemes, accent colour and density",
      "Icons and colours in the connection list and on tabs",
    ]);
    for (const img of images) {
      expect(img.getAttribute("loading")).toBe("lazy");
      expect(img.getAttribute("decoding")).toBe("async");
      expect(img.getAttribute("width")).toBe("1600");
      expect(img.getAttribute("height")).toBe("1000");
    }
    expect(within(first).getAllByText("Icons and colours in the connection list and on tabs")).toHaveLength(1);
    // Versions without screenshots show none.
    expect(within(sections[1] as HTMLElement).queryByRole("img")).toBeNull();
    expect(within(sections[2] as HTMLElement).queryByRole("img")).toBeNull();
  });

  it("closes with Escape", async () => {
    const { dialog } = await openReleaseNotes();
    await fireEvent.keyDown(dialog, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("opens the changelog on GitHub from the footer", async () => {
    const { dialog, mock } = await openReleaseNotes();
    await fireEvent.click(within(dialog).getByRole("button", { name: "View on GitHub" }));
    await waitFor(() => expect(mock.updates.openedChangelogs).toBe(1));
    const close = within(dialog).getAllByRole("button", { name: "Close" }).at(-1);
    if (!close) throw new Error("no Close button");
    await fireEvent.click(close);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("says when older versions were left out", async () => {
    const { dialog, mock } = await openReleaseNotes({ updates: { releaseNotes: { ...LIST, truncated: true } } });
    expect(within(dialog).getByText(/Older versions are not shown here/)).toBeTruthy();
    await fireEvent.click(within(dialog).getByRole("button", { name: "See the full changelog on GitHub" }));
    await waitFor(() => expect(mock.updates.openedChangelogs).toBe(1));
  });

  it("tells when the notes cannot be loaded", async () => {
    const mock = createMockIpc({ vault: "unlocked", updates: { releaseNotesError: "no changelog" } });
    const app = createApp(mock, undefined, FAST);
    render(App, { props: { app, demo: false } });
    await waitFor(() => expect(app.doc.status).toBe("ready"));
    mock.simulate.releaseNotesMenu();
    const dialog = await screen.findByRole("dialog", { name: "Release notes" });
    const alert = await within(dialog).findByTestId("release-notes-error");
    expect(alert.textContent).toContain("The release notes could not be loaded.");
    expect(alert.textContent).toContain("no changelog");
  });

  it("never turns notes into markup or images", async () => {
    const { dialog } = await openReleaseNotes({
      updates: {
        releaseNotes: {
          truncated: false,
          sections: [{ version: "0.3.0", date: null, notes: '![x](https://evil.example/a.png)\n\n<img src="https://evil.example/b.png">' }],
        },
      },
    });
    expect(dialog.querySelector("img")).toBeNull();
    expect(within(dialog).getByTestId("release-notes").textContent).toContain("https://evil.example/b.png");
  });
});

describe("Release notes entry points", () => {
  it("the Help menu item opens it", async () => {
    const { app } = await openReleaseNotes();
    expect(app.ui.dialog?.type).toBe("releaseNotes");
  });

  it("the menu item does not stack a second one", async () => {
    const { mock, app } = await openReleaseNotes();
    mock.simulate.releaseNotesMenu();
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    expect(app.notices.items).toEqual([]);
  });

  it("the menu item leaves another open dialog alone and says so", async () => {
    const { mock, app } = await mountApp();
    app.ui.open({ type: "credentials" });
    mock.simulate.releaseNotesMenu();
    expect(app.ui.dialog?.type).toBe("credentials");
    expect(app.notices.items.map((n) => n.title)).toEqual([
      "Close the open dialog, then choose “Release Notes…” again.",
    ]);
  });

  it("the menu item does not open over What's new", async () => {
    const { mock, app } = await mountApp({
      updates: { whatsNew: { version: "0.4.0", truncated: false, sections: LIST.sections } },
    });
    await screen.findByRole("dialog", { name: "What's new in 0.4.0" });
    mock.simulate.releaseNotesMenu();
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    expect(app.ui.dialog).toBeNull();
  });

  it("Settings shows the version and a Release notes button that replaces it", async () => {
    const { app } = await mountApp({ updates: { currentVersion: "0.4.0", releaseNotes: LIST } });
    await waitFor(() => expect(app.updates.settings?.currentVersion).toBe("0.4.0"));
    app.ui.open({ type: "settings" });
    const settings = await screen.findByRole("dialog", { name: "Settings" });
    expect(within(settings).getByTestId("settings-updates").textContent).toContain("Portolano 0.4.0");
    await fireEvent.click(within(settings).getByRole("button", { name: "Release notes" }));
    const notes = await screen.findByRole("dialog", { name: "Release notes" });
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    await within(notes).findByTestId("release-notes-list");
    expect(app.ui.dialog?.type).toBe("releaseNotes");
  });

  it("What's new links to it, instead of the web page, when it left older versions out", async () => {
    const { mock, app } = await mountApp({
      updates: { whatsNew: { version: "0.4.0", truncated: true, sections: LIST.sections.slice(0, 1) } },
    });
    const whatsNew = await screen.findByRole("dialog", { name: "What's new in 0.4.0" });
    await fireEvent.click(within(whatsNew).getByRole("button", { name: "See all release notes" }));
    const notes = await screen.findByRole("dialog", { name: "Release notes" });
    await within(notes).findByTestId("release-notes-list");
    expect(screen.queryByRole("dialog", { name: /What's new/ })).toBeNull();
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    expect(mock.updates.openedChangelogs).toBe(0);
    expect(app.updates.whatsNew).toBeNull();
  });

  it("What's new falls back to the web page when another dialog is open", async () => {
    const { mock, app } = await mountApp({
      updates: { whatsNew: { version: "0.4.0", truncated: true, sections: LIST.sections.slice(0, 1) } },
    });
    const whatsNew = await screen.findByRole("dialog", { name: "What's new in 0.4.0" });
    app.ui.open({ type: "credentials" });
    await fireEvent.click(within(whatsNew).getByRole("button", { name: "See all release notes" }));
    await waitFor(() => expect(mock.updates.openedChangelogs).toBe(1));
    expect(app.ui.dialog?.type).toBe("credentials");
  });
});

describe("screenshots in What's new", () => {
  const whatsNew = { version: "0.4.0", truncated: false, sections: LIST.sections };

  it("show under the heading of their version, before its notes", async () => {
    await mountApp({ updates: { whatsNew } });
    const dialog = await screen.findByRole("dialog", { name: "What's new in 0.4.0" });
    const section = within(dialog).getAllByTestId("release-section")[0] as HTMLElement;
    const children = [...section.children].map((c) => c.tagName + (c.getAttribute("data-testid") ?? ""));
    expect(children).toEqual(["H3", "FIGURErelease-shot", "FIGURErelease-shot", "DIVrelease-notes"]);
  });

  it("are not requested when no dialog that shows them is open", async () => {
    await mountApp();
    expect(document.querySelector("img")).toBeNull();
  });
});

describe("screenshots follow the look", () => {
  it("use the dark images when the theme is dark and the light ones when it is light", async () => {
    const { app } = await openReleaseNotes({ appearance: { theme: "dark", scheme: "portolano", accent: "blue", density: "comfortable", terminalFontSize: 13 } });
    const urls = () => screen.getAllByRole("img").map(srcOf);
    expect(urls()).toEqual([
      "https://portolano.cloud/whats-new/0.4.0/settings-dark.webp",
      "https://portolano.cloud/whats-new/0.4.0/connections-dark.webp",
    ]);
    await app.appearance.update(theme("light"));
    await waitFor(() => expect(urls()[0]).toBe("https://portolano.cloud/whats-new/0.4.0/settings-light.webp"));
    expect(urls()[1]).toBe("https://portolano.cloud/whats-new/0.4.0/connections-light.webp");
  });

  it("follow the system when the theme is System, and switch when it changes while open", async () => {
    const media = mediaQuery(false);
    vi.stubGlobal("matchMedia", () => media.query);
    const { app } = await openReleaseNotes();
    expect(app.appearance.value.theme).toBe("system");
    const urls = () => screen.getAllByRole("img").map(srcOf);
    expect(urls()[0]).toBe("https://portolano.cloud/whats-new/0.4.0/settings-light.webp");
    media.set(true);
    await waitFor(() => expect(urls()[0]).toBe("https://portolano.cloud/whats-new/0.4.0/settings-dark.webp"));
    media.set(false);
    await waitFor(() => expect(urls()[0]).toBe("https://portolano.cloud/whats-new/0.4.0/settings-light.webp"));
  });

  it("stop listening to the system when the window closes", async () => {
    const media = mediaQuery(false);
    vi.stubGlobal("matchMedia", () => media.query);
    const { app } = await openReleaseNotes();
    expect(media.listening).toBeGreaterThan(0);
    app.ui.closeDialog();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(media.listening).toBe(0);
  });

  it("use the light images when the system has no way to tell", async () => {
    vi.stubGlobal("matchMedia", undefined);
    await openReleaseNotes();
    expect(srcOf(screen.getAllByRole("img")[0] as HTMLElement)).toContain("settings-light.webp");
  });
});

describe("screenshots that fail to load", () => {
  it("remove their figure: no broken image, no empty frame, no caption", async () => {
    const { dialog } = await openReleaseNotes();
    const [settings] = within(dialog).getAllByRole("img");
    if (!settings) throw new Error("no image");
    await fireEvent.error(settings);
    await waitFor(() => expect(within(dialog).getAllByTestId("release-shot")).toHaveLength(1));
    expect(within(dialog).queryByText("Settings: colour schemes, accent colour and density")).toBeNull();
    expect(within(dialog).getAllByRole("img")).toHaveLength(1);
    const [connections] = within(dialog).getAllByRole("img");
    if (!connections) throw new Error("no image");
    await fireEvent.error(connections);
    await waitFor(() => expect(within(dialog).queryByTestId("release-shot")).toBeNull());
    // The notes are still there.
    expect(within(dialog).getByText("A big thing")).toBeTruthy();
  });

  it("try again when the look changes to one that did not fail", async () => {
    const { dialog, app } = await openReleaseNotes();
    const [first] = within(dialog).getAllByRole("img");
    if (!first) throw new Error("no image");
    await fireEvent.error(first);
    await waitFor(() => expect(within(dialog).getAllByTestId("release-shot")).toHaveLength(1));
    await app.appearance.update(theme("dark"));
    await waitFor(() => expect(within(dialog).getAllByTestId("release-shot")).toHaveLength(2));
    expect(srcOf(within(dialog).getAllByRole("img")[0] as HTMLElement)).toContain("settings-dark.webp");
  });
});
