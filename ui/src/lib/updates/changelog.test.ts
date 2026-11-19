// The release notes shown in the app are sections of the repository's CHANGELOG.md (extracted by
// .github/scripts/changelog-notes.py). This renders a real excerpt of that file, wrapped as the
// release body is (candidate line first, closing paragraph last), to keep the format and the
// small Markdown reader in step.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { cleanup, render, screen, within } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";

import ReleaseNotes from "./ReleaseNotes.svelte";
import { parseMarkdown } from "./markdown";

const CHANGELOG = readFileSync(resolve(__dirname, "../../../../CHANGELOG.md"), "utf8");

/** Body of `## [version]`, up to the next section or the link references. */
function section(version: string): string {
  const lines = CHANGELOG.split("\n");
  const start = lines.findIndex((l) => l.startsWith(`## [${version}]`));
  expect(start).toBeGreaterThanOrEqual(0);
  const rest = lines.slice(start + 1);
  const end = rest.findIndex((l) => l.startsWith("## [") || /^\[[^\]]+\]: /.test(l));
  return rest.slice(0, end === -1 ? undefined : end).join("\n").trim();
}

const BODY = [
  "> Release candidate 2 of 0.2.0 — for testing.",
  "",
  section("0.2.0"),
  "",
  "---",
  "",
  "First install? These builds are not signed. Check the files against `SHA256SUMS.txt`.",
].join("\n");

afterEach(cleanup);

describe("CHANGELOG.md as release notes", () => {
  it("is read into headings with their emoji, lists and plain paragraphs", () => {
    const blocks = parseMarkdown(BODY);
    const headings = blocks.flatMap((b) =>
      b.kind === "heading" ? [b.inline.map((i) => i.text).join("")] : [],
    );
    expect(headings).toEqual([
      "⚠️ Breaking changes",
      "✨ Added",
      "🔄 Changed",
      "🐛 Fixed",
      "🔒 Security",
    ]);
    expect(blocks.filter((b) => b.kind === "list")).toHaveLength(5);
    // The quote marker is dropped (the line stays as a paragraph); the rule is skipped.
    const first = blocks[0];
    expect(first?.kind).toBe("paragraph");
    expect(first?.kind === "paragraph" ? first.inline[0] : null).toEqual({
      kind: "text",
      text: "Release candidate 2 of 0.2.0 — for testing.",
    });
    expect(JSON.stringify(blocks)).not.toContain("---");
  });

  it("renders in the notes component as text, with emphasis and code kept", () => {
    render(ReleaseNotes, { props: { notes: BODY } });
    const notes = screen.getByTestId("release-notes");
    expect(within(notes).getByText("Release candidate 2 of 0.2.0 — for testing.")).toBeTruthy();
    expect(within(notes).getByRole("heading", { name: "✨ Added" })).toBeTruthy();
    expect(within(notes).getByRole("heading", { name: "🔒 Security" })).toBeTruthy();
    expect(within(notes).getByText("Check for Updates…").tagName).toBe("EM");
    expect(within(notes).getByText("SHA256SUMS.txt").tagName).toBe("CODE");
    expect(notes.querySelectorAll("li").length).toBeGreaterThan(10);
    expect(notes.innerHTML).not.toContain("](");
  });
});
