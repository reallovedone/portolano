<!-- Segmented control on native radio inputs: arrow keys move and select, Tab enters and leaves
     the group, and the group is announced as a radio group. -->
<script lang="ts">
  import type { IconName } from "../design/icons/registry";
  import Icon from "./Icon.svelte";

  export interface Segment {
    value: string;
    label: string;
    icon?: IconName;
    disabled?: boolean;
  }

  let {
    options,
    value = $bindable(),
    label,
    size = "regular",
    iconOnly = false,
    disabled = false,
    onchange,
  }: {
    options: Segment[];
    value?: string;
    /** Accessible name of the group. */
    label: string;
    size?: "compact" | "regular";
    /** Shows only the icons; the labels stay as accessible names. */
    iconOnly?: boolean;
    disabled?: boolean;
    onchange?: (value: string) => void;
  } = $props();

  const uid = $props.id();
  const name = `seg-${uid}`;
</script>

<div class="group {size}" class:disabled role="radiogroup" aria-label={label}>
  {#each options as option (option.value)}
    <label class="segment" class:selected={value === option.value}>
      <input
        type="radio"
        {name}
        value={option.value}
        checked={value === option.value}
        disabled={disabled || option.disabled}
        onchange={() => {
          value = option.value;
          onchange?.(option.value);
        }}
      />
      {#if option.icon}<Icon name={option.icon} size={14} />{/if}
      <span class:sr-only={iconOnly && option.icon}>{option.label}</span>
    </label>
  {/each}
</div>

<style>
  .group {
    display: inline-flex;
    gap: 1px;
    padding: 2px;
    background: var(--surface-inset);
    border-radius: var(--radius-control);
  }
  .segment {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-1);
    min-width: var(--control-h-regular);
    height: calc(var(--control-h-regular) - var(--space-1));
    padding: 0 var(--space-3);
    border-radius: var(--radius-xs);
    color: var(--text-secondary);
    font-size: var(--font-size-md);
    white-space: nowrap;
    transition:
      background var(--duration-fast) var(--ease-standard),
      color var(--duration-fast) var(--ease-standard);
  }
  .compact .segment {
    height: calc(var(--control-h-compact) - var(--space-1));
    padding: 0 var(--space-2);
    font-size: var(--font-size-sm);
  }
  .segment:hover:not(.selected) {
    color: var(--text-primary);
  }
  .segment.selected {
    background: var(--segment-selected-bg);
    box-shadow: var(--shadow-control);
    color: var(--text-primary);
    font-weight: var(--font-weight-medium);
  }
  .segment:has(input:focus-visible) {
    outline: var(--focus-ring-width) solid var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
  .segment:has(input:disabled) {
    color: var(--text-disabled);
  }
  input {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    margin: 0;
    opacity: 0;
    appearance: none;
    -webkit-appearance: none;
    cursor: default;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
</style>
