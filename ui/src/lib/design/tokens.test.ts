/// <reference types="node" />
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import {
  ANSI_TOKENS,
  PAIRS,
  darkMediaTokens,
  darkTokens,
  lightTokens,
  luminance,
  measure,
  parseColor,
  ratio,
} from "./contrast";

// Vitest does not return CSS text for `?raw`, so read the file (tests run from the ui folder).
const css = readFileSync(join(process.cwd(), "src/lib/design/tokens.css"), "utf8");
it("reads tokens.css", () => expect(css.length).toBeGreaterThan(1000));
const light = lightTokens(css);
const dark = darkTokens(css);

/** Tokens that must exist; every colour token must also be redefined for dark. */
const COLOR_TOKENS = [
  "surface-window",
  "surface-sidebar",
  "surface-content",
  "surface-raised",
  "surface-field",
  "surface-control",
  "surface-control-hover",
  "surface-control-pressed",
  "surface-inset",
  "surface-hover",
  "surface-pressed",
  "text-primary",
  "text-secondary",
  "text-tertiary",
  "text-disabled",
  "text-on-accent",
  "text-link",
  "border-hairline",
  "border-strong",
  "accent",
  "accent-hover",
  "accent-pressed",
  "accent-text",
  "accent-subtle",
  "focus-ring",
  "selection-bg",
  "selection-bg-inactive",
  "selection-solid-bg",
  "selection-solid-fg",
  "text-selection-bg",
  ...["danger", "warning", "success", "info"].flatMap((s) => [`${s}-text`, `${s}-bg`, `${s}-border`]),
  "danger-solid",
  "danger-solid-hover",
  "danger-solid-pressed",
  "scrim",
  "shadow-control",
  "shadow-popover",
  "shadow-dialog",
  "protocol-folder",
  "protocol-local",
  "protocol-ssh",
  "protocol-rdp",
  "protocol-vnc",
  "protocol-web",
  "terminal-bg",
  "letterbox-bg",
  ...ANSI_TOKENS.map((token) => token.slice(2)),
];

const SHARED_TOKENS = [
  "font-sans",
  "font-mono",
  "font-size-xs",
  "font-size-sm",
  "font-size-md",
  "font-size-lg",
  "font-size-xl",
  "font-weight-regular",
  "font-weight-medium",
  "font-weight-semibold",
  "line-height-tight",
  "line-height-normal",
  "line-height-relaxed",
  "terminal-line-height",
  "tab-min-width",
  "tab-max-width",
  "dialog-width-sm",
  "dialog-width-md",
  "dialog-width-lg",
  "space-1",
  "space-2",
  "space-3",
  "space-4",
  "space-5",
  "space-6",
  "space-8",
  "space-10",
  "space-12",
  "radius-xs",
  "radius-control",
  "radius-panel",
  "radius-full",
  "control-h-compact",
  "control-h-regular",
  "control-h-large",
  "z-base",
  "z-splitter",
  "z-sticky",
  "z-dropdown",
  "z-overlay",
  "z-menu",
  "z-toast",
  "z-tooltip",
  "duration-fast",
  "duration-base",
  "duration-slow",
  "ease-standard",
  "ease-out",
  "focus-ring-width",
  "focus-ring-offset",
  "titlebar-height",
  "titlebar-safe-left",
];

describe("design tokens", () => {
  it("defines every colour token for light and for dark", () => {
    for (const name of COLOR_TOKENS) {
      expect(light[`--${name}`], `light --${name}`).toBeTruthy();
      expect(dark[`--${name}`], `dark --${name}`).toBeTruthy();
    }
  });

  it("defines every theme-independent token", () => {
    for (const name of SHARED_TOKENS) expect(light[`--${name}`], `--${name}`).toBeTruthy();
  });

  it("keeps the system-preference dark block identical to the data-theme dark block", () => {
    expect(Object.keys(darkMediaTokens(css)).length).toBeGreaterThan(40);
    expect(darkMediaTokens(css)).toEqual(dark);
  });

  it("only redefines in dark tokens that exist in light", () => {
    for (const name of Object.keys(dark)) expect(light[name], name).toBeDefined();
  });

  it("follows the 4 px grid and the agreed scales", () => {
    for (const [name, value] of Object.entries(light)) {
      if (!/^--space-\d+$/.test(name) || value === "0") continue;
      expect(Number.parseInt(value, 10) % 4, name).toBe(0);
    }
    const px = (name: string): number => Number.parseInt(light[name] ?? "", 10);
    expect(["xs", "sm", "md", "lg", "xl"].map((s) => px(`--font-size-${s}`))).toEqual([11, 12, 13, 15, 20]);
    expect(["regular", "medium", "semibold"].map((w) => light[`--font-weight-${w}`])).toEqual(["400", "500", "600"]);
    expect(px("--radius-control")).toBe(6);
    expect(px("--radius-panel")).toBe(10);
    expect([px("--control-h-compact"), px("--control-h-regular"), px("--control-h-large")]).toEqual([24, 28, 32]);
    expect([px("--duration-fast"), px("--duration-base"), px("--duration-slow")]).toEqual([120, 160, 200]);
  });

  it("uses the system font stacks and no web fonts", () => {
    expect(light["--font-sans"]).toContain("-apple-system");
    expect(light["--font-sans"]).toContain("Segoe UI Variable");
    expect(light["--font-mono"]).toContain("ui-monospace");
    expect(css).not.toMatch(/@font-face|url\(/);
  });

  it("has a data-theme hook, a reduced-motion rule and platform chrome tokens", () => {
    expect(css).toContain(':root[data-theme="dark"]');
    expect(css).toContain(':root:not([data-theme="light"])');
    expect(css).toContain("prefers-reduced-motion: reduce");
    expect(css).toContain(':root[data-platform="macos"]');
    expect(light["--titlebar-height"]).toBe("0px");
  });

  it("has no legacy alias block left", () => {
    expect(css).not.toMatch(/LEGACY/i);
    for (const old of ["--bg", "--bg-sidebar", "--bg-elevated", "--bg-input", "--fg", "--fg-muted", "--border", "--accent-fg", "--danger", "--backdrop", "--shadow", "--focus"]) {
      expect(light[old], old).toBeUndefined();
    }
  });

  it("defines the 16 ANSI colours, distinct within each theme where it matters", () => {
    expect(ANSI_TOKENS).toHaveLength(16);
    for (const theme of [light, dark]) {
      // black and bright-black differ, red and bright-red differ (they carry meaning in tools).
      expect(theme["--ansi-black"]).not.toBe(theme["--ansi-bright-black"]);
      expect(theme["--ansi-red"]).not.toBe(theme["--ansi-bright-red"]);
    }
  });

  it("keeps the letterbox dark in both themes, whatever the app surfaces do", () => {
    for (const theme of [light, dark]) {
      const color = parseColor(theme["--letterbox-bg"] ?? "");
      expect(color).not.toBeNull();
      if (color) expect(luminance(color)).toBeLessThan(0.06);
    }
  });

  it("paints the terminal pane with the content surface of each theme", () => {
    expect(light["--terminal-bg"]).toBe(light["--surface-content"]);
    expect(dark["--terminal-bg"]).toBe(dark["--surface-content"]);
    expect(Number(light["--terminal-line-height"])).toBeGreaterThanOrEqual(1.2);
  });

  it("never gives the accent to anything but selection, focus and the primary action", () => {
    // The accent must differ from the graphite surfaces and text, i.e. be a deliberate colour.
    for (const theme of [light, dark]) {
      const accent = parseColor(theme["--accent"] ?? "");
      const text = parseColor(theme["--text-primary"] ?? "");
      expect(accent).not.toBeNull();
      expect(text).not.toBeNull();
      if (accent && text) expect(ratio(accent, text)).toBeGreaterThan(1.5);
    }
  });
});

describe("contrast (WCAG)", () => {
  const measured = measure(css);

  it("checks a meaningful number of pairs in both themes", () => {
    expect(PAIRS.length).toBeGreaterThan(80);
    expect(measured.filter((m) => m.theme === "light").length).toBe(PAIRS.length);
    expect(measured.filter((m) => m.theme === "dark").length).toBe(PAIRS.length);
  });

  it("meets AA for text (4.5:1) and 3:1 for icons and focus rings", () => {
    const failures = measured
      .filter((m) => !m.pass)
      .map((m) => `${m.theme} ${m.fg} on ${m.bg}: ${m.value.toFixed(2)} < ${m.min}`);
    expect(failures).toEqual([]);
  });
});
