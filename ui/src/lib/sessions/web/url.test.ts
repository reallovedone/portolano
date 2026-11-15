import { describe, expect, it } from "vitest";

import { normalizeUrl } from "./url";

describe("normalizeUrl", () => {
  it("keeps http and https addresses", () => {
    expect(normalizeUrl("https://example.com/a?b=1")).toEqual({
      ok: true,
      url: "https://example.com/a?b=1",
    });
    expect(normalizeUrl("  http://10.0.0.5:8080 ")).toEqual({ ok: true, url: "http://10.0.0.5:8080/" });
  });

  it("adds https to an address without a scheme", () => {
    expect(normalizeUrl("example.com")).toEqual({ ok: true, url: "https://example.com/" });
    expect(normalizeUrl("localhost:3000/x")).toEqual({ ok: true, url: "https://localhost:3000/x" });
  });

  it("rejects other schemes, empty input, whitespace and garbage", () => {
    for (const bad of [
      "",
      "   ",
      "file:///etc/passwd",
      "javascript:alert(1)",
      "ftp://example.com",
      "data:text/html,hi",
      "tauri://localhost",
      "two words",
      "http://",
      "https://exa mple.com",
    ]) {
      expect(normalizeUrl(bad), bad).toEqual({ ok: false });
    }
  });
});
