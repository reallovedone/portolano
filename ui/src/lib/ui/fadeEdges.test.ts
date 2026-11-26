import { describe, expect, it } from "vitest";

import { fadeEdges } from "./fadeEdges";

// jsdom has no layout: the box metrics are set by hand.
function box(client: number, scroll: number, top = 0): HTMLElement {
  const node = document.createElement("div");
  Object.defineProperty(node, "clientHeight", { value: client });
  Object.defineProperty(node, "scrollHeight", { value: scroll });
  node.scrollTop = top;
  return node;
}

describe("fadeEdges", () => {
  it("marks nothing when the content fits", () => {
    const node = box(100, 100);
    fadeEdges(node);
    expect(node.hasAttribute("data-fade-top")).toBe(false);
    expect(node.hasAttribute("data-fade-bottom")).toBe(false);
  });

  it("marks only the bottom at the start of a long box, then both after scrolling", () => {
    const node = box(100, 300);
    fadeEdges(node);
    expect(node.hasAttribute("data-fade-top")).toBe(false);
    expect(node.hasAttribute("data-fade-bottom")).toBe(true);

    node.scrollTop = 50;
    node.dispatchEvent(new Event("scroll"));
    expect(node.hasAttribute("data-fade-top")).toBe(true);
    expect(node.hasAttribute("data-fade-bottom")).toBe(true);

    node.scrollTop = 200;
    node.dispatchEvent(new Event("scroll"));
    expect(node.hasAttribute("data-fade-top")).toBe(true);
    expect(node.hasAttribute("data-fade-bottom")).toBe(false);
  });
});
