// Data and helpers for the welcome screen shown when no tab is open.

import type { StringKey } from "../i18n/strings";
import type { NodeKind } from "../tree/model";

export type TileKind = Exclude<NodeKind, "folder">;

export interface WelcomeTile {
  kind: TileKind;
  /** Default port shown as a caption; null shows the generic "local" caption instead. */
  port: number | null;
}

/** The quick-start tiles, in display order. Ports mirror the editor defaults. */
export const WELCOME_TILES: readonly WelcomeTile[] = [
  { kind: "ssh", port: 22 },
  { kind: "rdp", port: 3389 },
  { kind: "vnc", port: 5900 },
  { kind: "web", port: 443 },
  { kind: "localShell", port: null },
];

export const TILE_LABEL: Record<TileKind, StringKey> = {
  ssh: "kind.ssh",
  rdp: "kind.rdp",
  vnc: "kind.vnc",
  web: "kind.web",
  localShell: "kind.localShell",
};

/** Splits a shortcut label such as "⌘N", "⌘⇧L" or "Ctrl+N" into its individual keys. */
export function splitShortcut(label: string): string[] {
  if (label.includes("+") && !label.endsWith("+")) return label.split("+");
  return [...label];
}
