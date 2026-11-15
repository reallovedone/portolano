import { describe, expect, it } from "vitest";

import { WELCOME_TILES, splitShortcut } from "./welcome";

describe("splitShortcut", () => {
  it("splits macOS symbol shortcuts into single keys", () => {
    expect(splitShortcut("⌘N")).toEqual(["⌘", "N"]);
    expect(splitShortcut("⌘⇧L")).toEqual(["⌘", "⇧", "L"]);
  });

  it("splits plus-joined shortcuts", () => {
    expect(splitShortcut("Ctrl+N")).toEqual(["Ctrl", "N"]);
    expect(splitShortcut("Ctrl+Shift+Tab")).toEqual(["Ctrl", "Shift", "Tab"]);
  });
});

describe("welcome tiles", () => {
  it("lists the five connection kinds with their default ports", () => {
    expect(WELCOME_TILES.map((x) => [x.kind, x.port])).toEqual([
      ["ssh", 22],
      ["rdp", 3389],
      ["vnc", 5900],
      ["web", 443],
      ["localShell", null],
    ]);
  });
});
