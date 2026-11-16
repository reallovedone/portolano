<!-- Release notes as text. They come from the network: the Markdown is read into plain data
     (markdown.ts) and drawn with text nodes only, so nothing in it is ever parsed as HTML. -->
<script lang="ts">
  import { t } from "../i18n/strings";
  import { parseMarkdown, type Inline } from "./markdown";

  let {
    notes,
    version,
    onopen,
  }: {
    /** `null`: nothing could be loaded; the GitHub link is offered instead. */
    notes: string | null;
    version: string;
    onopen: () => void;
  } = $props();

  const blocks = $derived(notes === null ? [] : parseMarkdown(notes));
</script>

{#snippet inline(parts: Inline[])}
  {#each parts as part, index (index)}
    {#if part.kind === "code"}<code>{part.text}</code>
    {:else if part.kind === "strong"}<strong>{part.text}</strong>
    {:else if part.kind === "em"}<em>{part.text}</em>
    {:else}{part.text}{/if}
  {/each}
{/snippet}

{#if notes === null || blocks.length === 0}
  <div class="fallback" data-testid="notes-fallback">
    <p>{t("update.notesFallback")}</p>
    <button type="button" class="link" onclick={onopen} data-version={version}>
      {t("update.openNotes")}
    </button>
  </div>
{:else}
  <!-- A scrollable region must be reachable with the keyboard. -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div class="notes" data-testid="release-notes" tabindex="0" role="region" aria-label={t("update.notesHeading")}>
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
{/if}

<style>
  .notes {
    max-height: 40vh;
    overflow: auto;
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

  .fallback p {
    margin: 0 0 var(--space-2);
    color: var(--text-secondary);
  }

  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--text-link);
    font: inherit;
    text-decoration: underline;
    cursor: pointer;
  }
</style>
