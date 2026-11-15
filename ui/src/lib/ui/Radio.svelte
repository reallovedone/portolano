<!-- Radio button on a native input (arrow-key navigation inside a group, forms and screen
     readers for free). Bind `group` to the selected value of the set; radios that share `name`
     form one set. Same label + hint layout as Checkbox. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLInputAttributes } from "svelte/elements";

  type Props = Omit<HTMLInputAttributes, "type" | "checked" | "group" | "value" | "class" | "children"> & {
    /** Value this radio stands for. */
    value: string;
    /** Selected value of the set (bindable). */
    group?: string | null;
    label?: string;
    hint?: string;
    children?: Snippet;
    forceFocus?: boolean;
  };

  let {
    value,
    group = $bindable(null),
    label,
    hint,
    children,
    forceFocus = false,
    id,
    disabled = false,
    ...rest
  }: Props = $props();

  const uid = $props.id();
  const inputId = $derived(id ?? `radio-${uid}`);
  const hintId = $derived(`${inputId}-hint`);
</script>

<div class="row" class:disabled>
  <span class="circle" class:force-focus={forceFocus}>
    <input
      {...rest}
      id={inputId}
      type="radio"
      {value}
      bind:group
      {disabled}
      aria-describedby={hint ? hintId : undefined}
    />
    <span class="dot"></span>
  </span>
  {#if label || children || hint}
    <span class="text">
      {#if label || children}
        <label for={inputId}>{#if children}{@render children()}{:else}{label}{/if}</label>
      {/if}
      {#if hint}<span id={hintId} class="hint">{hint}</span>{/if}
    </span>
  {/if}
</div>

<style>
  .row {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    min-height: var(--control-h-compact);
  }
  .circle {
    position: relative;
    display: inline-flex;
    flex: none;
    width: var(--checkbox-size);
    height: var(--checkbox-size);
    margin-top: calc((var(--control-h-compact) - var(--checkbox-size)) / 2);
  }
  input {
    appearance: none;
    -webkit-appearance: none;
    width: 100%;
    height: 100%;
    margin: 0;
    background: var(--surface-field);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-full);
    box-shadow: var(--shadow-control);
    cursor: default;
    transition:
      background var(--duration-fast) var(--ease-standard),
      border-color var(--duration-fast) var(--ease-standard);
  }
  input:checked {
    background: var(--accent);
    border-color: var(--accent);
  }
  input:focus-visible,
  .force-focus input {
    outline: var(--focus-ring-width) solid var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
  input:disabled {
    background: var(--surface-inset);
    border-color: transparent;
    box-shadow: none;
  }
  input:disabled:checked {
    background: var(--text-disabled);
  }
  .dot {
    position: absolute;
    inset: var(--space-1);
    display: none;
    background: var(--text-on-accent);
    border-radius: var(--radius-full);
    pointer-events: none;
  }
  input:checked + .dot {
    display: block;
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding-top: calc((var(--control-h-compact) - var(--font-size-md) * var(--line-height-normal)) / 2);
  }
  label {
    color: var(--text-primary);
  }
  .hint {
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
  }
  .disabled label {
    color: var(--text-disabled);
  }
</style>
