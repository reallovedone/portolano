import { describe, expect, it } from "vitest";

import {
  CLIPBOARD_IDLE,
  dismissNotice,
  failedLocally,
  formatBytes,
  percent,
  reduceClipboard,
  startSaving,
} from "./clipboard";

const offer = { type: "remoteFiles", count: 3, bytes: 2500, skipped: 1 } as const;

describe("reduceClipboard", () => {
  it("offers the files of the remote clipboard and withdraws the offer", () => {
    const offered = reduceClipboard(CLIPBOARD_IDLE, offer);
    expect(offered.offer).toEqual({ count: 3, bytes: 2500, skipped: 1 });
    const gone = reduceClipboard(offered, { type: "remoteFiles", count: 0, bytes: 0, skipped: 0 });
    expect(gone.offer).toBeNull();
  });

  it("goes through saving to done, keeping the offer for a second save", () => {
    let s = reduceClipboard(CLIPBOARD_IDLE, offer);
    s = startSaving(s);
    expect(s.phase).toEqual({
      kind: "saving",
      progress: { filesDone: 0, filesTotal: 3, bytesDone: 0, bytesTotal: 2500 },
    });
    s = reduceClipboard(s, {
      type: "progress",
      filesDone: 1,
      filesTotal: 3,
      bytesDone: 1000,
      bytesTotal: 2500,
    });
    expect(s.phase).toMatchObject({ kind: "saving", progress: { filesDone: 1 } });
    s = reduceClipboard(s, { type: "done", files: 3, bytes: 2500, skipped: 1 });
    expect(s.phase).toEqual({ kind: "done", files: 3, bytes: 2500, skipped: 1 });
    expect(s.offer).not.toBeNull();
    expect(dismissNotice(s).phase).toEqual({ kind: "idle" });
    expect(dismissNotice(s).offer).not.toBeNull();
  });

  it("ignores remote clipboard changes while a save runs", () => {
    let s = startSaving(reduceClipboard(CLIPBOARD_IDLE, offer));
    s = reduceClipboard(s, { type: "remoteFiles", count: 0, bytes: 0, skipped: 0 });
    expect(s.phase.kind).toBe("saving");
    expect(s.offer).not.toBeNull();
  });

  it("a cancelled save keeps the offer; a limit or an empty list removes it", () => {
    let s = startSaving(reduceClipboard(CLIPBOARD_IDLE, offer));
    s = reduceClipboard(s, { type: "failed", reason: "cancelled" });
    expect(s.phase).toEqual({ kind: "failed", reason: "cancelled" });
    expect(s.offer).not.toBeNull();
    const limited = reduceClipboard(reduceClipboard(CLIPBOARD_IDLE, offer), {
      type: "failed",
      reason: "limit",
    });
    expect(limited.offer).toBeNull();
  });

  it("only a finished or failed notice can be dismissed", () => {
    const saving = startSaving(reduceClipboard(CLIPBOARD_IDLE, offer));
    expect(dismissNotice(saving)).toBe(saving);
    expect(failedLocally(saving).phase).toEqual({ kind: "failed", reason: "unavailable" });
  });
});

describe("formatBytes and percent", () => {
  it("uses decimal units", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(999)).toBe("999 B");
    expect(formatBytes(1500)).toBe("1.5 KB");
    expect(formatBytes(25_000_000)).toBe("25 MB");
    expect(formatBytes(16 * 1024 ** 3)).toBe("17 GB");
    expect(formatBytes(-1)).toBe("0 B");
    expect(formatBytes(Number.NaN)).toBe("0 B");
  });

  it("is a bounded whole percentage", () => {
    expect(percent(0, 0)).toBe(0);
    expect(percent(50, 200)).toBe(25);
    expect(percent(300, 200)).toBe(100);
    expect(percent(-5, 10)).toBe(0);
  });
});
