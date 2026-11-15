import { afterEach, describe, expect, it, vi } from "vitest";

import { ANSI_NAMES, deriveTerminalMetrics, deriveTerminalTheme, parseColor, watchTheme } from "./theme";

const NAMES = [
  "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
  "bright-black", "bright-red", "bright-green", "bright-yellow", "bright-blue", "bright-magenta", "bright-cyan", "bright-white",
];
const LIGHT_ANSI = [
  "#1c1c1e", "#c8261a", "#1b6b37", "#8a5a00", "#0a5bbf", "#7a3fa8", "#0b7285", "#666a73",
  "#585c65", "#d63a2c", "#23803f", "#9c6500", "#1f6fd6", "#8e52bd", "#0d8095", "#4f525a",
];
const DARK_ANSI = [
  "#8d909a", "#ff8d84", "#72d38d", "#f2b95c", "#7cb9ff", "#c9a8f5", "#5cd0d6", "#b9bbc3",
  "#a3a6ae", "#ffb0a9", "#98e3ab", "#f8d08c", "#a6d0ff", "#dcc7fa", "#8fe2e6", "#f2f2f5",
];
const ansi = (values: string[]): Record<string, string> =>
  Object.fromEntries(NAMES.map((name, i) => [`--ansi-${name}`, values[i] ?? ""]));

const LIGHT: Record<string, string> = {
  "--terminal-bg": "#ffffff",
  "--text-primary": "#1c1c1e",
  "--text-selection-bg": "rgb(11 100 207 / 28%)",
  "--selection-bg-inactive": "rgb(60 66 80 / 14%)",
  "--font-mono": 'ui-monospace, "SF Mono", Menlo, monospace',
  "--font-size-md": "13px",
  "--terminal-line-height": "1.2",
  ...ansi(LIGHT_ANSI),
};
const DARK: Record<string, string> = {
  ...LIGHT,
  "--terminal-bg": "#1e1f22",
  "--text-primary": "#f2f2f5",
  ...ansi(DARK_ANSI),
};

function applyTokens(tokens: Record<string, string>): void {
  const root = document.documentElement;
  for (const [name, value] of Object.entries(tokens)) root.style.setProperty(name, value);
}

afterEach(() => {
  document.documentElement.removeAttribute("style");
  document.documentElement.removeAttribute("data-theme");
  vi.unstubAllGlobals();
});

describe("parseColor", () => {
  it("reads hex and both rgb notations", () => {
    expect(parseColor("#fff")).toEqual({ r: 255, g: 255, b: 255, a: 1 });
    expect(parseColor("#1c1c1e")).toEqual({ r: 28, g: 28, b: 30, a: 1 });
    expect(parseColor("rgb(11, 100, 207)")).toEqual({ r: 11, g: 100, b: 207, a: 1 });
    expect(parseColor("rgb(11 100 207 / 28%)")).toEqual({ r: 11, g: 100, b: 207, a: 0.28 });
    expect(parseColor("rgba(11, 100, 207, 0.5)")).toEqual({ r: 11, g: 100, b: 207, a: 0.5 });
    expect(parseColor("nonsense")).toBeNull();
  });
});

describe("deriveTerminalTheme", () => {
  it("takes surface, text, cursor and selection from the tokens", () => {
    applyTokens(LIGHT);
    const theme = deriveTerminalTheme();
    expect(theme.background).toBe("#ffffff");
    expect(theme.foreground).toBe("#1c1c1e");
    expect(theme.cursor).toBe("#1c1c1e");
    expect(theme.cursorAccent).toBe("#ffffff");
    expect(theme.selectionBackground).toBe("#0b64cf47");
  });

  it("takes the 16 colours from the --ansi-* tokens of the current theme", () => {
    applyTokens(LIGHT);
    const light = deriveTerminalTheme();
    expect(light.red).toBe("#c8261a");
    expect(light.green).toBe("#1b6b37");
    expect(light.black).toBe("#1c1c1e");
    expect(light.brightWhite).toBe("#4f525a");
    expect(ANSI_NAMES).toHaveLength(16);
    expect(Object.keys(light).filter((k) => /^(bright)?(black|red|green|yellow|blue|magenta|cyan|white)$/i.test(k))).toHaveLength(16);

    applyTokens(DARK);
    const dark = deriveTerminalTheme();
    expect(dark.background).toBe("#1e1f22");
    expect(dark.red).toBe("#ff8d84");
    // Black is a readable mid grey on the dark pane, not the pane colour itself.
    expect(dark.black).toBe("#8d909a");
    expect(dark.black).not.toBe(dark.background);
  });

  it("paints the cursor accent with the pane background so the cursor cell reads cleanly", () => {
    applyTokens(DARK);
    expect(deriveTerminalTheme().cursorAccent).toBe("#1e1f22");
  });

  it("leaves slots out when their tokens are missing (xterm keeps its default)", () => {
    const theme = deriveTerminalTheme();
    expect(theme.background).toBeUndefined();
    expect(theme.red).toBeUndefined();
  });
});

describe("deriveTerminalMetrics", () => {
  it("reads font, size and line height from the typography tokens", () => {
    applyTokens(LIGHT);
    expect(deriveTerminalMetrics()).toEqual({
      fontFamily: 'ui-monospace, "SF Mono", Menlo, monospace',
      fontSize: 13,
      lineHeight: 1.2,
    });
  });
});

describe("watchTheme", () => {
  it("fires when data-theme changes and stops after cleanup", async () => {
    const onChange = vi.fn();
    const stop = watchTheme(onChange);
    document.documentElement.setAttribute("data-theme", "dark");
    await vi.waitFor(() => expect(onChange).toHaveBeenCalledTimes(1));
    stop();
    document.documentElement.setAttribute("data-theme", "light");
    await new Promise((r) => setTimeout(r, 0));
    expect(onChange).toHaveBeenCalledTimes(1);
  });

  it("fires on a system colour scheme change", () => {
    let listener: (() => void) | null = null;
    const removed = vi.fn();
    vi.stubGlobal("matchMedia", () => ({
      addEventListener: (_: string, fn: () => void) => (listener = fn),
      removeEventListener: removed,
    }));
    const onChange = vi.fn();
    const stop = watchTheme(onChange);
    (listener as (() => void) | null)?.();
    expect(onChange).toHaveBeenCalledTimes(1);
    stop();
    expect(removed).toHaveBeenCalled();
  });
});
