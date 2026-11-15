// Pure helpers of the import and export flows: which format a file is, how its name is shown,
// counting and grouping what a preview contains. No IPC, no Svelte.

import type { ImportFormat, ImportPreview, ImportWarning, Node } from "../ipc";
import { buildTree, isConnection, nodeKind, type NodeKind, type TreeNode } from "../tree/model";

export const IMPORT_FORMATS: readonly ImportFormat[] = [
  "mobaxterm",
  "rdp",
  "mremoteng",
  "csv",
  "json",
  "sshConfig",
];

/** The last path segment, for both `/` and `\` separators. */
export function baseName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] ?? path;
}

/**
 * Guesses the format from the file name: `.mxtsessions`, `.rdp`, `.xml` (mRemoteNG, also
 * `confCons.xml`), `.csv`, `.json`, or a file called `config` (OpenSSH). `null` when unknown;
 * the user then picks the format by hand.
 */
export function detectImportFormat(path: string): ImportFormat | null {
  const name = baseName(path).toLowerCase();
  if (name.endsWith(".mxtsessions")) return "mobaxterm";
  if (name.endsWith(".rdp")) return "rdp";
  if (name === "confcons.xml" || name.endsWith(".xml")) return "mremoteng";
  if (name.endsWith(".csv")) return "csv";
  if (name.endsWith(".json")) return "json";
  if (name === "config" || name === "ssh_config") return "sshConfig";
  return null;
}

/** Counts of what a preview would create, per kind (folders included), zero kinds left out. */
export function countByKind(nodes: readonly Node[]): { kind: NodeKind; count: number }[] {
  const counts = new Map<NodeKind, number>();
  for (const node of nodes) counts.set(nodeKind(node), (counts.get(nodeKind(node)) ?? 0) + 1);
  const order: NodeKind[] = ["folder", "ssh", "rdp", "vnc", "web", "localShell"];
  return order.flatMap((kind) => {
    const count = counts.get(kind) ?? 0;
    return count > 0 ? [{ kind, count }] : [];
  });
}

export interface PreviewRow {
  node: Node;
  depth: number;
}

/** The nodes in display order (every folder expanded), for the read-only preview list. */
export function previewRows(nodes: readonly Node[]): PreviewRow[] {
  const rows: PreviewRow[] = [];
  const walk = (level: readonly TreeNode[], depth: number): void => {
    for (const entry of level) {
      rows.push({ node: entry.node, depth });
      walk(entry.children, depth + 1);
    }
  };
  walk(buildTree(nodes), 0);
  return rows;
}

export interface WarningSummary {
  /** Passwords and stored credentials of the source that were dropped (one per warning). */
  ignoredSecrets: number;
  /** Everything else, in file order. */
  others: ImportWarning[];
}

/**
 * The importers say "… never imported" for every password or credential they drop (one
 * warning each). Those are collapsed into a count; the other warnings stay individual.
 */
export function summarizeWarnings(warnings: readonly ImportWarning[]): WarningSummary {
  const others: ImportWarning[] = [];
  let ignoredSecrets = 0;
  for (const warning of warnings) {
    if (warning.message.includes("never imported")) ignoredSecrets += 1;
    else others.push(warning);
  }
  return { ignoredSecrets, others };
}

export function connectionCount(preview: ImportPreview): number {
  return preview.nodes.filter(isConnection).length;
}

/** A file-name stem safe on both macOS and Windows. */
export function safeFileStem(name: string): string {
  const cleaned = name
    .replace(/[\\/:*?"<>|\u0000-\u001f]/g, "-")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/^\.+/, "")
    .replace(/[. ]+$/, "");
  return cleaned === "" ? "connections" : cleaned.slice(0, 80);
}
