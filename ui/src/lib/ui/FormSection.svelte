<!-- Group of form rows with a title and optional description, like a macOS settings pane.
     `grouped` draws the rows inside a rounded inset panel. -->
<script lang="ts">
  import type { Snippet } from "svelte";

  import Icon from "./Icon.svelte";

  let {
    title,
    description,
    grouped = false,
    collapsible = false,
    open = $bindable(true),
    summary,
    children,
  }: {
    title?: string;
    description?: string;
    grouped?: boolean;
    /** The title becomes a button that shows or hides the rows (needs a `title`). The rows stay
     *  in the DOM while hidden, so their values and validation keep working. */
    collapsible?: boolean;
    open?: boolean;
    /** One line shown next to the title while the section is closed. */
    summary?: string;
    children: Snippet;
  } = $props();

  const uid = $props.id();
  const headingId = `section-${uid}`;
  const bodyId = `${headingId}-body`;
  const canCollapse = $derived(collapsible && !!title);
</script>

<section class="section" aria-labelledby={title ? headingId : undefined}>
  {#if title || description}
    <header>
      {#if title && canCollapse}
        <h3 id={headingId}>
          <button
            type="button"
            class="toggle"
            aria-expanded={open}
            aria-controls={bodyId}
            onclick={() => (open = !open)}
          >
            <span class="chevron" class:open><Icon name="chevron-right" size={14} /></span>
            <span class="title">{title}</span>
            {#if summary && !open}<span class="summary">{summary}</span>{/if}
          </button>
        </h3>
      {:else if title}<h3 id={headingId}>{title}</h3>{/if}
      {#if description}<p class="description">{description}</p>{/if}
    </header>
  {/if}
  <div id={bodyId} class="rows" class:grouped hidden={canCollapse && !open}>{@render children()}</div>
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
  .toggle {
    all: unset;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: var(--space-1);
    inline-size: 100%;
    cursor: pointer;
    border-radius: var(--radius-control);
  }
  .toggle:focus-visible {
    outline: var(--focus-ring-width) solid var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
  .chevron {
    display: inline-flex;
    color: var(--text-secondary);
    transition: transform var(--duration-base) var(--ease-standard);
  }
  .chevron.open {
    transform: rotate(90deg);
  }
  .summary {
    margin-inline-start: auto;
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-regular);
  }
  .rows[hidden] {
    display: none;
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
