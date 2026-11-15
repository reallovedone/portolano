<!-- Header block of the editor dialogs: the kind icon on a plate, the kind name and a one-line
     summary. Purely presentational. -->
<script lang="ts">
  import type { NodeKind } from "../tree/model";
  import KindIcon from "../ui/KindIcon.svelte";

  let { kind, title, summary }: { kind: NodeKind; title: string; summary: string } = $props();
</script>

<div class="header">
  <span class="plate"><KindIcon {kind} size={28} /></span>
  <div class="text">
    <strong class="title">{title}</strong>
    <span class="summary">{summary}</span>
  </div>
</div>

<style>
  .header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
  }

  .plate {
    display: grid;
    flex: none;
    place-items: center;
    inline-size: calc(var(--space-4) * 3);
    block-size: calc(var(--space-4) * 3);
    background: var(--surface-inset);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-panel);
  }

  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .title {
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
    line-height: var(--line-height-tight);
  }

  .summary {
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
  }
</style>
