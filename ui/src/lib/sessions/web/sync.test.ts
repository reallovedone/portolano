import { describe, expect, it } from "vitest";

import type { Bounds } from "../../ipc";
import { BoundsSync } from "./sync";

const rect = (x: number, y: number, width: number, height: number): Bounds => ({ x, y, width, height });

function setup(initial: Bounds | null = rect(10, 20, 300, 200)) {
  const state = { measured: initial as Bounds | null, show: true };
  const sent: [Bounds, boolean][] = [];
  const errors: unknown[] = [];
  const frames: (() => void)[] = [];
  let gate: (() => void) | null = null;
  const sync = new BoundsSync({
    measure: () => state.measured,
    shouldShow: () => state.show,
    send: async (bounds, visible) => {
      sent.push([bounds, visible]);
      if (gate) await new Promise<void>((resolve) => (gate = resolve));
    },
    onError: (e) => errors.push(e),
    schedule: (cb) => {
      frames.push(cb);
      return () => frames.splice(frames.indexOf(cb), 1);
    },
  });
  const frame = async (): Promise<void> => {
    frames.splice(0).forEach((cb) => cb());
    await Promise.resolve();
    await Promise.resolve();
  };
  return { state, sent, errors, frames, sync, frame, hold: () => (gate = () => undefined), release: () => gate?.() };
}

describe("BoundsSync", () => {
  it("coalesces requests into one send per frame", async () => {
    const t = setup();
    t.sync.request();
    t.sync.request();
    t.sync.request();
    expect(t.frames).toHaveLength(1);
    await t.frame();
    expect(t.sent).toEqual([[rect(10, 20, 300, 200), true]]);
  });

  it("skips a send identical to the previous one", async () => {
    const t = setup();
    t.sync.request();
    await t.frame();
    t.sync.request();
    await t.frame();
    expect(t.sent).toHaveLength(1);
    t.state.measured = rect(10, 20, 301, 200);
    t.sync.request();
    await t.frame();
    expect(t.sent).toHaveLength(2);
  });

  it("flushNow sends immediately and cancels the pending frame", async () => {
    const t = setup();
    t.sync.request();
    t.sync.flushNow();
    expect(t.frames).toHaveLength(0);
    await Promise.resolve();
    expect(t.sent).toHaveLength(1);
  });

  it("hides immediately, shows on the next frame", async () => {
    const t = setup();
    t.sync.update();
    await t.frame();
    expect(t.sent.at(-1)?.[1]).toBe(true);
    t.state.show = false;
    t.sync.update();
    await Promise.resolve();
    expect(t.sent.at(-1)?.[1]).toBe(false);
    t.state.show = true;
    t.sync.update();
    expect(t.frames).toHaveLength(1);
    await t.frame();
    expect(t.sent.at(-1)?.[1]).toBe(true);
    expect(t.sent).toHaveLength(3);
  });

  it("keeps the last real rect, hidden, when the placeholder collapses", async () => {
    const t = setup();
    t.sync.request();
    await t.frame();
    t.state.measured = rect(0, 0, 0, 0);
    t.sync.request();
    await t.frame();
    expect(t.sent.at(-1)).toEqual([rect(10, 20, 300, 200), false]);
  });

  it("sends nothing while there is no layout yet", async () => {
    const t = setup(null);
    t.sync.request();
    await t.frame();
    expect(t.sent).toEqual([]);
  });

  it("never has two sends in flight and ends with the latest state", async () => {
    const t = setup();
    t.hold();
    t.sync.flushNow();
    await Promise.resolve();
    expect(t.sent).toHaveLength(1);
    t.state.measured = rect(1, 2, 3, 4);
    t.sync.flushNow();
    await Promise.resolve();
    expect(t.sent).toHaveLength(1);
    t.release();
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
    expect(t.sent.at(-1)).toEqual([rect(1, 2, 3, 4), true]);
  });

  it("reports send errors and keeps going", async () => {
    const errors: unknown[] = [];
    let fail = true;
    const frames: (() => void)[] = [];
    const sync = new BoundsSync({
      measure: () => rect(0, 0, 10, 10),
      shouldShow: () => true,
      send: async () => {
        if (fail) throw new Error("boom");
      },
      onError: (e) => errors.push(e),
      schedule: (cb) => {
        frames.push(cb);
        return () => undefined;
      },
    });
    sync.request();
    frames.splice(0).forEach((cb) => cb());
    await Promise.resolve();
    await Promise.resolve();
    expect(errors).toHaveLength(1);
    fail = false;
    sync.request();
    frames.splice(0).forEach((cb) => cb());
    await Promise.resolve();
    await Promise.resolve();
    expect(errors).toHaveLength(1);
  });

  it("does nothing after dispose", async () => {
    const t = setup();
    t.sync.request();
    t.sync.dispose();
    t.sync.request();
    t.sync.flushNow();
    await t.frame();
    expect(t.sent).toEqual([]);
  });
});
