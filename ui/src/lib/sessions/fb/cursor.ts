// Remote cursor: turns an `FbEvent` cursor into a CSS `cursor` value.
//
// Convention shared with the backend: a cursor with no pixels (w or h = 0) means "use the
// default local cursor"; the remote "hidden pointer" arrives as a fully transparent bitmap.

import type { FbEvent } from "../../ipc";

type CursorEvent = Extract<FbEvent, { type: "cursor" }>;

/** Browsers reject cursor images larger than this. */
export const MAX_CURSOR_SIZE = 128;

export type ToDataUrl = (rgba: Uint8Array, w: number, h: number) => string | null;

/** Encodes RGBA pixels as a PNG data URL through a scratch canvas. */
export const canvasToDataUrl: ToDataUrl = (rgba, w, h) => {
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d");
  if (!ctx) return null;
  const pixels = new Uint8ClampedArray(rgba.buffer as ArrayBuffer, rgba.byteOffset, rgba.length);
  ctx.putImageData(new ImageData(pixels, w, h), 0, 0);
  return canvas.toDataURL("image/png");
};

export function cursorStyle(event: CursorEvent, toDataUrl: ToDataUrl = canvasToDataUrl): string {
  const { w, h, hotspotX, hotspotY, rgba } = event;
  if (w === 0 || h === 0 || w > MAX_CURSOR_SIZE || h > MAX_CURSOR_SIZE) return "default";
  if (rgba.length !== w * h * 4) return "default";
  const url = toDataUrl(rgba, w, h);
  if (!url) return "default";
  const x = Math.min(hotspotX, w - 1);
  const y = Math.min(hotspotY, h - 1);
  return `url("${url}") ${x} ${y}, default`;
}
