<!-- Multi-line text field. Extra attributes go to <textarea>. -->
<script lang="ts">
  import type { HTMLTextareaAttributes } from "svelte/elements";

  import FieldShell from "./FieldShell.svelte";
  import { describedBy } from "./field";

  type Props = Omit<HTMLTextareaAttributes, "value" | "class"> & {
    value?: string | null;
    label?: string;
    hint?: string;
    error?: string | null;
    optionalLabel?: string;
    forceFocus?: boolean;
    /** Monospace text, for keys, command lines and other technical content. */
    monospace?: boolean;
  };

  let {
    value = $bindable(""),
    label,
    hint,
    error = null,
    optionalLabel,
    forceFocus = false,
    monospace = false,
    id,
    rows = 4,
    disabled = false,
    ...rest
  }: Props = $props();

  const uid = $props.id();
  const areaId = $derived(id ?? `area-${uid}`);
</script>

<FieldShell id={areaId} {label} {hint} {error} {optionalLabel}>
  <textarea
    {...rest}
    id={areaId}
    {rows}
    {disabled}
    bind:value
    class:invalid={!!error}
    class:force-focus={forceFocus}
    class:monospace
    aria-invalid={error ? "true" : undefined}
    aria-describedby={describedBy(areaId, hint, error)}
  ></textarea>
</FieldShell>

<style>
  textarea {
    width: 100%;
    min-height: calc(var(--control-h-large) * 2);
    padding: var(--space-1) var(--space-2);
    background: var(--surface-field);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
    outline: none;
    color: var(--text-primary);
    line-height: var(--line-height-normal);
    resize: vertical;
    transition:
      border-color var(--duration-fast) var(--ease-standard),
      box-shadow var(--duration-fast) var(--ease-standard);
  }
  textarea.monospace {
    font-family: var(--font-mono);
    font-size: var(--font-size-sm);
  }
  textarea::placeholder {
    color: var(--text-tertiary);
  }
  textarea:focus,
  textarea.force-focus {
    border-color: var(--focus-ring);
    box-shadow: 0 0 0 var(--focus-ring-width) var(--focus-ring);
  }
  textarea.invalid {
    border-color: var(--danger-text);
  }
  textarea:disabled {
    background: var(--surface-inset);
    border-color: transparent;
    color: var(--text-disabled);
    resize: none;
  }
</style>
