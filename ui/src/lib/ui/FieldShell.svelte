<!-- Label, control and hint/error layout shared by TextField, TextArea and Select. -->
<script lang="ts">
  import type { Snippet } from "svelte";

  import { fieldIds } from "./field";

  let {
    id,
    label,
    hint,
    error,
    optionalLabel,
    children,
  }: {
    id: string;
    label?: string;
    hint?: string;
    error?: string | null;
    /** Text appended to the label for optional fields, e.g. "optional". */
    optionalLabel?: string;
    children: Snippet;
  } = $props();

  const ids = $derived(fieldIds(id));
</script>

<div class="field">
  {#if label}
    <label class="label" for={id}>
      {label}{#if optionalLabel}<span class="optional"> · {optionalLabel}</span>{/if}
    </label>
  {/if}
  {@render children()}
  {#if hint}<p id={ids.hint} class="hint">{hint}</p>{/if}
  {#if error}<p id={ids.error} class="error" role="alert">{error}</p>{/if}
</div>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }
  .label {
    font-weight: var(--font-weight-medium);
    color: var(--text-primary);
  }
  .optional {
    font-weight: var(--font-weight-regular);
    color: var(--text-tertiary);
  }
  .hint {
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
    line-height: var(--line-height-normal);
  }
  .error {
    color: var(--danger-text);
    font-size: var(--font-size-sm);
    line-height: var(--line-height-normal);
  }
</style>
