/// <reference types="node" />
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { scrollEdges, wheelToScroll } from "./overflow";

describe("scrollEdges", () => {
  it("has no fades when everything fits", () => {
    expect(scrollEdges(0, 400, 400)).toEqual({ start: false, end: false });
  });

  it("fades the end when content is hidden on the right", () => {
    expect(scrollEdges(0, 400, 900)).toEqual({ start: false, end: true });
  });

  it("fades both edges in the middle", () => {
    expect(scrollEdges(200, 400, 900)).toEqual({ start: true, end: true });
  });

  it("fades only the start at the far end", () => {
    expect(scrollEdges(500, 400, 900)).toEqual({ start: true, end: false });
  });

  it("ignores sub-pixel remainders", () => {
    expect(scrollEdges(0.4, 400, 400.6)).toEqual({ start: false, end: false });
  });
});

describe("wheelToScroll", () => {
  it("maps a vertical wheel to a horizontal scroll", () => {
    expect(wheelToScroll(0, 40)).toBe(40);
    expect(wheelToScroll(0, -40)).toBe(-40);
  });

  it("leaves horizontal gestures and empty events alone", () => {
    expect(wheelToScroll(12, 3)).toBeNull();
    expect(wheelToScroll(0, 0)).toBeNull();
  });
});

// jsdom has no layout engine, so the sizing rule is pinned on the stylesheet itself: a tab takes its
// natural width up to a maximum and shrinks to a minimum only when the strip overflows. (A fixed
// flex-basis wider than the content-sized strip made every tab shrink and truncate its title.)
describe("tab sizing", () => {
  const source = readFileSync(join(process.cwd(), "src/lib/tabs/TabBar.svelte"), "utf8");
  const tokens = readFileSync(join(process.cwd(), "src/lib/design/tokens.css"), "utf8");
  const rule = (selector: string): string => {
    const start = source.indexOf(`\n  ${selector} {`);
    return source.slice(start, source.indexOf("}", start));
  };

  it("sizes a tab from its content, clamped by the min and max tokens", () => {
    const tab = rule(".tab");
    expect(tab).toMatch(/flex:\s*0 1 auto/);
    expect(tab).toContain("min-width: var(--tab-min-width)");
    expect(tab).toContain("max-width: var(--tab-max-width)");
  });

  it("keeps the strip content-sized so free space is never used to shrink tabs", () => {
    expect(rule(".tabs")).toMatch(/flex:\s*0 1 auto/);
    expect(rule(".tabs")).toContain("min-width: 0");
  });

  it("defines the 96 px minimum and the 200 px maximum", () => {
    expect(tokens).toMatch(/--tab-min-width:\s*96px/);
    expect(tokens).toMatch(/--tab-max-width:\s*200px/);
  });
});
