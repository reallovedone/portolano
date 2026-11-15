<!-- Small status or count label. -->
<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    variant = "neutral",
    children,
  }: {
    variant?: "neutral" | "accent" | "danger" | "warning" | "success" | "info";
    children: Snippet;
  } = $props();
</script>

<span class="badge {variant}">{@render children()}</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    height: var(--space-5);
    padding: 0 var(--space-2);
    border: 1px solid transparent;
    border-radius: var(--radius-full);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    line-height: 1;
    white-space: nowrap;
  }
  .neutral {
    background: var(--surface-inset);
    color: var(--text-secondary);
  }
  .accent {
    background: var(--accent-subtle);
    color: var(--accent-text);
  }
  .danger {
    background: var(--danger-bg);
    border-color: var(--danger-border);
    color: var(--danger-text);
  }
  .warning {
    background: var(--warning-bg);
    border-color: var(--warning-border);
    color: var(--warning-text);
  }
  .success {
    background: var(--success-bg);
    border-color: var(--success-border);
    color: var(--success-text);
  }
  .info {
    background: var(--info-bg);
    border-color: var(--info-border);
    color: var(--info-text);
  }
</style>
