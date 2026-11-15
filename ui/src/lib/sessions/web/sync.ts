// Keeps the backend's child webview over the placeholder element. Pure logic with injected
// scheduling, so it is tested without a DOM.
//
// - Requests are coalesced into one send per animation frame.
// - At most one `send` is in flight; a request that arrives meanwhile is sent afterwards
//   (so the backend always ends with the latest state, never an out-of-order one).
// - A send identical to the previous successful one is skipped.
// - Hiding is never delayed: the web view sits above the HTML UI, so a menu that opens must
//   not be covered for even one frame.

import type { Bounds } from "../../ipc";

export interface SyncOptions {
  /** Current rect of the placeholder in viewport CSS px, or null when it has no layout. */
  measure: () => Bounds | null;
  /** Whether the web view should be visible (tab active, no overlay open). */
  shouldShow: () => boolean;
  send: (bounds: Bounds, visible: boolean) => Promise<void>;
  onError: (error: unknown) => void;
  /** Runs `callback` before the next paint; returns a cancel function. */
  schedule?: (callback: () => void) => () => void;
}

const defaultSchedule = (callback: () => void): (() => void) => {
  if (typeof requestAnimationFrame === "function") {
    const handle = requestAnimationFrame(callback);
    return () => cancelAnimationFrame(handle);
  }
  const handle = setTimeout(callback, 16);
  return () => clearTimeout(handle);
};

function same(a: Bounds, b: Bounds): boolean {
  return a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;
}

export class BoundsSync {
  readonly #options: SyncOptions;
  #cancel: (() => void) | null = null;
  #inFlight = false;
  #dirty = false;
  #disposed = false;
  #last: Bounds | null = null;
  #lastSent: { bounds: Bounds; visible: boolean } | null = null;

  constructor(options: SyncOptions) {
    this.#options = options;
  }

  /** Syncs on the next frame (coalescing repeated calls). */
  request(): void {
    if (this.#disposed || this.#cancel) return;
    const schedule = this.#options.schedule ?? defaultSchedule;
    this.#cancel = schedule(() => {
      this.#cancel = null;
      void this.#flush();
    });
  }

  /** Syncs now. Used for the first sync and for hiding. */
  flushNow(): void {
    if (this.#disposed) return;
    this.#cancel?.();
    this.#cancel = null;
    void this.#flush();
  }

  /** Hides without delay when the web view must not be shown, otherwise syncs next frame. */
  update(): void {
    if (this.#options.shouldShow()) this.request();
    else this.flushNow();
  }

  dispose(): void {
    this.#disposed = true;
    this.#cancel?.();
    this.#cancel = null;
  }

  async #flush(): Promise<void> {
    if (this.#disposed) return;
    if (this.#inFlight) {
      this.#dirty = true;
      return;
    }
    const measured = this.#options.measure();
    // A collapsed placeholder (hidden tab) keeps the last real rect; the web view is hidden then.
    const usable = measured && measured.width > 0 && measured.height > 0 ? measured : null;
    if (usable) this.#last = usable;
    const bounds = usable ?? this.#last;
    if (!bounds) return;
    const visible = this.#options.shouldShow() && usable !== null;
    const previous = this.#lastSent;
    if (previous && previous.visible === visible && same(previous.bounds, bounds)) return;

    this.#inFlight = true;
    try {
      await this.#options.send(bounds, visible);
      this.#lastSent = { bounds, visible };
    } catch (error) {
      this.#options.onError(error);
    } finally {
      this.#inFlight = false;
    }
    if (this.#dirty && !this.#disposed) {
      this.#dirty = false;
      void this.#flush();
    }
  }
}
