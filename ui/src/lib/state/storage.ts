// Per-viewer UI conveniences in localStorage (tree expansion, sidebar width).
// Never store secrets or credential names here. Storage may be missing or throw.

export interface KeyValueStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

function defaultStore(): KeyValueStore | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    // Accessing localStorage can throw (blocked site data); the UI then simply forgets state.
    return null;
  }
}

export function loadJson<T>(
  key: string,
  fallback: T,
  validate: (value: unknown) => value is T,
  store: KeyValueStore | null = defaultStore(),
): T {
  if (!store) return fallback;
  try {
    const raw = store.getItem(key);
    if (raw === null) return fallback;
    const parsed: unknown = JSON.parse(raw);
    return validate(parsed) ? parsed : fallback;
  } catch {
    // Corrupt or unreadable value: fall back to the default.
    return fallback;
  }
}

export function saveJson(
  key: string,
  value: unknown,
  store: KeyValueStore | null = defaultStore(),
): boolean {
  if (!store) return false;
  try {
    store.setItem(key, JSON.stringify(value));
    return true;
  } catch {
    // Quota or blocked storage: the preference is just not remembered.
    return false;
  }
}

export const isStringArray = (v: unknown): v is string[] =>
  Array.isArray(v) && v.every((x) => typeof x === "string");

export const isNumber = (v: unknown): v is number => typeof v === "number" && Number.isFinite(v);
