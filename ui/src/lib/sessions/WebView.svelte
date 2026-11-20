<!-- Web session: a placeholder element (the address bar and page controls are in the session
     toolbar, which this view feeds through `publishToolbar`). In the real app the page itself is a
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
  import type { SessionToolbarModel, WebAddressFields } from "../tabs/toolbar.svelte";
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
  // The address bar lives in the session toolbar; these fields are shared with it.
  // svelte-ignore state_referenced_locally
  const fields: WebAddressFields = $state({
    address: initialAddress(),
    error: null,
    loading: false,
    editing: false,
  });

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
      fields.loading = change.phase === "started";
      if (!fields.editing) fields.address = change.url;
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
      fields.error = t("web.navigateFailed", { message: toIpcError(e).message });
    }
  }

  function submit(): void {
    const result = normalizeUrl(fields.address);
    if (!result.ok) {
      fields.error = t("web.invalidUrl");
      return;
    }
    fields.error = null;
    fields.address = result.url;
    void navigate({ type: "url", url: result.url });
  }

  const toolbarModel: SessionToolbarModel = {
    target: null,
    resolution: null,
    security: null,
    connection: null,
    ctrlAltDel: null,
    web: {
      fields,
      back: () => void navigate({ type: "back" }),
      forward: () => void navigate({ type: "forward" }),
      reload: () => void navigate({ type: "reload" }),
      submit,
    },
    // The page is a native view: it takes the keyboard when clicked, nothing to give back.
    focus: () => undefined,
  };
  onMount(() => session.publishToolbar(toolbarModel));
</script>

<div class="view" data-session={id}>
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

  .page {
    position: relative;
    flex: 1;
    min-height: 0;
    background: var(--surface-content);
  }
</style>
