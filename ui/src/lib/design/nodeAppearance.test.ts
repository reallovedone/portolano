/// <reference types="node" />
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { NODE_ICONS, nodeColorVar, nodeIcon } from "./icons/nodeIcons";
import { isNodeColor, isNodeIcon, NODE_COLORS, NODE_ICON_NAMES } from "./nodeAppearance";

// Rust validates icon and colour against this file. The test reads it (rather than the app
// importing it from the crate) so the bundle has no path into the crate and any difference fails.
// Tests run from the ui folder.
const shared = JSON.parse(
  readFileSync(join(process.cwd(), "../crates/core-model/src/appearance.json"), "utf8"),
) as { icons: string[]; colors: string[] };

describe("node appearance lists", () => {
  it("offers exactly the icons of crates/core-model/src/appearance.json, in the same order", () => {
    expect([...NODE_ICON_NAMES]).toEqual(shared.icons);
  });

  it("offers exactly the colours of appearance.json, in the same order", () => {
    expect([...NODE_COLORS]).toEqual(shared.colors);
  });

  it("has a component for every icon and nothing else", () => {
    expect(Object.keys(NODE_ICONS)).toEqual(shared.icons);
    for (const name of shared.icons) expect(NODE_ICONS[name as keyof typeof NODE_ICONS], name).toBeTruthy();
  });

  it("recognises only known names", () => {
    expect(isNodeIcon("server")).toBe(true);
    expect(isNodeIcon("toString")).toBe(false);
    expect(isNodeIcon("")).toBe(false);
    expect(isNodeIcon(undefined)).toBe(false);
    expect(isNodeColor("teal")).toBe(true);
    expect(isNodeColor("magenta")).toBe(false);
    expect(nodeIcon("server")).toBe(NODE_ICONS.server);
    expect(nodeIcon("nope")).toBeUndefined();
    expect(nodeColorVar("teal")).toBe("var(--node-teal)");
    expect(nodeColorVar("magenta")).toBeUndefined();
    expect(nodeColorVar(undefined)).toBeUndefined();
  });
});
