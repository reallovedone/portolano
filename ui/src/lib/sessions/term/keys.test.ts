import { describe, expect, it } from "vitest";

import type { KeyInfo } from "../../shortcuts";
import { clipboardAction, keepForTerminal } from "./keys";

const key = (k: string, mods: Partial<KeyInfo> = {}): KeyInfo => ({
  key: k,
  code: "",
  metaKey: false,
  ctrlKey: false,
  shiftKey: false,
  altKey: false,
  ...mods,
});

describe("clipboardAction on macOS", () => {
  it("Cmd+C copies only with a selection", () => {
    expect(clipboardAction(key("c", { metaKey: true }), true, true)).toBe("copy");
    expect(clipboardAction(key("c", { metaKey: true }), true, false)).toBeNull();
  });

  it("Cmd+V pastes", () => {
    expect(clipboardAction(key("v", { metaKey: true }), true, false)).toBe("paste");
  });

  it("Ctrl+V is not a paste (it stays with the remote program)", () => {
    expect(clipboardAction(key("v", { ctrlKey: true }), true, false)).toBeNull();
  });

  it("Ctrl+C is the interrupt, never a copy", () => {
    expect(clipboardAction(key("c", { ctrlKey: true }), true, true)).toBeNull();
  });
});

describe("clipboardAction on Windows and Linux", () => {
  it("Ctrl+Shift+C / Ctrl+Shift+V", () => {
    expect(clipboardAction(key("C", { ctrlKey: true, shiftKey: true }), false, false)).toBe("copy");
    expect(clipboardAction(key("V", { ctrlKey: true, shiftKey: true }), false, false)).toBe("paste");
  });

  it("Ctrl+C copies with a selection and is SIGINT without", () => {
    expect(clipboardAction(key("c", { ctrlKey: true }), false, true)).toBe("copy");
    expect(clipboardAction(key("c", { ctrlKey: true }), false, false)).toBeNull();
  });

  it("Ctrl+Insert copies, Shift+Insert pastes", () => {
    expect(clipboardAction(key("Insert", { ctrlKey: true }), false, true)).toBe("copy");
    expect(clipboardAction(key("Insert", { shiftKey: true }), false, false)).toBe("paste");
  });

  it("plain Ctrl+V pastes, like Windows Terminal", () => {
    expect(clipboardAction(key("v", { ctrlKey: true }), false, false)).toBe("paste");
    expect(clipboardAction(key("v", { ctrlKey: true }), false, true)).toBe("paste");
  });

  it("ignores Alt and Meta combinations", () => {
    expect(clipboardAction(key("c", { ctrlKey: true, altKey: true }), false, true)).toBeNull();
    expect(clipboardAction(key("c", { metaKey: true }), false, true)).toBeNull();
  });
});

describe("keepForTerminal", () => {
  it("keeps Ctrl+W, Ctrl+K and Ctrl+N from the shell on Windows/Linux", () => {
    for (const k of ["w", "k", "n"]) {
      expect(keepForTerminal(key(k, { ctrlKey: true }), false)).toBe(true);
    }
  });

  it("leaves tab switching global", () => {
    expect(keepForTerminal(key("Tab", { ctrlKey: true }), false)).toBe(false);
    expect(keepForTerminal(key("PageDown", { ctrlKey: true }), false)).toBe(false);
  });

  it("leaves the vault lock shortcut global", () => {
    expect(keepForTerminal(key("L", { ctrlKey: true, shiftKey: true }), false)).toBe(false);
  });

  it("keeps nothing on macOS, where the shell shortcuts use Cmd", () => {
    expect(keepForTerminal(key("w", { metaKey: true }), true)).toBe(false);
    expect(keepForTerminal(key("w", { ctrlKey: true }), true)).toBe(false);
  });

  it("does not touch plain keys", () => {
    expect(keepForTerminal(key("w"), false)).toBe(false);
  });
});
