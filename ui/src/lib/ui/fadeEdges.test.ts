import { describe, expect, it } from "vitest";

import { fadeEdges } from "./fadeEdges";

// jsdom has no layout: the box metrics are set by hand and can change later.
function box(client: number, scroll: number): { node: HTMLElement; metrics: { scroll: number } } {
  const node = document.createElement("div");
  const metrics = { scroll };
  Object.defineProperty(node, "clientHeight", { value: client });
  Object.defineProperty(node, "scrollHeight", { get: () => metrics.scroll });
  return { node, metrics };
}

const fades = (node: HTMLElement): [boolean, boolean] => [
  node.hasAttribute("data-fade-top"),
  node.hasAttribute("data-fade-bottom"),
];

describe("fadeEdges", () => {
  it("marks nothing when the content fits", () => {
    const { node } = box(100, 100);
    fadeEdges(node);
    expect(fades(node)).toEqual([false, false]);
  });

  it("marks only the bottom at the start of a long box, then both after scrolling", () => {
    const { node } = box(100, 300);
    fadeEdges(node);
    expect(fades(node)).toEqual([false, true]);

    node.scrollTop = 50;
    node.dispatchEvent(new Event("scroll"));
    expect(fades(node)).toEqual([true, true]);

    node.scrollTop = 200;
    node.dispatchEvent(new Event("scroll"));
    expect(fades(node)).toEqual([true, false]);
  });

  it("follows the content when it grows or shrinks", async () => {
    const { node, metrics } = box(100, 100);
    fadeEdges(node);
    expect(fades(node)).toEqual([false, false]);

    metrics.scroll = 300;
    node.append(document.createElement("p"));
    await new Promise((resolve) => setTimeout(resolve));
    expect(fades(node)).toEqual([false, true]);

    metrics.scroll = 100;
    node.append(document.createElement("p"));
    await new Promise((resolve) => setTimeout(resolve));
    expect(fades(node)).toEqual([false, false]);
  });
});
