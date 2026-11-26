<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    size = 16,
    label,
    children,
  }: {
    size?: 14 | 16 | 20;
    /** Accessible name. Omit when the surrounding control already announces the busy state. */
    label?: string;
    /** Visible text beside the spinner, for a task running on its own (announced as a status). */
    children?: Snippet;
  } = $props();
</script>

{#if children}
  <span class="busy" role="status">
    <span class="spinner" style:--spinner-size="{size}px" aria-hidden="true"></span>
    <span>{@render children()}</span>
  </span>
{:else}
  <span
    class="spinner"
    style:--spinner-size="{size}px"
    role={label ? "status" : undefined}
    aria-label={label}
    aria-hidden={label ? undefined : "true"}
  ></span>
{/if}

<style>
  .busy {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--text-secondary);
  }

  .spinner {
    flex: none;
    display: inline-block;
    width: var(--spinner-size);
    height: var(--spinner-size);
    border: var(--spinner-width) solid var(--border-strong);
    border-top-color: currentColor;
    border-radius: var(--radius-full);
    animation: spin var(--duration-spin) linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
