<!-- Gallery section with an anchor. -->
<script lang="ts">
  import type { Snippet } from "svelte";

  let { id, title, note, children }: { id: string; title: string; note?: string; children: Snippet } =
    $props();
</script>

<section {id} class="section">
  <h2><a href="#{id}">{title}</a></h2>
  {#if note}<p class="note">{note}</p>{/if}
  {@render children()}
</section>

<style>
  .section {
    padding-top: var(--space-6);
    margin-bottom: var(--space-8);
    border-top: 1px solid var(--border-hairline);
    scroll-margin-top: var(--space-12);
  }
  h2 {
    margin-bottom: var(--space-1);
    font-size: var(--font-size-xl);
    font-weight: var(--font-weight-semibold);
  }
  h2 a {
    color: inherit;
    text-decoration: none;
  }
  .note {
    margin-bottom: var(--space-4);
    color: var(--text-secondary);
  }
</style>
