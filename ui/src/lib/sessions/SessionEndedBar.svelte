<!-- Floating bar at the bottom of a finished terminal session: the output stays readable above
     it. Same visual family as the full-area `SessionState`. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";

  import Icon from "../ui/Icon.svelte";

  type Props = Omit<HTMLAttributes<HTMLDivElement>, "children" | "class"> & {
    text: string;
    actions?: Snippet;
  };

  let { text, actions, ...rest }: Props = $props();
</script>

<div {...rest} class="wrap">
  <div class="bar" role="status">
    <Icon name="unplug" />
    <span class="text">{text}</span>
    {#if actions}<span class="actions">{@render actions()}</span>{/if}
  </div>
</div>

<style>
  .wrap {
    position: absolute;
    inset: auto 0 var(--space-4);
    display: flex;
    justify-content: center;
    padding: 0 var(--space-4);
    pointer-events: none;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    max-width: 100%;
    padding: var(--space-2) var(--space-2) var(--space-2) var(--space-4);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-panel);
    background: var(--surface-raised);
    box-shadow: var(--shadow-popover);
    color: var(--text-secondary);
    font-size: var(--font-size-md);
    pointer-events: auto;
  }
  .text {
    min-width: 0;
    color: var(--text-primary);
  }
  .actions {
    display: inline-flex;
    gap: var(--space-2);
  }
</style>
