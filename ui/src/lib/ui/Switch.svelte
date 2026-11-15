<!-- On/off switch on a native checkbox with role="switch" (Space toggles it). The label sits on
     the left and the switch on the right, like macOS settings. -->
<script lang="ts">
  import type { HTMLInputAttributes } from "svelte/elements";

  type Props = Omit<HTMLInputAttributes, "type" | "checked" | "class" | "role"> & {
    checked?: boolean;
    label?: string;
    hint?: string;
    forceFocus?: boolean;
  };

  let {
    checked = $bindable(false),
    label,
    hint,
    forceFocus = false,
    id,
    disabled = false,
    ...rest
  }: Props = $props();

  const uid = $props.id();
  const inputId = $derived(id ?? `switch-${uid}`);
  const hintId = $derived(`${inputId}-hint`);
</script>

<div class="row" class:disabled>
  {#if label || hint}
    <span class="text">
      {#if label}<label for={inputId}>{label}</label>{/if}
      {#if hint}<span id={hintId} class="hint">{hint}</span>{/if}
    </span>
  {/if}
  <input
    {...rest}
    id={inputId}
    type="checkbox"
    role="switch"
    bind:checked
    {disabled}
    class:force-focus={forceFocus}
    aria-describedby={hint ? hintId : undefined}
  />
</div>

<style>
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    min-height: var(--control-h-regular);
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: 1px;
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
  input {
    position: relative;
    flex: none;
    appearance: none;
    -webkit-appearance: none;
    width: var(--switch-width);
    height: var(--switch-height);
    margin: 0;
    background: var(--border-strong);
    border-radius: var(--radius-full);
    cursor: default;
    transition: background var(--duration-base) var(--ease-standard);
  }
  input::after {
    content: "";
    position: absolute;
    top: calc((var(--switch-height) - var(--switch-thumb)) / 2);
    left: calc((var(--switch-height) - var(--switch-thumb)) / 2);
    width: var(--switch-thumb);
    height: var(--switch-thumb);
    background: var(--surface-control);
    border-radius: var(--radius-full);
    box-shadow: var(--shadow-control);
    transition: transform var(--duration-base) var(--ease-out);
  }
  input:checked {
    background: var(--accent);
  }
  input:checked::after {
    transform: translateX(calc(var(--switch-width) - var(--switch-height)));
  }
  input:focus-visible,
  input.force-focus {
    outline: var(--focus-ring-width) solid var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
  input:disabled {
    background: var(--surface-inset);
  }
  input:disabled::after {
    background: var(--text-disabled);
    box-shadow: none;
  }
</style>
