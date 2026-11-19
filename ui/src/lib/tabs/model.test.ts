import { describe, expect, it } from "vitest";

import {
  activateTab,
  addTab,
  cycleTab,
  emptyTabs,
  moveTab,
  newHomeTab,
  newTab,
  openHomeTab,
  patchTab,
  removeOtherTabs,
  removeTab,
  removeTabsWhere,
  sessionFailed,
  sessionOpened,
  sessionRetrying,
  type TabsState,
} from "./model";

const tab = (id: string, connectionId = `c-${id}`) => newTab(id, connectionId, id, "ssh");
const ids = (s: TabsState): string[] => s.tabs.map((t) => t.id);

function stateOf(...list: string[]): TabsState {
  return list.reduce((s, id) => addTab(s, tab(id)), emptyTabs);
}

describe("tab strip transitions", () => {
  it("adds a tab after the active one and activates it", () => {
    let s = stateOf("a", "b", "c");
    expect(s.activeId).toBe("c");
    s = activateTab(s, "a");
    s = addTab(s, tab("d"));
    expect(ids(s)).toEqual(["a", "d", "b", "c"]);
    expect(s.activeId).toBe("d");
  });

  it("ignores activation of unknown tabs", () => {
    const s = stateOf("a");
    expect(activateTab(s, "zzz")).toBe(s);
  });

  it("closing the active tab activates its right neighbour, else the left one", () => {
    let s = activateTab(stateOf("a", "b", "c"), "b");
    s = removeTab(s, "b");
    expect(s.activeId).toBe("c");
    s = removeTab(s, "c");
    expect(s.activeId).toBe("a");
    s = removeTab(s, "a");
    expect(s).toEqual(emptyTabs);
  });

  it("closing an inactive tab keeps the active one", () => {
    const s = removeTab(activateTab(stateOf("a", "b", "c"), "b"), "a");
    expect(s.activeId).toBe("b");
    expect(ids(s)).toEqual(["b", "c"]);
  });

  it("closes the others and activates the kept tab", () => {
    const s = removeOtherTabs(stateOf("a", "b", "c"), "b");
    expect(ids(s)).toEqual(["b"]);
    expect(s.activeId).toBe("b");
    const same = stateOf("a");
    expect(removeOtherTabs(same, "nope")).toBe(same);
  });

  it("removes tabs by predicate and fixes the active tab", () => {
    const s0 = activateTab(stateOf("a", "b", "c"), "b");
    const s1 = removeTabsWhere(s0, (t) => t.id === "b" || t.id === "c");
    expect(ids(s1)).toEqual(["a"]);
    expect(s1.activeId).toBe("a");
    expect(removeTabsWhere(s0, () => false)).toBe(s0);
  });

  it("reorders tabs, clamping the target", () => {
    const s = stateOf("a", "b", "c");
    expect(ids(moveTab(s, 0, 2))).toEqual(["b", "c", "a"]);
    expect(ids(moveTab(s, 2, 0))).toEqual(["c", "a", "b"]);
    expect(ids(moveTab(s, 0, 99))).toEqual(["b", "c", "a"]);
    expect(moveTab(s, 1, 1)).toBe(s);
    expect(moveTab(s, 7, 0)).toBe(s);
    expect(moveTab(s, 0, 2).activeId).toBe(s.activeId);
  });

  it("cycles with wrap-around", () => {
    let s = activateTab(stateOf("a", "b", "c"), "c");
    s = cycleTab(s, 1);
    expect(s.activeId).toBe("a");
    s = cycleTab(s, -1);
    expect(s.activeId).toBe("c");
    expect(cycleTab(emptyTabs, 1)).toBe(emptyTabs);
  });
});

describe("session lifecycle on a tab", () => {
  it("starts opening, then opened or failed", () => {
    let s = stateOf("a");
    expect(s.tabs[0]).toMatchObject({ status: "opening", sessionId: null, error: null, attempt: 0 });

    s = sessionOpened(s, "a", "sess-1");
    expect(s.tabs[0]).toMatchObject({ status: "open", sessionId: "sess-1", error: null });

    s = sessionFailed(s, "a", { code: "unsupported", message: "nope" });
    expect(s.tabs[0]).toMatchObject({
      status: "failed",
      sessionId: null,
      error: { code: "unsupported", message: "nope" },
    });
  });

  it("retry resets to opening and bumps the attempt counter", () => {
    let s = sessionFailed(stateOf("a"), "a", { code: "io", message: "x" });
    s = sessionRetrying(s, "a");
    expect(s.tabs[0]).toMatchObject({ status: "opening", error: null, sessionId: null, attempt: 1 });
    expect(sessionRetrying(s, "missing")).toBe(s);
  });

  it("does nothing for unknown tabs", () => {
    const s = stateOf("a");
    expect(patchTab(s, "zzz", { title: "x" })).toBe(s);
  });
});

describe("Home tab", () => {
  it("opens next to the active tab and becomes active", () => {
    let s = stateOf("a", "b", "c");
    s = activateTab(s, "a");
    s = openHomeTab(s, "h");
    expect(ids(s)).toEqual(["a", "h", "b", "c"]);
    expect(s.activeId).toBe("h");
    expect(s.tabs[1]).toEqual({ id: "h", type: "home" });
    expect(newHomeTab("x")).toEqual({ id: "x", type: "home" });
  });

  it("activates the existing Home instead of opening a second one", () => {
    let s = openHomeTab(stateOf("a", "b"), "h");
    s = activateTab(s, "a");
    const again = openHomeTab(s, "h2");
    expect(ids(again)).toEqual(["a", "b", "h"]);
    expect(again.activeId).toBe("h");
  });

  it("ignores retry transitions", () => {
    const s = openHomeTab(emptyTabs, "h");
    expect(sessionRetrying(s, "h")).toBe(s);
    expect(sessionFailed(s, "h", { code: "not_found", message: "y" })).toBe(s);
    expect(sessionOpened(s, "h", "s1")).toBe(s);
  });
});
