import { describe, expect, it } from "vitest";

import type { Connection, CredentialRef, Folder, Node } from "../ipc";
import {
  ancestorIds,
  buildTree,
  checkMove,
  defaultParentFor,
  descendantCount,
  effectiveCredential,
  effectiveCredentialOf,
  filterNodes,
  flattenTree,
  folderOptions,
  moveTargets,
  nodeKind,
  subtreeIds,
} from "./model";

const folder = (
  id: string,
  parentId: string | null,
  name: string,
  credential: CredentialRef = { type: "inherit" },
): Node => ({ type: "folder", id, parentId, name, credential } satisfies { type: "folder" } & Folder);

const conn = (
  id: string,
  parentId: string | null,
  name: string,
  extra: Partial<Connection> = {},
): Node => ({
  type: "connection",
  id,
  parentId,
  name,
  host: `${name.toLowerCase()}.example`,
  port: null,
  credential: { type: "inherit" },
  username: null,
  tags: [],
  notes: "",
  options: { type: "vnc", viewOnly: false },
  ...extra,
});

//  root
//  ├─ A (folder, cred c1)
//  │   ├─ A1 (folder, inherit)
//  │   │   └─ deep (connection)
//  │   └─ web (connection, tags prod)
//  ├─ B (folder, none)
//  └─ zed (connection)
const nodes: Node[] = [
  conn("zed", null, "zed"),
  folder("B", null, "B", { type: "none" }),
  folder("A", null, "A", { type: "id", id: "c1" }),
  conn("web", "A", "web", { tags: ["prod"], options: { type: "web", url: "https://x", isolatedProfile: false } }),
  folder("A1", "A", "A1"),
  conn("deep", "A1", "deep", { host: "10.1.2.3", tags: ["Db"] }),
];

describe("buildTree", () => {
  it("nests nodes and sorts folders first, then names", () => {
    const tree = buildTree(nodes);
    expect(tree.map((t) => t.node.id)).toEqual(["A", "B", "zed"]);
    const a = tree[0];
    expect(a?.children.map((t) => t.node.id)).toEqual(["A1", "web"]);
    expect(a?.children[0]?.children.map((t) => t.node.id)).toEqual(["deep"]);
  });

  it("sorts names case-insensitively and numerically", () => {
    const list = [conn("1", null, "host10"), conn("2", null, "Host2"), conn("3", null, "alpha")];
    expect(buildTree(list).map((t) => t.node.name)).toEqual(["alpha", "Host2", "host10"]);
  });

  it("shows orphans and nodes in a parent cycle at the top level", () => {
    const broken: Node[] = [
      conn("o", "missing", "orphan"),
      folder("x", "y", "x"),
      folder("y", "x", "y"),
      conn("c", "o", "child-of-connection"),
    ];
    const ids = buildTree(broken).map((t) => t.node.id).sort();
    expect(ids).toEqual(["c", "o", "x", "y"]);
  });

  it("returns an empty forest for an empty document", () => {
    expect(buildTree([])).toEqual([]);
  });
});

describe("subtree helpers", () => {
  it("collects subtree ids, counts and ancestors", () => {
    expect([...subtreeIds(nodes, "A")].sort()).toEqual(["A", "A1", "deep", "web"]);
    expect(descendantCount(nodes, "A")).toBe(3);
    expect(descendantCount(nodes, "B")).toBe(0);
    expect(ancestorIds(nodes, "deep")).toEqual(["A", "A1"]);
    expect(ancestorIds(nodes, "zed")).toEqual([]);
  });
});

describe("filterNodes", () => {
  it("returns null for an empty query", () => {
    expect(filterNodes(nodes, "   ")).toBeNull();
  });

  it("matches name, host and tag, case-insensitively", () => {
    expect([...(filterNodes(nodes, "WEB")?.matches ?? [])]).toEqual(["web"]);
    expect([...(filterNodes(nodes, "10.1.2")?.matches ?? [])]).toEqual(["deep"]);
    expect([...(filterNodes(nodes, "db")?.matches ?? [])]).toEqual(["deep"]);
    expect([...(filterNodes(nodes, "prod")?.matches ?? [])]).toEqual(["web"]);
  });

  it("requires every token to match", () => {
    expect([...(filterNodes(nodes, "deep db")?.matches ?? [])]).toEqual(["deep"]);
    expect([...(filterNodes(nodes, "deep prod")?.matches ?? [])]).toEqual([]);
  });

  it("keeps ancestors of matches visible", () => {
    const result = filterNodes(nodes, "deep");
    expect([...(result?.visible ?? [])].sort()).toEqual(["A", "A1", "deep"]);
  });

  it("shows the whole subtree of a matching folder", () => {
    const result = filterNodes(nodes, "A1");
    expect(result?.matches.has("A1")).toBe(true);
    expect(result?.visible.has("deep")).toBe(true);
    expect(result?.visible.has("web")).toBe(false);
  });
});

describe("flattenTree", () => {
  const tree = buildTree(nodes);

  it("hides the children of collapsed folders", () => {
    const rows = flattenTree(tree, new Set(), null);
    expect(rows.map((r) => r.node.id)).toEqual(["A", "B", "zed"]);
    expect(rows[0]).toMatchObject({ depth: 0, expandable: true, expanded: false, posInSet: 1, setSize: 3 });
  });

  it("shows expanded folders with depth", () => {
    const rows = flattenTree(tree, new Set(["A", "A1"]), null);
    expect(rows.map((r) => [r.node.id, r.depth])).toEqual([
      ["A", 0],
      ["A1", 1],
      ["deep", 2],
      ["web", 1],
      ["B", 0],
      ["zed", 0],
    ]);
    expect(rows[2]?.parentId).toBe("A1");
  });

  it("expands the path to matches while filtering", () => {
    const rows = flattenTree(tree, new Set(), filterNodes(nodes, "deep"));
    expect(rows.map((r) => r.node.id)).toEqual(["A", "A1", "deep"]);
  });
});

describe("checkMove", () => {
  it("allows moving into a folder or to the top level", () => {
    expect(checkMove(nodes, "zed", "A")).toEqual({ ok: true });
    expect(checkMove(nodes, "deep", null)).toEqual({ ok: true });
    expect(checkMove(nodes, "A1", "B")).toEqual({ ok: true });
  });

  it("refuses to drop a folder into itself or a descendant", () => {
    expect(checkMove(nodes, "A", "A")).toEqual({ ok: false, reason: "intoSelf" });
    expect(checkMove(nodes, "A", "A1")).toEqual({ ok: false, reason: "intoSelf" });
  });

  it("refuses a connection as parent, unknown nodes and no-op moves", () => {
    expect(checkMove(nodes, "A", "zed")).toEqual({ ok: false, reason: "notFolder" });
    expect(checkMove(nodes, "nope", "A")).toEqual({ ok: false, reason: "missing" });
    expect(checkMove(nodes, "A", "nope")).toEqual({ ok: false, reason: "missing" });
    expect(checkMove(nodes, "web", "A")).toEqual({ ok: false, reason: "unchanged" });
    expect(checkMove(nodes, "A", null)).toEqual({ ok: false, reason: "unchanged" });
  });

  it("lists valid targets only", () => {
    expect(moveTargets(nodes, "A").map((t) => t.id)).toEqual(["B"]);
    expect(moveTargets(nodes, "zed").map((t) => [t.id, t.depth])).toEqual([
      ["A", 0],
      ["A1", 1],
      ["B", 0],
    ]);
    expect(folderOptions(nodes, "A").map((f) => f.id)).toEqual(["B"]);
    expect(folderOptions(nodes).map((f) => f.id)).toEqual(["A", "A1", "B"]);
  });
});

describe("effective credential", () => {
  it("uses the node's own reference when it is not inherit", () => {
    expect(effectiveCredential(nodes, { type: "id", id: "x" }, "A")).toEqual({
      kind: "id",
      id: "x",
      source: { own: true },
    });
    expect(effectiveCredential(nodes, { type: "none" }, "A")).toEqual({
      kind: "none",
      source: { own: true },
    });
  });

  it("walks up to the closest folder that sets a value", () => {
    expect(effectiveCredentialOf(nodes, "deep")).toEqual({
      kind: "id",
      id: "c1",
      source: { own: false, folderId: "A", folderName: "A" },
    });
  });

  it("reports an inherited 'none' and the folder it comes from", () => {
    const list = [...nodes, conn("inB", "B", "inB")];
    expect(effectiveCredentialOf(list, "inB")).toEqual({
      kind: "none",
      source: { own: false, folderId: "B", folderName: "B" },
    });
  });

  it("is unset when nothing on the path sets a value, and survives cycles", () => {
    expect(effectiveCredentialOf(nodes, "zed")).toEqual({ kind: "unset" });
    const cyc: Node[] = [folder("x", "y", "x"), folder("y", "x", "y"), conn("c", "x", "c")];
    expect(effectiveCredentialOf(cyc, "c")).toEqual({ kind: "unset" });
    expect(effectiveCredentialOf(nodes, "missing")).toEqual({ kind: "unset" });
  });

  it("resolves a draft against a prospective parent", () => {
    expect(effectiveCredential(nodes, { type: "inherit" }, "A1")).toMatchObject({
      kind: "id",
      id: "c1",
    });
    expect(effectiveCredential(nodes, { type: "inherit" }, null)).toEqual({ kind: "unset" });
  });
});

describe("misc", () => {
  it("names node kinds and picks the default parent", () => {
    expect(nodeKind(nodes[0] as Node)).toBe("vnc");
    expect(nodeKind(nodes[1] as Node)).toBe("folder");
    expect(defaultParentFor(nodes, "A")).toBe("A");
    expect(defaultParentFor(nodes, "deep")).toBe("A1");
    expect(defaultParentFor(nodes, "zed")).toBeNull();
    expect(defaultParentFor(nodes, null)).toBeNull();
  });
});
