<!-- Row of a list or tree: the shared building block (hover, selected, keyboard focus, leading
     icon, trailing accessory). It is presentational: pass role, tabindex, aria-* and handlers
     through, so the same row works inside a tree, a listbox or a plain list.
     `depth` indents by one step per level. `dropTarget` and `dragging` show the state of a drag and
     drop; `comfortable` density is the roomy row of the connection tree. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";

  type Props = Omit<HTMLAttributes<HTMLDivElement>, "children" | "class"> & {
    selected?: boolean;
    /** Indentation level (0 = none). */
    depth?: number;
    density?: "compact" | "regular" | "comfortable";
    /** A drag is hovering over this row and may drop here. */
    dropTarget?: boolean;
    /** Insertion line above or below the row: a drag would drop before or after it. */
    dropEdge?: "before" | "after" | null;
    /** This row is the one being dragged. */
    dragging?: boolean;
    /** Force the hover / keyboard-focus look (gallery only). */
    forceHover?: boolean;
    forceFocus?: boolean;
    disabled?: boolean;
    leading?: Snippet;
    trailing?: Snippet;
    /** Second, quieter line under the main label. */
    secondary?: Snippet;
    children: Snippet;
  };

  let {
    selected = false,
    depth = 0,
    density = "regular",
    dropTarget = false,
    dragging = false,
    dropEdge = null,
    forceHover = false,
    forceFocus = false,
    disabled = false,
    leading,
    trailing,
    secondary,
    children,
    ...rest
  }: Props = $props();
</script>

<div
  {...rest}
  class="row {density}"
  class:selected
  class:force-hover={forceHover}
  class:force-focus={forceFocus}
  class:disabled
  class:drop-target={dropTarget}
  class:dragging
  class:edge-before={dropEdge === "before"}
  class:edge-after={dropEdge === "after"}
  style:--depth={depth}
>
  {#if leading}<span class="leading">{@render leading()}</span>{/if}
  <span class="main">
    <span class="label">{@render children()}</span>
    {#if secondary}<span class="secondary">{@render secondary()}</span>{/if}
  </span>
  {#if trailing}<span class="trailing">{@render trailing()}</span>{/if}
</div>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-height: var(--row-h-regular);
    padding: 0 var(--space-2);
    padding-left: calc(var(--space-2) + var(--depth, 0) * var(--row-indent));
    border-radius: var(--radius-control);
    color: var(--text-primary);
    cursor: default;
  }
  .compact {
    min-height: var(--row-h-compact);
  }
  .comfortable {
    min-height: var(--row-h-comfortable);
  }
  /* Compact density: name and second line (host) must fit the shorter row without touching the
     rows around them, so their lines get the tight line height. */
  :global(:root[data-density="compact"]) .comfortable .main {
    line-height: var(--line-height-tight);
  }
  .row:hover:not(.disabled),
  .force-hover {
    background: var(--surface-hover);
  }
  .row.selected {
    background: var(--selection-bg);
  }
  .row.drop-target,
  .row.drop-target:hover:not(.disabled) {
    background: var(--accent-subtle);
    outline: var(--focus-ring-width) solid var(--accent);
    outline-offset: calc(-1 * var(--focus-ring-width));
  }
  .row.edge-before {
    box-shadow: inset 0 var(--focus-ring-width) 0 var(--accent);
  }
  .row.edge-after {
    box-shadow: inset 0 calc(-1 * var(--focus-ring-width)) 0 var(--accent);
  }
  @media (forced-colors: active) {
    .row.edge-before {
      border-top: var(--focus-ring-width) solid Highlight;
    }
    .row.edge-after {
      border-bottom: var(--focus-ring-width) solid Highlight;
    }
  }
  .row.dragging {
    opacity: 0.5;
  }
  .row:focus-visible,
  .force-focus {
    outline: var(--focus-ring-width) solid var(--focus-ring);
    outline-offset: calc(-1 * var(--focus-ring-width));
  }
  .row.disabled {
    color: var(--text-disabled);
  }
  .leading,
  .trailing {
    display: inline-flex;
    flex: none;
    align-items: center;
  }
  .trailing {
    margin-left: auto;
    color: var(--text-secondary);
  }
  .main {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .secondary {
    overflow: hidden;
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
