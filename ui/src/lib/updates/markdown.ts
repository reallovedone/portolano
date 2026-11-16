// A deliberately small Markdown reader for release notes.
//
// The notes come from the network, so they are never turned into HTML. This module reads the
// handful of constructs release notes use (headings, bullet and numbered lists, paragraphs, code
// blocks, `code`, **bold**, *italic*, links) into plain data, and the Svelte component renders
// that data as text nodes: Svelte escapes text, so markup in the notes (`<img onerror=…>`,
// `<script>`) is shown literally and nothing in it can run. Link targets are dropped on purpose:
// only the link text is shown, and nothing in the notes is clickable.

export type Inline =
  | { kind: "text"; text: string }
  | { kind: "code"; text: string }
  | { kind: "strong"; text: string }
  | { kind: "em"; text: string };

export type Block =
  | { kind: "heading"; level: 1 | 2 | 3; inline: Inline[] }
  | { kind: "paragraph"; inline: Inline[] }
  | { kind: "list"; ordered: boolean; items: Inline[][] }
  | { kind: "code"; text: string };

/** Longest text parsed; the backend already caps the notes, this is a second guard. */
const MAX_CHARS = 100_000;

const INLINE =
  /`([^`\n]+)`|\*\*([^*\n]+)\*\*|__([^_\n]+)__|\[([^\]\n]*)\]\((?:[^()\n]|\([^()\n]*\))*\)|(?<![\w*])\*([^*\s][^*\n]*?)\*(?![\w*])|(?<![\w_])_([^_\s][^_\n]*?)_(?![\w_])/g;

export function parseInline(text: string): Inline[] {
  const out: Inline[] = [];
  let last = 0;
  const push = (kind: Inline["kind"], value: string): void => {
    if (value === "") return;
    const previous = out[out.length - 1];
    if (kind === "text" && previous?.kind === "text") previous.text += value;
    else out.push({ kind, text: value });
  };
  for (const match of text.matchAll(INLINE)) {
    const index = match.index ?? 0;
    push("text", text.slice(last, index));
    const [, code, strongStars, strongBars, linkText, emStars, emBars] = match;
    if (code !== undefined) push("code", code);
    else if (strongStars !== undefined) push("strong", strongStars);
    else if (strongBars !== undefined) push("strong", strongBars);
    else if (linkText !== undefined) push("text", linkText);
    else if (emStars !== undefined) push("em", emStars);
    else if (emBars !== undefined) push("em", emBars ?? "");
    last = index + match[0].length;
  }
  push("text", text.slice(last));
  return out;
}

const HEADING = /^(#{1,6})\s+(.*?)\s*#*\s*$/;
const BULLET = /^\s*[-*+]\s+(.*)$/;
const NUMBERED = /^\s*\d+[.)]\s+(.*)$/;
const FENCE = /^\s*(```|~~~)/;
const RULE = /^\s*([-*_])(\s*\1){2,}\s*$/;

export function parseMarkdown(source: string): Block[] {
  const lines = source.slice(0, MAX_CHARS).replace(/\r\n?/g, "\n").split("\n");
  const blocks: Block[] = [];
  let paragraph: string[] = [];

  const flushParagraph = (): void => {
    if (paragraph.length === 0) return;
    blocks.push({ kind: "paragraph", inline: parseInline(paragraph.join(" ")) });
    paragraph = [];
  };

  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i] ?? "";

    if (FENCE.test(line)) {
      flushParagraph();
      const code: string[] = [];
      i += 1;
      while (i < lines.length && !FENCE.test(lines[i] ?? "")) {
        code.push(lines[i] ?? "");
        i += 1;
      }
      blocks.push({ kind: "code", text: code.join("\n") });
      continue;
    }
    if (line.trim() === "" || RULE.test(line)) {
      flushParagraph();
      continue;
    }
    const heading = HEADING.exec(line);
    if (heading) {
      flushParagraph();
      const level = Math.min((heading[1] ?? "#").length, 3) as 1 | 2 | 3;
      blocks.push({ kind: "heading", level, inline: parseInline(heading[2] ?? "") });
      continue;
    }
    const bullet = BULLET.exec(line);
    const numbered = bullet ? null : NUMBERED.exec(line);
    const item = bullet ?? numbered;
    if (item) {
      flushParagraph();
      const ordered = numbered !== null;
      const previous = blocks[blocks.length - 1];
      const entry = parseInline(item[1] ?? "");
      if (previous?.kind === "list" && previous.ordered === ordered) previous.items.push(entry);
      else blocks.push({ kind: "list", ordered, items: [entry] });
      continue;
    }
    // Quotes lose their marker; anything else (tables, raw HTML) stays as literal text.
    paragraph.push(line.replace(/^\s*>\s?/, "").trim());
  }
  flushParagraph();
  return blocks;
}
