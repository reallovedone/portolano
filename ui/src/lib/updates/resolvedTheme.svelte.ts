// The look the page has right now, light or dark, whether the user chose it or follows the system.
// Call it while a component is being created: it listens to the system preference until the
// component is destroyed.

import { getApp } from "../state/app";
import type { ShotTheme } from "./screenshots";

const QUERY = "(prefers-color-scheme: dark)";

export function useResolvedTheme(): { readonly current: ShotTheme } {
  const { appearance } = getApp();
  const query = typeof window.matchMedia === "function" ? window.matchMedia(QUERY) : null;
  let systemDark = $state(query?.matches ?? false);

  $effect(() => {
    if (!query) return;
    const onChange = (event: MediaQueryListEvent): void => {
      systemDark = event.matches;
    };
    systemDark = query.matches;
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  });

  return {
    get current(): ShotTheme {
      const chosen = appearance.value.theme;
      if (chosen === "light" || chosen === "dark") return chosen;
      return systemDark ? "dark" : "light";
    },
  };
}
