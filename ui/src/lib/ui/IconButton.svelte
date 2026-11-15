<!-- Icon-only button. `label` is required: it is the accessible name and the tooltip text. -->
<script lang="ts">
  import type { HTMLButtonAttributes } from "svelte/elements";

  import type { IconName } from "../design/icons/registry";
  import Icon from "./Icon.svelte";
  import Tooltip from "./Tooltip.svelte";

  type Props = Omit<HTMLButtonAttributes, "children" | "class" | "aria-label"> & {
    icon: IconName;
    label: string;
    size?: "compact" | "regular" | "large";
    /** "plain" has no chrome until hovered; "filled" always shows the control surface. */
    variant?: "plain" | "filled";
    /** Toggle state (adds aria-pressed). */
    pressed?: boolean;
    /** Tooltip text; defaults to the label. Pass an empty string to disable the tooltip. */
    tooltip?: string;
    forceHover?: boolean;
    forceFocus?: boolean;
  };

  let {
    icon,
    label,
    size = "regular",
    variant = "plain",
    pressed,
    tooltip,
    forceHover = false,
    forceFocus = false,
    type = "button",
    ...rest
  }: Props = $props();

  const tip = $derived(tooltip ?? label);
</script>

{#snippet button()}
  <button
    {...rest}
    {type}
    class="icon-btn {variant} {size}"
    class:force-hover={forceHover}
    class:force-focus={forceFocus}
    class:on={pressed}
    aria-label={label}
    aria-pressed={pressed === undefined ? undefined : pressed}
  >
    <Icon name={icon} size={size === "large" ? 20 : 16} />
  </button>
{/snippet}

{#if tip}
  <Tooltip text={tip}>{@render button()}</Tooltip>
{:else}
  {@render button()}
{/if}

<style>
  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--text-secondary);
    cursor: default;
    transition:
      background var(--duration-fast) var(--ease-standard),
      color var(--duration-fast) var(--ease-standard);
  }
  .compact {
    width: var(--control-h-compact);
    height: var(--control-h-compact);
  }
  .regular {
    width: var(--control-h-regular);
    height: var(--control-h-regular);
  }
  .large {
    width: var(--control-h-large);
    height: var(--control-h-large);
  }
  .filled {
    background: var(--surface-control);
    border-color: var(--border-strong);
    box-shadow: var(--shadow-control);
  }
  .icon-btn:hover:not(:disabled),
  .icon-btn.force-hover {
    background: var(--surface-hover);
    color: var(--text-primary);
  }
  .filled:hover:not(:disabled),
  .filled.force-hover {
    background: var(--surface-control-hover);
  }
  .icon-btn:active:not(:disabled) {
    background: var(--surface-pressed);
  }
  .icon-btn.on {
    background: var(--surface-pressed);
    color: var(--text-primary);
  }
  .force-focus {
    outline: var(--focus-ring-width) solid var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
  .icon-btn:disabled {
    color: var(--text-disabled);
    box-shadow: none;
  }
</style>
