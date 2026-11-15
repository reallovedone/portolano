// Copying a secret to the system clipboard, and clearing it again.
//
// The clipboard is shared with every other application, so a copied password must not stay on
// it. After `CLIPBOARD_CLEAR_SECONDS` the clipboard is emptied, but only if it still holds the
// secret: something the user copied in the meantime is left alone. When the clipboard cannot be
// read back (the system may ask for permission and nobody is there to answer) it is NOT
// touched and `onNotCleared` says so, so the user knows to clear it.

/** How long a copied secret stays on the clipboard. */
export const CLIPBOARD_CLEAR_SECONDS = 30;

/** The part of `navigator.clipboard` used here. */
export interface ClipboardPort {
  writeText(text: string): Promise<void>;
  readText(): Promise<string>;
}

export interface CopyOptions {
  clipboard?: ClipboardPort;
  /** Replaceable for tests. */
  setTimer?: (callback: () => void, ms: number) => unknown;
  /** The clipboard was emptied. */
  onCleared?: () => void;
  /** The clipboard was left alone because it could not be checked (the reason is the error). */
  onNotCleared?: (reason: unknown) => void;
  /** The clipboard no longer held the secret: nothing to clear. */
  onChanged?: () => void;
}

/**
 * Writes `text` to the clipboard and schedules its removal. Resolves when the text is on the
 * clipboard (it rejects when the write fails); the removal happens later, on its own.
 */
export async function copySecret(text: string, options: CopyOptions = {}): Promise<void> {
  const clipboard = options.clipboard ?? navigator.clipboard;
  const setTimer = options.setTimer ?? ((callback, ms) => setTimeout(callback, ms));
  await clipboard.writeText(text);
  setTimer(() => {
    void clearIfStillThere(clipboard, text, options);
  }, CLIPBOARD_CLEAR_SECONDS * 1000);
}

async function clearIfStillThere(
  clipboard: ClipboardPort,
  text: string,
  options: CopyOptions,
): Promise<void> {
  let current: string;
  try {
    current = await clipboard.readText();
  } catch (e) {
    options.onNotCleared?.(e);
    return;
  }
  if (current !== text) {
    options.onChanged?.();
    return;
  }
  try {
    await clipboard.writeText("");
    options.onCleared?.();
  } catch (e) {
    options.onNotCleared?.(e);
  }
}
