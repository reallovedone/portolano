<!-- Vertical splitter: drag with the mouse or use the arrow keys. -->
<script lang="ts">
  import { t } from "../i18n/strings";

  let {
    width = $bindable(),
    min = 180,
    max = 640,
  }: { width: number; min?: number; max?: number } = $props();

  let dragging = $state(false);

  const clamp = (value: number): number => Math.max(min, Math.min(max, Math.round(value)));

  function onpointerdown(event: PointerEvent): void {
    dragging = true;
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    event.preventDefault();
  }

  function onpointermove(event: PointerEvent): void {
    if (dragging) width = clamp(event.clientX);
  }

  function onpointerup(event: PointerEvent): void {
    dragging = false;
    (event.currentTarget as HTMLElement).releasePointerCapture(event.pointerId);
  }

  function onkeydown(event: KeyboardEvent): void {
    const step = event.shiftKey ? 48 : 16;
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      width = clamp(width - step);
    } else if (event.key === "ArrowRight") {
      event.preventDefault();
      width = clamp(width + step);
    } else if (event.key === "Home") {
      event.preventDefault();
      width = min;
    } else if (event.key === "End") {
      event.preventDefault();
      width = max;
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  class="splitter"
  class:dragging
  role="separator"
  aria-orientation="vertical"
  aria-label={t("sidebar.resize")}
  aria-valuenow={width}
  aria-valuemin={min}
  aria-valuemax={max}
  tabindex="0"
  {onpointerdown}
  {onpointermove}
  {onpointerup}
  {onkeydown}
></div>

<style>
  .splitter {
    width: var(--splitter-hit);
    margin: 0 calc(-1 * var(--splitter-hit) / 2);
    z-index: var(--z-splitter);
    cursor: col-resize;
    background: transparent;
    transition: background var(--duration-base) var(--ease-standard);
  }

  .splitter:hover,
  .splitter.dragging {
    background: var(--border-strong);
  }

  .splitter:focus-visible {
    outline-offset: calc(-1 * var(--focus-ring-width));
  }
</style>
