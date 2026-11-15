// Pure helpers for the tab strip: which edges of the scrolling tab list have hidden content (to
// show a fade there) and how a wheel gesture maps to horizontal scrolling.

export interface ScrollEdges {
  /** Content is hidden past the left (start) edge. */
  start: boolean;
  /** Content is hidden past the right (end) edge. */
  end: boolean;
}

/** Sub-pixel scroll positions must not flicker the fades. */
const TOLERANCE = 1;

export function scrollEdges(scrollLeft: number, clientWidth: number, scrollWidth: number): ScrollEdges {
  return {
    start: scrollLeft > TOLERANCE,
    end: scrollLeft + clientWidth < scrollWidth - TOLERANCE,
  };
}

/**
 * Horizontal scroll delta for a wheel event over the strip: a plain mouse wheel (vertical only)
 * scrolls the strip sideways; a trackpad already sends horizontal deltas and is left alone
 * (returns null).
 */
export function wheelToScroll(deltaX: number, deltaY: number): number | null {
  if (deltaX !== 0 || deltaY === 0) return null;
  return deltaY;
}
