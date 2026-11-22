// Helpers for showing and checking what comes from a share file or from the user's input.
// Owner, signer and share names are written by other people: they are cleaned before display.

import { t } from "../i18n/strings";
import type { PendingHost, ShareLocation } from "../ipc";
import { safeFileStem } from "../transfer/formats";

/**
 * Unicode bidirectional controls (U+202A-U+202E embeddings and overrides, U+2066-U+2069
 * isolates) plus the marks U+200E, U+200F and U+061C. They can make a name read backwards or
 * hide what follows it, so they never reach the screen.
 */
const BIDI_CONTROLS = /[\u202A-\u202E\u2066-\u2069\u200E\u200F\u061C]/g;

/** Longest name shown as is (titles, notices, rows). */
export const NAME_MAX = 80;
/** Longest error text shown. */
export const ERROR_MAX = 300;
/** The skipped connections are listed up to this many names. */
export const SKIPPED_SHOWN = 5;

/**
 * A name from an untrusted file, safe to show: no bidi controls, no line breaks, and at most
 * `max` characters (cut between code points, with "…" added when something was left out).
 */
export function displayText(value: string | null | undefined, max: number = NAME_MAX): string {
  if (!value) return "";
  const clean = value.replace(BIDI_CONTROLS, "").replace(/[\u0000-\u001f\u007f]/g, " ").trim();
  const points = Array.from(clean);
  return points.length <= max ? clean : `${points.slice(0, max).join("").trimEnd()}…`;
}

/** One line for a connection of a share: "name: protocol user@host:port", all cleaned. */
export function hostLine(host: PendingHost): string {
  const port = host.port === null ? "" : `:${host.port}`;
  const user = host.username ? `${displayText(host.username)}@` : "";
  return t("share.pendingHost", {
    name: displayText(host.name),
    protocol: displayText(host.protocol, 10),
    target: `${user}${displayText(host.host, 400)}${port}`,
  });
}

export type LocationProblem = "empty" | "file" | "notHttps" | "credentials" | "invalid";

export type LocationCheck =
  | { ok: true; location: ShareLocation }
  | { ok: false; problem: LocationProblem };

/** `scheme://...`, which is how an address differs from a file path (`C:\x` has no `//`). */
const HAS_SCHEME = /^[a-z][a-z0-9+.-]*:\/\//i;

/**
 * What the "Add remote folder" field holds: an https address or a file path. Plain http and
 * every other scheme are refused (the file is signed, but the address must still not travel in
 * clear), and so are addresses with a user name or password in them (the backend refuses
 * them too, so the user hears it before the request).
 */
export function checkLocation(input: string): LocationCheck {
  const value = input.trim();
  if (value === "") return { ok: false, problem: "empty" };
  if (/^file:/i.test(value)) return { ok: false, problem: "file" };
  if (!HAS_SCHEME.test(value)) return { ok: true, location: { type: "path", value } };
  if (!/^https:\/\//i.test(value)) return { ok: false, problem: "notHttps" };
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return { ok: false, problem: "invalid" };
  }
  if (url.username !== "" || url.password !== "") return { ok: false, problem: "credentials" };
  // `https://user@host` is caught above; an `@` before the first path slash is the same thing
  // in an address the parser read differently.
  const authority = value.slice(value.indexOf("//") + 2).split(/[/?#]/, 1)[0] ?? "";
  if (authority === "") return { ok: false, problem: "invalid" };
  if (authority.includes("@")) return { ok: false, problem: "credentials" };
  if (url.hostname === "") return { ok: false, problem: "invalid" };
  return { ok: true, location: { type: "url", value } };
}

/** Suggested file name when sharing a folder. */
export function shareFileName(folderName: string): string {
  return `${safeFileStem(displayText(folderName) || "folder")}.portolano-share`;
}
