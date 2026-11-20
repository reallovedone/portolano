<!-- One slim row above a session, with a small tab hanging under it that hides and shows it:
     Reconnect, Duplicate, Close (and Ctrl+Alt+Del for remote desktops, the page controls for web)
     on the left, what is connected on the right. It reads
     what the session view published (`toolbar.svelte.ts`) and knows nothing about RDP, VNC or
     SSH. Clicks on its buttons do not take the keyboard from the session (mousedown is not
     allowed to move the focus), and the view gets the focus back after Ctrl+Alt+Del. -->
<script lang="ts">
  import type { IconName } from "../design/icons/registry";
  import { t } from "../i18n/strings";
  import { getApp } from "../state/app";
  import IconButton from "../ui/IconButton.svelte";
  import Icon from "../ui/Icon.svelte";
  import Spinner from "../ui/Spinner.svelte";
  import TextField from "../ui/TextField.svelte";
  import Toolbar from "../ui/Toolbar.svelte";
  import Tooltip from "../ui/Tooltip.svelte";
  import type { ConnectionTab } from "./model";

  /** A little longer than the slide (`--duration-slow`, 200 ms). */
  const MOVE_TIMEOUT_MS = 400;

  let { tab }: { tab: ConnectionTab } = $props();

  const { tabs, closeConfirm, toolbar } = getApp();
  const model = $derived(toolbar.model(tab.id));
  const web = $derived(model?.web ?? null);
  const remoteDesktop = $derived(tab.kind === "rdp" || tab.kind === "vnc");
  /** Hidden: a web tab still keeps its page controls and address, nothing else. */
  const full = $derived(toolbar.visible);
  /** Closed: the row is gone. A web tab keeps its row once its view is open (the page needs the
   *  address); before that it has nothing to show, like any other tab. */
  const collapsed = $derived(!toolbar.visible && !web);
  // A web page is a native view drawn over the HTML below: tooltips must open upwards.
  const placement = $derived(tab.kind === "web" ? "top" : "bottom");

  /** Pressing a button must not move the focus off the session (keys would be lost or stuck). */
  function keepFocus(event: MouseEvent): void {
    // A web page is a native view: it keeps the keyboard until the main webview takes it back, and
    // a click on the bar is what does that (the app's shortcuts work again). Leave the default.
    if (tab.kind === "web") return;
    if (event.target instanceof Element && event.target.closest("button")) event.preventDefault();
  }

  // The slide is a CSS transition on the row. With "reduce motion" it is not applied at all (the
  // browser preview and tests have no matchMedia: no motion there either). The preference is
  // followed while the app runs.
  let animated = $state(false);
  $effect(() => {
    if (typeof matchMedia !== "function") return;
    const query = matchMedia("(prefers-reduced-motion: reduce)");
    const update = (): void => {
      animated = !query.matches;
    };
    update();
    query.addEventListener?.("change", update);
    return () => query.removeEventListener?.("change", update);
  });

  /** True only while the row is changing size: the content is clipped then (and when closed). */
  let moving = $state(false);
  let movingTimer: ReturnType<typeof setTimeout> | undefined;
  let wasVisible: boolean | undefined;

  function stopMoving(): void {
    clearTimeout(movingTimer);
    moving = false;
  }

  // Only a change of the visibility itself slides the row (from the tab or the sidebar menu; every
  // dock follows). When the row appears or goes because the page controls were published or
  // withdrawn (open, reconnect), it switches at once: a slide would move the page for 200 ms. This
  // runs before the DOM update, so the transition is in place when the row changes. A timer is the
  // fallback if the browser never reports the end (inactive pane, interrupted transition).
  $effect.pre(() => {
    const now = toolbar.visible;
    const before = wasVisible;
    wasVisible = now;
    // A web row with its controls stays where it is: nothing slides.
    if (before === undefined || before === now || !animated || web) return;
    moving = true;
    clearTimeout(movingTimer);
    movingTimer = setTimeout(stopMoving, MOVE_TIMEOUT_MS);
  });
  $effect(() => () => clearTimeout(movingTimer));

  function settle(event: TransitionEvent): void {
    if (event.target === event.currentTarget) stopMoving();
  }

  function sendCtrlAltDel(): void {
    model?.ctrlAltDel?.send();
    model?.focus();
  }

  function onAddressKeydown(event: KeyboardEvent): void {
    if (event.key === "Enter") {
      event.preventDefault();
      web?.submit();
    }
  }
</script>

{#snippet control(icon: IconName, label: string, action: () => void, disabled = false)}
  <Tooltip text={label} {placement}>
    {#snippet children()}
      <IconButton {icon} {label} size="compact" tooltip="" {disabled} onclick={action} />
    {/snippet}
  </Tooltip>
{/snippet}

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="dock" class:animated data-testid="session-dock" onmousedown={keepFocus}>
  <div class="clip" class:moving class:collapsed ontransitionend={settle} ontransitioncancel={settle}>
    <div class="inner" class:moving inert={collapsed} aria-hidden={collapsed ? "true" : undefined}>
      <div class="bar" data-testid="session-toolbar">
        <Toolbar label={t(web ? "web.toolbar" : "toolbar.label")} flush>
          {#snippet start()}
            {#if full}
              {@render control("refresh", t("toolbar.reconnect"), () => void tabs.reconnect(tab.id))}
              {@render control("copy", t("toolbar.duplicate"), () => void tabs.duplicate(tab.id))}
              {@render control("close", t("toolbar.close"), () => void closeConfirm.closeTab(tab.id))}
            {/if}
            {#if full && remoteDesktop}
              {@render control("keyboard", t("toolbar.ctrlAltDel"), sendCtrlAltDel, !model?.ctrlAltDel?.enabled)}
            {/if}
            {#if web}
              {#if full}<span class="sep" aria-hidden="true"></span>{/if}
              {@render control("arrow-left", t("web.back"), () => web.back())}
              {@render control("arrow-right", t("web.forward"), () => web.forward())}
              {@render control("refresh", t("web.reload"), () => web.reload())}
            {/if}
          {/snippet}
          {#if web}
            <div class="address">
              <TextField
                bind:value={web.fields.address}
                size="compact"
                monospace
                type="text"
                inputmode="url"
                spellcheck={false}
                autocomplete="off"
                aria-label={t("web.address")}
                error={web.fields.error}
                onkeydown={onAddressKeydown}
                oninput={() => (web.fields.error = null)}
                onfocus={() => (web.fields.editing = true)}
                onblur={() => (web.fields.editing = false)}
              />
            </div>
          {/if}
          {#snippet end()}
            {#if web?.fields.loading}<Spinner size={14} label={t("web.loading")} />{/if}
            {#if model && full}
              <span class="info" data-testid="session-info">
                {#if model.connection}
                  <span class="state" class:down={model.connection === "disconnected"}>
                    {model.connection === "connected" ? t("toolbar.connected") : t("toolbar.disconnected")}
                  </span>
                {/if}
                {#if model.target}<span class="mono">{model.target}</span>{/if}
                {#if model.resolution}<span class="mono">{model.resolution}</span>{/if}
                {#if model.security}
                  {@const lock = model.security}
                  <Tooltip text={lock.text} {placement}>
                    {#snippet children()}
                      <span
                        class="lock"
                        class:warn={lock.warning}
                        role="img"
                        aria-label={lock.text}
                        data-testid="session-lock"
                      >
                        <Icon name={lock.warning ? "lock-open" : "lock"} size={14} />
                      </span>
                    {/snippet}
                  </Tooltip>
                {/if}
              </span>
            {/if}
          {/snippet}
        </Toolbar>
      </div>
    </div>
  </div>
  <!-- The handle: always there, hangs under the bar. It overlays the top of the session, except
       on a web tab, where the native page would be drawn over it: that tab reserves a strip. -->
  <div class="hang" class:reserve={tab.kind === "web"}>
    <button
      type="button"
      class="handle"
      data-testid="session-toolbar-tab"
      aria-expanded={toolbar.visible}
      aria-label={toolbar.visible ? t("toolbar.hide") : t("toolbar.show")}
      title={toolbar.visible ? t("toolbar.hide") : t("toolbar.show")}
      onclick={() => toolbar.toggle()}
    >
      <Icon name={toolbar.visible ? "chevron-up" : "chevron-down"} size={14} />
    </button>
  </div>
</div>

<style>
  .dock {
    position: relative;
    z-index: var(--z-sticky);
    flex: none;
  }

  /* The bar takes layout space: its row grows from 0 to its height (the session below resizes
     with it). The remote desktop debounces its resize request, so only the final size is sent. */
  .clip {
    display: grid;
    grid-template-rows: 1fr;
  }

  .animated .clip.moving {
    transition: grid-template-rows var(--duration-slow) var(--ease-standard);
  }

  .clip.collapsed {
    grid-template-rows: 0fr;
  }

  .inner {
    min-height: 0;
  }

  /* Clipped only while it slides (and when closed), so focus rings are never cut at rest. */
  .clip.collapsed .inner,
  .inner.moving {
    overflow: hidden;
  }

  .clip.collapsed .inner {
    visibility: hidden;
  }

  .animated .clip.moving.collapsed .inner {
    transition: visibility var(--duration-slow) step-end;
  }

  .bar {
    display: flex;
    align-items: center;
    height: var(--control-h-regular);
    padding-inline: var(--space-2);
    border-bottom: 1px solid var(--border-hairline);
    background: var(--surface-content);
  }

  .bar > :global(.toolbar) {
    flex: 1;
    min-width: 0;
  }

  .hang {
    position: relative;
    height: 0;
  }

  .hang.reserve {
    height: var(--toolbar-tab-height);
    background: var(--surface-content);
  }

  .handle {
    position: absolute;
    inset: 0 auto auto 50%;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--toolbar-tab-width);
    height: var(--toolbar-tab-height);
    padding: 0;
    border: 1px solid var(--border-hairline);
    border-top: 0;
    border-radius: 0 0 var(--radius-control) var(--radius-control);
    background: var(--surface-content);
    box-shadow: var(--shadow-control);
    color: var(--text-secondary);
    cursor: pointer;
    transform: translateX(-50%);
  }

  .handle:hover {
    background: var(--surface-control-hover);
    color: var(--accent-text);
  }

  .handle:active {
    background: var(--surface-control-pressed);
  }

  .handle:focus-visible {
    outline: var(--focus-ring-width) solid var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .sep {
    width: 1px;
    height: var(--space-4);
    margin-inline: var(--space-1);
    background: var(--border-hairline);
  }

  .address {
    flex: 1;
    min-width: 0;
  }

  .info {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    color: var(--text-secondary);
    font-size: var(--font-size-xs);
    white-space: nowrap;
  }

  .mono {
    font-family: var(--font-mono);
  }

  .state.down {
    color: var(--text-tertiary);
  }

  .lock {
    display: inline-flex;
    color: var(--text-secondary);
  }

  .lock.warn {
    color: var(--warning-text);
  }
</style>
