<!-- Keyboard shortcut hint, e.g. <Kbd>⌘N</Kbd>. -->
<script lang="ts">
  import type { Snippet } from "svelte";

  let { children }: { children: Snippet } = $props();
</script>

<kbd class="kbd">{@render children()}</kbd>

<style>
  .kbd {
    display: inline-flex;
    align-items: center;
    min-width: var(--space-5);
    height: var(--space-5);
    padding: 0 var(--space-1);
    justify-content: center;
    font-family: var(--font-sans);
    font-size: var(--font-size-xs);
    line-height: 1;
    color: var(--text-secondary);
    background: var(--surface-control);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-xs);
    box-shadow: var(--shadow-control);
    white-space: nowrap;
  }
</style>
