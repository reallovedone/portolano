<!-- Native <select> styled as a macOS pop-up button: robust, accessible and keyboard-complete by
     construction. Pass `options`, or <option> elements as children. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLSelectAttributes } from "svelte/elements";

  import FieldShell from "./FieldShell.svelte";
  import { describedBy } from "./field";
  import Icon from "./Icon.svelte";

  export interface SelectOption {
    value: string;
    label: string;
    disabled?: boolean;
  }

  type Props = Omit<HTMLSelectAttributes, "value" | "class" | "children" | "size"> & {
    value?: string;
    options?: SelectOption[];
    label?: string;
    hint?: string;
    error?: string | null;
    optionalLabel?: string;
    size?: "compact" | "regular" | "large";
    forceFocus?: boolean;
    children?: Snippet;
  };

  let {
    value = $bindable(),
    options,
    label,
    hint,
    error = null,
    optionalLabel,
    size = "regular",
    forceFocus = false,
    id,
    disabled = false,
    children,
    ...rest
  }: Props = $props();

  const uid = $props.id();
  const selectId = $derived(id ?? `select-${uid}`);
</script>

<FieldShell id={selectId} {label} {hint} {error} {optionalLabel}>
  <div class="control {size}" class:disabled>
    <select
      {...rest}
      id={selectId}
      {disabled}
      bind:value
      class:invalid={!!error}
      class:force-focus={forceFocus}
      aria-invalid={error ? "true" : undefined}
      aria-describedby={describedBy(selectId, hint, error)}
    >
      {#if options}
        {#each options as option (option.value)}
          <option value={option.value} disabled={option.disabled}>{option.label}</option>
        {/each}
      {/if}
      {#if children}{@render children()}{/if}
    </select>
    <span class="chevron"><Icon name="chevrons-up-down" size={14} /></span>
  </div>
</FieldShell>

<style>
  .control {
    position: relative;
    display: flex;
    min-width: 0;
  }
  select {
    appearance: none;
    -webkit-appearance: none;
    width: 100%;
    height: var(--control-h-regular);
    padding: 0 calc(var(--space-2) + var(--icon-size-sm) + var(--space-1)) 0 var(--space-2);
    background: var(--surface-control);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
    box-shadow: var(--shadow-control);
    color: var(--text-primary);
    text-overflow: ellipsis;
    transition:
      background var(--duration-fast) var(--ease-standard),
      border-color var(--duration-fast) var(--ease-standard);
  }
  .compact select {
    height: var(--control-h-compact);
    font-size: var(--font-size-sm);
  }
  .large select {
    height: var(--control-h-large);
  }
  select:hover:not(:disabled) {
    background: var(--surface-control-hover);
  }
  select:focus-visible,
  select.force-focus {
    outline: none;
    border-color: var(--focus-ring);
    box-shadow: 0 0 0 var(--focus-ring-width) var(--focus-ring);
  }
  select.invalid {
    border-color: var(--danger-text);
  }
  select:disabled {
    background: var(--surface-inset);
    border-color: transparent;
    box-shadow: none;
    color: var(--text-disabled);
  }
  .chevron {
    position: absolute;
    top: 50%;
    right: var(--space-2);
    display: inline-flex;
    transform: translateY(-50%);
    color: var(--text-secondary);
    pointer-events: none;
  }
  .disabled .chevron {
    color: var(--text-disabled);
  }
</style>
