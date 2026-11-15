<!-- Tooltip shown on hover (after a short delay) and on keyboard focus; Escape hides it.
     The bubble is `position: fixed` so scrolling or clipping ancestors (dialog bodies) never cut
     it. `children` receives the tooltip id so a caller can wire aria-describedby if the trigger
     has no accessible name of its own. -->
<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    text,
    placement = "bottom",
    delay = 500,
    open = false,
    children,
  }: {
    text: string;
    placement?: "top" | "bottom";
    /** Hover delay in milliseconds. Focus shows it immediately. */
    delay?: number;
    /** Forces it visible (gallery only). */
    open?: boolean;
    children: Snippet<[string]>;
  } = $props();

  const uid = $props.id();
  const id = `tip-${uid}`;
  let wrap: HTMLSpanElement | undefined = $state();
  let shown = $state(false);
  let left = $state(0);
  let top = $state(0);
  let timer: ReturnType<typeof setTimeout> | undefined;

  const visible = $derived(shown || open);

  function place(): void {
    if (!wrap) return;
    const rect = wrap.getBoundingClientRect();
    left = Math.round(rect.left + rect.width / 2);
    top = Math.round(placement === "top" ? rect.top : rect.bottom);
  }

  function show(immediately: boolean): void {
    clearTimeout(timer);
    const run = (): void => {
      place();
      shown = true;
    };
    if (immediately || delay <= 0) run();
    else timer = setTimeout(run, delay);
  }

  function hide(): void {
    clearTimeout(timer);
    shown = false;
  }

  $effect(() => {
    if (open) place();
    return () => clearTimeout(timer);
  });
</script>

<!-- The wrapper only observes events bubbling from its interactive child. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<span
  bind:this={wrap}
  class="wrap"
  onmouseenter={() => show(false)}
  onmouseleave={hide}
  onfocusin={() => show(true)}
  onfocusout={hide}
  onpointerdown={hide}
  onkeydown={(e) => {
    if (e.key === "Escape") hide();
  }}
>
  {@render children(id)}
  <span
    {id}
    class="tip {placement}"
    class:visible
    role="tooltip"
    data-open={visible ? "true" : "false"}
    style:left="{left}px"
    style:top="{top}px">{text}</span
  >
</span>

<style>
  .wrap {
    display: inline-flex;
  }

  .tip {
    position: fixed;
    z-index: var(--z-tooltip);
    max-width: var(--tooltip-max-width);
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-control);
    background: var(--surface-raised);
    box-shadow: var(--shadow-popover);
    color: var(--text-primary);
    font-size: var(--font-size-sm);
    line-height: var(--line-height-tight);
    pointer-events: none;
    opacity: 0;
    visibility: hidden;
    transition:
      opacity var(--duration-fast) var(--ease-standard),
      visibility var(--duration-fast);
  }
  .bottom {
    transform: translate(-50%, var(--space-1));
  }
  .top {
    transform: translate(-50%, calc(-100% - var(--space-1)));
  }
  .tip.visible {
    opacity: 1;
    visibility: visible;
  }
</style>
