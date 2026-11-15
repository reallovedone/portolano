import { describe, expect, it } from "vitest";

import { resolveIpcMode } from "./index";

describe("resolveIpcMode", () => {
  it("prefers the query, then the build variable, then autodetection", () => {
    expect(resolveIpcMode("mock", "tauri", true)).toBe("mock");
    expect(resolveIpcMode(null, "mock", true)).toBe("mock");
    expect(resolveIpcMode(null, undefined, true)).toBe("tauri");
    expect(resolveIpcMode(null, undefined, false)).toBe("mock");
  });

  it("ignores unknown values", () => {
    expect(resolveIpcMode("bogus", "nope", true)).toBe("tauri");
  });
});
