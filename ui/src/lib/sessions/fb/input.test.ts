import { describe, expect, it, vi } from "vitest";

import { InputController } from "./input";
import { ManualClock, fakeKey, flushPromises, recordingApi } from "./testing";

const geometry = () => ({
  rect: { left: 10, top: 20, width: 960, height: 540 },
  remote: { w: 1920, h: 1080 },
});

function setup(options: { delayMs?: number; onEscape?: () => void; mac?: boolean } = {}) {
  const clock = new ManualClock();
  const { api, calls } = recordingApi({ delayMs: options.delayMs });
  const onError = vi.fn();
  const input = new InputController({
    api,
    geometry,
    onEscape: options.onEscape,
    onError,
    macShortcuts: options.mac === undefined ? undefined : () => options.mac === true,
    ...clock,
  });
  return { input, calls, clock, onError };
}

const keys = (calls: ReturnType<typeof setup>["calls"]) =>
  calls.filter((c) => c.fn === "key").map((c) => (c.fn === "key" ? `${c.code}${c.down ? "+" : "-"}` : ""));

describe("keyboard", () => {
  it("sends mapped keys down and up, and keeps them from the shell and the browser", async () => {
    const { input, calls } = setup();
    const down = fakeKey("KeyA");
    const up = fakeKey("KeyA");
    input.keyDown(down);
    input.keyUp(up);
    await flushPromises();
    expect(keys(calls)).toEqual(["KeyA+", "KeyA-"]);
    expect(down.prevented && down.stopped && up.prevented && up.stopped).toBe(true);
  });

  it("lets unmapped keys through untouched", async () => {
    const { input, calls } = setup();
    const e = fakeKey("MediaPlayPause");
    input.keyDown(e);
    await flushPromises();
    expect(calls).toHaveLength(0);
    expect(e.prevented || e.stopped).toBe(false);
  });

  it("keeps the order of key events even when the backend is slow", async () => {
    const { input, calls } = setup({ delayMs: 3 });
    for (const code of ["KeyA", "KeyB", "KeyC"]) {
      input.keyDown(fakeKey(code));
      input.keyUp(fakeKey(code));
    }
    await vi.waitFor(() => expect(calls).toHaveLength(6));
    expect(keys(calls)).toEqual(["KeyA+", "KeyA-", "KeyB+", "KeyB-", "KeyC+", "KeyC-"]);
  });

  it("does not re-press a held modifier on auto-repeat but repeats other keys", async () => {
    const { input, calls } = setup();
    input.keyDown(fakeKey("ShiftLeft"));
    input.keyDown(fakeKey("ShiftLeft", { repeat: true }));
    input.keyDown(fakeKey("KeyA"));
    input.keyDown(fakeKey("KeyA", { repeat: true }));
    await flushPromises();
    expect(keys(calls)).toEqual(["ShiftLeft+", "KeyA+", "KeyA+"]);
  });

  it("releases every held key on releaseAll (blur, tab switch)", async () => {
    const { input, calls } = setup();
    input.keyDown(fakeKey("ControlLeft"));
    input.keyDown(fakeKey("ShiftLeft"));
    input.keyDown(fakeKey("KeyC"));
    expect(input.pressedKeys).toEqual(["ControlLeft", "ShiftLeft", "KeyC"]);
    input.releaseAll();
    await flushPromises();
    expect(keys(calls).slice(3).sort()).toEqual(["ControlLeft-", "KeyC-", "ShiftLeft-"]);
    expect(input.pressedKeys).toEqual([]);
    // Nothing is released twice.
    input.releaseAll();
    await flushPromises();
    expect(calls).toHaveLength(6);
  });

  it("releases a held mouse button on releaseAll", async () => {
    const { input, calls } = setup();
    input.pointerButton({ clientX: 10, clientY: 20, buttons: 1 });
    input.releaseAll();
    await flushPromises();
    expect(calls.at(-1)).toEqual({ fn: "pointer", x: 0, y: 0, buttons: 0, wheel: 0 });
  });

  it("Ctrl+Alt+Esc releases the keyboard and is not sent", async () => {
    const onEscape = vi.fn();
    const { input, calls } = setup({ onEscape });
    input.keyDown(fakeKey("ControlLeft"));
    input.keyDown(fakeKey("AltLeft"));
    const chord = fakeKey("Escape", { ctrlKey: true, altKey: true });
    input.keyDown(chord);
    await flushPromises();
    expect(onEscape).toHaveBeenCalledTimes(1);
    expect(chord.prevented && chord.stopped).toBe(true);
    expect(keys(calls)).toEqual(["ControlLeft+", "AltLeft+", "ControlLeft-", "AltLeft-"]);
    // A plain Escape is an ordinary key.
    input.keyDown(fakeKey("Escape"));
    await flushPromises();
    expect(keys(calls).at(-1)).toBe("Escape+");
  });

  it("releases keys pressed with Cmd at once (macOS sends no key-up for them)", async () => {
    const { input, calls } = setup();
    input.keyDown(fakeKey("MetaLeft", { metaKey: true }));
    input.keyDown(fakeKey("KeyC", { metaKey: true }));
    await flushPromises();
    expect(keys(calls)).toEqual(["MetaLeft+", "KeyC+", "KeyC-"]);
    input.keyUp(fakeKey("MetaLeft"));
    await flushPromises();
    expect(keys(calls).at(-1)).toBe("MetaLeft-");
    expect(input.pressedKeys).toEqual([]);
  });

  it("taps CapsLock once per toggle whichever event the platform sends", async () => {
    // Windows: down and up for one toggle.
    const win = setup();
    win.input.keyDown(fakeKey("CapsLock"));
    win.input.keyUp(fakeKey("CapsLock"));
    await flushPromises();
    expect(keys(win.calls)).toEqual(["CapsLock+", "CapsLock-"]);
    // macOS: down when switched on, only up when switched off.
    const mac = setup();
    mac.input.keyDown(fakeKey("CapsLock"));
    mac.input.keyUp(fakeKey("CapsLock"));
    mac.input.keyUp(fakeKey("CapsLock"));
    await flushPromises();
    expect(keys(mac.calls)).toEqual(["CapsLock+", "CapsLock-", "CapsLock+", "CapsLock-"]);
  });
});

describe("pointer", () => {
  it("maps client coordinates to the remote desktop", async () => {
    const { input, calls, clock } = setup();
    input.pointerMove({ clientX: 10 + 480, clientY: 20 + 270, buttons: 0 });
    clock.frame();
    await flushPromises();
    expect(calls).toEqual([{ fn: "pointer", x: 960, y: 540, buttons: 0, wheel: 0 }]);
  });

  it("sends at most one move per animation frame (the latest)", async () => {
    const { input, calls, clock } = setup();
    for (let i = 0; i < 20; i++) input.pointerMove({ clientX: 10 + i, clientY: 20, buttons: 0 });
    expect(clock.pendingFrames).toBe(1);
    clock.frame();
    await flushPromises();
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ x: 38 });
  });

  it("falls back to a timer when animation frames are paused", async () => {
    const { input, calls, clock } = setup();
    input.pointerMove({ clientX: 100, clientY: 100, buttons: 0 });
    clock.advance(50);
    await flushPromises();
    expect(calls).toHaveLength(1);
    expect(clock.pendingFrames).toBe(0);
  });

  it("sends button changes at once with the DOM buttons bitmask, superseding a pending move", async () => {
    const { input, calls, clock } = setup();
    input.pointerMove({ clientX: 50, clientY: 50, buttons: 0 });
    input.pointerButton({ clientX: 60, clientY: 60, buttons: 1 });
    input.pointerButton({ clientX: 60, clientY: 60, buttons: 0b101 });
    input.pointerButton({ clientX: 60, clientY: 60, buttons: 0 });
    clock.frame();
    await flushPromises();
    expect(calls.map((c) => (c.fn === "pointer" ? c.buttons : -1))).toEqual([1, 5, 0]);
  });

  it("sends the wheel in 120-unit notches, positive down", async () => {
    const { input, calls } = setup();
    input.wheel({ clientX: 10, clientY: 20, buttons: 0, deltaY: 100, deltaMode: 0 });
    input.wheel({ clientX: 10, clientY: 20, buttons: 0, deltaY: -1, deltaMode: 2 });
    input.wheel({ clientX: 10, clientY: 20, buttons: 0, deltaY: 0, deltaMode: 0 });
    await flushPromises();
    expect(calls.map((c) => (c.fn === "pointer" ? c.wheel : 0))).toEqual([120, -120]);
  });

  it("collapses queued moves while the backend is busy", async () => {
    const { input, calls, clock } = setup({ delayMs: 5 });
    input.keyDown(fakeKey("KeyA"));
    for (let i = 0; i < 5; i++) {
      input.pointerMove({ clientX: 10 + i, clientY: 20, buttons: 0 });
      clock.frame();
    }
    await vi.waitFor(() => expect(calls.length).toBeGreaterThanOrEqual(2));
    await new Promise((r) => setTimeout(r, 30));
    const moves = calls.filter((c) => c.fn === "pointer");
    expect(moves.length).toBeLessThan(5);
    expect(moves.at(-1)).toMatchObject({ x: 8 });
  });

  it("does nothing before the first desktop size is known", async () => {
    const clock = new ManualClock();
    const { api, calls } = recordingApi();
    const input = new InputController({ api, geometry: () => null, ...clock });
    input.pointerMove({ clientX: 1, clientY: 1, buttons: 0 });
    input.pointerButton({ clientX: 1, clientY: 1, buttons: 1 });
    clock.frame();
    await flushPromises();
    expect(calls).toHaveLength(0);
  });

  it("reports backend failures without stopping the queue", async () => {
    const clock = new ManualClock();
    const calls: string[] = [];
    const onError = vi.fn();
    const input = new InputController({
      api: {
        key: async (code) => {
          calls.push(code);
          if (code === "KeyA") throw new Error("boom");
        },
        pointer: async () => undefined,
      },
      geometry,
      onError,
      ...clock,
    });
    input.keyDown(fakeKey("KeyA"));
    input.keyDown(fakeKey("KeyB"));
    await flushPromises();
    expect(calls).toEqual(["KeyA", "KeyB"]);
    expect(onError).toHaveBeenCalledTimes(1);
  });
});

describe("Mac shortcuts", () => {
  const meta = { metaKey: true };

  it("sends Cmd+C as Ctrl+C, wrapped so the remote keeps no modifier", async () => {
    const { input, calls } = setup({ mac: true });
    input.keyDown(fakeKey("MetaLeft", meta));
    input.keyDown(fakeKey("KeyC", meta));
    input.keyUp(fakeKey("MetaLeft"));
    await flushPromises();
    expect(keys(calls)).toEqual(["ControlLeft+", "KeyC+", "KeyC-", "ControlLeft-"]);
    expect(input.pressedKeys).toEqual([]);
  });

  it("combines with the other modifiers: Cmd+Shift+Z is Ctrl+Shift+Z", async () => {
    const { input, calls } = setup({ mac: true });
    input.keyDown(fakeKey("MetaLeft", meta));
    input.keyDown(fakeKey("ShiftLeft", meta));
    input.keyDown(fakeKey("KeyZ", meta));
    input.keyUp(fakeKey("ShiftLeft", meta));
    input.keyUp(fakeKey("MetaLeft"));
    await flushPromises();
    expect(keys(calls)).toEqual([
      "ShiftLeft+",
      "ControlLeft+",
      "KeyZ+",
      "KeyZ-",
      "ControlLeft-",
      "ShiftLeft-",
    ]);
  });

  it("repeats a held Cmd shortcut as complete Ctrl shortcuts and handles several in a row", async () => {
    const { input, calls } = setup({ mac: true });
    input.keyDown(fakeKey("MetaLeft", meta));
    input.keyDown(fakeKey("KeyV", meta));
    input.keyDown(fakeKey("KeyV", { ...meta, repeat: true }));
    input.keyDown(fakeKey("KeyA", meta));
    await vi.waitFor(() => expect(calls).toHaveLength(12));
    expect(keys(calls)).toEqual([
      "ControlLeft+", "KeyV+", "KeyV-", "ControlLeft-",
      "ControlLeft+", "KeyV+", "KeyV-", "ControlLeft-",
      "ControlLeft+", "KeyA+", "KeyA-", "ControlLeft-",
    ]);
  });

  it("taps the Windows key when Cmd is pressed and released alone", async () => {
    const { input, calls } = setup({ mac: true });
    input.keyDown(fakeKey("MetaLeft", meta));
    input.keyDown(fakeKey("MetaLeft", { ...meta, repeat: true }));
    await flushPromises();
    expect(calls).toHaveLength(0);
    input.keyUp(fakeKey("MetaLeft"));
    await flushPromises();
    expect(keys(calls)).toEqual(["MetaLeft+", "MetaLeft-"]);
    expect(input.pressedKeys).toEqual([]);
  });

  it("does not tap the Windows key after a shortcut or a modifier was used with Cmd", async () => {
    const { input, calls } = setup({ mac: true });
    input.keyDown(fakeKey("MetaRight", meta));
    input.keyDown(fakeKey("ShiftLeft", meta));
    input.keyUp(fakeKey("ShiftLeft", meta));
    input.keyUp(fakeKey("MetaRight"));
    await flushPromises();
    expect(keys(calls)).toEqual(["ShiftLeft+", "ShiftLeft-"]);
    // The next lone press is a tap again.
    input.keyDown(fakeKey("MetaRight", meta));
    input.keyUp(fakeKey("MetaRight"));
    await flushPromises();
    expect(keys(calls).slice(2)).toEqual(["MetaRight+", "MetaRight-"]);
  });

  it("leaves a physical Ctrl alone: Ctrl is neither pressed twice nor released early", async () => {
    const { input, calls } = setup({ mac: true });
    input.keyDown(fakeKey("ControlLeft", { ctrlKey: true }));
    input.keyDown(fakeKey("MetaLeft", { ctrlKey: true, metaKey: true }));
    input.keyDown(fakeKey("KeyC", { ctrlKey: true, metaKey: true }));
    await flushPromises();
    expect(keys(calls)).toEqual(["ControlLeft+", "KeyC+", "KeyC-"]);
    input.keyUp(fakeKey("ControlLeft"));
    await flushPromises();
    expect(keys(calls).at(-1)).toBe("ControlLeft-");
  });

  it("sends a physical Ctrl+key as it is", async () => {
    const { input, calls } = setup({ mac: true });
    input.keyDown(fakeKey("ControlLeft", { ctrlKey: true }));
    input.keyDown(fakeKey("KeyC", { ctrlKey: true }));
    input.keyUp(fakeKey("KeyC", { ctrlKey: true }));
    input.keyUp(fakeKey("ControlLeft"));
    await flushPromises();
    expect(keys(calls)).toEqual(["ControlLeft+", "KeyC+", "KeyC-", "ControlLeft-"]);
  });

  it("forgets a held Cmd on releaseAll (blur, tab switch): nothing is sent, no tap follows", async () => {
    const { input, calls } = setup({ mac: true });
    input.keyDown(fakeKey("MetaLeft", meta));
    input.releaseAll();
    input.keyUp(fakeKey("MetaLeft"));
    await flushPromises();
    expect(calls).toHaveLength(0);
    // A shortcut after coming back still works, whatever the controller forgot.
    input.keyDown(fakeKey("KeyC", meta));
    await flushPromises();
    expect(keys(calls)).toEqual(["ControlLeft+", "KeyC+", "KeyC-", "ControlLeft-"]);
  });

  it("releases a held physical key on releaseAll as before", async () => {
    const { input, calls } = setup({ mac: true });
    input.keyDown(fakeKey("ShiftLeft"));
    input.keyDown(fakeKey("MetaLeft", meta));
    input.releaseAll();
    await flushPromises();
    expect(keys(calls)).toEqual(["ShiftLeft+", "ShiftLeft-"]);
  });

  it("keeps the escape chord working, with Cmd held or not", async () => {
    const onEscape = vi.fn();
    const { input, calls } = setup({ mac: true, onEscape });
    input.keyDown(fakeKey("MetaLeft", meta));
    const chord = fakeKey("Escape", { ctrlKey: true, altKey: true, metaKey: true });
    input.keyDown(chord);
    await flushPromises();
    expect(onEscape).toHaveBeenCalledTimes(1);
    expect(chord.prevented && chord.stopped).toBe(true);
    expect(calls).toHaveLength(0);
  });

  it("keeps keys the remote cannot take away from the shell and the browser alone", async () => {
    const { input, calls } = setup({ mac: true });
    const e = fakeKey("MediaPlayPause", meta);
    input.keyDown(e);
    await flushPromises();
    expect(calls).toHaveLength(0);
    expect(e.prevented || e.stopped).toBe(false);
  });

  it("behaves exactly as before when the option is off", async () => {
    for (const mac of [undefined, false]) {
      const { input, calls } = setup({ mac });
      input.keyDown(fakeKey("MetaLeft", meta));
      input.keyDown(fakeKey("KeyC", meta));
      input.keyUp(fakeKey("MetaLeft"));
      await flushPromises();
      expect(keys(calls)).toEqual(["MetaLeft+", "KeyC+", "KeyC-", "MetaLeft-"]);
    }
  });
});
