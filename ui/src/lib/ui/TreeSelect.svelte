<!-- Pop-up button for choosing one item of a hierarchy (folders). Unlike the native <select>,
     the list can draw real indentation and icons, so the tree reads like the sidebar. The
     button shows the selected item with its ancestors as a breadcrumb.

     Accessibility: the button is a combobox that owns a listbox; focus stays on the button and
     `aria-activedescendant` tracks the highlighted option. Arrows, Home/End, Enter/Space, Escape
     and Tab work as in a native pop-up; typing a letter jumps to the next matching item. -->
<script lang="ts">
  import type { HTMLButtonAttributes } from "svelte/elements";

  import type { IconName } from "../design/icons/registry";
  import FieldShell from "./FieldShell.svelte";
  import { describedBy } from "./field";
  import Icon from "./Icon.svelte";

  export interface TreeSelectOption {
    value: string;
    label: string;
    /** Nesting level, 0 for top-level items. */
    depth?: number;
    /** Icon shown before the label; `null` leaves the slot empty. Defaults to a folder. */
    icon?: IconName | null;
    /** Shown greyed out, cannot be chosen (it keeps the tree readable). */
    disabled?: boolean;
  }

  type Props = Omit<HTMLButtonAttributes, "value" | "class" | "children" | "type" | "role"> & {
    value?: string;
    options: TreeSelectOption[];
    label?: string;
    hint?: string;
    error?: string | null;
    optionalLabel?: string;
    size?: "compact" | "regular" | "large";
    /** Called after the user picks a different item. */
    onchange?: (value: string) => void;
  };

  let {
    value = $bindable(""),
    options,
    label,
    hint,
    error = null,
    optionalLabel,
    size = "regular",
    onchange,
    id,
    disabled = false,
    ...rest
  }: Props = $props();

  const uid = $props.id();
  const controlId = $derived(id ?? `tree-select-${uid}`);
  const listId = $derived(`${controlId}-list`);

  let open = $state(false);
  let active = $state(0);
  let button = $state<HTMLButtonElement | null>(null);
  let list = $state<HTMLUListElement | null>(null);
  let placement = $state("");
  let typed = "";
  let typedAt = 0;

  const selectedIndex = $derived(Math.max(0, options.findIndex((option) => option.value === value)));
  const selected = $derived(options[selectedIndex]);

  /** Ancestors of the selected option, outermost first, derived from the flat ordered list. */
  const trail = $derived.by(() => {
    const out: string[] = [];
    let depth = selected?.depth ?? 0;
    for (let i = selectedIndex - 1; i >= 0 && depth > 0; i -= 1) {
      const candidate = options[i];
      if ((candidate.depth ?? 0) < depth) {
        out.unshift(candidate.label);
        depth = candidate.depth ?? 0;
      }
    }
    return out;
  });

  const optionId = (index: number): string => `${listId}-${index}`;

  function place(): void {
    if (!button) return;
    const rect = button.getBoundingClientRect();
    const gap = 4;
    const margin = 12;
    const below = window.innerHeight - rect.bottom - margin;
    const above = rect.top - margin;
    const openUp = below < 160 && above > below;
    const room = Math.max(120, Math.floor(openUp ? above : below) - gap);
    const vertical = openUp
      ? `bottom:${Math.round(window.innerHeight - rect.top + gap)}px`
      : `top:${Math.round(rect.bottom + gap)}px`;
    placement = `left:${Math.round(rect.left)}px;${vertical};min-width:${Math.round(rect.width)}px;max-height:${room}px`;
  }

  function show(): void {
    if (disabled || open) return;
    active = selectedIndex;
    place();
    open = true;
  }

  function hide(): void {
    open = false;
  }

  function choose(index: number): void {
    const option = options[index];
    if (!option || option.disabled) return;
    hide();
    if (option.value !== value) {
      value = option.value;
      onchange?.(option.value);
    }
  }

  /** Moves the highlight to `to`, or to the nearest enabled option in the direction of travel. */
  function move(to: number): void {
    const step = to >= active ? 1 : -1;
    let index = Math.min(options.length - 1, Math.max(0, to));
    while (options[index]?.disabled && index + step >= 0 && index + step < options.length) index += step;
    if (!options[index]?.disabled) active = index;
  }

  function typeAhead(key: string): void {
    const now = Date.now();
    typed = now - typedAt > 700 ? key : typed + key;
    typedAt = now;
    const needle = typed.toLowerCase();
    const from = open ? active : selectedIndex;
    for (let step = 1; step <= options.length; step += 1) {
      const index = (from + step) % options.length;
      if (!options[index].disabled && options[index].label.toLowerCase().startsWith(needle)) {
        if (open) active = index;
        else choose(index);
        return;
      }
    }
  }

  function onkeydown(event: KeyboardEvent): void {
    if (event.metaKey || event.ctrlKey || event.altKey) return;
    const key = event.key;
    if (!open) {
      if (key === "ArrowDown" || key === "ArrowUp" || key === "Enter" || key === " ") {
        event.preventDefault();
        show();
      } else if (key.length === 1) {
        typeAhead(key);
      }
      return;
    }
    if (key === "Escape") {
      // Close only the list: the surrounding dialog must stay open.
      event.preventDefault();
      event.stopPropagation();
      hide();
    } else if (key === "ArrowDown") {
      event.preventDefault();
      move(active + 1);
    } else if (key === "ArrowUp") {
      event.preventDefault();
      move(active - 1);
    } else if (key === "Home") {
      event.preventDefault();
      move(0);
    } else if (key === "End") {
      event.preventDefault();
      move(options.length - 1);
    } else if (key === "Enter" || key === " ") {
      event.preventDefault();
      choose(active);
    } else if (key === "Tab") {
      hide();
    } else if (key.length === 1) {
      typeAhead(key);
    }
  }

  /** Renders the list at the end of <body>, so no dialog can clip it. */
  function portal(node: HTMLElement): { destroy: () => void } {
    document.body.appendChild(node);
    return { destroy: () => node.remove() };
  }

  $effect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent): void => {
      const target = event.target as Node | null;
      if (target && (button?.contains(target) || list?.contains(target))) return;
      hide();
    };
    const onScroll = (event: Event): void => {
      if (list && event.target instanceof Node && list.contains(event.target)) return;
      hide();
    };
    // Focus moving elsewhere (a dialog that opens over this one) closes the list too.
    const onFocusIn = (event: FocusEvent): void => {
      const target = event.target as Node | null;
      if (target && (button?.contains(target) || list?.contains(target))) return;
      hide();
    };
    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("focusin", onFocusIn);
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", hide);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("focusin", onFocusIn);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", hide);
    };
  });

  $effect(() => {
    if (!open || !list) return;
    const node = list.querySelector<HTMLElement>(`[data-index="${active}"]`);
    node?.scrollIntoView?.({ block: "nearest" });
  });
</script>

<FieldShell id={controlId} {label} {hint} {error} {optionalLabel}>
  <button
    {...rest}
    bind:this={button}
    id={controlId}
    type="button"
    role="combobox"
    class="control {size}"
    class:invalid={!!error}
    class:open
    {disabled}
    data-value={value}
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-controls={listId}
    aria-activedescendant={open ? optionId(active) : undefined}
    aria-invalid={error ? "true" : undefined}
    aria-describedby={describedBy(controlId, hint, error)}
    onclick={() => (open ? hide() : show())}
    {onkeydown}
  >
    <span class="value">
      {#if selected?.icon !== null}
        <span class="icon"><Icon name={selected?.icon ?? "folder"} size={16} /></span>
      {/if}
      <span class="text">
        {#each trail as ancestor, i (i)}
          <span class="ancestor">{ancestor}</span><span class="sep" aria-hidden="true">›</span>
        {/each}
        <span class="current">{selected?.label ?? ""}</span>
      </span>
    </span>
    <span class="chevron"><Icon name="chevrons-up-down" size={14} /></span>
  </button>
</FieldShell>

{#if open}
  <ul
    bind:this={list}
    use:portal
    id={listId}
    class="list"
    role="listbox"
    aria-labelledby={controlId}
    style={placement}
    onpointerdown={(event) => event.preventDefault()}
  >
    {#each options as option, index (option.value)}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <li
        id={optionId(index)}
        class="option"
        class:active={index === active && !option.disabled}
        class:disabled={option.disabled}
        role="option"
        aria-selected={option.value === value}
        aria-disabled={option.disabled ? "true" : undefined}
        data-index={index}
        data-value={option.value}
        style="--depth: {option.depth ?? 0}"
        onpointermove={() => {
          if (!option.disabled) active = index;
        }}
        onclick={() => choose(index)}
      >
        <span class="icon">
          {#if option.icon !== null}<Icon name={option.icon ?? "folder"} size={16} />{/if}
        </span>
        <span class="name">{option.label}</span>
        <span class="check">
          {#if option.value === value}<Icon name="check" size={14} />{/if}
        </span>
      </li>
    {/each}
  </ul>
{/if}

<style>
  .control {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    min-width: 0;
    height: var(--control-h-regular);
    padding: 0 var(--space-2);
    background: var(--surface-control);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
    box-shadow: var(--shadow-control);
    color: var(--text-primary);
    font: inherit;
    text-align: start;
    cursor: default;
    transition:
      background var(--duration-fast) var(--ease-standard),
      border-color var(--duration-fast) var(--ease-standard);
  }
  .control.compact {
    height: var(--control-h-compact);
    font-size: var(--font-size-sm);
  }
  .control.large {
    height: var(--control-h-large);
  }
  .control:hover:not(:disabled) {
    background: var(--surface-control-hover);
  }
  .control:focus-visible,
  .control.open {
    outline: none;
    border-color: var(--focus-ring);
    box-shadow: 0 0 0 var(--focus-ring-width) var(--focus-ring);
  }
  .control.invalid {
    border-color: var(--danger-text);
  }
  .control:disabled {
    background: var(--surface-inset);
    border-color: transparent;
    box-shadow: none;
    color: var(--text-disabled);
  }
  .value {
    display: flex;
    flex: 1;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }
  .icon {
    display: inline-flex;
    flex: none;
    width: var(--icon-size-md);
    color: var(--text-secondary);
  }
  .text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ancestor,
  .sep {
    color: var(--text-tertiary);
  }
  .sep {
    margin: 0 var(--space-1);
  }
  .chevron {
    display: inline-flex;
    flex: none;
    color: var(--text-secondary);
  }
  .control:disabled .chevron,
  .control:disabled .icon {
    color: var(--text-disabled);
  }

  .list {
    position: fixed;
    z-index: var(--z-menu);
    display: flex;
    flex-direction: column;
    gap: 1px;
    margin: 0;
    padding: var(--space-1);
    overflow-y: auto;
    list-style: none;
    background: var(--surface-raised);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-panel);
    box-shadow: var(--shadow-popover);
    color: var(--text-primary);
    font-size: var(--font-size-md);
    user-select: none;
  }
  .option {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--space-2);
    min-height: var(--control-h-regular);
    padding-block: 0;
    padding-inline: calc(var(--space-2) + var(--depth) * var(--space-5)) var(--space-2);
    border-radius: var(--radius-control);
    cursor: default;
    white-space: nowrap;
  }
  .option .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .option .check {
    display: inline-flex;
    flex: none;
    width: var(--icon-size-sm);
    color: var(--accent-text);
  }
  .option.active {
    background: var(--selection-solid-bg);
    color: var(--selection-solid-fg);
  }
  .option.active .icon,
  .option.active .check {
    color: inherit;
  }
  .option.disabled,
  .option.disabled .icon {
    color: var(--text-disabled);
  }
</style>
