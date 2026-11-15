<!-- Button. One primary action per screen; the rest are secondary, plain or destructive.
     `loading` keeps the button focusable (aria-busy, aria-disabled) but ignores activation. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLButtonAttributes } from "svelte/elements";

  import Spinner from "./Spinner.svelte";

  type Props = Omit<HTMLButtonAttributes, "children" | "class"> & {
    variant?: "primary" | "secondary" | "plain" | "destructive";
    size?: "compact" | "regular" | "large";
    loading?: boolean;
    /** Leading icon (an <Icon> usually). Replaced by a spinner while loading. */
    icon?: Snippet;
    /** Forces the hover look (gallery only). */
    forceHover?: boolean;
    /** Forces the keyboard-focus ring (gallery only). */
    forceFocus?: boolean;
    /** Stretches to the full width of the container. */
    block?: boolean;
    children?: Snippet;
  };

  let {
    variant = "secondary",
    size = "regular",
    loading = false,
    icon,
    forceHover = false,
    forceFocus = false,
    block = false,
    type = "button",
    disabled = false,
    onclick,
    children,
    ...rest
  }: Props = $props();

  function click(event: MouseEvent & { currentTarget: EventTarget & HTMLButtonElement }): void {
    if (loading) {
      event.preventDefault();
      return;
    }
    // Svelte types `onclick` as a single handler; arrays are not used in this code base.
    if (typeof onclick === "function") onclick(event);
  }
</script>

<button
  {...rest}
  {type}
  {disabled}
  class="btn-ui {variant} {size}"
  class:force-hover={forceHover}
  class:force-focus={forceFocus}
  class:loading
  class:block
  aria-busy={loading ? "true" : undefined}
  aria-disabled={loading ? "true" : undefined}
  onclick={click}
>
  {#if loading}
    <Spinner size={14} />
  {:else if icon}
    <span class="slot">{@render icon()}</span>
  {/if}
  {#if children}<span class="label">{@render children()}</span>{/if}
</button>

<style>
  .btn-ui {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-1);
    min-width: 0;
    padding: 0 var(--space-3);
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-regular);
    line-height: 1;
    white-space: nowrap;
    cursor: default;
    transition:
      background var(--duration-fast) var(--ease-standard),
      border-color var(--duration-fast) var(--ease-standard);
  }

  .compact {
    height: var(--control-h-compact);
    padding: 0 var(--space-2);
    font-size: var(--font-size-sm);
  }
  .regular {
    height: var(--control-h-regular);
  }
  .large {
    height: var(--control-h-large);
    padding: 0 var(--space-4);
  }

  .block {
    display: flex;
    width: 100%;
  }

  .slot {
    display: inline-flex;
  }

  /* secondary: white (dark: graphite) raised control with a hairline and a whisper of shadow */
  .secondary {
    background: var(--surface-control);
    border-color: var(--border-strong);
    box-shadow: var(--shadow-control);
    color: var(--text-primary);
  }
  .secondary:hover:not(:disabled):not(.loading),
  .secondary.force-hover {
    background: var(--surface-control-hover);
  }
  .secondary:active:not(:disabled):not(.loading) {
    background: var(--surface-control-pressed);
  }

  /* primary: the only place (with selection and focus) that carries the accent */
  .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--text-on-accent);
    font-weight: var(--font-weight-medium);
  }
  .primary:hover:not(:disabled):not(.loading),
  .primary.force-hover {
    background: var(--accent-hover);
    border-color: var(--accent-hover);
  }
  .primary:active:not(:disabled):not(.loading) {
    background: var(--accent-pressed);
    border-color: var(--accent-pressed);
  }

  /* plain: text only, no chrome until hovered */
  .plain {
    background: transparent;
    color: var(--text-primary);
  }
  .plain:hover:not(:disabled):not(.loading),
  .plain.force-hover {
    background: var(--surface-hover);
  }
  .plain:active:not(:disabled):not(.loading) {
    background: var(--surface-pressed);
  }

  /* destructive: solid red, reserved for confirming irreversible actions */
  .destructive {
    background: var(--danger-solid);
    border-color: var(--danger-solid);
    color: var(--text-on-accent);
    font-weight: var(--font-weight-medium);
  }
  .destructive:hover:not(:disabled):not(.loading),
  .destructive.force-hover {
    background: var(--danger-solid-hover);
    border-color: var(--danger-solid-hover);
  }
  .destructive:active:not(:disabled):not(.loading) {
    background: var(--danger-solid-pressed);
    border-color: var(--danger-solid-pressed);
  }

  .btn-ui:disabled {
    background: var(--surface-inset);
    border-color: transparent;
    box-shadow: none;
    color: var(--text-disabled);
  }
  .plain:disabled {
    background: transparent;
  }

  .btn-ui:focus-visible,
  .force-focus {
    outline: var(--focus-ring-width) solid var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
</style>
