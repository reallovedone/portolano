<!-- Label-control row of a form. "inline" puts the label in a right-aligned column on the left
     (macOS settings); "stacked" puts it above. Give `for` the id of the control so the label is
     programmatically associated; without it the row is a labelled group. -->
<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    label,
    for: forId,
    hint,
    error,
    layout = "inline",
    children,
  }: {
    label: string;
    /** Id of the control this label belongs to. */
    for?: string;
    hint?: string;
    error?: string | null;
    layout?: "inline" | "stacked";
    children: Snippet;
  } = $props();

  const uid = $props.id();
  const labelId = `row-${uid}`;
  const hintId = `${labelId}-hint`;
  const errorId = `${labelId}-err`;
</script>

<div
  class="form-row {layout}"
  role={forId ? undefined : "group"}
  aria-labelledby={forId ? undefined : labelId}
>
  {#if forId}
    <label id={labelId} class="label" for={forId}>{label}</label>
  {:else}
    <span id={labelId} class="label">{label}</span>
  {/if}
  <div class="control">
    {@render children()}
    {#if hint}<p id={hintId} class="hint">{hint}</p>{/if}
    {#if error}<p id={errorId} class="error" role="alert">{error}</p>{/if}
  </div>
</div>

<style>
  .form-row {
    display: grid;
    gap: var(--space-1) var(--space-3);
    align-items: start;
  }
  .inline {
    grid-template-columns: var(--form-label-width) minmax(0, 1fr);
  }
  .stacked {
    grid-template-columns: minmax(0, 1fr);
  }
  .label {
    min-height: var(--control-h-regular);
    display: flex;
    align-items: center;
    color: var(--text-primary);
    font-weight: var(--font-weight-medium);
  }
  .inline .label {
    justify-content: flex-end;
    text-align: right;
  }
  .stacked .label {
    min-height: 0;
  }
  .control {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }
  .hint {
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
  }
  .error {
    color: var(--danger-text);
    font-size: var(--font-size-sm);
  }
</style>
