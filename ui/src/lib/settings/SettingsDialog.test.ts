import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import App from "../../App.svelte";
import { createMockIpc } from "../ipc/mock";
import { createApp, type App as AppState } from "../state/app";

vi.mock("@xterm/xterm", async () => ({
  Terminal: (await import("../sessions/term/fakeXterm")).FakeTerminal,
}));

async function openSettings(ipc = createMockIpc({ vault: "unlocked" })): Promise<{ app: AppState; ipc: typeof ipc }> {
  const app = createApp(ipc);
  render(App, { props: { app, demo: false } });
  await waitFor(() => expect(app.doc.status).toBe("ready"));
  app.ui.open({ type: "settings" });
  await screen.findByRole("dialog", { name: "Settings" });
  return { app, ipc };
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  for (const name of ["data-theme", "data-scheme", "data-accent", "data-density"]) document.documentElement.removeAttribute(name);
  window.localStorage.clear();
});

describe("SettingsDialog", () => {
  it("shows the saved appearance", async () => {
    await openSettings(
      createMockIpc({
        vault: "unlocked",
        appearance: { theme: "dark", scheme: "nord", accent: "purple", density: "compact", terminalFontSize: 18 },
      }),
    );
    await waitFor(() => expect((screen.getByRole("radio", { name: "Purple" }) as HTMLInputElement).checked).toBe(true));
    expect((screen.getByRole("radio", { name: "Dark" }) as HTMLInputElement).checked).toBe(true);
    expect((screen.getByRole("radio", { name: "Nord" }) as HTMLInputElement).checked).toBe(true);
    expect((screen.getByRole("radio", { name: "Compact" }) as HTMLInputElement).checked).toBe(true);
    expect((screen.getByRole("slider", { name: "Terminal font size" }) as HTMLInputElement).value).toBe("18");
    expect(screen.getByRole("img", { name: "Preview of the terminal text" }).style.fontSize).toBe("18px");
  });

  it("applies each choice at once and saves it", async () => {
    const { app, ipc } = await openSettings();
    await fireEvent.click(screen.getByRole("radio", { name: "Dark" }));
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
    await fireEvent.click(screen.getByRole("radio", { name: "Solarized" }));
    expect(document.documentElement.getAttribute("data-scheme")).toBe("solarized");
    await fireEvent.click(screen.getByRole("radio", { name: "Orange" }));
    expect(document.documentElement.getAttribute("data-accent")).toBe("orange");
    await fireEvent.click(screen.getByRole("radio", { name: "Compact" }));
    expect(document.documentElement.getAttribute("data-density")).toBe("compact");
    const slider = screen.getByRole("slider", { name: "Terminal font size" });
    await fireEvent.input(slider, { target: { value: "20" } });
    // Dragging shows the size in the preview at once, before anything is saved.
    expect(screen.getByRole("img", { name: "Preview of the terminal text" }).style.fontSize).toBe("20px");
    expect(app.appearance.value.terminalFontSize).toBe(13);
    await fireEvent.change(slider, { target: { value: "20" } });
    await waitFor(async () =>
      expect(await ipc.appearanceGet()).toEqual({
        theme: "dark",
        scheme: "solarized",
        accent: "orange",
        density: "compact",
        terminalFontSize: 20,
      }),
    );
    expect(app.appearance.value.terminalFontSize).toBe(20);
  });

  it("groups every choice as a radio group with the options of the design", async () => {
    await openSettings();
    const names = (group: string): string[] =>
      within(screen.getByRole("radiogroup", { name: group }))
        .getAllByRole("radio")
        .map((radio) => (radio as HTMLInputElement).closest("label")?.querySelector(".caption")?.textContent?.trim() ?? "");
    expect(names("Theme")).toEqual(["System", "Light", "Dark"]);
    expect(names("Colour scheme")).toEqual(["Portolano", "Nord", "Solarized", "Rosé Pine"]);
    expect(names("Accent colour")).toEqual(["Blue", "Teal", "Purple", "Orange", "Graphite"]);
    expect(names("List density")).toEqual(["Comfortable", "Compact"]);
  });

  it("previews each colour scheme with its own tokens", async () => {
    await openSettings();
    const group = screen.getByRole("radiogroup", { name: "Colour scheme" });
    const previews = [...group.querySelectorAll("[data-scheme]")].map((el) => el.getAttribute("data-scheme"));
    expect(previews).toEqual(["portolano", "nord", "solarized", "rose-pine"]);
  });

  it("marks the chosen option and nothing else", async () => {
    await openSettings();
    const group = screen.getByRole("radiogroup", { name: "Theme" });
    expect(group.querySelectorAll(".check")).toHaveLength(1);
    await fireEvent.click(within(group).getByRole("radio", { name: "Light" }));
    expect(group.querySelector(".selected")?.textContent).toContain("Light");
    expect(group.querySelectorAll(".check")).toHaveLength(1);
  });

  it("closes with the Close button", async () => {
    await openSettings();
    await fireEvent.click(within(screen.getByRole("dialog", { name: "Settings" })).getAllByRole("button", { name: "Close" }).at(-1) as HTMLElement);
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Settings" })).toBeNull());
  });

  it("opens from the ⋯ menu", async () => {
    const app = createApp(createMockIpc({ vault: "unlocked" }));
    render(App, { props: { app, demo: false } });
    await waitFor(() => expect(app.doc.status).toBe("ready"));
    await fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    await fireEvent.click(await screen.findByRole("menuitem", { name: /Settings…/ }));
    expect(await screen.findByRole("dialog", { name: "Settings" })).toBeTruthy();
  });
});
