import { afterEach, describe, expect, it } from "vitest";

import { detectPlatform, initPlatform } from "./platform";

describe("platform helper", () => {
  afterEach(() => {
    delete document.documentElement.dataset["platform"];
    delete document.documentElement.dataset["windowActive"];
  });

  it("detects macOS, Windows and others", () => {
    expect(detectPlatform({ platform: "MacIntel" })).toBe("macos");
    expect(detectPlatform({ userAgentData: { platform: "macOS" } })).toBe("macos");
    expect(detectPlatform({ platform: "Win32" })).toBe("windows");
    expect(detectPlatform({ userAgent: "Mozilla/5.0 (X11; Linux x86_64)" })).toBe("other");
    expect(detectPlatform(null)).toBe("other");
  });

  it("sets data-platform and tracks window activity, and undoes both", () => {
    const dispose = initPlatform(document.documentElement, "macos");
    expect(document.documentElement.dataset["platform"]).toBe("macos");
    expect(["true", "false"]).toContain(document.documentElement.dataset["windowActive"]);
    window.dispatchEvent(new Event("blur"));
    expect(document.documentElement.dataset["windowActive"]).toBe("false");
    dispose();
    expect(document.documentElement.dataset["platform"]).toBeUndefined();
    expect(document.documentElement.dataset["windowActive"]).toBeUndefined();
  });
});
