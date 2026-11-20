// The appearance choices on the page: which attributes of <html> they set, and the copy in
// localStorage that `index.html` reads before the first paint (so a dark theme never starts as a
// white flash). The copy is only a cache; `settings.json` is the truth and replaces it as soon
// as the backend answers (`AppearanceStore.load`).
//
// KEEP IN STEP with `ui/public/appearance-init.js` (loaded by `ui/index.html`), which does the
// same job without modules: same key, same lists, same attributes. `appearance.test.ts` runs the real script
// against this module so the two cannot drift apart.

import {
  ACCENT_CHOICES,
  DEFAULT_APPEARANCE,
  DENSITY_CHOICES,
  SCHEME_CHOICES,
  TERMINAL_FONT_SIZE_MAX,
  TERMINAL_FONT_SIZE_MIN,
  THEME_CHOICES,
  type Appearance,
} from "../ipc/types";
import { loadJson, saveJson, type KeyValueStore } from "../state/storage";

export const APPEARANCE_KEY = "portolano.appearance";

const oneOf = <T extends string>(list: readonly T[], value: unknown, fallback: T): T =>
  typeof value === "string" && (list as readonly string[]).includes(value) ? (value as T) : fallback;

/** Anything from storage or the backend, reduced to valid choices (a bad field gets its default). */
export function normalizeAppearance(value: unknown): Appearance {
  const raw = typeof value === "object" && value !== null ? (value as Record<string, unknown>) : {};
  const size = raw["terminalFontSize"];
  return {
    theme: oneOf(THEME_CHOICES, raw["theme"], DEFAULT_APPEARANCE.theme),
    scheme: oneOf(SCHEME_CHOICES, raw["scheme"], DEFAULT_APPEARANCE.scheme),
    accent: oneOf(ACCENT_CHOICES, raw["accent"], DEFAULT_APPEARANCE.accent),
    density: oneOf(DENSITY_CHOICES, raw["density"], DEFAULT_APPEARANCE.density),
    terminalFontSize:
      typeof size === "number" &&
      Number.isInteger(size) &&
      size >= TERMINAL_FONT_SIZE_MIN &&
      size <= TERMINAL_FONT_SIZE_MAX
        ? size
        : DEFAULT_APPEARANCE.terminalFontSize,
  };
}

/**
 * Sets the attributes tokens.css reacts to. The default of each choice has no attribute (the
 * base tokens are the default look), and `system` leaves the theme to the system preference.
 */
export function applyAppearance(appearance: Appearance, root: HTMLElement = document.documentElement): void {
  const set = (name: string, value: string | null): void => {
    if (value === null) root.removeAttribute(name);
    else root.setAttribute(name, value);
  };
  set("data-theme", appearance.theme === "system" ? null : appearance.theme);
  set("data-scheme", appearance.scheme === "portolano" ? null : appearance.scheme);
  set("data-accent", appearance.accent === "blue" ? null : appearance.accent);
  set("data-density", appearance.density === "comfortable" ? null : appearance.density);
}

export function loadStoredAppearance(store?: KeyValueStore | null): Appearance {
  const isAny = (v: unknown): v is unknown => true;
  return normalizeAppearance(loadJson<unknown>(APPEARANCE_KEY, null, isAny, store));
}

export function saveStoredAppearance(appearance: Appearance, store?: KeyValueStore | null): boolean {
  return saveJson(APPEARANCE_KEY, appearance, store);
}
