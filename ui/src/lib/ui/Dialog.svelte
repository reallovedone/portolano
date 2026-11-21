<!-- Modal dialog: focus trap, Escape to close, focus restored on close, and it marks the
     app as "overlay open" while mounted (child webviews must hide). -->
<script lang="ts" module>
  // Dialogs can open over each other (an unlock from an editor, a confirmation from a
  // manager). The stack decides who is on top, whatever the DOM order: the newest dialog gets
  // the highest z-index, every dialog below is `inert` (no pointer, focus or keyboard) and
  // keeps its state because it stays mounted.
  interface StackEntry {
    backdrop: HTMLElement;
    root: HTMLElement;
    close: () => void;
    focusFirst: () => void;
  }

  const stack: StackEntry[] = [];

  function syncStack(): void {
    stack.forEach((entry, index) => {
      const covered = index < stack.length - 1;
      entry.backdrop.toggleAttribute("inert", covered);
      entry.backdrop.dataset.stackDepth = String(index);
      entry.backdrop.dataset.covered = covered ? "true" : "false";
      entry.backdrop.style.setProperty("z-index", `calc(var(--z-overlay) + ${index})`);
    });
  }

  // Keys pressed while focus is outside the top dialog (e.g. after clicking its scrim).
  function onDocumentKeydown(event: KeyboardEvent): void {
    const top = stack[stack.length - 1];
    if (!top || event.defaultPrevented) return;
    if (event.target instanceof globalThis.Node && top.root.contains(event.target)) return;
    if (event.key === "Escape") {
      event.preventDefault();
      top.close();
    } else if (event.key === "Tab") {
      event.preventDefault();
      top.focusFirst();
    }
  }

  function push(entry: StackEntry): void {
    if (stack.length === 0) document.addEventListener("keydown", onDocumentKeydown);
    stack.push(entry);
    syncStack();
  }

  function remove(entry: StackEntry): void {
    const index = stack.indexOf(entry);
    if (index >= 0) stack.splice(index, 1);
    syncStack();
    if (stack.length === 0) document.removeEventListener("keydown", onDocumentKeydown);
  }
</script>

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
    /** Preset width from the --dialog-width-* tokens: sm, md, lg, xl. */
    size?: "sm" | "md" | "lg" | "xl";
  } = $props();

  const app = getApp();
  const uid = $props.id();
  const titleId = `dlg-${uid}`;
  let root: HTMLDivElement | undefined = $state();
  let backdrop: HTMLDivElement | undefined = $state();

  const FOCUSABLE =
    'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

  function focusables(): HTMLElement[] {
    if (!root) return [];
    return [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
      (el) => el.getAttribute("aria-hidden") !== "true",
    );
  }

  function focusFirst(): void {
    const preferred = root?.querySelector<HTMLElement>("[data-autofocus]");
    (preferred ?? focusables()[0] ?? root)?.focus();
  }

  onMount(() => {
    const release = app.overlay.acquire();
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    let entry: StackEntry | null = null;
    if (backdrop && root) {
      entry = { backdrop, root, close: () => onclose(), focusFirst };
      push(entry);
    }
    focusFirst();
    return () => {
      release();
      if (entry) remove(entry);
      // The dialog below (if any) is no longer inert at this point: give it its focus back.
      if (previous?.isConnected && !previous.closest("[inert]")) previous.focus();
      else stack[stack.length - 1]?.focusFirst();
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

<div bind:this={backdrop} class="backdrop" role="presentation">
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

  .xl {
    width: min(var(--dialog-width-xl), calc(100vw - var(--space-8)));
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
