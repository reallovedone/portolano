<!-- Checkbox on a native input (keyboard, forms and screen readers for free). Supports the
     indeterminate state. The label is the `label` prop or the children. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLInputAttributes } from "svelte/elements";

  import Icon from "./Icon.svelte";

  type Props = Omit<HTMLInputAttributes, "type" | "checked" | "class" | "children"> & {
    checked?: boolean;
    indeterminate?: boolean;
    label?: string;
    hint?: string;
    children?: Snippet;
    forceFocus?: boolean;
  };

  let {
    checked = $bindable(false),
    indeterminate = false,
    label,
    hint,
    children,
    forceFocus = false,
    id,
    disabled = false,
    ...rest
  }: Props = $props();

  const uid = $props.id();
  const inputId = $derived(id ?? `check-${uid}`);
  const hintId = $derived(`${inputId}-hint`);
  let input: HTMLInputElement | undefined = $state();

  $effect(() => {
    if (input) input.indeterminate = indeterminate && !checked;
  });
</script>

<div class="row" class:disabled>
  <span class="box" class:force-focus={forceFocus}>
    <input
      {...rest}
      bind:this={input}
      id={inputId}
      type="checkbox"
      bind:checked
      {disabled}
      aria-describedby={hint ? hintId : undefined}
    />
    <span class="mark">
      {#if indeterminate && !checked}<Icon name="minus" size={14} strokeWidth={2.5} />{:else}<Icon name="check" size={14} strokeWidth={2.5} />{/if}
    </span>
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
  .box {
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
    border-radius: var(--radius-xs);
    box-shadow: var(--shadow-control);
    cursor: default;
    transition:
      background var(--duration-fast) var(--ease-standard),
      border-color var(--duration-fast) var(--ease-standard);
  }
  input:checked,
  input:indeterminate {
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
  input:disabled:checked,
  input:disabled:indeterminate {
    background: var(--text-disabled);
  }
  .mark {
    position: absolute;
    inset: 0;
    display: none;
    align-items: center;
    justify-content: center;
    color: var(--text-on-accent);
    pointer-events: none;
  }
  input:checked + .mark,
  input:indeterminate + .mark {
    display: inline-flex;
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
