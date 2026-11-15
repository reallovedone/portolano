<!-- Labelled, selectable monospace block for a fingerprint, a host:port or another identifier
     the user has to compare by eye. One click selects the whole value. -->
<script lang="ts">
  let { label, value }: { label?: string; value: string } = $props();
</script>

<div class="block">
  {#if label}<span class="label">{label}</span>{/if}
  <code class="value">{value}</code>
</div>

<style>
  .block {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
    text-align: left;
  }
  .label {
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    color: var(--text-tertiary);
  }
  .value {
    display: block;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-control);
    background: var(--surface-inset);
    color: var(--text-primary);
    font-family: var(--font-mono);
    font-size: var(--font-size-sm);
    line-height: var(--line-height-normal);
    overflow-wrap: anywhere;
    user-select: all;
  }
</style>
