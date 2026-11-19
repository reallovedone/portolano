import { describe, expect, it } from "vitest";

import { matchShortcut, shortcutLabel, type KeyInfo } from "./shortcuts";
import { passwordStrength } from "./vault/strength";

const key = (k: string, mods: Partial<KeyInfo> = {}): KeyInfo => ({
  key: k,
  code: "",
  metaKey: false,
  ctrlKey: false,
  shiftKey: false,
  altKey: false,
  ...mods,
});

describe("matchShortcut", () => {
  it("uses Cmd on macOS", () => {
    expect(matchShortcut(key("n", { metaKey: true }), true)).toBe("newConnection");
    expect(matchShortcut(key("k", { metaKey: true }), true)).toBe("search");
    expect(matchShortcut(key("w", { metaKey: true }), true)).toBe("closeTab");
    expect(matchShortcut(key("L", { metaKey: true, shiftKey: true }), true)).toBe("lockVault");
    expect(matchShortcut(key("}", { metaKey: true, shiftKey: true, code: "BracketRight" }), true)).toBe("nextTab");
    expect(matchShortcut(key("{", { metaKey: true, shiftKey: true, code: "BracketLeft" }), true)).toBe("prevTab");
    expect(matchShortcut(key("n", { ctrlKey: true }), true)).toBeNull();
  });

  it("reconnects the tab with Cmd+Shift+R on macOS and Ctrl+Shift+R on Windows", () => {
    expect(matchShortcut(key("R", { metaKey: true, shiftKey: true, code: "KeyR" }), true)).toBe("reconnectTab");
    expect(matchShortcut(key("r", { ctrlKey: true, shiftKey: true, code: "KeyR" }), false)).toBe("reconnectTab");
    expect(matchShortcut(key("r", { metaKey: true, code: "KeyR" }), true)).toBeNull();
    expect(matchShortcut(key("R", { ctrlKey: true, shiftKey: true }), true)).toBeNull();
    expect(matchShortcut(key("R", { metaKey: true, shiftKey: true }), false)).toBeNull();
    expect(shortcutLabel("reconnectTab", true)).toBe("⌘⇧R");
    expect(shortcutLabel("reconnectTab", false)).toBe("Ctrl+Shift+R");
  });

  it("opens Home with Cmd+Shift+H on macOS and Ctrl+Shift+H on Windows", () => {
    expect(matchShortcut(key("H", { metaKey: true, shiftKey: true, code: "KeyH" }), true)).toBe("openHome");
    expect(matchShortcut(key("h", { ctrlKey: true, shiftKey: true, code: "KeyH" }), false)).toBe("openHome");
    expect(matchShortcut(key("h", { metaKey: true, code: "KeyH" }), true)).toBeNull();
    expect(matchShortcut(key("H", { ctrlKey: true, shiftKey: true }), true)).toBeNull();
    expect(matchShortcut(key("H", { metaKey: true, shiftKey: true }), false)).toBeNull();
    expect(shortcutLabel("openHome", true)).toBe("⌘⇧H");
    expect(shortcutLabel("openHome", false)).toBe("Ctrl+Shift+H");
  });

  it("uses Ctrl on Windows", () => {
    expect(matchShortcut(key("n", { ctrlKey: true }), false)).toBe("newConnection");
    expect(matchShortcut(key("w", { ctrlKey: true }), false)).toBe("closeTab");
    expect(matchShortcut(key("l", { ctrlKey: true, shiftKey: true }), false)).toBe("lockVault");
    expect(matchShortcut(key("n", { metaKey: true }), false)).toBeNull();
  });

  it("cycles tabs with Ctrl+Tab on both platforms", () => {
    for (const mac of [true, false]) {
      expect(matchShortcut(key("Tab", { ctrlKey: true }), mac)).toBe("nextTab");
      expect(matchShortcut(key("Tab", { ctrlKey: true, shiftKey: true }), mac)).toBe("prevTab");
      expect(matchShortcut(key("PageDown", { ctrlKey: true }), mac)).toBe("nextTab");
      expect(matchShortcut(key("PageUp", { ctrlKey: true }), mac)).toBe("prevTab");
    }
  });

  it("ignores plain keys, Alt combinations and unknown shortcuts", () => {
    expect(matchShortcut(key("n"), false)).toBeNull();
    expect(matchShortcut(key("n", { ctrlKey: true, altKey: true }), false)).toBeNull();
    expect(matchShortcut(key("q", { ctrlKey: true }), false)).toBeNull();
  });

  it("labels shortcuts per platform", () => {
    expect(shortcutLabel("newConnection", true)).toBe("⌘N");
    expect(shortcutLabel("newConnection", false)).toBe("Ctrl+N");
    expect(shortcutLabel("nextTab", false)).toBe("Ctrl+Tab");
  });
});

describe("passwordStrength", () => {
  it("grades passwords", () => {
    expect(passwordStrength("")).toBe("empty");
    expect(passwordStrength("abc")).toBe("weak");
    expect(passwordStrength("aaaaaaaaaaaa")).toBe("weak");
    expect(passwordStrength("abcdefgh")).toBe("weak");
    expect(passwordStrength("abcdefghijkl")).toBe("fair");
    expect(passwordStrength("Abcdefgh1jkl")).toBe("good");
    expect(passwordStrength("correct-Horse-battery-9-staple")).toBe("strong");
  });
});
