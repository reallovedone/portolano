// Pure geometry of the framebuffer view: fitting the remote desktop into the tab area and
// mapping pointer positions back to remote coordinates.

export interface Size {
  w: number;
  h: number;
}

export interface Box {
  /** CSS pixels of the canvas box. */
  width: number;
  height: number;
  scale: number;
}

/** Largest box with the aspect ratio of `remote` that fits in `area` (scales up and down). */
export function fitToArea(remote: Size, area: Size): Box {
  if (remote.w <= 0 || remote.h <= 0 || area.w <= 0 || area.h <= 0) {
    return { width: 0, height: 0, scale: 1 };
  }
  const scale = Math.min(area.w / remote.w, area.h / remote.h);
  return { width: Math.floor(remote.w * scale), height: Math.floor(remote.h * scale), scale };
}

export interface RectLike {
  left: number;
  top: number;
  width: number;
  height: number;
}

/**
 * Maps a client position to remote desktop coordinates, given the on-screen rectangle of the
 * canvas. The result is clamped to the desktop so that a drag that leaves the canvas keeps
 * the pointer on the nearest edge.
 */
export function toRemote(
  clientX: number,
  clientY: number,
  rect: RectLike,
  remote: Size,
): { x: number; y: number } {
  if (rect.width <= 0 || rect.height <= 0 || remote.w <= 0 || remote.h <= 0) return { x: 0, y: 0 };
  const x = Math.floor(((clientX - rect.left) * remote.w) / rect.width);
  const y = Math.floor(((clientY - rect.top) * remote.h) / rect.height);
  return {
    x: Math.max(0, Math.min(remote.w - 1, x)),
    y: Math.max(0, Math.min(remote.h - 1, y)),
  };
}

/**
 * Wheel units for the backend: 120 per notch, positive scrolls down (the sign of
 * `WheelEvent.deltaY`). `deltaMode` is the DOM one (0 pixels, 1 lines, 2 pages).
 */
export function wheelUnits(deltaY: number, deltaMode: number): number {
  const perUnit = deltaMode === 1 ? 40 : deltaMode === 2 ? 120 : 1.2;
  const units = Math.round(deltaY * perUnit);
  return Math.max(-1200, Math.min(1200, units));
}
