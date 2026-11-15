import { describe, expect, it } from "vitest";

import { FrameRenderer, type RectPixels } from "./renderer";
import { ManualClock } from "./testing";

const rect = (x: number, y: number, w = 2, h = 2): RectPixels => ({
  x,
  y,
  w,
  h,
  rgba: new Uint8Array(w * h * 4),
});

function setup(options: { maxPendingBytes?: number } = {}) {
  const clock = new ManualClock();
  const drawn: Array<{ x: number; y: number; w: number; h: number }> = [];
  const renderer = new FrameRenderer(
    {
      putImageData: (image, x, y) => {
        const { width, height } = image as unknown as { width: number; height: number };
        drawn.push({ x, y, w: width, h: height });
      },
    },
    {
      ...clock,
      makeImage: (_rgba, w, h) => ({ width: w, height: h }) as unknown as ImageData,
      maxPendingBytes: options.maxPendingBytes,
    },
  );
  return { clock, drawn, renderer };
}

describe("FrameRenderer", () => {
  it("batches every rectangle of one animation frame", () => {
    const { clock, drawn, renderer } = setup();
    renderer.enqueue(rect(0, 0));
    renderer.enqueue(rect(10, 10));
    renderer.enqueue(rect(20, 20));
    expect(drawn).toHaveLength(0);
    expect(renderer.pending).toBe(3);
    expect(clock.pendingFrames).toBe(1);

    clock.frame();
    expect(drawn.map((d) => d.x)).toEqual([0, 10, 20]);
    expect(renderer.stats).toMatchObject({ rects: 3, batches: 1, fallbackBatches: 0 });
    // The fallback timer was cancelled by the frame.
    expect(clock.pendingTimers).toBe(0);
  });

  it("draws through the timer when animation frames never come (hidden or occluded window)", () => {
    const { clock, drawn, renderer } = setup();
    renderer.enqueue(rect(1, 1));
    clock.advance(99);
    expect(drawn).toHaveLength(0);
    clock.advance(1);
    expect(drawn).toHaveLength(1);
    expect(renderer.stats.fallbackBatches).toBe(1);
    expect(clock.pendingFrames).toBe(0);
  });

  it("never accumulates beyond the byte budget", () => {
    const { drawn, renderer } = setup({ maxPendingBytes: 100 });
    renderer.enqueue(rect(0, 0, 4, 4)); // 64 bytes
    expect(drawn).toHaveLength(0);
    renderer.enqueue(rect(5, 5, 4, 4)); // 128 bytes >= 100: drawn at once
    expect(drawn).toHaveLength(2);
    expect(renderer.pending).toBe(0);
  });

  it("discards queued rectangles when the desktop changes size", () => {
    const { clock, drawn, renderer } = setup();
    renderer.enqueue(rect(0, 0));
    renderer.discard();
    clock.frame();
    clock.advance(1000);
    expect(drawn).toHaveLength(0);
    expect(clock.pendingFrames + clock.pendingTimers).toBe(0);
  });

  it("ignores empty or truncated rectangles", () => {
    const { clock, drawn, renderer } = setup();
    renderer.enqueue({ x: 0, y: 0, w: 0, h: 5, rgba: new Uint8Array(0) });
    renderer.enqueue({ x: 0, y: 0, w: 4, h: 4, rgba: new Uint8Array(8) });
    clock.frame();
    expect(drawn).toHaveLength(0);
  });
});
