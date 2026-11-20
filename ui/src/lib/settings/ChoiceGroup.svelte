<!-- A radio group drawn as cards or round swatches. Every option is a real radio input (hidden,
     but it keeps the focus and the arrow keys), so the group is announced as a radio group, the
     arrow keys move and select, and the label of the card is the accessible name. The preview
     of each option is decorative (`aria-hidden`). Used by the Settings window. -->
<script lang="ts" generics="T extends string">
  import type { Snippet } from "svelte";

  import Icon from "../ui/Icon.svelte";

  let {
    label,
    options,
    value,
    onselect,
    preview,
    variant = "card",
    columns,
  }: {
    /** Accessible name of the group. */
    label: string;
    options: readonly { value: T; label: string }[];
    value: T;
    onselect: (value: T) => void;
    /** Draws the picture of one option; `selected` is true for the chosen one. */
    preview: Snippet<[T, boolean]>;
    /** `card`: a framed card with the picture above the name. `swatch`: a round picture. */
    variant?: "card" | "swatch";
    /** Fixed number of columns; by default as many as fit. */
    columns?: number;
  } = $props();

  const uid = $props.id();
</script>

<div
  class="group {variant}"
  class:fixed={columns !== undefined}
  role="radiogroup"
  aria-label={label}
  style:--columns={columns}
>
  {#each options as option (option.value)}
    {@const selected = value === option.value}
    <label class="choice" class:selected>
      <input
        type="radio"
        name="choice-{uid}"
        value={option.value}
        checked={selected}
        onchange={() => onselect(option.value)}
      />
      <span class="picture" aria-hidden="true">{@render preview(option.value, selected)}</span>
      <span class="caption">{option.label}</span>
      {#if variant === "card" && selected}
        <span class="check" aria-hidden="true"><Icon name="check" size={14} strokeWidth={2.5} /></span>
      {/if}
    </label>
  {/each}
</div>

<style>
  .group {
    display: grid;
    gap: var(--space-2);
  }
  .card {
    grid-template-columns: repeat(auto-fit, minmax(calc(var(--space-10) * 3), 1fr));
  }
  .card.fixed {
    grid-template-columns: repeat(var(--columns), 1fr);
  }
  .swatch {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }

  /* The input covers the whole label: it is the click target and the focus target. */
  .choice {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
    color: var(--text-primary);
    cursor: default;
  }
  .choice input {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    margin: 0;
    opacity: 0;
    cursor: default;
  }
  .caption {
    font-size: var(--font-size-md);
  }

  .card .choice {
    padding: var(--space-2);
    background: var(--surface-control);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-panel);
    transition: background var(--duration-fast) var(--ease-standard);
  }
  .card .choice:hover {
    background: var(--surface-control-hover);
  }
  .card .choice.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }
  .card .choice:has(input:focus-visible) {
    outline: var(--focus-ring-width) solid var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
  .card .picture {
    display: block;
  }
  .card .caption {
    padding-inline: var(--space-1);
    font-weight: var(--font-weight-medium);
  }
  .check {
    position: absolute;
    top: var(--space-3);
    right: var(--space-3);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--icon-size-md);
    height: var(--icon-size-md);
    background: var(--accent);
    color: var(--text-on-accent);
    border-radius: var(--radius-full);
  }

  .swatch .choice {
    align-items: center;
    gap: var(--space-1);
    min-width: var(--space-12);
  }
  .swatch .picture {
    display: inline-flex;
    padding: var(--space-1);
    border: var(--focus-ring-width) solid transparent;
    border-radius: var(--radius-full);
    transition: border-color var(--duration-fast) var(--ease-standard);
  }
  .swatch .choice:hover .picture {
    border-color: var(--border-strong);
  }
  .swatch .choice.selected .picture {
    border-color: var(--text-primary);
  }
  .swatch .choice:has(input:focus-visible) .picture {
    outline: var(--focus-ring-width) solid var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
  .swatch .caption {
    font-size: var(--font-size-sm);
    color: var(--text-secondary);
  }
  .swatch .choice.selected .caption {
    color: var(--text-primary);
  }
</style>
