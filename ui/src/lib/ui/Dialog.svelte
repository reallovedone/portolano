<!-- Modal dialog: focus trap, Escape to close, focus restored on close, and it marks the
     app as "overlay open" while mounted (child webviews must hide). -->
<script lang="ts">
  import { onMount, type Snippet } from "svelte";

  import { t } from "../i18n/strings";
  import { getApp } from "../state/app";
  import IconButton from "./IconButton.svelte";

  let {
    title,
    onclose,
    children,
    footer,
    hero,
    width,
    size = "md",
  }: {
    title: string;
    onclose: () => void;
    children: Snippet;
    footer?: Snippet;
    /** Decorative full-bleed strip between the title row and the body (never holds controls). */
    hero?: Snippet;
    /** Explicit width in pixels; overrides `size`. */
    width?: number;
    /** Preset width from the --dialog-width-* tokens: sm, md, lg. */
    size?: "sm" | "md" | "lg";
  } = $props();

  const app = getApp();
  const uid = $props.id();
  const titleId = `dlg-${uid}`;
  let root: HTMLDivElement | undefined = $state();

  const FOCUSABLE =
    'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

  function focusables(): HTMLElement[] {
    if (!root) return [];
    return [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
      (el) => el.getAttribute("aria-hidden") !== "true",
    );
  }

  onMount(() => {
    const release = app.overlay.acquire();
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const preferred = root?.querySelector<HTMLElement>("[data-autofocus]");
    (preferred ?? focusables()[0] ?? root)?.focus();
    return () => {
      release();
      if (previous?.isConnected) previous.focus();
    };
  });

  function onkeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.stopPropagation();
      event.preventDefault();
      onclose();
      return;
    }
    if (event.key !== "Tab") return;
    const items = focusables();
    if (items.length === 0) {
      event.preventDefault();
      return;
    }
    const first = items[0];
    const last = items[items.length - 1];
    if (!first || !last) return;
    if (event.shiftKey && (document.activeElement === first || document.activeElement === root)) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }
</script>

<div class="backdrop" role="presentation">
  <div
    bind:this={root}
    class="dialog {size}"
    role="dialog"
    aria-modal="true"
    aria-labelledby={titleId}
    tabindex="-1"
    style:width={width !== undefined ? `min(${width}px, calc(100vw - var(--space-8)))` : undefined}
    {onkeydown}
  >
    <header>
      <h2 id={titleId}>{title}</h2>
      <IconButton icon="close" label={t("common.close")} size="compact" tooltip="" onclick={onclose} />
    </header>
    {#if hero}<div class="hero">{@render hero()}</div>{/if}
    <div class="body">{@render children()}</div>
    {#if footer}
      <footer>{@render footer()}</footer>
    {/if}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: var(--z-overlay);
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding-top: 8vh;
    background: var(--scrim);
    animation: fade var(--duration-base) var(--ease-standard);
  }

  .dialog {
    display: flex;
    flex-direction: column;
    max-height: 84vh;
    background: var(--surface-raised);
    color: var(--text-primary);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-panel);
    box-shadow: var(--shadow-dialog);
    outline: none;
    animation: pop var(--duration-slow) var(--ease-out);
  }

  .sm {
    width: min(var(--dialog-width-sm), calc(100vw - var(--space-8)));
  }

  .md {
    width: min(var(--dialog-width-md), calc(100vw - var(--space-8)));
  }

  .lg {
    width: min(var(--dialog-width-lg), calc(100vw - var(--space-8)));
  }

  /* Full-bleed: spans the dialog edge to edge, with a hairline below. */
  .hero {
    position: relative;
    flex: none;
    overflow: hidden;
    border-bottom: 1px solid var(--border-hairline);
    background: var(--surface-window);
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-4) var(--space-4) var(--space-1) var(--space-5);
  }

  h2 {
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
    line-height: var(--line-height-tight);
  }

  .body {
    padding: var(--space-3) var(--space-5);
    overflow: auto;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-5) var(--space-4);
  }

  @keyframes fade {
    from {
      opacity: 0;
    }
  }

  @keyframes pop {
    from {
      opacity: 0;
      transform: scale(0.98) translateY(calc(-1 * var(--space-1)));
    }
  }
</style>
