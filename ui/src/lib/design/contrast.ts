// Contrast maths for the design tokens. Pure functions over the text of tokens.css, used by
// tokens.test.ts (and runnable by hand) so the AA claims in the report are computed, not guessed.

export type Theme = "light" | "dark";
export type Rgba = { r: number; g: number; b: number; a: number };
export type TokenMap = Record<string, string>;

/** Extracts `--name: value;` declarations of one rule body. */
function declarations(body: string): TokenMap {
  const out: TokenMap = {};
  const re = /(--[a-z0-9-]+)\s*:\s*([^;]+);/g;
  for (let m = re.exec(body); m; m = re.exec(body)) {
    const name = m[1];
    const value = m[2];
    if (name && value) out[name] = value.replace(/\s+/g, " ").trim();
  }
  return out;
}

/** Returns the body of the first rule whose selector (text before `{`) equals `selector`. */
export function ruleBody(css: string, selector: string, afterIndex = 0): string | null {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, (c) => " ".repeat(c.length));
  const start = stripped.indexOf(`${selector} {`, afterIndex);
  if (start < 0) return null;
  const open = stripped.indexOf("{", start);
  let depth = 0;
  for (let i = open; i < stripped.length; i++) {
    if (stripped[i] === "{") depth++;
    if (stripped[i] === "}" && --depth === 0) return stripped.slice(open + 1, i);
  }
  return null;
}

export function lightTokens(css: string): TokenMap {
  const body = ruleBody(css, ":root");
  return body ? declarations(body) : {};
}

export function darkTokens(css: string): TokenMap {
  const body = ruleBody(css, ':root[data-theme="dark"]');
  return body ? declarations(body) : {};
}

/** The dark block inside the prefers-color-scheme media query. */
export function darkMediaTokens(css: string): TokenMap {
  const media = css.indexOf("@media (prefers-color-scheme: dark)");
  if (media < 0) return {};
  const body = ruleBody(css, ':root:not([data-theme="light"])', media);
  return body ? declarations(body) : {};
}

/** Accent names the user can choose. Blue is the base of tokens.css; the others have blocks. */
export const ACCENTS = ["blue", "teal", "purple", "orange", "graphite"] as const;
export type AccentName = (typeof ACCENTS)[number];

/** What an accent redefines for one theme (empty for blue, which is the base). */
export function accentTokens(css: string, accent: string, theme: Theme): TokenMap {
  const selector =
    theme === "light" ? `:root[data-accent="${accent}"]` : `:root[data-theme="dark"][data-accent="${accent}"]`;
  const body = ruleBody(css, selector);
  return body ? declarations(body) : {};
}

/** The dark block of an accent inside the prefers-color-scheme media query. */
export function accentMediaTokens(css: string, accent: string): TokenMap {
  const media = css.indexOf("@media (prefers-color-scheme: dark)");
  if (media < 0) return {};
  const body = ruleBody(css, `:root:not([data-theme="light"])[data-accent="${accent}"]`, media);
  return body ? declarations(body) : {};
}

/** Colour schemes the user can choose. Portolano is the base of tokens.css (no attribute on <html>). */
export const SCHEMES = ["portolano", "nord", "solarized", "rose-pine"] as const;
export type SchemeName = (typeof SCHEMES)[number];

/** Selector of the light block of a scheme (root or preview card): a line of its own. */
const schemeLightSelector = (scheme: string): string => `\n[data-scheme="${scheme}"]`;
/** The dark selector lists: system preference (inside the media query) and forced. */
export const schemeMediaSelector = (scheme: string): string =>
  `:root:not([data-theme="light"])[data-scheme="${scheme}"], :root:not([data-theme="light"]) [data-scheme="${scheme}"]`;
export const schemeForcedSelector = (scheme: string): string =>
  `:root[data-theme="dark"][data-scheme="${scheme}"], :root[data-theme="dark"] [data-scheme="${scheme}"]`;

/** What a scheme redefines for one theme. */
export function schemeTokens(css: string, scheme: string, theme: Theme): TokenMap {
  const body = ruleBody(css, theme === "light" ? schemeLightSelector(scheme) : schemeForcedSelector(scheme));
  return body ? declarations(body) : {};
}

/** The dark block of a scheme inside the prefers-color-scheme media query. */
export function schemeMediaTokens(css: string, scheme: string): TokenMap {
  const media = css.indexOf("@media (prefers-color-scheme: dark)");
  if (media < 0) return {};
  const body = ruleBody(css, schemeMediaSelector(scheme), media);
  return body ? declarations(body) : {};
}

/** The tokens the page resolves for a scheme, theme and accent. Portolano is the bare base:
 *  a page with that choice has no scheme attribute, so its blocks (for the preview cards) are
 *  not applied here. */
export function themeTokens(
  css: string,
  theme: Theme,
  accent: string = "blue",
  scheme: string = "portolano",
): TokenMap {
  const light = lightTokens(css);
  const base = theme === "light" ? light : { ...light, ...darkTokens(css) };
  const schemed = scheme === "portolano" ? {} : schemeTokens(css, scheme, theme);
  return { ...base, ...schemed, ...accentTokens(css, accent, theme) };
}

export function parseColor(value: string): Rgba | null {
  const v = value.trim().toLowerCase();
  const hex = /^#([0-9a-f]{3}|[0-9a-f]{6})$/.exec(v);
  if (hex?.[1]) {
    let h = hex[1];
    if (h.length === 3) h = [...h].map((c) => c + c).join("");
    return {
      r: parseInt(h.slice(0, 2), 16),
      g: parseInt(h.slice(2, 4), 16),
      b: parseInt(h.slice(4, 6), 16),
      a: 1,
    };
  }
  const fn = /^rgb\(\s*(\d+)\s+(\d+)\s+(\d+)\s*(?:\/\s*([\d.]+)%\s*)?\)$/.exec(v);
  if (fn) {
    return {
      r: Number(fn[1]),
      g: Number(fn[2]),
      b: Number(fn[3]),
      a: fn[4] === undefined ? 1 : Number(fn[4]) / 100,
    };
  }
  return null;
}

/** Source-over compositing of `top` on an opaque `bottom`. */
export function over(top: Rgba, bottom: Rgba): Rgba {
  const a = top.a;
  return {
    r: top.r * a + bottom.r * (1 - a),
    g: top.g * a + bottom.g * (1 - a),
    b: top.b * a + bottom.b * (1 - a),
    a: 1,
  };
}

function channel(c: number): number {
  const s = c / 255;
  return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
}

export function luminance(c: Rgba): number {
  return 0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b);
}

export function ratio(fg: Rgba, bg: Rgba): number {
  const a = luminance(fg);
  const b = luminance(bg);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

export interface Pair {
  fg: string;
  bg: string;
  /** Minimum ratio: 4.5 for text, 3 for icons and focus indicators. */
  min: number;
  note?: string;
}

const TEXT = 4.5;
const GRAPHIC = 3;

const SURFACES = [
  "--surface-window",
  "--surface-sidebar",
  "--surface-content",
  "--surface-raised",
  "--surface-field",
  "--surface-control",
];

/** ANSI slot names (xterm order); the token of each is `--ansi-<name>`. */
export const ANSI_TOKENS = [
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
].map((name) => `--ansi-${name}`);

/** The colour names a node can have; the token of each is `--node-<name>`. */
export const NODE_COLOR_TOKENS = ["red", "orange", "yellow", "green", "teal", "blue", "purple", "pink", "gray"].map(
  (name) => `--node-${name}`,
);

/** Every foreground/background combination the design promises to keep readable. */
export const PAIRS: Pair[] = [
  ...SURFACES.flatMap((bg) =>
    ["--text-primary", "--text-secondary", "--text-tertiary"].map((fg) => ({ fg, bg, min: TEXT })),
  ),
  // The VNC security note puts a HelpDisclosure (secondary text, link button) inside an info banner.
  { fg: "--text-secondary", bg: "--info-bg", min: TEXT, note: "help text in an info banner" },
  { fg: "--text-link", bg: "--info-bg", min: TEXT, note: "link in an info banner" },
  // "Learn more" buttons sit on the grouped-section panel (surface-window).
  { fg: "--text-link", bg: "--surface-window", min: TEXT, note: "link inside a grouped section" },
  ...["--surface-content", "--surface-sidebar", "--surface-raised"].flatMap((bg) => [
    { fg: "--accent-text", bg, min: TEXT },
    { fg: "--text-link", bg, min: TEXT },
    { fg: "--focus-ring", bg, min: GRAPHIC, note: "focus indicator" },
  ]),
  { fg: "--text-on-accent", bg: "--accent", min: TEXT },
  { fg: "--text-on-accent", bg: "--accent-hover", min: TEXT },
  { fg: "--text-on-accent", bg: "--accent-pressed", min: TEXT },
  { fg: "--selection-solid-fg", bg: "--selection-solid-bg", min: TEXT },
  { fg: "--text-on-accent", bg: "--danger-solid", min: TEXT },
  { fg: "--text-on-accent", bg: "--danger-solid-hover", min: TEXT },
  { fg: "--text-on-accent", bg: "--danger-solid-pressed", min: TEXT },
  ...["danger", "warning", "success", "info"].flatMap((s) => [
    { fg: `--${s}-text`, bg: `--${s}-bg`, min: TEXT },
    { fg: `--${s}-text`, bg: "--surface-content", min: TEXT },
  ]),
  // Translucent overlays are composited over the surface they sit on.
  ...["--surface-content", "--surface-sidebar"].flatMap((bg) => [
    { fg: "--text-primary", bg: `${bg}+--selection-bg`, min: TEXT },
    { fg: "--text-primary", bg: `${bg}+--selection-bg-inactive`, min: TEXT },
    { fg: "--text-primary", bg: `${bg}+--surface-hover`, min: TEXT },
    { fg: "--text-secondary", bg: `${bg}+--surface-hover`, min: TEXT },
    { fg: "--text-primary", bg: `${bg}+--surface-pressed`, min: TEXT },
  ]),
  // Every terminal colour, the default text and the cursor must be readable on the terminal pane.
  ...[...ANSI_TOKENS, "--text-primary"].map((fg) => ({
    fg,
    bg: "--terminal-bg",
    min: TEXT,
    note: "terminal text",
  })),
  ...["--surface-content", "--surface-sidebar"].flatMap((bg) =>
    [
      "--protocol-folder",
      "--protocol-local",
      "--protocol-ssh",
      "--protocol-rdp",
      "--protocol-vnc",
      "--protocol-web",
    ].map((fg) => ({ fg, bg, min: GRAPHIC, note: "icon" })),
  ),
  // Node colours (a dot in the tree, a bar on a tab) on every surface they sit on, including the
  // selected, inactive-selected, hovered and drop-target row (translucent overlays over the sidebar).
  ...NODE_COLOR_TOKENS.flatMap((fg) =>
    [
      "--surface-window",
      "--surface-content",
      "--surface-sidebar",
      "--surface-sidebar+--selection-bg",
      "--surface-sidebar+--selection-bg-inactive",
      "--surface-sidebar+--surface-hover",
      "--surface-sidebar+--accent-subtle",
    ].map((bg) => ({ fg, bg, min: GRAPHIC, note: "node colour" })),
  ),
];

/** Resolves `--a` or `--base+--overlay` (overlay composited over base) to an opaque colour. */
function resolve(tokens: TokenMap, spec: string): Rgba {
  const [base, overlay] = spec.split("+");
  const baseValue = base ? tokens[base] : undefined;
  const baseColor = baseValue ? parseColor(baseValue) : null;
  if (!baseColor) throw new Error(`token ${base ?? spec} missing or not a colour`);
  if (!overlay) return baseColor;
  const overlayValue = tokens[overlay];
  const overlayColor = overlayValue ? parseColor(overlayValue) : null;
  if (!overlayColor) throw new Error(`token ${overlay} missing or not a colour`);
  return over(overlayColor, baseColor);
}

export interface Measured extends Pair {
  theme: Theme;
  value: number;
  pass: boolean;
}

export function measure(css: string, accent: string = "blue", scheme: string = "portolano"): Measured[] {
  const out: Measured[] = [];
  for (const theme of ["light", "dark"] as const) {
    const tokens = themeTokens(css, theme, accent, scheme);
    for (const pair of PAIRS) {
      const bg = resolve(tokens, pair.bg);
      // A translucent foreground (none today) would be composited over the background.
      const fgRaw = tokens[pair.fg];
      const fgParsed = fgRaw ? parseColor(fgRaw) : null;
      if (!fgParsed) throw new Error(`token ${pair.fg} missing or not a colour`);
      const value = ratio(over(fgParsed, bg), bg);
      out.push({ ...pair, theme, value, pass: value >= pair.min });
    }
  }
  return out;
}
