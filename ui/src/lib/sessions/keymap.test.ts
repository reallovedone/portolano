import { describe, expect, it } from "vitest";

import { isMappedKey, isModifier, mappedCodes, scancodeFor } from "./keymap";

describe("keymap", () => {
  it("maps ordinary keys to PS/2 set-1 scancodes", () => {
    expect(scancodeFor("KeyA")).toBe(0x1e);
    expect(scancodeFor("Digit1")).toBe(0x02);
    expect(scancodeFor("Enter")).toBe(0x1c);
    expect(scancodeFor("Space")).toBe(0x39);
    expect(scancodeFor("F12")).toBe(0x58);
    expect(scancodeFor("IntlBackslash")).toBe(0x56);
  });

  it("flags extended keys with 0xE000", () => {
    expect(scancodeFor("ArrowLeft")).toBe(0xe04b);
    expect(scancodeFor("NumpadEnter")).toBe(0xe01c);
    expect(scancodeFor("ControlRight")).toBe(0xe01d);
    expect(scancodeFor("AltRight")).toBe(0xe038);
    expect(scancodeFor("MetaLeft")).toBe(0xe05b);
    expect(scancodeFor("Delete")).toBe(0xe053);
  });

  it("does not map unknown codes, including inherited object keys", () => {
    expect(scancodeFor("Unidentified")).toBeUndefined();
    expect(scancodeFor("toString")).toBeUndefined();
    expect(scancodeFor("__proto__")).toBeUndefined();
    expect(isMappedKey("KeyZ")).toBe(true);
    expect(isMappedKey("MediaPlayPause")).toBe(false);
  });

  it("has a unique scancode per code and knows the modifiers", () => {
    const codes = mappedCodes();
    const values = codes.map((c) => scancodeFor(c));
    expect(new Set(values).size).toBe(values.length);
    expect(isModifier("ShiftLeft")).toBe(true);
    expect(isModifier("MetaRight")).toBe(true);
    expect(isModifier("KeyA")).toBe(false);
  });
});
