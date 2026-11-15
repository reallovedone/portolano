import { describe, expect, it } from "vitest";

import { fitToArea, toRemote, wheelUnits } from "./geometry";

describe("fitToArea", () => {
  it("fits by the limiting side and keeps the aspect ratio", () => {
    expect(fitToArea({ w: 1920, h: 1080 }, { w: 960, h: 800 })).toEqual({
      width: 960,
      height: 540,
      scale: 0.5,
    });
    expect(fitToArea({ w: 1000, h: 1000 }, { w: 500, h: 800 })).toEqual({
      width: 500,
      height: 500,
      scale: 0.5,
    });
  });

  it("scales small desktops up and handles empty sizes", () => {
    expect(fitToArea({ w: 800, h: 600 }, { w: 1600, h: 1200 }).scale).toBe(2);
    expect(fitToArea({ w: 0, h: 0 }, { w: 100, h: 100 })).toEqual({ width: 0, height: 0, scale: 1 });
    expect(fitToArea({ w: 100, h: 100 }, { w: 0, h: 0 }).width).toBe(0);
  });
});

describe("toRemote", () => {
  const remote = { w: 1920, h: 1080 };

  it("is the identity at scale 1 with an offset canvas", () => {
    const rect = { left: 100, top: 50, width: 1920, height: 1080 };
    expect(toRemote(100, 50, rect, remote)).toEqual({ x: 0, y: 0 });
    expect(toRemote(1100, 650, rect, remote)).toEqual({ x: 1000, y: 600 });
  });

  it("undoes the scale", () => {
    const rect = { left: 10, top: 20, width: 960, height: 540 };
    expect(toRemote(10 + 480, 20 + 270, rect, remote)).toEqual({ x: 960, y: 540 });
    expect(toRemote(10 + 100.4, 20 + 10, rect, remote)).toEqual({ x: 200, y: 20 });
  });

  it("clamps to the desktop while dragging outside", () => {
    const rect = { left: 0, top: 0, width: 960, height: 540 };
    expect(toRemote(-50, -50, rect, remote)).toEqual({ x: 0, y: 0 });
    expect(toRemote(5000, 5000, rect, remote)).toEqual({ x: 1919, y: 1079 });
    expect(toRemote(960, 540, rect, remote)).toEqual({ x: 1919, y: 1079 });
  });

  it("returns the origin for degenerate rectangles", () => {
    expect(toRemote(5, 5, { left: 0, top: 0, width: 0, height: 0 }, remote)).toEqual({ x: 0, y: 0 });
  });
});

describe("wheelUnits", () => {
  it("converts pixel, line and page deltas to 120-unit notches (positive is down)", () => {
    expect(wheelUnits(100, 0)).toBe(120);
    expect(wheelUnits(-100, 0)).toBe(-120);
    expect(wheelUnits(3, 1)).toBe(120);
    expect(wheelUnits(1, 2)).toBe(120);
    expect(wheelUnits(0, 0)).toBe(0);
  });

  it("clamps fast flings", () => {
    expect(wheelUnits(100000, 0)).toBe(1200);
    expect(wheelUnits(-100000, 0)).toBe(-1200);
  });
});
