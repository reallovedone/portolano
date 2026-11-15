// Display helpers shared by the session views.

import type { Connection } from "../ipc";

/** Default port per protocol, used when the connection leaves the port empty. */
const DEFAULT_PORT: Partial<Record<Connection["options"]["type"], number>> = {
  ssh: 22,
  rdp: 3389,
  vnc: 5900,
};

/** `host:port` of a connection for display, or `null` when there is no host (local shell). */
export function targetLabel(connection: Connection | null): string | null {
  if (!connection || connection.host === "") return null;
  const port = connection.port ?? DEFAULT_PORT[connection.options.type] ?? null;
  return port === null ? connection.host : `${connection.host}:${port}`;
}

export interface MismatchDetails {
  /** The part of the message before the key list. */
  summary: string;
  stored: string | null;
  presented: string | null;
}

/**
 * The host and port a host key mismatch is about, read from the backend's message ("The host key
 * of HOST:PORT has CHANGED ..."). With jump hosts this is the hop whose key changed, which is
 * not necessarily the connection's own host.
 */
export function parseMismatchHost(message: string): { host: string; port: number } | null {
  const match = /^The host key of (.+) has CHANGED/s.exec(message.trim());
  const at = match?.[1];
  if (!at) return null;
  const colon = at.lastIndexOf(":");
  if (colon <= 0) return null;
  const port = Number(at.slice(colon + 1));
  const host = at.slice(0, colon);
  if (!Number.isInteger(port) || port < 1 || port > 65535 || host === "") return null;
  return { host, port };
}

/** Splits the backend's host key mismatch message ("... Stored: A. Presented: B.") so the two
 *  keys can be compared side by side. Falls back to the whole message. */
export function parseMismatch(message: string): MismatchDetails {
  const match = /^(.*?)\s*Stored:\s*(.*?)\.\s*Presented:\s*(.*?)\.?$/s.exec(message.trim());
  if (!match) return { summary: message, stored: null, presented: null };
  return { summary: (match[1] ?? "").trim(), stored: match[2] ?? null, presented: match[3] ?? null };
}
