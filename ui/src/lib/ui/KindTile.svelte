<!-- Large tile for a connection kind: a rounded square with the kind icon and a caption.
     Used on the welcome screen as a quick "new connection" action. -->
<script lang="ts">
  import type { HTMLButtonAttributes } from "svelte/elements";

  import type { NodeKind } from "../tree/model";
  import KindIcon from "./KindIcon.svelte";

  let {
    kind,
    label,
    hint,
    ...rest
  }: {
    kind: Exclude<NodeKind, "folder">;
    label: string;
    /** Optional second line, for example the default port. */
    hint?: string;
  } & HTMLButtonAttributes = $props();
</script>

<button type="button" class="tile" {...rest}>
  <span class="plate"><KindIcon {kind} size={28} /></span>
  <span class="label">{label}</span>
  {#if hint}<span class="hint">{hint}</span>{/if}
</button>

<style>
  .tile {
    display: inline-flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-2);
    border: 0;
    border-radius: var(--radius-panel);
    background: none;
    color: var(--text-primary);
    font: inherit;
    cursor: default;
  }
  .plate {
    display: grid;
    place-items: center;
    inline-size: calc(var(--space-4) * 4);
    block-size: calc(var(--space-4) * 4);
    margin-bottom: var(--space-1);
    border-radius: var(--radius-panel);
    background: var(--surface-raised);
    box-shadow: var(--shadow-control);
    transition:
      translate var(--duration-base) var(--ease-standard),
      box-shadow var(--duration-base) var(--ease-standard);
  }
  .tile:hover .plate {
    translate: 0 calc(var(--space-1) * -0.5);
    box-shadow: var(--shadow-popover);
  }
  .tile:active .plate {
    translate: 0 0;
    box-shadow: var(--shadow-control);
  }
  .tile:focus-visible {
    outline: var(--focus-ring-width) solid var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
  .label {
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-medium);
  }
  .hint {
    font-family: var(--font-mono);
    font-size: var(--font-size-xs);
    color: var(--text-tertiary);
  }
</style>
