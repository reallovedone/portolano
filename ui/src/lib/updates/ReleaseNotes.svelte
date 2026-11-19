<!-- Release notes as text. The Markdown is read into plain data (markdown.ts) and drawn with text
     nodes only, so nothing in it is ever parsed as HTML. -->
<script lang="ts">
  import { t } from "../i18n/strings";
  import { parseMarkdown, type Inline } from "./markdown";

  let {
    notes,
    bounded = true,
  }: {
    notes: string;
    /** Scrolls on its own (a tab stop, a labelled region). Off when a parent scrolls. */
    bounded?: boolean;
  } = $props();

  const blocks = $derived(parseMarkdown(notes));
</script>

{#snippet inline(parts: Inline[])}
  {#each parts as part, index (index)}
    {#if part.kind === "code"}<code>{part.text}</code>
    {:else if part.kind === "strong"}<strong>{part.text}</strong>
    {:else if part.kind === "em"}<em>{part.text}</em>
    {:else}{part.text}{/if}
  {/each}
{/snippet}

<!-- A scrollable region must be reachable with the keyboard. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div
  class="notes"
  class:bounded
  data-testid="release-notes"
  tabindex={bounded ? 0 : undefined}
  role={bounded ? "region" : undefined}
  aria-label={bounded ? t("update.notesHeading") : undefined}
>
    {#each blocks as block, index (index)}
      {#if block.kind === "heading"}
        <p class="heading level-{block.level}" role="heading" aria-level={block.level + 2}>{@render inline(block.inline)}</p>
      {:else if block.kind === "paragraph"}
        <p>{@render inline(block.inline)}</p>
      {:else if block.kind === "list"}
        {#if block.ordered}
          <ol>
            {#each block.items as item, i (i)}<li>{@render inline(item)}</li>{/each}
          </ol>
        {:else}
          <ul>
            {#each block.items as item, i (i)}<li>{@render inline(item)}</li>{/each}
          </ul>
        {/if}
      {:else}
        <pre>{block.text}</pre>
      {/if}
    {/each}
</div>

<style>
  .bounded {
    max-height: 40vh;
    overflow: auto;
  }

  .notes {
    padding: var(--space-3);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-control);
    background: var(--surface-inset);
    font-size: var(--font-size-md);
    line-height: var(--line-height-relaxed);
    overflow-wrap: anywhere;
  }

  .notes > :global(*:first-child) {
    margin-top: 0;
  }

  p {
    margin: var(--space-2) 0 0;
  }

  .heading {
    font-weight: var(--font-weight-semibold);
  }

  .level-1 {
    font-size: var(--font-size-lg);
  }

  ul,
  ol {
    margin: var(--space-2) 0 0;
    padding-inline-start: var(--space-5);
  }

  li + li {
    margin-top: var(--space-1);
  }

  code,
  pre {
    font-family: var(--font-mono);
    font-size: var(--font-size-sm);
  }

  pre {
    margin: var(--space-2) 0 0;
    padding: var(--space-2);
    overflow: auto;
    border-radius: var(--radius-xs);
    background: var(--surface-content);
  }
</style>
