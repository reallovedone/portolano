import { describe, expect, it, vi } from "vitest";

import { CLIPBOARD_CLEAR_SECONDS, copySecret, type ClipboardPort } from "./clipboard";

function fakeClipboard(initial = ""): ClipboardPort & { value: string } {
  const board = {
    value: initial,
    async writeText(text: string) {
      board.value = text;
    },
    async readText() {
      return board.value;
    },
  };
  return board;
}

/** A timer the test fires by hand. */
function manualTimer() {
  const pending: { callback: () => void; ms: number }[] = [];
  return {
    setTimer: (callback: () => void, ms: number) => {
      pending.push({ callback, ms });
      return pending.length;
    },
    pending,
    async fire() {
      for (const { callback } of pending.splice(0)) callback();
      // Let the read/write promises settle.
      await new Promise((resolve) => setTimeout(resolve, 0));
    },
  };
}

describe("copying a secret", () => {
  it("puts it on the clipboard and clears it after 30 seconds when it is still there", async () => {
    const clipboard = fakeClipboard();
    const timer = manualTimer();
    const onCleared = vi.fn();
    await copySecret("s3cret", { clipboard, setTimer: timer.setTimer, onCleared });
    expect(clipboard.value).toBe("s3cret");
    expect(timer.pending).toHaveLength(1);
    expect(timer.pending[0]?.ms).toBe(CLIPBOARD_CLEAR_SECONDS * 1000);
    expect(CLIPBOARD_CLEAR_SECONDS).toBe(30);
    await timer.fire();
    expect(clipboard.value).toBe("");
    expect(onCleared).toHaveBeenCalledOnce();
  });

  it("leaves the clipboard alone when the user copied something else in the meantime", async () => {
    const clipboard = fakeClipboard();
    const timer = manualTimer();
    const onChanged = vi.fn();
    const onCleared = vi.fn();
    await copySecret("s3cret", { clipboard, setTimer: timer.setTimer, onChanged, onCleared });
    clipboard.value = "something the user copied later";
    await timer.fire();
    expect(clipboard.value).toBe("something the user copied later");
    expect(onChanged).toHaveBeenCalledOnce();
    expect(onCleared).not.toHaveBeenCalled();
  });

  it("does not clear what it cannot check and says so", async () => {
    const clipboard = fakeClipboard();
    clipboard.readText = () => Promise.reject(new Error("permission denied"));
    const timer = manualTimer();
    const onNotCleared = vi.fn();
    await copySecret("s3cret", { clipboard, setTimer: timer.setTimer, onNotCleared });
    await timer.fire();
    expect(clipboard.value).toBe("s3cret");
    expect(onNotCleared).toHaveBeenCalledOnce();
  });

  it("rejects when the write fails and schedules nothing", async () => {
    const clipboard = fakeClipboard();
    clipboard.writeText = () => Promise.reject(new Error("no clipboard"));
    const timer = manualTimer();
    await expect(copySecret("s3cret", { clipboard, setTimer: timer.setTimer })).rejects.toThrow(
      "no clipboard",
    );
    expect(timer.pending).toHaveLength(0);
  });
});
