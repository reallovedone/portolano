import { describe, expect, it } from "vitest";

import { parseInline, parseMarkdown } from "./markdown";

describe("parseInline", () => {
  it("reads code, bold, italic and links", () => {
    expect(parseInline("a `b` **c** *d* [e](https://x.test/y) f")).toEqual([
      { kind: "text", text: "a " },
      { kind: "code", text: "b" },
      { kind: "text", text: " " },
      { kind: "strong", text: "c" },
      { kind: "text", text: " " },
      { kind: "em", text: "d" },
      { kind: "text", text: " e f" },
    ]);
  });

  it("drops link targets: only the text is kept", () => {
    expect(parseInline("[click](javascript:alert(1))")).toEqual([{ kind: "text", text: "click" }]);
  });

  it("keeps snake_case and 2*3*4 as they are", () => {
    expect(parseInline("a_b_c and 2*3*4")).toEqual([{ kind: "text", text: "a_b_c and 2*3*4" }]);
  });

  it("never produces markup: HTML stays literal text", () => {
    const parsed = parseInline("<img src=x onerror=alert(1)> <script>x</script>");
    expect(parsed).toEqual([
      { kind: "text", text: "<img src=x onerror=alert(1)> <script>x</script>" },
    ]);
  });
});

describe("parseMarkdown", () => {
  it("reads headings, lists, paragraphs and code blocks", () => {
    const blocks = parseMarkdown(
      [
        "## What's Changed",
        "- one",
        "* two **bold**",
        "",
        "1. first",
        "2. second",
        "",
        "Plain line",
        "continues here",
        "",
        "```",
        "a < b",
        "```",
        "---",
        "**Full Changelog**: v0.1.0...v0.2.0",
      ].join("\n"),
    );
    expect(blocks.map((b) => b.kind)).toEqual([
      "heading",
      "list",
      "list",
      "paragraph",
      "code",
      "paragraph",
    ]);
    expect(blocks[1]).toMatchObject({ kind: "list", ordered: false });
    expect(blocks[1]?.kind === "list" && blocks[1].items).toHaveLength(2);
    expect(blocks[2]).toMatchObject({ kind: "list", ordered: true });
    expect(blocks[3]).toMatchObject({
      kind: "paragraph",
      inline: [{ kind: "text", text: "Plain line continues here" }],
    });
    expect(blocks[4]).toEqual({ kind: "code", text: "a < b" });
  });

  it("caps heading levels and handles CRLF", () => {
    expect(parseMarkdown("###### deep\r\n- a\r\n- b")).toEqual([
      { kind: "heading", level: 3, inline: [{ kind: "text", text: "deep" }] },
      {
        kind: "list",
        ordered: false,
        items: [[{ kind: "text", text: "a" }], [{ kind: "text", text: "b" }]],
      },
    ]);
  });

  it("copes with an unterminated code fence and empty input", () => {
    expect(parseMarkdown("```\nx")).toEqual([{ kind: "code", text: "x" }]);
    expect(parseMarkdown("")).toEqual([]);
  });
});
