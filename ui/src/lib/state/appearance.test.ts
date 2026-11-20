import { afterEach, describe, expect, it, vi } from "vitest";

import { createMockIpc } from "../ipc/mock";
import { APPEARANCE_KEY } from "../settings/appearance";
import { createApp } from "./app";

const DARK = { theme: "dark", scheme: "solarized", accent: "teal", density: "compact", terminalFontSize: 16 } as const;

afterEach(() => {
  for (const name of ["data-theme", "data-scheme", "data-accent", "data-density"]) document.documentElement.removeAttribute(name);
  window.localStorage.clear();
});

describe("appearance store", () => {
  it("aligns the page and the stored copy with settings.json on load", async () => {
    const app = createApp(createMockIpc({ appearance: DARK }));
    await app.appearance.load();
    expect(app.appearance.value).toEqual(DARK);
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
    expect(document.documentElement.getAttribute("data-scheme")).toBe("solarized");
    expect(document.documentElement.getAttribute("data-accent")).toBe("teal");
    expect(document.documentElement.getAttribute("data-density")).toBe("compact");
    expect(JSON.parse(window.localStorage.getItem(APPEARANCE_KEY) ?? "null")).toEqual(DARK);
  });

  it("applies a change at once and saves it", async () => {
    const ipc = createMockIpc();
    const app = createApp(ipc);
    const pending = app.appearance.update({ accent: "orange" });
    expect(document.documentElement.getAttribute("data-accent")).toBe("orange");
    expect(await pending).toBe(true);
    expect((await ipc.appearanceGet()).accent).toBe("orange");
  });

  it("puts the previous look back and says so when saving fails", async () => {
    const ipc = createMockIpc();
    const app = createApp({ ...ipc, appearanceSet: vi.fn(async () => Promise.reject(new Error("disk full"))) });
    expect(await app.appearance.update({ theme: "dark" })).toBe(false);
    expect(app.appearance.value.theme).toBe("system");
    expect(document.documentElement.getAttribute("data-theme")).toBeNull();
    expect(app.notices.items.map((n) => n.title)).toContain("The appearance could not be saved");
  });

  it("keeps a choice made while settings.json was being read", async () => {
    const ipc = createMockIpc({ appearance: DARK });
    let release!: () => void;
    const gate = new Promise<void>((resolve) => (release = resolve));
    const app = createApp({
      ...ipc,
      appearanceGet: async () => {
        await gate;
        return DARK;
      },
    });
    const loading = app.appearance.load();
    await app.appearance.update({ accent: "orange" });
    release();
    await loading;
    expect(app.appearance.value.accent).toBe("orange");
    expect(document.documentElement.getAttribute("data-accent")).toBe("orange");
  });

  it("keeps the stored copy and reports when the backend cannot be read", async () => {
    const ipc = createMockIpc();
    const app = createApp({ ...ipc, appearanceGet: vi.fn(async () => Promise.reject(new Error("no"))) });
    await app.appearance.load();
    expect(app.appearance.value.theme).toBe("system");
    expect(app.notices.items).toHaveLength(1);
  });
});
