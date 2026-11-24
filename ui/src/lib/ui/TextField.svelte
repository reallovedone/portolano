<!-- Single-line text field with label, hint, error, prefix/suffix and (for passwords) a reveal
     toggle. Extra attributes (name, autocomplete, data-autofocus, onkeydown, ...) go to <input>.
     `bind:input` exposes the element; search fields hide the native cancel button. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLInputAttributes } from "svelte/elements";

  import { t } from "../i18n/strings";
  import FieldShell from "./FieldShell.svelte";
  import { describedBy } from "./field";
  import IconButton from "./IconButton.svelte";

  type Props = Omit<HTMLInputAttributes, "size" | "value" | "class" | "prefix"> & {
    value?: string | number | null;
    label?: string;
    hint?: string;
    error?: string | null;
    optionalLabel?: string;
    size?: "compact" | "regular" | "large";
    /** Text or snippet shown inside the field before the input (icon, "https://"). */
    prefix?: Snippet | string;
    /** Text or snippet shown inside the field after the input (unit, button). */
    suffix?: Snippet | string;
    /** Accessible names of the reveal toggle of password fields. */
    showLabel?: string;
    hideLabel?: string;
    /** Forces the focus look (gallery only). */
    forceFocus?: boolean;
    /** Monospace text, for host names, ports, paths and other technical values. */
    monospace?: boolean;
    /** The inner <input> element (bindable), for callers that move focus into the field. */
    input?: HTMLInputElement;
  };

  let {
    value = $bindable(""),
    label,
    hint,
    error = null,
    optionalLabel,
    size = "regular",
    prefix,
    suffix,
    showLabel = t("common.show"),
    hideLabel = t("common.hide"),
    forceFocus = false,
    monospace = false,
    input = $bindable(),
    type = "text",
    id,
    disabled = false,
    ...rest
  }: Props = $props();

  const uid = $props.id();
  const inputId = $derived(id ?? `field-${uid}`);
  let revealed = $state(false);
  const isPassword = $derived(type === "password");
  const effectiveType = $derived(isPassword && revealed ? "text" : type);
</script>

<FieldShell id={inputId} {label} {hint} {error} {optionalLabel}>
  <div class="control {size}" class:invalid={!!error} class:disabled class:force-focus={forceFocus}>
    {#if prefix}
      <span class="affix">
        {#if typeof prefix === "string"}{prefix}{:else}{@render prefix()}{/if}
      </span>
    {/if}
    <input
      {...rest}
      bind:this={input}
      class:monospace
      id={inputId}
      type={effectiveType}
      bind:value
      {disabled}
      aria-invalid={error ? "true" : undefined}
      aria-describedby={describedBy(inputId, hint, error)}
    />
    {#if suffix}
      <span class="affix end">
        {#if typeof suffix === "string"}{suffix}{:else}{@render suffix()}{/if}
      </span>
    {/if}
    {#if isPassword}
      <span class="affix end">
        <IconButton
          icon={revealed ? "eye-off" : "eye"}
          label={revealed ? hideLabel : showLabel}
          size="compact"
          pressed={revealed}
          tooltip=""
          {disabled}
          onclick={() => (revealed = !revealed)}
        />
      </span>
    {/if}
  </div>
</FieldShell>

<style>
  .control {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    min-width: 0;
    padding: 0 var(--space-2);
    background: var(--surface-field);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
    transition:
      border-color var(--duration-fast) var(--ease-standard),
      box-shadow var(--duration-fast) var(--ease-standard);
  }
  .compact {
    height: var(--control-h-compact);
    font-size: var(--font-size-sm);
  }
  .regular {
    height: var(--control-h-regular);
  }
  .large {
    height: var(--control-h-large);
  }
  .control:focus-within,
  .control.force-focus {
    border-color: var(--focus-ring);
    box-shadow: 0 0 0 var(--focus-ring-width) var(--focus-ring);
  }
  .control.invalid {
    border-color: var(--danger-text);
  }
  .control.invalid:focus-within {
    box-shadow: 0 0 0 var(--focus-ring-width) var(--danger-text);
  }
  .control.disabled {
    background: var(--surface-inset);
    border-color: transparent;
  }
  input {
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0;
    background: transparent;
    border: 0;
    outline: none;
    box-shadow: none;
    color: var(--text-primary);
  }
  input.monospace {
    font-family: var(--font-mono);
    font-size: var(--font-size-sm);
  }
  /* The field draws its own clear button where one is needed; hide the native one. */
  input[type="search"] {
    appearance: none;
    -webkit-appearance: none;
  }
  input[type="search"]::-webkit-search-cancel-button {
    display: none;
  }
  input::placeholder {
    color: var(--text-tertiary);
  }
  input:disabled {
    color: var(--text-disabled);
  }
  .affix {
    display: inline-flex;
    align-items: center;
    flex: none;
    color: var(--text-secondary);
  }
  /* Buttons at the end of the field are part of it: full height and flush with its right edge,
     like a segment of the control, instead of a smaller button floating inside it. */
  .control:has(> .end:last-child :global(button)) {
    padding-right: 0;
  }
  .end {
    align-self: stretch;
    align-items: stretch;
  }
  .end :global(:is(.btn-ui, .icon-btn)) {
    height: auto;
    border: 0;
    border-radius: 0;
    box-shadow: none;
  }
  .end :global(.icon-btn) {
    width: auto;
    aspect-ratio: 1;
  }
  .end :global(.btn-ui) {
    padding: 0 var(--space-3);
    border-left: 1px solid var(--border-strong);
  }
  .end:last-child :global(:is(.btn-ui, .icon-btn)) {
    border-top-right-radius: calc(var(--radius-control) - 1px);
    border-bottom-right-radius: calc(var(--radius-control) - 1px);
  }
  .control.disabled .end :global(.btn-ui) {
    border-left-color: var(--border-hairline);
  }
</style>
