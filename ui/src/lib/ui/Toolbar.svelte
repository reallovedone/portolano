<!-- Horizontal tool strip: role="toolbar", arrow keys / Home / End move between its buttons
     (typing inside a field is never intercepted). Slots: start, children (flexible middle), end. -->
<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    label,
    bordered = false,
    flush = false,
    start,
    end,
    children,
  }: {
    /** Accessible name of the toolbar. */
    label: string;
    /** Adds a hairline under the toolbar. */
    bordered?: boolean;
    /** No padding and no minimum height: the parent already provides the gutter. */
    flush?: boolean;
    start?: Snippet;
    end?: Snippet;
    children?: Snippet;
  } = $props();

  let root: HTMLDivElement | undefined = $state();

  function onkeydown(event: KeyboardEvent): void {
    const target = event.target;
    if (!(target instanceof HTMLButtonElement) || !root) return;
    const keys = ["ArrowLeft", "ArrowRight", "Home", "End"];
    if (!keys.includes(event.key)) return;
    const buttons = [...root.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
    const index = buttons.indexOf(target);
    if (index < 0) return;
    event.preventDefault();
    let next = index;
    if (event.key === "ArrowLeft") next = (index - 1 + buttons.length) % buttons.length;
    else if (event.key === "ArrowRight") next = (index + 1) % buttons.length;
    else if (event.key === "Home") next = 0;
    else next = buttons.length - 1;
    buttons[next]?.focus();
  }
</script>

<!-- svelte-ignore a11y_interactive_supports_focus -->
<div
  bind:this={root}
  class="toolbar"
  class:bordered
  class:flush
  role="toolbar"
  aria-label={label}
  aria-orientation="horizontal"
  {onkeydown}
>
  {#if start}<div class="group">{@render start()}</div>{/if}
  <div class="middle">{#if children}{@render children()}{/if}</div>
  {#if end}<div class="group">{@render end()}</div>{/if}
</div>

<style>
  .toolbar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-height: var(--control-h-large);
    padding: var(--space-1) var(--space-2);
  }
  .flush {
    min-height: 0;
    padding: 0;
  }
  .bordered {
    border-bottom: 1px solid var(--border-hairline);
  }
  .group {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  .middle {
    display: flex;
    flex: 1;
    align-items: center;
    gap: var(--space-1);
    min-width: 0;
  }
</style>
