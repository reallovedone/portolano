<!-- Web session: a slim toolbar above a placeholder element. In the real app the page itself is a
     child webview of the main window, owned by the backend, that sits exactly over the
     placeholder: this component only keeps the two aligned (`web/sync.ts`) and sends the
     toolbar's commands. The child webview is native and always drawn ABOVE the HTML, so it is
     hidden whenever the tab is not active or a menu, dialog or unlock screen is open
     (`session.shouldShow`).

     Keyboard: while the page has focus the app's shortcuts do not fire (they are handled by
     the main webview, which does not receive the keys). Clicking the toolbar gives the focus
     back to the main webview. The browser preview (mock IPC) has no child webview, so it shows
     a labelled stand-in panel instead of a page. -->
<script lang="ts">
  import { onMount } from "svelte";

  import { t } from "../i18n/strings";
  import { ipcMode, toIpcError, type Bounds, type SessionId } from "../ipc";
  import { getApp } from "../state/app";
  import { getSessionContext } from "../tabs/session";
  import type { IconName } from "../design/icons/registry";
  import IconButton from "../ui/IconButton.svelte";
  import Spinner from "../ui/Spinner.svelte";
  import TextField from "../ui/TextField.svelte";
  import Toolbar from "../ui/Toolbar.svelte";
  import Tooltip from "../ui/Tooltip.svelte";
  import SessionState from "./SessionState.svelte";
  import { BoundsSync } from "./web/sync";
  import { noUrlEvents, tauriUrlEvents, type UrlSubscribe } from "./web/urlEvents";
  import { normalizeUrl } from "./web/url";

  let {
    sessionId = null,
    // The mock IPC has no webview: show a labelled stand-in for the page area.
    standIn = ipcMode === "mock",
    urlEvents = ipcMode === "mock" ? noUrlEvents : tauriUrlEvents,
  }: {
    sessionId?: SessionId | null;
    standIn?: boolean;
    urlEvents?: UrlSubscribe;
  } = $props();

  const session = getSessionContext();
  const app = getApp();
  const { ipc } = app;
  // svelte-ignore state_referenced_locally
  const id: SessionId = sessionId ?? session.sessionId;

  const connection = $derived.by(() => {
    const node = app.doc.node(session.connectionId);
    return node && node.type === "connection" ? node : null;
  });

  let page: HTMLDivElement | undefined = $state();
  let sync: BoundsSync | undefined = $state.raw();
  // svelte-ignore state_referenced_locally
  let address = $state(initialAddress());
  let editing = $state(false);
  let error = $state<string | null>(null);
  let loading = $state(false);

  function initialAddress(): string {
    const node = app.doc.node(session.connectionId);
    return node && node.type === "connection" && node.options.type === "web" ? node.options.url : "";
  }

  function measure(): Bounds | null {
    if (!page) return null;
    const rect = page.getBoundingClientRect();
    return { x: rect.left, y: rect.top, width: rect.width, height: rect.height };
  }

  function logFailure(what: string, e: unknown): void {
    // Messages of IPC errors never carry secrets.
    ipc.uiLog("warn", `${what} failed: ${toIpcError(e).message}`).catch(() => undefined);
  }

  onMount(() => {
    const synced = new BoundsSync({
      measure,
      shouldShow: () => session.shouldShow,
      send: (bounds, visible) => ipc.webSetBounds(id, bounds, visible),
      onError: (e) => {
        const failure = toIpcError(e);
        if (failure.code === "not_found") {
          // The backend no longer knows the session: show the error panel with its retry.
          synced.dispose();
          session.fail({ code: failure.code, message: failure.message });
        } else {
          logFailure("web_set_bounds", e);
        }
      },
    });
    sync = synced;
    synced.flushNow();

    const request = (): void => synced.request();
    const observer = new ResizeObserver(request);
    if (page) observer.observe(page);
    // The window and its body change size on resize and fullscreen; the placeholder can also
    // move without resizing (sidebar collapse next to a fixed-width area).
    observer.observe(document.documentElement);
    window.addEventListener("resize", request);
    document.addEventListener("fullscreenchange", request);

    const unsubscribe = urlEvents((change) => {
      if (change.sessionId !== id) return;
      loading = change.phase === "started";
      if (!editing) address = change.url;
    });

    return () => {
      unsubscribe();
      observer.disconnect();
      window.removeEventListener("resize", request);
      document.removeEventListener("fullscreenchange", request);
      synced.dispose();
    };
  });

  // Tab activation, overlays opening or closing: the web view follows `shouldShow`.
  $effect(() => {
    void session.shouldShow;
    sync?.update();
  });

  async function navigate(action: Parameters<typeof ipc.webNavigate>[1]): Promise<void> {
    try {
      await ipc.webNavigate(id, action);
    } catch (e) {
      error = t("web.navigateFailed", { message: toIpcError(e).message });
    }
  }

  function submit(): void {
    const result = normalizeUrl(address);
    if (!result.ok) {
      error = t("web.invalidUrl");
      return;
    }
    error = null;
    address = result.url;
    void navigate({ type: "url", url: result.url });
  }

  function onkeydown(event: KeyboardEvent): void {
    if (event.key === "Enter") {
      event.preventDefault();
      submit();
    }
  }
</script>

{#snippet control(icon: IconName, label: string, action: () => void)}
  <!-- The page is directly below: the tooltip goes above, or the web view would cover it. -->
  <Tooltip text={label} placement="top">
    {#snippet children()}
      <IconButton {icon} {label} size="compact" tooltip="" onclick={action} />
    {/snippet}
  </Tooltip>
{/snippet}

<div class="view" data-session={id}>
  <Toolbar label={t("web.toolbar")} bordered>
    {#snippet start()}
      {@render control("arrow-left", t("web.back"), () => void navigate({ type: "back" }))}
      {@render control("arrow-right", t("web.forward"), () => void navigate({ type: "forward" }))}
      {@render control("refresh", t("web.reload"), () => void navigate({ type: "reload" }))}
    {/snippet}
    <div class="address">
      <TextField
        bind:value={address}
      size="compact"
      monospace
      type="text"
      inputmode="url"
      spellcheck={false}
      autocomplete="off"
      aria-label={t("web.address")}
      error={error}
      {onkeydown}
      oninput={() => (error = null)}
      onfocus={() => (editing = true)}
      onblur={() => (editing = false)}
      />
    </div>
    {#snippet end()}
      {#if loading}<Spinner size={14} label={t("web.loading")} />{/if}
    {/snippet}
  </Toolbar>
  <div class="page" bind:this={page} data-testid="web-page">
    {#if standIn}
      <SessionState icon="globe" title={t("web.standInTitle")} text={t("web.standInText")} />
    {/if}
  </div>
</div>

<style>
  .view {
    position: relative;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    background: var(--surface-content);
  }

  .address {
    flex: 1;
    min-width: 0;
  }

  .page {
    position: relative;
    flex: 1;
    min-height: 0;
    background: var(--surface-content);
  }
</style>
