// The screenshots shown next to the release notes. They are published by the project website, one
// light and one dark image per name and version, and are never part of the application.
//
// The list is fixed here, in the code: a version, a name and a caption must all come from
// `SCREENSHOTS` for an address to exist. Nothing from the release notes (untrusted text) takes
// part in building an address.

export type ShotTheme = "light" | "dark";

export interface Screenshot {
  /** File name without theme or extension: `<name>-<theme>.webp`. */
  name: string;
  /** What the picture shows. Used as its description and as the caption under it. */
  alt: string;
}

/** Where the website serves them: `<base><version>/<name>-<theme>.webp`. */
export const SCREENSHOT_BASE = "https://portolano.cloud/whats-new/";

/** Size of every image (16:10); the frame reserves it before the image has loaded. */
export const SCREENSHOT_WIDTH = 1600;
export const SCREENSHOT_HEIGHT = 1000;

/** Screenshots per released version. Files of a released version never change. */
export const SCREENSHOTS: ReadonlyMap<string, readonly Screenshot[]> = new Map([
  [
    "0.4.0",
    [
      { name: "settings", alt: "Settings: colour schemes, accent colour and density" },
      { name: "connections", alt: "Icons and colours in the connection list and on tabs" },
    ],
  ],
]);

const VERSION = /^\d+\.\d+\.\d+$/;

/** The screenshots of `version`; none for a version that is not listed (or not a version). */
export function screenshotsFor(version: string): readonly Screenshot[] {
  if (!VERSION.test(version)) return [];
  return SCREENSHOTS.get(version) ?? [];
}

/**
 * The address of one screenshot, or `null` when `version` is not a plain `X.Y.Z`, the pair
 * (version, name) is not in the list, or the theme is not light or dark.
 */
export function screenshotUrl(version: string, name: string, theme: ShotTheme): string | null {
  if (theme !== "light" && theme !== "dark") return null;
  if (!screenshotsFor(version).some((shot) => shot.name === name)) return null;
  return `${SCREENSHOT_BASE}${version}/${name}-${theme}.webp`;
}
