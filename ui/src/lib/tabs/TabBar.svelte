<script lang="ts">
  import { tick } from "svelte";

  import { t } from "../i18n/strings";
  import { detectMac, shortcutLabel } from "../shortcuts";
  import { getApp } from "../state/app";
  import { defaultParentFor } from "../tree/model";
  import ContextMenu from "../ui/ContextMenu.svelte";
  import IconButton from "../ui/IconButton.svelte";
  import KindIcon from "../ui/KindIcon.svelte";
  import type { MenuItem } from "../ui/menu";
  import Spinner from "../ui/Spinner.svelte";
  import { scrollEdges, wheelToScroll, type ScrollEdges } from "./overflow";

  const { tabs, ui, doc } = getApp();
  const isMac = detectMac();

  let menu = $state<{ x: number; y: number; tabId: string } | null>(null);
  let dragIndex = $state<number | null>(null);
  let overIndex = $state<number | null>(null);
  let bar: HTMLDivElement | undefined = $state();
  let edges = $state<ScrollEdges>({ start: false, end: false });

  function updateEdges(): void {
    if (bar) edges = scrollEdges(bar.scrollLeft, bar.clientWidth, bar.scrollWidth);
  }

  // Many tabs: they shrink to a minimum width, then the strip scrolls with a fade at the edges.
  $effect(() => {
    if (!bar) return;
    updateEdges();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(updateEdges);
    observer.observe(bar);
    return () => observer.disconnect();
  });

  // Tab count and titles change the content width.
  $effect(() => {
    void tabs.tabs.length;
    void tick().then(updateEdges);
  });

  // Keep the active tab visible when it changes (new tab, keyboard cycling).
  $effect(() => {
    const id = tabs.state.activeId;
    if (!id || !bar) return;
    void tick().then(() => {
      const el = bar?.querySelector<HTMLElement>(`[data-tab-id="${CSS.escape(id)}"]`);
      // Not implemented in every test DOM.
      el?.scrollIntoView?.({ inline: "nearest", block: "nearest" });
      updateEdges();
    });
  });

  function onWheel(event: WheelEvent): void {
    const delta = wheelToScroll(event.deltaX, event.deltaY);
    if (delta === null || !bar) return;
    event.preventDefault();
    bar.scrollLeft += delta;
  }

  function newConnection(): void {
    ui.open({
      type: "connection",
      id: null,
      parentId: defaultParentFor(doc.nodes, ui.selectedId),
    });
  }

  async function focusTab(id: string): Promise<void> {
    await tick();
    bar?.querySelector<HTMLElement>(`[data-tab-id="${CSS.escape(id)}"]`)?.focus();
  }

  function tabMenu(tabId: string): MenuItem[] {
    const index = tabs.tabs.findIndex((x) => x.id === tabId);
    return [
      {
        label: t("tabs.close"),
        action: () => void tabs.close(tabId),
        icon: "close",
        shortcut: shortcutLabel("closeTab", isMac),
      },
      {
        label: t("tabs.closeOthers"),
        action: () => void tabs.closeOthers(tabId),
        disabled: tabs.tabs.length < 2,
        icon: "x-circle",
      },
      { label: t("tabs.closeAll"), action: () => void tabs.closeAll(), icon: "x-circle" },
      {
        label: t("tabs.moveLeft"),
        action: () => tabs.move(index, index - 1),
        disabled: index <= 0,
        separatorBefore: true,
        icon: "arrow-left",
      },
      {
        label: t("tabs.moveRight"),
        action: () => tabs.move(index, index + 1),
        disabled: index < 0 || index >= tabs.tabs.length - 1,
        icon: "arrow-right",
      },
    ];
  }

  function onKeydown(event: KeyboardEvent, id: string, index: number): void {
    if (event.ctrlKey || event.metaKey) return;
    const last = tabs.tabs.length - 1;
    switch (event.key) {
      case "ArrowLeft":
      case "ArrowRight": {
        event.preventDefault();
        const step = event.key === "ArrowLeft" ? -1 : 1;
        if (event.altKey) {
          tabs.move(index, index + step);
          void focusTab(id);
        } else {
          const target = tabs.tabs[Math.max(0, Math.min(last, index + step))];
          if (target) {
            tabs.activate(target.id);
            void focusTab(target.id);
          }
        }
        break;
      }
      case "Home":
      case "End": {
        event.preventDefault();
        const target = tabs.tabs[event.key === "Home" ? 0 : last];
        if (target) {
          tabs.activate(target.id);
          void focusTab(target.id);
        }
        break;
      }
      case "Delete":
        event.preventDefault();
        void tabs.close(id);
        break;
      case "ContextMenu": {
        event.preventDefault();
        const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
        menu = { x: rect.left, y: rect.bottom, tabId: id };
        break;
      }
    }
  }

  function onDrop(event: DragEvent, to: number): void {
    event.preventDefault();
    if (dragIndex !== null) tabs.move(dragIndex, to);
    dragIndex = null;
    overIndex = null;
  }
</script>

<!-- The strip doubles as the title bar: its own background (and the filler after the tabs) is a
     window drag handle; tabs and buttons are separate elements and stay clickable. -->
<div class="strip" data-tauri-drag-region data-testid="tabstrip">
  <div
    bind:this={bar}
    class="tabs"
    class:fade-start={edges.start}
    class:fade-end={edges.end}
    role="tablist"
    aria-label={t("tabs.label")}
    onscroll={updateEdges}
    onwheel={onWheel}
  >
    {#each tabs.tabs as tab, index (tab.id)}
      {@const active = tab.id === tabs.state.activeId}
      <div
        class="tab"
        class:active
        class:over={overIndex === index && dragIndex !== index}
        class:dragging={dragIndex === index}
        role="presentation"
        draggable="true"
        ondragstart={(e) => {
          dragIndex = index;
          if (e.dataTransfer) {
            e.dataTransfer.effectAllowed = "move";
            e.dataTransfer.setData("text/plain", tab.id);
          }
        }}
        ondragover={(e) => {
          if (dragIndex !== null) {
            e.preventDefault();
            overIndex = index;
          }
        }}
        ondragend={() => {
          dragIndex = null;
          overIndex = null;
        }}
        ondrop={(e) => onDrop(e, index)}
        onauxclick={(e) => {
          if (e.button === 1) {
            e.preventDefault();
            void tabs.close(tab.id);
          }
        }}
        oncontextmenu={(e) => {
          e.preventDefault();
          menu = { x: e.clientX, y: e.clientY, tabId: tab.id };
        }}
      >
        <button
          type="button"
          role="tab"
          class="label"
          aria-selected={active}
          aria-controls="tabpanel-{tab.id}"
          id="tab-{tab.id}"
          data-tab-id={tab.id}
          data-status={tab.status}
          tabindex={active ? 0 : -1}
          onclick={() => tabs.activate(tab.id)}
          onkeydown={(e) => onKeydown(e, tab.id, index)}
        >
          {#if tab.status === "opening"}
            <Spinner size={14} />
          {:else}
            <KindIcon kind={tab.kind} />
          {/if}
          <span class="title">{tabs.title(tab)}</span>
          {#if tab.status === "failed"}<span class="flag" title={tab.error?.code}>!</span>{/if}
        </button>
        <span class="close">
          <IconButton
            icon="close"
            size="compact"
            tooltip=""
            tabindex={-1}
            label={`${t("tabs.close")}: ${tabs.title(tab)}`}
            onclick={() => void tabs.close(tab.id)}
          />
        </span>
      </div>
    {/each}
  </div>
  <span class="plus">
    <IconButton icon="plus" size="compact" label={t("tree.newConnection")} onclick={newConnection} />
  </span>
  <div class="filler" data-tauri-drag-region></div>
</div>

{#if menu}
  <ContextMenu
    x={menu.x}
    y={menu.y}
    label={t("tabs.label")}
    items={tabMenu(menu.tabId)}
    onclose={() => (menu = null)}
  />
{/if}

<style>
  .strip {
    position: relative;
    display: flex;
    flex: none;
    align-items: flex-end;
    gap: var(--space-1);
    min-height: max(var(--titlebar-height), calc(var(--control-h-large) + var(--space-1)));
    padding: 0 var(--space-2);
    background: var(--surface-window);
  }

  /* Hairline under the strip; the active tab paints over it so it joins the content. */
  .strip::after {
    content: "";
    position: absolute;
    inset: auto 0 0;
    height: 1px;
    background: var(--border-hairline);
  }

  .tabs {
    position: relative;
    z-index: var(--z-sticky);
    display: flex;
    flex: 0 1 auto;
    align-items: flex-end;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }

  .tabs::-webkit-scrollbar {
    display: none;
  }

  .tabs.fade-start {
    mask-image: linear-gradient(to right, transparent, black var(--space-6));
  }

  .tabs.fade-end {
    mask-image: linear-gradient(to left, transparent, black var(--space-6));
  }

  .tabs.fade-start.fade-end {
    mask-image: linear-gradient(
      to right,
      transparent,
      black var(--space-6),
      black calc(100% - var(--space-6)),
      transparent
    );
  }

  .tab {
    position: relative;
    display: flex;
    /* Natural width (icon + title + close) up to the maximum; the tab only shrinks, down to the
       minimum, once the strip really overflows. A fixed flex-basis would be wider than the
       content-sized container and make every tab shrink even with plenty of free space. */
    flex: 0 1 auto;
    align-items: center;
    min-width: var(--tab-min-width);
    max-width: var(--tab-max-width);
    height: var(--control-h-large);
    border-radius: var(--radius-control) var(--radius-control) 0 0;
    color: var(--text-secondary);
    transition: background var(--duration-fast) var(--ease-standard);
  }

  .tab:hover:not(.active) {
    background: var(--surface-hover);
  }

  .tab.active {
    background: var(--surface-content);
    color: var(--text-primary);
  }

  .tab.over {
    box-shadow: inset 2px 0 0 var(--accent);
  }

  .tab.dragging {
    opacity: 0.5;
  }

  .label {
    display: flex;
    flex: 1;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    height: 100%;
    padding: 0 0 0 var(--space-3);
    background: none;
    border: 0;
    border-radius: var(--radius-control) 0 0 0;
    color: inherit;
    font-weight: var(--font-weight-regular);
    text-align: start;
  }

  .tab.active .label {
    font-weight: var(--font-weight-medium);
  }

  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .flag {
    color: var(--danger-text);
    font-weight: var(--font-weight-semibold);
  }

  /* The close button keeps its space so tab widths do not jump; it shows on hover, on the
     active tab and while focused. */
  .close {
    display: inline-flex;
    flex: none;
    margin-inline: 0 var(--space-1);
    opacity: 0;
    transition: opacity var(--duration-fast) var(--ease-standard);
  }

  .tab:hover .close,
  .tab.active .close,
  .tab:focus-within .close {
    opacity: 1;
  }

  .plus {
    display: inline-flex;
    flex: none;
    margin-bottom: var(--space-1);
  }

  .filler {
    flex: 1;
    align-self: stretch;
    min-width: var(--space-4);
  }
</style>
