<!-- Calm placeholder for empty lists and first-run screens. -->
<script lang="ts">
  import type { Snippet } from "svelte";

  import type { IconName } from "../design/icons/registry";
  import Icon from "./Icon.svelte";

  let {
    icon,
    title,
    text,
    graphic,
    action,
  }: {
    icon?: IconName;
    title: string;
    text?: string;
    /** Custom graphic (for example the Logo); replaces `icon`. */
    graphic?: Snippet;
    action?: Snippet;
  } = $props();
</script>

<div class="empty">
  {#if graphic}
    <div class="graphic">{@render graphic()}</div>
  {:else if icon}
    <div class="graphic"><Icon name={icon} size={20} /></div>
  {/if}
  <h2 class="title">{title}</h2>
  {#if text}<p class="text">{text}</p>{/if}
  {#if action}<div class="action">{@render action()}</div>{/if}
</div>

<style>
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-8) var(--space-6);
    text-align: center;
    color: var(--text-secondary);
  }
  .graphic {
    display: inline-flex;
    margin-bottom: var(--space-1);
    color: var(--text-tertiary);
  }
  .title {
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
    color: var(--text-primary);
  }
  .text {
    max-width: calc(var(--tooltip-max-width) * 1.5);
    font-size: var(--font-size-md);
    line-height: var(--line-height-relaxed);
  }
  .action {
    margin-top: var(--space-2);
  }
</style>
