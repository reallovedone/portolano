<!-- Window drag handle for the integrated macOS title bar (titleBarStyle "Overlay").
     `data-tauri-drag-region` only drags from the element itself, not from its children, so keep
     interactive content outside it. Its height is --titlebar-height, which is 0 on Windows, so it
     takes no space there. Needs the core:window:allow-start-dragging permission. -->
<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    fill = false,
    children,
  }: {
    /** Grow to fill the available space instead of using the title bar height. */
    fill?: boolean;
    children?: Snippet;
  } = $props();
</script>

<div class="drag" class:fill data-tauri-drag-region aria-hidden={children ? undefined : "true"}>
  {#if children}{@render children()}{/if}
</div>

<style>
  .drag {
    flex: none;
    width: 100%;
    height: var(--titlebar-height);
  }
  .drag.fill {
    flex: 1;
    height: auto;
    min-height: var(--titlebar-height);
  }
</style>
