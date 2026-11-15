import { describe, expect, it } from "vitest";

import type { Node } from "../ipc";
import {
  baseName,
  countByKind,
  detectImportFormat,
  previewRows,
  safeFileStem,
  summarizeWarnings,
} from "./formats";

const folder = (id: string, parentId: string | null, name: string): Node => ({
  type: "folder",
  id,
  parentId,
  name,
  credential: { type: "inherit" },
});
const ssh = (id: string, parentId: string | null, name: string): Node => ({
  type: "connection",
  id,
  parentId,
  name,
  host: `${name}.example.org`,
  port: null,
  credential: { type: "none" },
  username: null,
  tags: [],
  notes: "",
  options: { type: "ssh", jumpHost: null, keepaliveSecs: null },
});

describe("detectImportFormat", () => {
  it("recognises the formats by extension or name, on both path styles", () => {
    expect(detectImportFormat("/home/a/MobaXterm Sessions.mxtsessions")).toBe("mobaxterm");
    expect(detectImportFormat("C:\\Users\\a\\Server.RDP")).toBe("rdp");
    expect(detectImportFormat("/x/confCons.xml")).toBe("mremoteng");
    expect(detectImportFormat("/x/anything.xml")).toBe("mremoteng");
    expect(detectImportFormat("/x/hosts.csv")).toBe("csv");
    expect(detectImportFormat("/x/export.json")).toBe("json");
    expect(detectImportFormat("/home/a/.ssh/config")).toBe("sshConfig");
  });

  it("returns null for anything else", () => {
    expect(detectImportFormat("/x/notes.txt")).toBeNull();
    expect(detectImportFormat("/x/config.bak")).toBeNull();
    expect(detectImportFormat("/x/")).toBeNull();
  });
});

describe("preview helpers", () => {
  const nodes: Node[] = [
    folder("f1", null, "Prod"),
    folder("f2", "f1", "Web"),
    ssh("c1", "f2", "web-01"),
    ssh("c2", null, "jump"),
  ];

  it("counts per kind, folders first, leaving out empty kinds", () => {
    expect(countByKind(nodes)).toEqual([
      { kind: "folder", count: 2 },
      { kind: "ssh", count: 2 },
    ]);
    expect(countByKind([])).toEqual([]);
  });

  it("lists the nodes depth first with their depth", () => {
    expect(previewRows(nodes).map((r) => [r.node.name, r.depth])).toEqual([
      ["Prod", 0],
      ["Web", 1],
      ["web-01", 2],
      ["jump", 0],
    ]);
  });

  it("collapses the dropped passwords into a count and keeps the other warnings", () => {
    const summary = summarizeWarnings([
      { message: "password ignored: passwords are never imported", line: 3 },
      { message: "invalid port ignored for 'x': 0", line: 5 },
      { message: "stored credential ignored: credentials from MobaXterm's password manager are never imported", line: 8 },
      { message: "no connections found in the file", line: null },
    ]);
    expect(summary.ignoredSecrets).toBe(2);
    expect(summary.others.map((w) => w.line)).toEqual([5, null]);
  });
});

describe("file names", () => {
  it("takes the last segment", () => {
    expect(baseName("/a/b/c.json")).toBe("c.json");
    expect(baseName("C:\\a\\b.rdp")).toBe("b.rdp");
    expect(baseName("plain")).toBe("plain");
  });

  it("makes safe stems for both platforms", () => {
    expect(safeFileStem("Prod / Web: \"main\"")).toBe("Prod - Web- -main-");
    expect(safeFileStem("  ..hidden. ")).toBe("hidden");
    expect(safeFileStem("???")).toBe("---");
    expect(safeFileStem("   ")).toBe("connections");
    expect(safeFileStem("x".repeat(200)).length).toBe(80);
  });
});
