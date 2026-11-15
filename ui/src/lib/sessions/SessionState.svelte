<!-- Shared "session state" panel for terminal, framebuffer and web views: connecting, prompts,
     ended, cancelled. It covers the session area, so every non-session state looks the same.

     - `backdrop`: opaque content surface with the chart decoration (the session has nothing to
       show). Without it the panel floats over a translucent scrim above the frozen session.
     - `card`: the content sits in a raised panel (prompts); otherwise it is centred plain text.
     - `tone="danger"`: security warning styling (changed host key or certificate).
     - `role`: "status" for passive states, "alert" for failures, "alertdialog" for prompts
       (named by the title, described by the body).
     - `focusSafe`: focuses the element marked `data-autofocus` inside the panel when it appears.
     Pass `data-testid` and other attributes through. -->
<script lang="ts">
  import { onMount, type Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";

  import type { IconName } from "../design/icons/registry";
  import ChartBackdrop from "../ui/ChartBackdrop.svelte";
  import Icon from "../ui/Icon.svelte";
  import Spinner from "../ui/Spinner.svelte";

  type Props = Omit<HTMLAttributes<HTMLDivElement>, "children" | "class" | "title" | "role"> & {
    icon?: IconName;
    /** Shows a spinner instead of the icon. */
    busy?: boolean;
    title: string;
    text?: string;
    tone?: "neutral" | "danger";
    role?: "status" | "alert" | "alertdialog";
    backdrop?: boolean;
    card?: boolean;
    focusSafe?: boolean;
    /** Wider card, for panels with two fingerprints. */
    wide?: boolean;
    children?: Snippet;
    actions?: Snippet;
  };

  let {
    icon,
    busy = false,
    title,
    text,
    tone = "neutral",
    role = "status",
    backdrop = true,
    card = false,
    focusSafe = false,
    wide = false,
    children,
    actions,
    ...rest
  }: Props = $props();

  const uid = $props.id();
  const titleId = `ss-title-${uid}`;
  const textId = `ss-text-${uid}`;
  const extraId = `ss-extra-${uid}`;
  let panel: HTMLDivElement | undefined = $state();

  onMount(() => {
    if (focusSafe) panel?.querySelector<HTMLElement>("[data-autofocus]")?.focus();
  });
</script>

<div {...rest} class="state" class:scrim={!backdrop}>
  {#if backdrop}<ChartBackdrop />{/if}
  <div
    bind:this={panel}
    class="panel"
    class:card
    class:wide
    class:danger={tone === "danger"}
    {role}
    aria-labelledby={titleId}
    aria-describedby={text ? textId : children ? extraId : undefined}
  >
    {#if busy || icon}
      <div class="glyph" class:tile={card}>
        {#if busy}<Spinner size={20} />{:else if icon}<Icon name={icon} size={20} />{/if}
      </div>
    {/if}
    <div class="body">
      <h2 id={titleId} class="title">{title}</h2>
      {#if text}<p id={textId} class="text">{text}</p>{/if}
      {#if children}<div id={extraId} class="extra">{@render children()}</div>{/if}
    </div>
    {#if actions}<div class="actions">{@render actions()}</div>{/if}
  </div>
</div>

<style>
  .state {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-6);
    overflow: auto;
    background: var(--surface-content);
  }
  .state.scrim {
    background: var(--scrim);
  }

  .panel {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-3);
    max-width: 100%;
    margin: auto;
    text-align: center;
    color: var(--text-secondary);
  }
  .panel.card {
    align-items: stretch;
    width: var(--dialog-width-md);
    padding: var(--space-6);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-panel);
    background: var(--surface-raised);
    box-shadow: var(--shadow-popover);
    text-align: left;
  }
  .panel.card.wide {
    width: var(--dialog-width-lg);
  }
  .panel.card.danger {
    border: var(--focus-ring-width) solid var(--danger-solid);
    box-shadow: var(--shadow-dialog);
  }

  .glyph {
    display: inline-flex;
    color: var(--text-tertiary);
  }
  .glyph.tile {
    align-items: center;
    justify-content: center;
    width: var(--control-h-large);
    height: var(--control-h-large);
    border-radius: var(--radius-control);
    background: var(--surface-inset);
    color: var(--text-secondary);
  }
  .danger .glyph.tile {
    background: var(--danger-bg);
    color: var(--danger-text);
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }
  .card .body {
    gap: var(--space-3);
  }
  .title {
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
    line-height: var(--line-height-tight);
    color: var(--text-primary);
  }
  .danger .title {
    color: var(--danger-text);
  }
  .text {
    max-width: calc(var(--tooltip-max-width) * 2);
    font-size: var(--font-size-md);
    line-height: var(--line-height-relaxed);
  }
  .card .text {
    max-width: none;
  }
  .extra {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
    align-items: inherit;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    margin-top: var(--space-1);
  }
  .card .actions {
    justify-content: flex-end;
    margin-top: var(--space-2);
  }
</style>
