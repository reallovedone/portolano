// Applies the saved appearance (theme, colour scheme, accent, density) before the first paint, so a dark theme
// never starts as a white flash. It is a separate file, not an inline script, because the
// Content-Security-Policy only allows scripts from 'self'. It is loaded without `defer`, in the
// head, so it runs before the stylesheet is used for the first time.
//
// It reads the copy that the app keeps in localStorage (`src/lib/settings/appearance.ts` writes
// it and `appearance.test.ts` runs this file against that module: keep the two in step). The
// copy is only a cache; the app replaces it with `settings.json` once the backend answers.
(function () {
  try {
    var raw = window.localStorage.getItem("portolano.appearance");
    if (!raw) return;
    var saved = JSON.parse(raw);
    if (!saved || typeof saved !== "object") return;
    var root = document.documentElement;
    if (saved.theme === "light" || saved.theme === "dark") root.setAttribute("data-theme", saved.theme);
    if (["nord", "solarized", "rose-pine"].indexOf(saved.scheme) >= 0) {
      root.setAttribute("data-scheme", saved.scheme);
    }
    if (["teal", "purple", "orange", "graphite"].indexOf(saved.accent) >= 0) {
      root.setAttribute("data-accent", saved.accent);
    }
    if (saved.density === "compact") root.setAttribute("data-density", "compact");
  } catch (e) {
    // Storage blocked or damaged: the default look is shown, and the app fixes it once loaded.
  }
})();
