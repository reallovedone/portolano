<!-- Context menu with keyboard support (arrows, Home/End, Enter, Escape). While open it marks
     the app as "overlay open". -->
<script lang="ts">
  import { onMount } from "svelte";

  import { getApp } from "../state/app";
  import Icon from "./Icon.svelte";
  import type { MenuItem } from "./menu";

  let {
    x,
    y,
    items,
    onclose,
    label,
  }: { x: number; y: number; items: MenuItem[]; onclose: () => void; label: string } = $props();

  const app = getApp();
  let root: HTMLUListElement | undefined = $state();

  function buttons(): HTMLButtonElement[] {
    return root ? [...root.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")] : [];
  }

  onMount(() => {
    const release = app.overlay.acquire();
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    buttons()[0]?.focus();
    // Keep the menu inside the viewport.
    if (root) {
      const rect = root.getBoundingClientRect();
      root.style.left = `${Math.max(4, Math.min(x, window.innerWidth - rect.width - 4))}px`;
      root.style.top = `${Math.max(4, Math.min(y, window.innerHeight - rect.height - 4))}px`;
    }
    return () => {
      release();
      if (previous?.isConnected) previous.focus();
    };
  });

  function onkeydown(event: KeyboardEvent): void {
    const list = buttons();
    const index = list.indexOf(document.activeElement as HTMLButtonElement);
    switch (event.key) {
      case "Escape":
        event.preventDefault();
        event.stopPropagation();
        onclose();
        break;
      case "ArrowDown":
        event.preventDefault();
        list[(index + 1) % list.length]?.focus();
        break;
      case "ArrowUp":
        event.preventDefault();
        list[(index - 1 + list.length) % list.length]?.focus();
        break;
      case "Home":
        event.preventDefault();
        list[0]?.focus();
        break;
      case "End":
        event.preventDefault();
        list[list.length - 1]?.focus();
        break;
      case "Tab":
        event.preventDefault();
        onclose();
        break;
    }
  }

  function run(item: MenuItem): void {
    onclose();
    item.action();
  }
</script>

<svelte:window onpointerdown={(e) => {
  if (root && !root.contains(e.target as Node)) onclose();
}} onblur={onclose} />

<ul bind:this={root} class="menu" role="menu" aria-label={label} style:left="{x}px" style:top="{y}px" {onkeydown}>
  {#each items as item (item.label)}
    {#if item.separatorBefore}<li role="separator"></li>{/if}
    <li role="none">
      <button
        type="button"
        role="menuitem"
        class:danger={item.danger}
        disabled={item.disabled}
        onclick={() => run(item)}
      >
        <span class="icon-slot">{#if item.icon}<Icon name={item.icon} size={14} />{/if}</span>
        <span class="label">{item.label}</span>
        {#if item.shortcut}<span class="shortcut" aria-hidden="true">{item.shortcut}</span>{/if}
      </button>
    </li>
  {/each}
</ul>

<style>
  .menu {
    position: fixed;
    z-index: var(--z-menu);
    min-width: var(--menu-min-width);
    margin: 0;
    padding: var(--space-1);
    list-style: none;
    background: var(--surface-raised);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-panel);
    box-shadow: var(--shadow-popover);
    animation: appear var(--duration-fast) var(--ease-out);
  }

  li[role="separator"] {
    height: 1px;
    margin: var(--space-1) var(--space-2);
    background: var(--border-hairline);
  }

  button {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    height: var(--control-h-regular);
    padding: 0 var(--space-2);
    text-align: left;
    background: transparent;
    border: 0;
    border-radius: var(--radius-control);
    color: var(--text-primary);
    cursor: default;
  }

  .icon-slot {
    display: inline-flex;
    flex: none;
    width: var(--icon-size-sm);
    color: var(--text-secondary);
  }

  .label {
    flex: 1;
    white-space: nowrap;
  }

  .shortcut {
    padding-left: var(--space-4);
    color: var(--text-tertiary);
    font-size: var(--font-size-sm);
  }

  /* macOS menus highlight with the solid accent and white text. */
  button:hover:not(:disabled),
  button:focus-visible {
    background: var(--selection-solid-bg);
    color: var(--selection-solid-fg);
    outline: none;
  }

  button:hover:not(:disabled) .icon-slot,
  button:focus-visible .icon-slot,
  button:hover:not(:disabled) .shortcut,
  button:focus-visible .shortcut {
    color: inherit;
  }

  button.danger {
    color: var(--danger-text);
  }

  button.danger:hover:not(:disabled),
  button.danger:focus-visible {
    background: var(--danger-solid);
    color: var(--text-on-accent);
  }

  button:disabled {
    color: var(--text-disabled);
  }

  @keyframes appear {
    from {
      opacity: 0;
      transform: scale(0.98);
    }
  }
</style>
