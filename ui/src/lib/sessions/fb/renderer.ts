// Draws the rectangles of the framebuffer onto a canvas.
//
// Rectangles are batched per animation frame. `requestAnimationFrame` is paused entirely
// while the window is hidden or occluded (the spike measured memory growing without bound
// that way), so every batch also arms a timer: whichever fires first draws, and a batch that
// grows beyond `maxPendingBytes` is drawn immediately. The backend stops sending while the
// tab is hidden, so these are safety nets, not the primary flow control.

export interface RectPixels {
  x: number;
  y: number;
  w: number;
  h: number;
  rgba: Uint8Array;
}

export interface Surface {
  putImageData(image: ImageData, x: number, y: number): void;
}

export interface RendererOptions {
  requestFrame?: (callback: () => void) => number;
  cancelFrame?: (handle: number) => void;
  setTimer?: (callback: () => void, ms: number) => number;
  clearTimer?: (handle: number) => void;
  makeImage?: (rgba: Uint8Array, w: number, h: number) => ImageData;
  /** Delay after which a batch is drawn even if no animation frame came. */
  fallbackMs?: number;
  maxPendingBytes?: number;
}

export interface RendererStats {
  rects: number;
  batches: number;
  fallbackBatches: number;
}

const defaultMakeImage = (rgba: Uint8Array, w: number, h: number): ImageData =>
  // A view over the received buffer: no copy.
  new ImageData(new Uint8ClampedArray(rgba.buffer as ArrayBuffer, rgba.byteOffset, rgba.length), w, h);

export class FrameRenderer {
  readonly stats: RendererStats = { rects: 0, batches: 0, fallbackBatches: 0 };
  #pending: RectPixels[] = [];
  #pendingBytes = 0;
  #frame: number | null = null;
  #timer: number | null = null;
  readonly #surface: Surface;
  readonly #o: Required<RendererOptions>;

  constructor(surface: Surface, options: RendererOptions = {}) {
    this.#surface = surface;
    this.#o = {
      requestFrame: options.requestFrame ?? ((cb) => requestAnimationFrame(cb)),
      cancelFrame: options.cancelFrame ?? ((h) => cancelAnimationFrame(h)),
      setTimer: options.setTimer ?? ((cb, ms) => window.setTimeout(cb, ms)),
      clearTimer: options.clearTimer ?? ((h) => window.clearTimeout(h)),
      makeImage: options.makeImage ?? defaultMakeImage,
      fallbackMs: options.fallbackMs ?? 100,
      maxPendingBytes: options.maxPendingBytes ?? 32 * 1024 * 1024,
    };
  }

  /** Number of rectangles waiting for the next frame. */
  get pending(): number {
    return this.#pending.length;
  }

  enqueue(rect: RectPixels): void {
    if (rect.w <= 0 || rect.h <= 0 || rect.rgba.length < rect.w * rect.h * 4) return;
    this.stats.rects++;
    this.#pending.push(rect);
    this.#pendingBytes += rect.rgba.length;
    if (this.#pendingBytes >= this.#o.maxPendingBytes) {
      this.flush(true);
      return;
    }
    if (this.#frame === null) this.#frame = this.#o.requestFrame(() => this.flush(false));
    if (this.#timer === null) this.#timer = this.#o.setTimer(() => this.flush(true), this.#o.fallbackMs);
  }

  /** Drops what has not been drawn yet (the desktop changed size: it would not fit). */
  discard(): void {
    this.#cancel();
    this.#pending = [];
    this.#pendingBytes = 0;
  }

  /** Draws everything pending now. */
  flush(fallback = false): void {
    this.#cancel();
    const batch = this.#pending;
    this.#pending = [];
    this.#pendingBytes = 0;
    if (batch.length === 0) return;
    this.stats.batches++;
    if (fallback) this.stats.fallbackBatches++;
    for (const r of batch) {
      this.#surface.putImageData(this.#o.makeImage(r.rgba, r.w, r.h), r.x, r.y);
    }
  }

  dispose(): void {
    this.discard();
  }

  #cancel(): void {
    if (this.#frame !== null) this.#o.cancelFrame(this.#frame);
    if (this.#timer !== null) this.#o.clearTimer(this.#timer);
    this.#frame = null;
    this.#timer = null;
  }
}
