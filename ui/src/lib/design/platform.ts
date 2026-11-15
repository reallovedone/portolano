// Platform and window helpers for the window chrome. The one place that detects the platform:
// it sets `data-platform` on <html> (tokens.css derives --titlebar-height from it) and tracks
// whether the window is focused (`data-window-active`, used for the inactive selection colour).

export type Platform = "macos" | "windows" | "other";

interface NavigatorWithHints {
  userAgentData?: { platform?: string };
  platform?: string;
  userAgent?: string;
}

export function detectPlatform(nav: NavigatorWithHints | null | undefined = globalThis.navigator): Platform {
  if (!nav) return "other";
  const hint = nav.userAgentData?.platform || nav.platform || nav.userAgent || "";
  if (/mac|iphone|ipad/i.test(hint)) return "macos";
  if (/win/i.test(hint)) return "windows";
  return "other";
}

/** Sets `data-platform` and starts tracking window focus. Returns a function that undoes both. */
export function initPlatform(
  root: HTMLElement = document.documentElement,
  platform: Platform = detectPlatform(),
): () => void {
  root.dataset["platform"] = platform;
  const set = (active: boolean): void => {
    root.dataset["windowActive"] = active ? "true" : "false";
  };
  const onFocus = (): void => set(true);
  const onBlur = (): void => set(false);
  set(document.hasFocus());
  window.addEventListener("focus", onFocus);
  window.addEventListener("blur", onBlur);
  return () => {
    window.removeEventListener("focus", onFocus);
    window.removeEventListener("blur", onBlur);
    delete root.dataset["platform"];
    delete root.dataset["windowActive"];
  };
}
