// Address-bar input -> a URL the backend accepts (only http and https are ever loaded).

export type UrlResult = { ok: true; url: string } | { ok: false };

const HAS_SCHEME = /^[a-z][a-z0-9+.-]*:\/\//i;

/**
 * Normalises what the user typed. An address without a scheme ("example.com", "10.0.0.5:8080")
 * gets `https://`. Anything with another scheme, whitespace inside, or no host is invalid. The
 * backend validates again: this only avoids a round trip and gives the error inline.
 */
export function normalizeUrl(input: string): UrlResult {
  const text = input.trim();
  if (text === "" || /\s/.test(text)) return { ok: false };
  const candidate = HAS_SCHEME.test(text) ? text : `https://${text}`;
  let parsed: URL;
  try {
    parsed = new URL(candidate);
  } catch {
    return { ok: false };
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return { ok: false };
  if (parsed.hostname === "") return { ok: false };
  return { ok: true, url: parsed.href };
}
