<!-- A set of Radio buttons, one per option, with a label and an optional hint each. The group is
     announced as a radio group named by `label`. `idPrefix` gives every input a stable id
     (`<idPrefix>-<value>`, hint `<idPrefix>-<value>-hint`). -->
<script lang="ts">
  import Radio from "./Radio.svelte";
  import type { RadioOption } from "./radio";

  let {
    label,
    options,
    value = $bindable(null),
    name,
    idPrefix,
    disabled = false,
  }: {
    /** Accessible name of the group. */
    label: string;
    options: readonly RadioOption[];
    value?: string | null;
    /** Name shared by the inputs; generated when omitted. */
    name?: string;
    idPrefix?: string;
    disabled?: boolean;
  } = $props();

  const uid = $props.id();
  const groupName = $derived(name ?? `radios-${uid}`);
</script>

<div class="radios" role="radiogroup" aria-label={label}>
  {#each options as option (option.value)}
    <Radio
      id={idPrefix ? `${idPrefix}-${option.value}` : undefined}
      name={groupName}
      value={option.value}
      label={option.label}
      hint={option.hint}
      disabled={disabled || option.disabled}
      bind:group={value}
    />
  {/each}
</div>

<style>
  .radios {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
</style>
