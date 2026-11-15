/// <reference types="node" />
// Guards the rule "no colours, sizes or fonts hard-coded outside tokens.css": scans the styles of
// every component under src and the global stylesheet for literals.

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

import { describe, expect, it } from "vitest";

import { darkTokens, lightTokens } from "./contrast";

// Tests run from the ui folder (pnpm --dir ui test).
const ROOT = join(process.cwd(), "src");

function walk(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) walk(full, out);
    else out.push(full);
  }
  return out;
}

// Every Svelte component under src (lib/ui, tree, tabs, editors, vault, sessions, state, dev,
// App.svelte) plus the global stylesheet. tokens.css is the one file allowed to hold literals.
const PATHS = [...walk(ROOT).filter((f) => f.endsWith(".svelte")), join(ROOT, "app.css")];
const FILES = PATHS.map((f) => `src/${relative(ROOT, f)}`);
const BY_NAME = new Map(PATHS.map((f) => [`src/${relative(ROOT, f)}`, readFileSync(f, "utf8")]));
const TOKENS_CSS = readFileSync(join(ROOT, "lib/design/tokens.css"), "utf8");

/** Style text of a file: the <style> blocks of a .svelte file, or the whole .css file. */
function styleText(file: string): string {
  const source = BY_NAME.get(file) ?? "";
  const text = file.endsWith(".css")
    ? source
    : [...source.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map((m) => m[1] ?? "").join("\n");
  return text.replace(/\/\*[\s\S]*?\*\//g, "");
}

interface Rule {
  name: string;
  pattern: RegExp;
}

const RULES: Rule[] = [
  { name: "hex colour", pattern: /#[0-9a-fA-F]{3,8}\b/ },
  { name: "colour function", pattern: /\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color-mix)\s*\(/ },
  {
    name: "named colour",
    pattern: /(?:color|background|border[\w-]*|fill|stroke|outline[\w-]*)\s*:[^;{}]*\b(?:white|black|red|green|blue|gray|grey|orange|yellow|purple|pink)\b/,
  },
  { name: "font-family literal", pattern: /font-family\s*:(?!\s*(?:var\(|inherit))/ },
  { name: "font-size literal", pattern: /font-size\s*:\s*[\d.]/ },
  { name: "font-weight literal", pattern: /font-weight\s*:\s*\d/ },
  { name: "z-index literal", pattern: /z-index\s*:\s*-?\d/ },
  { name: "duration literal", pattern: /(?<![\w.#])[\d.]+m?s\b(?![\w-])/ },
  { name: "easing literal", pattern: /cubic-bezier\(/ },
  // Any pixel length other than the 1px hairline and the 2px focus/outline width.
  { name: "pixel literal", pattern: /(?<![\w.#])(?!0*[12]px\b)\d*\.?\d+px\b/ },
];

/** `file` (path starting at src/) -> rule names that are allowed there, with the reason. */
const ALLOWLIST: Record<string, { rule: string; why: string }[]> = {};

describe("no hard-coded design literals outside tokens.css", () => {
  it("scans the expected files", () => {
    expect(FILES).toContain("src/lib/ui/Button.svelte");
    expect(FILES).toContain("src/lib/ui/Dialog.svelte");
    expect(FILES).toContain("src/dev/Gallery.svelte");
    expect(FILES).toContain("src/app.css");
    // `state` holds no components, so it has no styles to scan.
    for (const area of ["tree", "tabs", "editors", "vault", "sessions"]) {
      expect(FILES.some((f) => f.startsWith(`src/lib/${area}/`)), area).toBe(true);
    }
    expect(FILES).toContain("src/App.svelte");
    expect(FILES).toContain("src/lib/tree/ConnectionTree.svelte");
    expect(FILES).toContain("src/lib/sessions/TerminalView.svelte");
    expect(FILES.length).toBeGreaterThan(60);
  });

  for (const file of FILES) {
    const name = file;
    it(name, () => {
      const allowed = new Set((ALLOWLIST[name] ?? []).map((a) => a.rule));
      const text = styleText(file);
      const violations: string[] = [];
      text.split("\n").forEach((line, index) => {
        for (const rule of RULES) {
          if (allowed.has(rule.name)) continue;
          if (rule.pattern.test(line)) violations.push(`${name}:${index + 1} ${rule.name}: ${line.trim()}`);
        }
      });
      expect(violations).toEqual([]);
    });
  }

  it("only references tokens that exist", () => {
    const css = TOKENS_CSS;
    const defined = new Set([...Object.keys(lightTokens(css)), ...Object.keys(darkTokens(css))]);
    // Variables set by components themselves for their own styles.
    const local = new Set(["--depth", "--spinner-size", "--sidebar-width"]);
    const missing: string[] = [];
    for (const file of FILES) {
      const source = BY_NAME.get(file) ?? "";
      for (const m of source.matchAll(/var\((--[a-z0-9-]+)/g)) {
        const token = m[1] ?? "";
        // A trailing "-" is a template such as var(--space-{n}): some token must start with it.
        const ok = token.endsWith("-")
          ? [...defined].some((d) => d.startsWith(token))
          : defined.has(token) || local.has(token);
        if (!ok) missing.push(`${file}: ${token}`);
      }
    }
    expect([...new Set(missing)]).toEqual([]);
  });
});
