import { describe, expect, it } from "vitest";

import { SCREENSHOTS, SCREENSHOT_BASE, screenshotUrl, screenshotsFor } from "./screenshots";

describe("screenshot addresses", () => {
  it("are built from the list: base, version, name and theme", () => {
    expect(screenshotUrl("0.4.0", "settings", "light")).toBe(
      "https://portolano.cloud/whats-new/0.4.0/settings-light.webp",
    );
    expect(screenshotUrl("0.4.0", "connections", "dark")).toBe(
      "https://portolano.cloud/whats-new/0.4.0/connections-dark.webp",
    );
    expect(SCREENSHOT_BASE).toBe("https://portolano.cloud/whats-new/");
  });

  it("list the two screenshots of 0.4.0 with a description each", () => {
    expect(screenshotsFor("0.4.0").map((s) => s.name)).toEqual(["settings", "connections"]);
    for (const shot of screenshotsFor("0.4.0")) expect(shot.alt.length).toBeGreaterThan(10);
  });

  it("refuse anything that is not a plain X.Y.Z version", () => {
    for (const bad of ["", "0.4", "0.4.0-rc1", "v0.4.0", "0.4.0/", "../0.4.0", "0.4.0?x=1", "0.4.0\n", " 0.4.0", "__proto__", "constructor"]) {
      expect(screenshotsFor(bad)).toEqual([]);
      expect(screenshotUrl(bad, "settings", "light")).toBeNull();
    }
  });

  it("refuse versions and names that are not in the list", () => {
    expect(screenshotUrl("9.9.9", "settings", "light")).toBeNull();
    expect(screenshotUrl("0.4.0", "other", "light")).toBeNull();
    expect(screenshotUrl("0.4.0", "../settings", "light")).toBeNull();
    expect(screenshotUrl("0.4.0", "settings-light", "light")).toBeNull();
    expect(screenshotUrl("0.4.0", "", "light")).toBeNull();
    expect(screenshotUrl("0.4.0", "settings", "sepia" as never)).toBeNull();
  });

  it("only list plain names and versions", () => {
    for (const [version, shots] of SCREENSHOTS) {
      expect(version).toMatch(/^\d+\.\d+\.\d+$/);
      for (const shot of shots) expect(shot.name).toMatch(/^[a-z0-9-]+$/);
    }
  });
});
