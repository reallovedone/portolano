// jsdom lacks a few browser APIs the shell relies on.
if (typeof globalThis.ResizeObserver === "undefined") {
  globalThis.ResizeObserver = class {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  };
}

// jsdom has no canvas: a no-op 2d context lets framebuffer views mount (WP6). Tests that look at
// what is drawn spy on `getContext` themselves.
HTMLCanvasElement.prototype.getContext = function getContext() {
  return { putImageData(): void {} };
} as unknown as typeof HTMLCanvasElement.prototype.getContext;
