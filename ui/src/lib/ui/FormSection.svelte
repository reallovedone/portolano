<!-- Group of form rows with a title and optional description, like a macOS settings pane.
     `grouped` draws the rows inside a rounded inset panel. -->
<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    title,
    description,
    grouped = false,
    children,
  }: {
    title?: string;
    description?: string;
    grouped?: boolean;
    children: Snippet;
  } = $props();

  const uid = $props.id();
  const headingId = `section-${uid}`;
</script>

<section class="section" aria-labelledby={title ? headingId : undefined}>
  {#if title || description}
    <header>
      {#if title}<h3 id={headingId}>{title}</h3>{/if}
      {#if description}<p class="description">{description}</p>{/if}
    </header>
  {/if}
  <div class="rows" class:grouped>{@render children()}</div>
</section>

<style>
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-bottom: var(--space-5);
  }
  header {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  h3 {
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-semibold);
  }
  .description {
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .rows.grouped {
    gap: 0;
    padding: var(--space-1) var(--space-3);
    background: var(--surface-window);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-panel);
  }
  .rows.grouped :global(.form-row) {
    padding: var(--space-2) 0;
  }
  .rows.grouped :global(.form-row + .form-row) {
    border-top: 1px solid var(--border-hairline);
  }
</style>
