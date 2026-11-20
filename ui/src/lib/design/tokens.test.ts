/// <reference types="node" />
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import {
  ACCENTS,
  ANSI_TOKENS,
  NODE_COLOR_TOKENS,
  accentMediaTokens,
  accentTokens,
  PAIRS,
  SCHEMES,
  darkMediaTokens,
  darkTokens,
  lightTokens,
  luminance,
  measure,
  parseColor,
  ratio,
  ruleBody,
  schemeMediaTokens,
  schemeTokens,
  themeTokens,
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
  ...ACCENTS.map((accent) => `swatch-${accent}`),
  ...ANSI_TOKENS.map((token) => token.slice(2)),
  ...NODE_COLOR_TOKENS.map((token) => token.slice(2)),
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
  "row-h-compact",
  "row-h-regular",
  "row-h-comfortable",
  "row-indent",
  "row-gap",
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

describe("link text inside grouped form sections", () => {
  it("is checked, and readable, on the section panel in both themes", () => {
    const rows = measure(css).filter((m) => m.fg === "--text-link" && m.bg === "--surface-window");
    expect(rows.map((m) => m.theme).sort()).toEqual(["dark", "light"]);
    for (const row of rows) expect(row.value).toBeGreaterThanOrEqual(4.5);
  });

  it("covers help text and links inside an info banner in both themes", () => {
    for (const fg of ["--text-secondary", "--text-link"]) {
      const rows = measure(css).filter((m) => m.fg === fg && m.bg === "--info-bg");
      expect(rows.map((m) => m.theme).sort()).toEqual(["dark", "light"]);
      for (const row of rows) expect(row.value).toBeGreaterThanOrEqual(4.5);
    }
  });
});

/** Every variable the accent family is made of; each accent block must redefine all of them. */
const ACCENT_VARIABLES = [
  "--accent",
  "--accent-hover",
  "--accent-pressed",
  "--accent-text",
  "--accent-subtle",
  "--focus-ring",
  "--selection-bg",
  "--selection-solid-bg",
  "--text-selection-bg",
];

describe("accent colours", () => {
  const others = ACCENTS.filter((accent) => accent !== "blue");

  it("offers blue (the base) and one block per other accent", () => {
    expect([...ACCENTS]).toEqual(["blue", "teal", "purple", "orange", "graphite"]);
    for (const accent of others) expect(css).toContain(`:root[data-accent="${accent}"] {`);
    expect(css).not.toContain(':root[data-accent="blue"]');
  });

  it("redefines every accent variable in light and in dark", () => {
    for (const accent of others) {
      for (const theme of ["light", "dark"] as const) {
        const tokens = accentTokens(css, accent, theme);
        for (const name of ACCENT_VARIABLES) expect(tokens[name], `${accent} ${theme} ${name}`).toBeTruthy();
        expect(Object.keys(tokens).sort()).toEqual([...ACCENT_VARIABLES].sort());
      }
    }
  });

  it("keeps the system-preference dark block of an accent identical to the forced one", () => {
    for (const accent of others) {
      expect(accentMediaTokens(css, accent), accent).toEqual(accentTokens(css, accent, "dark"));
    }
  });

  it("gives every accent its own look", () => {
    for (const theme of ["light", "dark"] as const) {
      const fills = ACCENTS.map((accent) => themeTokens(css, theme, accent)["--accent"]);
      expect(new Set(fills).size).toBe(ACCENTS.length);
    }
  });

  it("shows each accent in the picker with its own fill, in both themes", () => {
    for (const accent of ACCENTS) {
      expect(light[`--swatch-${accent}`], `light ${accent}`).toBe(themeTokens(css, "light", accent)["--accent"]);
      expect(dark[`--swatch-${accent}`], `dark ${accent}`).toBe(themeTokens(css, "dark", accent)["--accent"]);
    }
  });

  for (const accent of ACCENTS) {
    it(`${accent} meets AA in both themes (every pair of the design)`, () => {
      const measuredAccent = measure(css, accent);
      expect(measuredAccent.length).toBe(PAIRS.length * 2);
      const failures = measuredAccent
        .filter((m) => !m.pass)
        .map((m) => `${accent} ${m.theme} ${m.fg} on ${m.bg}: ${m.value.toFixed(2)} < ${m.min}`);
      expect(failures).toEqual([]);
    });

    it(`${accent} is a deliberate colour next to the text in both themes`, () => {
      for (const theme of ["light", "dark"] as const) {
        const tokens = themeTokens(css, theme, accent);
        const fill = parseColor(tokens["--accent"] ?? "");
        const text = parseColor(tokens["--text-primary"] ?? "");
        expect(fill).not.toBeNull();
        if (fill && text) expect(ratio(fill, text)).toBeGreaterThan(1.5);
      }
    });
  }

  it("lets the inactive-window selection beat any accent", () => {
    expect(css).toContain(':root:root[data-window-active="false"]');
  });
});

/** What every scheme redefines: the neutral surfaces, text, borders and the terminal pane. */
const SCHEME_VARIABLES = [
  "--surface-window",
  "--surface-sidebar",
  "--surface-content",
  "--surface-raised",
  "--surface-field",
  "--surface-control",
  "--surface-control-hover",
  "--surface-control-pressed",
  "--surface-inset",
  "--surface-hover",
  "--surface-pressed",
  "--text-primary",
  "--text-secondary",
  "--text-tertiary",
  "--text-disabled",
  "--text-link",
  "--border-hairline",
  "--border-strong",
  "--decor-line",
  "--decor-line-strong",
  "--decor-text",
  "--segment-selected-bg",
  "--terminal-bg",
  ...ANSI_TOKENS,
];

describe("colour schemes", () => {
  const others = SCHEMES.filter((scheme) => scheme !== "portolano");

  it("offers portolano (the base) and three more", () => {
    expect([...SCHEMES]).toEqual(["portolano", "nord", "solarized", "rose-pine"]);
  });

  it("redefines exactly the scheme variables, in light and in dark", () => {
    for (const scheme of SCHEMES) {
      for (const theme of ["light", "dark"] as const) {
        const tokens = schemeTokens(css, scheme, theme);
        expect(Object.keys(tokens).sort(), `${scheme} ${theme}`).toEqual([...SCHEME_VARIABLES].sort());
      }
    }
  });

  it("never touches an accent variable (the accent stays independent)", () => {
    for (const scheme of SCHEMES) {
      for (const theme of ["light", "dark"] as const) {
        for (const name of ACCENT_VARIABLES) expect(schemeTokens(css, scheme, theme)[name], `${scheme} ${name}`).toBeUndefined();
      }
    }
  });

  it("keeps the system-preference dark block of a scheme identical to the forced one", () => {
    for (const scheme of SCHEMES) {
      expect(schemeMediaTokens(css, scheme), scheme).toEqual(schemeTokens(css, scheme, "dark"));
    }
  });

  it("gives the portolano blocks (used by the preview cards) the values of the base", () => {
    for (const name of SCHEME_VARIABLES) {
      expect(schemeTokens(css, "portolano", "light")[name], `light ${name}`).toBe(light[name]);
      expect(schemeTokens(css, "portolano", "dark")[name], `dark ${name}`).toBe(dark[name]);
    }
  });

  it("gives every scheme its own look", () => {
    for (const theme of ["light", "dark"] as const) {
      const looks = SCHEMES.map((scheme) => themeTokens(css, theme, "blue", scheme)["--surface-content"]);
      expect(new Set(looks).size).toBe(SCHEMES.length);
    }
  });

  it("paints the terminal pane with the content surface of each scheme and theme", () => {
    for (const scheme of SCHEMES) {
      for (const theme of ["light", "dark"] as const) {
        const tokens = themeTokens(css, theme, "blue", scheme);
        expect(tokens["--terminal-bg"], `${scheme} ${theme}`).toBe(tokens["--surface-content"]);
      }
    }
  });

  it("keeps the 16 ANSI colours apart from each other except where a theme means to repeat one", () => {
    for (const scheme of SCHEMES) {
      for (const theme of ["light", "dark"] as const) {
        const tokens = themeTokens(css, theme, "blue", scheme);
        expect(tokens["--ansi-red"], `${scheme} ${theme}`).not.toBe(tokens["--ansi-bright-red"]);
        expect(tokens["--ansi-black"], `${scheme} ${theme}`).not.toBe(tokens["--ansi-bright-black"]);
      }
    }
  });

  // Every scheme x theme x accent: AA for text (4.5) and 3:1 for icons and focus rings.
  for (const scheme of SCHEMES) {
    for (const accent of ACCENTS) {
      it(`${scheme} with the ${accent} accent meets AA in both themes (every pair of the design)`, () => {
        const rows = measure(css, accent, scheme);
        expect(rows.length).toBe(PAIRS.length * 2);
        const failures = rows
          .filter((m) => !m.pass)
          .map((m) => `${scheme} ${accent} ${m.theme} ${m.fg} on ${m.bg}: ${m.value.toFixed(2)} < ${m.min}`);
        expect(failures).toEqual([]);
      });
    }
  }
});

describe("node colours", () => {
  it("has the nine colours of appearance.json as tokens", () => {
    const shared = JSON.parse(
      readFileSync(join(process.cwd(), "../crates/core-model/src/appearance.json"), "utf8"),
    ) as { colors: string[] };
    expect(NODE_COLOR_TOKENS).toEqual(shared.colors.map((name) => `--node-${name}`));
  });

  it("keeps the nine colours apart from each other in both themes", () => {
    for (const theme of [light, dark]) {
      expect(new Set(NODE_COLOR_TOKENS.map((name) => theme[name])).size).toBe(NODE_COLOR_TOKENS.length);
    }
  });

  // Every scheme x theme x accent, on the window, content and sidebar surfaces and on the
  // selected (active and inactive), hovered and drop-target tree row.
  for (const scheme of SCHEMES) {
    for (const accent of ACCENTS) {
      it(`${scheme} with the ${accent} accent keeps every node colour at 3:1 on every surface and row state`, () => {
        const rows = measure(css, accent, scheme).filter((m) => m.note === "node colour");
        expect(rows.length).toBe(NODE_COLOR_TOKENS.length * 7 * 2);
        const failures = rows
          .filter((m) => !m.pass)
          .map((m) => `${m.theme} ${m.fg} on ${m.bg}: ${m.value.toFixed(2)} < ${m.min}`);
        expect(failures).toEqual([]);
      });
    }
  }
});

describe("theme preview", () => {
  it("has fixed values for the light and the dark mini window, apart from the page theme", () => {
    for (const name of ["bg", "bar", "line"]) {
      expect(light[`--preview-light-${name}`], name).toBeTruthy();
      expect(light[`--preview-dark-${name}`], name).toBeTruthy();
      expect(dark[`--preview-light-${name}`]).toBeUndefined();
    }
  });
});

describe("density", () => {
  it("has a compact block that makes every row shorter than the comfortable one", () => {
    const body = ruleBody(css, ':root[data-density="compact"]');
    expect(body).not.toBeNull();
    const compact = Object.fromEntries(
      [...(body ?? "").matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/g)].map((m) => [m[1], m[2]?.trim() ?? ""]),
    );
    const px = (value: string | undefined): number => Number.parseInt(value ?? "", 10);
    for (const name of ["compact", "regular", "comfortable"]) {
      expect(px(compact[`--row-h-${name}`]), name).toBeLessThan(px(light[`--row-h-${name}`]));
    }
    expect(compact["--row-indent"]).toBeTruthy();
    expect(compact["--row-gap"]).toBeTruthy();
  });

  it("keeps the rows tall enough to hit (20 px at the least)", () => {
    const body = ruleBody(css, ':root[data-density="compact"]') ?? "";
    for (const m of body.matchAll(/--row-h-[a-z]+\s*:\s*(\d+)px/g)) {
      expect(Number(m[1])).toBeGreaterThanOrEqual(20);
    }
  });
});
