import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import { DEFAULT_APPEARANCE } from "../ipc/types";
import {
  APPEARANCE_KEY,
  applyAppearance,
  loadStoredAppearance,
  normalizeAppearance,
  saveStoredAppearance,
} from "./appearance";

const INIT_SCRIPT = readFileSync(resolve(__dirname, "../../../public/appearance-init.js"), "utf8");

function memory(initial: Record<string, string> = {}) {
  const data = { ...initial };
  return { data, getItem: (k: string) => data[k] ?? null, setItem: (k: string, v: string) => void (data[k] = v) };
}

function attributes(): Record<string, string | null> {
  const root = document.documentElement;
  return {
    theme: root.getAttribute("data-theme"),
    scheme: root.getAttribute("data-scheme"),
    accent: root.getAttribute("data-accent"),
    density: root.getAttribute("data-density"),
  };
}

afterEach(() => {
  for (const name of ["data-theme", "data-scheme", "data-accent", "data-density"]) document.documentElement.removeAttribute(name);
  window.localStorage.clear();
});

describe("normalizeAppearance", () => {
  it("keeps valid values and replaces bad fields one by one", () => {
    expect(normalizeAppearance({ theme: "dark", scheme: "nord", accent: "teal", density: "compact", terminalFontSize: 20 })).toEqual({
      theme: "dark",
      scheme: "nord",
      accent: "teal",
      density: "compact",
      terminalFontSize: 20,
    });
    expect(normalizeAppearance({ theme: "sepia", scheme: "dracula", accent: "orange", terminalFontSize: 99 })).toEqual({
      ...DEFAULT_APPEARANCE,
      accent: "orange",
    });
    // A file saved before colour schemes existed has no scheme: the default look.
    expect(normalizeAppearance({ theme: "dark", accent: "teal", density: "compact", terminalFontSize: 16 }).scheme).toBe(
      "portolano",
    );
    expect(normalizeAppearance(null)).toEqual(DEFAULT_APPEARANCE);
    expect(normalizeAppearance("dark")).toEqual(DEFAULT_APPEARANCE);
  });
});

describe("applyAppearance", () => {
  it("leaves no attribute for the defaults and sets the others", () => {
    applyAppearance(DEFAULT_APPEARANCE);
    expect(attributes()).toEqual({ theme: null, scheme: null, accent: null, density: null });
    applyAppearance({ theme: "light", scheme: "rose-pine", accent: "purple", density: "compact", terminalFontSize: 13 });
    expect(attributes()).toEqual({ theme: "light", scheme: "rose-pine", accent: "purple", density: "compact" });
    applyAppearance(DEFAULT_APPEARANCE);
    expect(attributes()).toEqual({ theme: null, scheme: null, accent: null, density: null });
  });
});

describe("stored copy", () => {
  it("round-trips and falls back to the defaults when missing or damaged", () => {
    const store = memory();
    expect(loadStoredAppearance(store)).toEqual(DEFAULT_APPEARANCE);
    const chosen = { theme: "dark", scheme: "solarized", accent: "graphite", density: "compact", terminalFontSize: 18 } as const;
    expect(saveStoredAppearance(chosen, store)).toBe(true);
    expect(loadStoredAppearance(store)).toEqual(chosen);
    store.data[APPEARANCE_KEY] = "{not json";
    expect(loadStoredAppearance(store)).toEqual(DEFAULT_APPEARANCE);
  });
});

describe("appearance-init.js (the script that runs before the first paint)", () => {
  const run = (): void => void new Function(INIT_SCRIPT)();

  it("sets the same attributes as applyAppearance for every combination", () => {
    for (const theme of ["system", "light", "dark"] as const) {
      for (const scheme of ["portolano", "nord", "solarized", "rose-pine"] as const) {
        for (const accent of ["blue", "teal", "purple", "orange", "graphite"] as const) {
          for (const density of ["comfortable", "compact"] as const) {
            const chosen = { theme, scheme, accent, density, terminalFontSize: 13 };
            applyAppearance(DEFAULT_APPEARANCE);
            applyAppearance(chosen);
            const expected = attributes();
            applyAppearance(DEFAULT_APPEARANCE);
            saveStoredAppearance(chosen);
            run();
            expect(attributes()).toEqual(expected);
            window.localStorage.clear();
          }
        }
      }
    }
  });

  it("ignores a missing, damaged or foreign copy", () => {
    run();
    expect(attributes()).toEqual({ theme: null, scheme: null, accent: null, density: null });
    window.localStorage.setItem(APPEARANCE_KEY, "{oops");
    run();
    window.localStorage.setItem(APPEARANCE_KEY, JSON.stringify({ theme: "sepia", scheme: "dracula", accent: "pink", density: "tiny" }));
    run();
    expect(attributes()).toEqual({ theme: null, scheme: null, accent: null, density: null });
  });

  it("is allowed by the Content-Security-Policy: a file, not an inline script", () => {
    const html = readFileSync(resolve(__dirname, "../../../index.html"), "utf8");
    expect(html).toContain('<script src="/appearance-init.js"></script>');
    expect(html).not.toMatch(/<script(?![^>]*\bsrc=)[^>]*>/);
  });
});
