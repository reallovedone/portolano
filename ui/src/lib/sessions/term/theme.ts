// xterm.js appearance derived from the design tokens, so the terminal follows the app theme.
//
// Colours are read from the computed custom properties of the root element and converted to
// hex (xterm cannot parse the modern `rgb(r g b / a)` notation of the tokens on every engine).
// Every colour comes from a token: the pane background `--terminal-bg`, the default text, and the
// 16-colour ANSI palette `--ansi-*` (each contrast-checked against the terminal background).

import type { ITheme } from "@xterm/xterm";

/** ANSI slot names in xterm order (0-15). The matching token is `--ansi-<name>`. */
export const ANSI_NAMES = [
  "black",
  "red",
  "green",
  "yellow",
  "blue",
  "magenta",
  "cyan",
  "white",
  "bright-black",
  "bright-red",
  "bright-green",
  "bright-yellow",
  "bright-blue",
  "bright-magenta",
  "bright-cyan",
  "bright-white",
] as const;

interface Rgba {
  r: number;
  g: number;
  b: number;
  a: number;
}

function channel(value: number): number {
  return Math.max(0, Math.min(255, Math.round(value)));
}

/** Parses `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb(r, g, b)` and `rgb(r g b / a)`. */
export function parseColor(input: string): Rgba | null {
  const text = input.trim().toLowerCase();
  if (text.startsWith("#")) {
    const hex = text.slice(1);
    const full =
      hex.length === 3 || hex.length === 4
        ? [...hex].map((c) => c + c).join("")
        : hex;
    if (!/^[0-9a-f]+$/.test(full) || (full.length !== 6 && full.length !== 8)) return null;
    return {
      r: parseInt(full.slice(0, 2), 16),
      g: parseInt(full.slice(2, 4), 16),
      b: parseInt(full.slice(4, 6), 16),
      a: full.length === 8 ? parseInt(full.slice(6, 8), 16) / 255 : 1,
    };
  }
  const match = /^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)(?:\s*[,/]\s*([\d.]+)(%?))?\s*\)$/.exec(text);
  if (!match) return null;
  const alpha = match[4] === undefined ? 1 : match[5] === "%" ? Number(match[4]) / 100 : Number(match[4]);
  return { r: Number(match[1]), g: Number(match[2]), b: Number(match[3]), a: alpha };
}

function toHex({ r, g, b, a }: Rgba): string {
  const part = (n: number): string => channel(n).toString(16).padStart(2, "0");
  const base = `#${part(r)}${part(g)}${part(b)}`;
  return a >= 1 ? base : `${base}${part(a * 255)}`;
}

function token(style: CSSStyleDeclaration, name: string): string | undefined {
  const value = style.getPropertyValue(name).trim();
  return value === "" ? undefined : value;
}

function colorOf(style: CSSStyleDeclaration, ...names: string[]): string | undefined {
  for (const name of names) {
    const raw = token(style, name);
    const parsed = raw === undefined ? null : parseColor(raw);
    if (parsed) return toHex(parsed);
  }
  return undefined;
}

/** The xterm theme for the current light/dark state of `root`. Unresolvable slots are left
 *  out so xterm keeps its own default for them. */
export function deriveTerminalTheme(root: Element = document.documentElement): ITheme {
  const style = getComputedStyle(root);
  const theme: ITheme = {
    background: colorOf(style, "--terminal-bg"),
    foreground: colorOf(style, "--text-primary"),
    cursor: colorOf(style, "--text-primary"),
    cursorAccent: colorOf(style, "--terminal-bg"),
    selectionBackground: colorOf(style, "--text-selection-bg"),
    selectionInactiveBackground: colorOf(style, "--selection-bg-inactive"),
  };
  ANSI_NAMES.forEach((name, index) => {
    const key = ANSI_KEYS[index];
    const value = colorOf(style, `--ansi-${name}`);
    if (key && value) theme[key] = value;
  });
  return theme;
}

const ANSI_KEYS = [
  "black",
  "red",
  "green",
  "yellow",
  "blue",
  "magenta",
  "cyan",
  "white",
  "brightBlack",
  "brightRed",
  "brightGreen",
  "brightYellow",
  "brightBlue",
  "brightMagenta",
  "brightCyan",
  "brightWhite",
] as const satisfies readonly (keyof ITheme)[];

export interface TerminalMetrics {
  fontFamily: string;
  fontSize: number;
  lineHeight: number;
}

/** Font, size and line height from the typography and terminal tokens. */
export function deriveTerminalMetrics(root: Element = document.documentElement): TerminalMetrics {
  const style = getComputedStyle(root);
  const family = token(style, "--font-mono");
  const size = Number.parseFloat(token(style, "--font-size-md") ?? "");
  const line = Number.parseFloat(token(style, "--terminal-line-height") ?? "");
  return {
    fontFamily: family ?? "monospace",
    fontSize: Number.isFinite(size) && size > 0 ? size : 13,
    lineHeight: Number.isFinite(line) && line >= 1 ? line : 1,
  };
}

/** Calls `onChange` when the system colour scheme or the root `data-theme`, `data-scheme` or
 *  `data-accent` attribute changes. Returns the stop function. */
export function watchTheme(onChange: () => void, root: Element = document.documentElement): () => void {
  const media =
    typeof window.matchMedia === "function" ? window.matchMedia("(prefers-color-scheme: dark)") : null;
  media?.addEventListener("change", onChange);
  const observer = new MutationObserver(onChange);
  observer.observe(root, { attributes: true, attributeFilter: ["data-theme", "data-scheme", "data-accent"] });
  return () => {
    media?.removeEventListener("change", onChange);
    observer.disconnect();
  };
}
