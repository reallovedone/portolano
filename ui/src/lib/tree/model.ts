// Pure logic over the flat node list from `doc_get`: tree building, filtering, move
// validation and credential inheritance display. No IPC, no Svelte.

import type { Connection, ConnectionOptions, CredentialRef, Folder, Node, Uuid } from "../ipc";

export type ConnectionKind = ConnectionOptions["type"];
export type NodeKind = ConnectionKind | "folder";

export const CONNECTION_KINDS: readonly ConnectionKind[] = [
  "localShell",
  "ssh",
  "rdp",
  "vnc",
  "web",
];

export function nodeKind(node: Node): NodeKind {
  return node.type === "folder" ? "folder" : node.options.type;
}

export function isFolder(node: Node): node is { type: "folder" } & Folder {
  return node.type === "folder";
}

export function isConnection(node: Node): node is { type: "connection" } & Connection {
  return node.type === "connection";
}

export interface TreeNode {
  node: Node;
  children: TreeNode[];
}

/**
 * Display order (same rule as the backend): folders first; within a group, nodes with a
 * `sortIndex` by index, then the ones without, by name. Without any index this is the plain
 * alphabetical order of a 0.3.x file.
 */
export function compareNodes(a: Node, b: Node): number {
  if (a.type !== b.type) return a.type === "folder" ? -1 : 1;
  const ai = a.sortIndex;
  const bi = b.sortIndex;
  if (ai !== undefined && bi !== undefined && ai !== bi) return ai - bi;
  if (ai !== undefined && bi === undefined) return -1;
  if (ai === undefined && bi !== undefined) return 1;
  return a.name.localeCompare(b.name, undefined, { sensitivity: "base", numeric: true });
}

/**
 * Builds the forest. Folders sort before connections, then by manual order and name (see `compareNodes`). A node whose parent is
 * missing (or is not a folder) is shown at the top level instead of being lost; a node in a
 * parent cycle is also promoted to the top level.
 */
export function buildTree(nodes: readonly Node[]): TreeNode[] {
  const byId = new Map<Uuid, Node>(nodes.map((n) => [n.id, n]));
  const childrenOf = new Map<Uuid | null, Node[]>();

  const reachesRoot = (node: Node): boolean => {
    const seen = new Set<Uuid>([node.id]);
    let current: Node | undefined = node;
    while (current && current.parentId !== null) {
      const parent: Node | undefined = byId.get(current.parentId);
      if (!parent || parent.type !== "folder" || seen.has(parent.id)) return false;
      seen.add(parent.id);
      current = parent;
    }
    return true;
  };

  for (const node of nodes) {
    const parentKey = node.parentId !== null && reachesRoot(node) ? node.parentId : null;
    const list = childrenOf.get(parentKey);
    if (list) list.push(node);
    else childrenOf.set(parentKey, [node]);
  }

  const build = (parentKey: Uuid | null): TreeNode[] =>
    (childrenOf.get(parentKey) ?? [])
      .slice()
      .sort(compareNodes)
      .map((node) => ({ node, children: node.type === "folder" ? build(node.id) : [] }));

  return build(null);
}

/** Ids of `rootId` and everything below it. */
export function subtreeIds(nodes: readonly Node[], rootId: Uuid): Set<Uuid> {
  const ids = new Set<Uuid>([rootId]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const n of nodes) {
      if (n.parentId !== null && ids.has(n.parentId) && !ids.has(n.id)) {
        ids.add(n.id);
        grew = true;
      }
    }
  }
  return ids;
}

/** Number of nodes below a folder (the folder itself excluded). */
export function descendantCount(nodes: readonly Node[], folderId: Uuid): number {
  return subtreeIds(nodes, folderId).size - 1;
}

/** Folders from the top level down to (and including) the parent of `id`. */
export function ancestorIds(nodes: readonly Node[], id: Uuid): Uuid[] {
  const byId = new Map<Uuid, Node>(nodes.map((n) => [n.id, n]));
  const path: Uuid[] = [];
  const seen = new Set<Uuid>([id]);
  let parentId = byId.get(id)?.parentId ?? null;
  while (parentId !== null && !seen.has(parentId)) {
    seen.add(parentId);
    path.unshift(parentId);
    parentId = byId.get(parentId)?.parentId ?? null;
  }
  return path;
}

// ---- filtering ----

export interface FilterResult {
  /** Nodes that match the query themselves. */
  matches: Set<Uuid>;
  /** Matches plus their ancestors, plus the subtree of matching folders. */
  visible: Set<Uuid>;
}

function tokenize(query: string): string[] {
  return query.toLowerCase().split(/\s+/).filter(Boolean);
}

function connectionMatches(c: Connection, tokens: string[]): boolean {
  const haystacks = [c.name, c.host, ...c.tags].map((s) => s.toLowerCase());
  return tokens.every((token) => haystacks.some((h) => h.includes(token)));
}

/**
 * Quick filter: every whitespace-separated token must appear in the name, host or a tag of a
 * connection. A folder matches on its name and then shows its whole subtree. Returns `null`
 * for an empty query (no filtering).
 */
export function filterNodes(nodes: readonly Node[], query: string): FilterResult | null {
  const tokens = tokenize(query);
  if (tokens.length === 0) return null;

  const matches = new Set<Uuid>();
  for (const node of nodes) {
    if (node.type === "connection") {
      if (connectionMatches(node, tokens)) matches.add(node.id);
    } else {
      const name = node.name.toLowerCase();
      if (tokens.every((token) => name.includes(token))) matches.add(node.id);
    }
  }

  const visible = new Set<Uuid>();
  for (const id of matches) {
    for (const ancestor of ancestorIds(nodes, id)) visible.add(ancestor);
    const node = nodes.find((n) => n.id === id);
    if (node?.type === "folder") {
      for (const below of subtreeIds(nodes, id)) visible.add(below);
    } else {
      visible.add(id);
    }
  }
  return { matches, visible };
}

// ---- flattening for rendering / keyboard navigation ----

export interface TreeRow {
  node: Node;
  depth: number;
  expandable: boolean;
  expanded: boolean;
  /** Index among the siblings (1-based) and the number of siblings, for ARIA. */
  posInSet: number;
  setSize: number;
  parentId: Uuid | null;
}

/**
 * Visible rows in display order. When a filter is active, ancestors of matches are shown
 * expanded regardless of the remembered expansion state.
 */
export function flattenTree(
  roots: readonly TreeNode[],
  expanded: ReadonlySet<Uuid>,
  filter: FilterResult | null,
): TreeRow[] {
  const rows: TreeRow[] = [];
  const walk = (level: readonly TreeNode[], depth: number, parentId: Uuid | null): void => {
    const shown = filter ? level.filter((t) => filter.visible.has(t.node.id)) : level;
    shown.forEach((entry, index) => {
      const expandable = entry.node.type === "folder";
      const isExpanded = expandable && (filter ? true : expanded.has(entry.node.id));
      rows.push({
        node: entry.node,
        depth,
        expandable,
        expanded: isExpanded,
        posInSet: index + 1,
        setSize: shown.length,
        parentId,
      });
      if (isExpanded) walk(entry.children, depth + 1, entry.node.id);
    });
  };
  walk(roots, 0, null);
  return rows;
}

// ---- move validation ----

export type MoveCheck =
  | { ok: true }
  | { ok: false; reason: "missing" | "notFolder" | "intoSelf" | "unchanged" };

/** Validates a move before sending it to the backend (which re-validates). */
export function checkMove(nodes: readonly Node[], id: Uuid, newParentId: Uuid | null): MoveCheck {
  const node = nodes.find((n) => n.id === id);
  if (!node) return { ok: false, reason: "missing" };
  if (newParentId !== null) {
    const parent = nodes.find((n) => n.id === newParentId);
    if (!parent) return { ok: false, reason: "missing" };
    if (parent.type !== "folder") return { ok: false, reason: "notFolder" };
    if (subtreeIds(nodes, id).has(newParentId)) return { ok: false, reason: "intoSelf" };
  }
  if (node.parentId === newParentId) return { ok: false, reason: "unchanged" };
  return { ok: true };
}

/** Folders that `id` may be moved into, in tree order, with their depth for indentation. */
export function moveTargets(
  nodes: readonly Node[],
  id: Uuid,
): { id: Uuid | null; name: string; depth: number }[] {
  const targets: { id: Uuid | null; name: string; depth: number }[] = [];
  const walk = (level: readonly TreeNode[], depth: number): void => {
    for (const entry of level) {
      if (entry.node.type !== "folder") continue;
      if (checkMove(nodes, id, entry.node.id).ok) {
        targets.push({ id: entry.node.id, name: entry.node.name, depth });
      }
      walk(entry.children, depth + 1);
    }
  };
  walk(buildTree(nodes), 0);
  return targets;
}

// ---- manual ordering ----

/** Parent as shown: nodes `buildTree` promotes to the top level (orphans, cycles) count as roots. */
function shownParentId(nodes: readonly Node[], node: Node): Uuid | null {
  if (node.parentId === null) return null;
  return buildTree(nodes).some((t) => t.node.id === node.id) ? null : node.parentId;
}

/** Ids of the folders (or connections) under `parentId`, in display order. */
export function siblingIds(nodes: readonly Node[], parentId: Uuid | null, folders: boolean): Uuid[] {
  if (parentId === null) {
    // The root group is what is actually shown at the top level.
    return buildTree(nodes)
      .filter((t) => (t.node.type === "folder") === folders)
      .map((t) => t.node.id);
  }
  return nodes
    .filter((n) => n.parentId === parentId && (n.type === "folder") === folders)
    .sort(compareNodes)
    .map((n) => n.id);
}

/** Whether any node of that group has a manual position yet. */
export function groupHasIndices(
  nodes: readonly Node[],
  parentId: Uuid | null,
  folders: boolean,
): boolean {
  return nodes.some(
    (n) => n.parentId === parentId && (n.type === "folder") === folders && n.sortIndex !== undefined,
  );
}

export type DropEdge = "before" | "after";

/** Arguments for `nodeReorder`. */
export interface ReorderPlan {
  parentId: Uuid | null;
  before: Uuid | null;
  /** The target group as shown now. */
  visibleOrder: Uuid[];
}

/**
 * Places `id` before or after the sibling-to-be `targetId` (same kind: folders are never
 * mixed with connections). Dropping "after" X means before X's next sibling, or the end of the
 * group. Returns `null` when the drop is not allowed or would change nothing.
 */
export function planReorder(
  nodes: readonly Node[],
  id: Uuid,
  targetId: Uuid,
  edge: DropEdge,
): ReorderPlan | null {
  const node = nodes.find((n) => n.id === id);
  const target = nodes.find((n) => n.id === targetId);
  if (!node || !target || id === targetId || node.type !== target.type) return null;
  const parentId = shownParentId(nodes, target);
  if (parentId !== null && subtreeIds(nodes, id).has(parentId)) return null;
  const folders = node.type === "folder";
  const current = siblingIds(nodes, parentId, folders);
  const others = current.filter((other) => other !== id);
  const position = others.indexOf(targetId) + (edge === "after" ? 1 : 0);
  const before = others[position] ?? null;
  if (shownParentId(nodes, node) === parentId) {
    const next = [...others.slice(0, position), id, ...others.slice(position)];
    if (next.every((other, index) => other === current[index])) return null;
  }
  return { parentId, before, visibleOrder: current };
}

/** Plan for "Move up" (`-1`) or "Move down" (`1`) within the node's group; `null` at the edge. */
export function planStep(nodes: readonly Node[], id: Uuid, delta: -1 | 1): ReorderPlan | null {
  const node = nodes.find((n) => n.id === id);
  if (!node) return null;
  const group = siblingIds(nodes, shownParentId(nodes, node), node.type === "folder");
  const target = group[group.indexOf(id) + delta];
  if (target === undefined) return null;
  return planReorder(nodes, id, target, delta < 0 ? "before" : "after");
}

// ---- credential inheritance ----

export type EffectiveCredential =
  | { kind: "id"; id: Uuid; source: EffectiveSource }
  | { kind: "none"; source: EffectiveSource }
  | { kind: "unset" };

/** Where the effective value was set: the node itself or an ancestor folder. */
export type EffectiveSource = { own: true } | { own: false; folderId: Uuid; folderName: string };

/**
 * Resolves what a node with credential `ref` and parent `parentId` effectively uses: walks up
 * the folders until a non-inherit value is found (mirrors core-model `resolve_credential`).
 */
export function effectiveCredential(
  nodes: readonly Node[],
  ref: CredentialRef,
  parentId: Uuid | null,
): EffectiveCredential {
  if (ref.type === "id") return { kind: "id", id: ref.id, source: { own: true } };
  if (ref.type === "none") return { kind: "none", source: { own: true } };

  const byId = new Map<Uuid, Node>(nodes.map((n) => [n.id, n]));
  const seen = new Set<Uuid>();
  let currentId = parentId;
  while (currentId !== null && !seen.has(currentId)) {
    seen.add(currentId);
    const folder = byId.get(currentId);
    if (!folder || folder.type !== "folder") return { kind: "unset" };
    const source: EffectiveSource = { own: false, folderId: folder.id, folderName: folder.name };
    if (folder.credential.type === "id") {
      return { kind: "id", id: folder.credential.id, source };
    }
    if (folder.credential.type === "none") return { kind: "none", source };
    currentId = folder.parentId;
  }
  return { kind: "unset" };
}

/** Effective credential of an existing node. */
export function effectiveCredentialOf(nodes: readonly Node[], nodeId: Uuid): EffectiveCredential {
  const node = nodes.find((n) => n.id === nodeId);
  if (!node) return { kind: "unset" };
  return effectiveCredential(nodes, node.credential, node.parentId);
}

/** Where "new connection / folder" should go: the selected folder, or the selection's parent. */
export function defaultParentFor(nodes: readonly Node[], selectedId: Uuid | null): Uuid | null {
  const selected = selectedId === null ? undefined : nodes.find((n) => n.id === selectedId);
  if (!selected) return null;
  return selected.type === "folder" ? selected.id : selected.parentId;
}

/** All folders in tree order with their depth, optionally leaving out a subtree. */
export function folderOptions(
  nodes: readonly Node[],
  excludeSubtreeOf: Uuid | null = null,
): { id: Uuid; name: string; depth: number }[] {
  const excluded = excludeSubtreeOf === null ? new Set<Uuid>() : subtreeIds(nodes, excludeSubtreeOf);
  const options: { id: Uuid; name: string; depth: number }[] = [];
  const walk = (level: readonly TreeNode[], depth: number): void => {
    for (const entry of level) {
      if (entry.node.type !== "folder" || excluded.has(entry.node.id)) continue;
      options.push({ id: entry.node.id, name: entry.node.name, depth });
      walk(entry.children, depth + 1);
    }
  };
  walk(buildTree(nodes), 0);
  return options;
}

// ---- what a deletion leaves dangling ----

/**
 * SSH connections that stay in the document but use one of the `doomed` connections as their
 * jump host (the backend clears that reference when the deletion happens).
 */
export function jumpHostDependents(nodes: readonly Node[], doomed: ReadonlySet<Uuid>): Connection[] {
  return nodes.filter(
    (n): n is { type: "connection" } & Connection =>
      n.type === "connection" &&
      !doomed.has(n.id) &&
      n.options.type === "ssh" &&
      n.options.jumpHost !== null &&
      doomed.has(n.options.jumpHost),
  );
}

/** Folders and connections that point at the credential directly (inheriting ones are not counted). */
export function credentialUsers(nodes: readonly Node[], credentialId: Uuid): Node[] {
  return nodes.filter((n) => n.credential.type === "id" && n.credential.id === credentialId);
}
