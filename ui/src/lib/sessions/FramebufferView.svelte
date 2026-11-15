<!-- Canvas view of a framebuffer session (RDP, VNC). Protocol-agnostic: it only knows `FbEvent`
     and the `fb_*` wrappers. All logic that does not need the DOM lives in `fb/controller.ts`.

     Keyboard: while the canvas has focus every key is sent to the remote machine and kept
     from the shell and the browser. Ctrl+Alt+Esc releases the keyboard (the canvas loses
     focus and held keys are released). -->
<script lang="ts">
  import { onMount } from "svelte";

  import { toIpcError, type FbEvent, type SessionId } from "../ipc";
  import { t } from "../i18n/strings";
  import { getApp } from "../state/app";
  import { getSessionContext } from "../tabs/session";
  import Banner from "../ui/Banner.svelte";
  import Button from "../ui/Button.svelte";
  import Icon from "../ui/Icon.svelte";
  import { FbController, type FbViewState } from "./fb/controller";
  import { fitToArea } from "./fb/geometry";
  import CredentialPromptDialog from "./CredentialPromptDialog.svelte";
  import FingerprintBlock from "./FingerprintBlock.svelte";
  import SessionState from "./SessionState.svelte";
  import { targetLabel } from "./target";

  /** How long the keyboard hint stays visible after it appears. */
  const CHIP_VISIBLE_MS = 4000;

  let { sessionId = null }: { sessionId?: SessionId | null } = $props();

  const session = getSessionContext();
  const app = getApp();
  const { ipc } = app;
  // svelte-ignore state_referenced_locally
  const id: SessionId = sessionId ?? session.sessionId;

  const FB_EVENTS = new Set([
    "resize",
    "rect",
    "cursor",
    "certPrompt",
    "closed",
    "error",
    "credentialPrompt",
    "security",
    "unencryptedPrompt",
  ]);

  let area: HTMLDivElement | undefined = $state();
  let canvas: HTMLCanvasElement | undefined = $state();
  let controller: FbController | undefined = $state.raw();
  let view = $state<FbViewState>({
    phase: "connecting",
    transport: "unknown",
    remote: { w: 0, h: 0 },
    cert: null,
    unencryptedPrompt: false,
    downgrade: null,
    credential: null,
    closedReason: null,
    cursor: "default",
  });
  const connection = $derived.by(() => {
    const node = app.doc.node(session.connectionId);
    return node && node.type === "connection" ? node : null;
  });
  let areaSize = $state({ w: 0, h: 0 });
  let focused = $state(false);
  let chipShown = $state(true);
  /** The "not encrypted" notice was dismissed (it shows once per session view). */
  let clearNoticeDismissed = $state(false);
  let chipTimer: ReturnType<typeof setTimeout> | undefined;
  const target = $derived(targetLabel(connection));

  const box = $derived(fitToArea(view.remote, areaSize));

  onMount(() => {
    if (!canvas || !area) return;
    const target = canvas;
    const ctx = target.getContext("2d", { alpha: false });
    if (!ctx) {
      session.fail({ code: "internal", message: "The canvas could not be live." });
      return;
    }
    const logFailure = (what: string, error: unknown): void => {
      // Messages of IPC errors never carry secrets.
      ipc.uiLog("warn", `${what} failed: ${toIpcError(error).message}`).catch(() => undefined);
    };
    const live: FbController = new FbController({
      api: {
        pointer: (x, y, buttons, wheel) => ipc.fbPointer(id, x, y, buttons, wheel),
        key: (code, down) => ipc.fbKey(id, code, down),
        resize: (w, h) => ipc.fbResize(id, w, h),
        setVisible: (visible) => ipc.fbSetVisible(id, visible),
        certDecision: (accept) => ipc.fbCertDecision(id, accept),
        unencryptedDecision: (allow) => ipc.fbUnencryptedDecision(id, allow),
        forgetCertificate: () => ipc.fbForgetCertificate(id),
        credential: (answer, save) => ipc.sessionCredential(id, answer, save),
        credentialCancel: () => ipc.sessionCredentialCancel(id),
      },
      surface: ctx,
      setCanvasSize: (w, h) => {
        target.width = w;
        target.height = h;
      },
      onState: (state) => {
        view = state;
      },
      onFatal: (error) => session.fail(error),
      onError: logFailure,
      // The backend saved the typed credential: the connection now points at it.
      onCredentialSaved: () => {
        void app.doc.reload();
        void app.vault.refresh();
      },
      geometry: () => {
        const remote = live.state.remote;
        return remote.w > 0 ? { rect: target.getBoundingClientRect(), remote } : null;
      },
      onEscape: () => target.blur(),
      onReconnect: () => session.retry(),
    });
    controller = live;

    const unsubscribe = session.bridge.subscribe((event) => {
      if (FB_EVENTS.has(event.type)) live.handleEvent(event as FbEvent);
    });

    const observer = new ResizeObserver((entries) => {
      const rect = entries[0]?.contentRect;
      if (rect) areaSize = { w: Math.floor(rect.width), h: Math.floor(rect.height) };
    });
    observer.observe(area);

    // Wheel listeners must be non-passive to stop the page from scrolling.
    const onWheel = (e: WheelEvent): void => {
      e.preventDefault();
      live.input.wheel(e);
    };
    target.addEventListener("wheel", onWheel, { passive: false });
    // Svelte delegates `onkeydown` to the root, after the event has already bubbled past every
    // element in between: keys must be stopped on the canvas itself to stay away from the shell.
    const onKeyDown = (e: KeyboardEvent): void => live.input.keyDown(e);
    const onKeyUp = (e: KeyboardEvent): void => live.input.keyUp(e);
    target.addEventListener("keydown", onKeyDown);
    target.addEventListener("keyup", onKeyUp);

    return () => {
      target.removeEventListener("wheel", onWheel);
      target.removeEventListener("keydown", onKeyDown);
      target.removeEventListener("keyup", onKeyUp);
      observer.disconnect();
      unsubscribe();
      live.dispose();
      controller = undefined;
    };
  });

  // Hidden tabs and open overlays stop the stream; showing again redraws the whole desktop.
  $effect(() => {
    controller?.setVisible(session.shouldShow);
  });

  // The remote desktop follows the size of the tab area (the backend ignores it when the
  // connection has a fixed size or the server cannot resize; the view scales instead).
  $effect(() => {
    if (session.shouldShow) controller?.requestResize(areaSize.w, areaSize.h);
  });

  // The keyboard hint shows when the stream goes live and whenever capture changes, then fades.
  $effect(() => {
    if (view.phase !== "live") return;
    void focused;
    chipShown = true;
    clearTimeout(chipTimer);
    chipTimer = setTimeout(() => {
      chipShown = false;
    }, CHIP_VISIBLE_MS);
    return () => clearTimeout(chipTimer);
  });
</script>

<svelte:window onblur={() => controller?.input.releaseAll()} />

<div class="view" bind:this={area} data-session={sessionId}>
  <div
    class="stage"
    class:waiting={view.remote.w === 0}
    style:width="{box.width}px"
    style:height="{box.height}px"
  >
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <canvas
      bind:this={canvas}
      class="canvas"
      class:closed={view.phase === "closed"}
      tabindex="0"
      aria-label={t("fb.canvasLabel")}
      style:cursor={view.cursor}
      onfocus={() => (focused = true)}
      onblur={() => {
        focused = false;
        controller?.input.releaseAll();
      }}
      onpointerdown={(e) => {
        canvas?.focus();
        canvas?.setPointerCapture?.(e.pointerId);
        e.preventDefault();
        controller?.input.pointerButton(e);
      }}
      onpointerup={(e) => {
        controller?.input.pointerButton(e);
      }}
      onpointermove={(e) => controller?.input.pointerMove(e)}
      onpointercancel={() => controller?.input.releaseAll()}
      oncontextmenu={(e) => e.preventDefault()}
    ></canvas>
  </div>

  {#if view.phase === "live"}
    <div class="chip-wrap">
      <p class="chip" class:faded={!chipShown} class:active={focused} role="status">
        <Icon name="keyboard" size={14} />
        <span>{focused ? t("fb.keyboardCaptured") : t("fb.keyboardFree")}</span>
        {#if view.transport === "encrypted"}
          <span class="lock" title={t("fb.encrypted")} data-testid="fb-encrypted">
            <Icon name="lock" size={14} label={t("fb.encrypted")} />
          </span>
        {/if}
      </p>
    </div>
    {#if view.transport === "clear" && !clearNoticeDismissed}
      <div class="notice-wrap">
        <Banner
          variant="warning"
          role="status"
          title={t("fb.unencryptedTitle")}
          ondismiss={() => (clearNoticeDismissed = true)}
        >
          {t("fb.unencryptedText")}
        </Banner>
      </div>
    {/if}
  {/if}

  {#if view.credential && session.visible}
    <CredentialPromptDialog
      prompt={view.credential}
      connectionName={connection?.name ?? null}
      onsubmit={(answer, save) => controller?.submitCredential(answer, save)}
      oncancel={() => controller?.cancelCredential()}
    />
  {/if}

  {#if view.phase === "cancelled"}
    <SessionState
      data-testid="fb-cancelled"
      icon="x-circle"
      title={t("prompt.cancelledTitle")}
      text={t("prompt.cancelledText")}
      backdrop={view.remote.w === 0}
    >
      {#snippet actions()}
        <Button variant="primary" onclick={() => session.retry()}>{t("prompt.retry")}</Button>
        <Button onclick={() => void app.tabs.close(session.tabId)}>{t("prompt.closeTab")}</Button>
      {/snippet}
    </SessionState>
  {:else if view.downgrade !== null}
    <SessionState
      card
      wide
      focusSafe
      tone="danger"
      role="alertdialog"
      icon="shield-alert"
      title={t("fb.downgrade.title")}
      backdrop={view.remote.w === 0}
    >
      <Banner variant="danger" role="note">{view.downgrade}</Banner>
      <p class="hint">{t("fb.downgrade.hint")}</p>
      {#if target}<FingerprintBlock label={t("fb.cert.server")} value={target} />{/if}
      {#snippet actions()}
        <span class="apart">
          <Button variant="plain" onclick={() => controller?.forgetCertificateAndReconnect()}>
            {#snippet icon()}<Icon name="trash" />{/snippet}
            {t("fb.cert.removeAndReconnect")}
          </Button>
        </span>
        <Button variant="primary" data-autofocus onclick={() => void app.tabs.close(session.tabId)}>
          {t("fb.cert.closeTab")}
        </Button>
      {/snippet}
    </SessionState>
  {:else if view.unencryptedPrompt}
    <SessionState
      card
      focusSafe
      tone="danger"
      role="alertdialog"
      icon="shield-alert"
      title={t("fb.plain.title")}
      text={t("fb.plain.intro")}
      backdrop={view.remote.w === 0}
    >
      {#if target}<FingerprintBlock label={t("fb.cert.server")} value={target} />{/if}
      <p class="hint">{t("fb.plain.remember")}</p>
      {#snippet actions()}
        <span class="apart">
          <Button variant="plain" onclick={() => controller?.decideUnencrypted(true)}>
            {t("fb.plain.allow")}
          </Button>
        </span>
        <Button variant="primary" data-autofocus onclick={() => controller?.decideUnencrypted(false)}>
          {t("fb.plain.cancel")}
        </Button>
      {/snippet}
    </SessionState>
  {:else if view.cert}
    {#if view.cert.changed}
      <SessionState
        card
        wide
        focusSafe
        tone="danger"
        role="alertdialog"
        icon="shield-alert"
        title={t("fb.cert.changedTitle")}
        backdrop={view.remote.w === 0}
      >
        <Banner variant="danger" role="note">{t("fb.cert.changedWarning")}</Banner>
        {#if target}<FingerprintBlock label={t("fb.cert.server")} value={target} />{/if}
        {#if view.cert.stored}
          <FingerprintBlock label={t("fb.cert.stored")} value={view.cert.stored} />
        {/if}
        <FingerprintBlock label={t("fb.cert.presented")} value={view.cert.fingerprint} />
        {#snippet actions()}
          <span class="apart">
            <Button variant="plain" onclick={() => controller?.forgetCertificateAndReconnect()}>
              {#snippet icon()}<Icon name="trash" />{/snippet}
              {t("fb.cert.removeAndReconnect")}
            </Button>
          </span>
          <Button variant="primary" data-autofocus onclick={() => void app.tabs.close(session.tabId)}>
            {t("fb.cert.closeTab")}
          </Button>
        {/snippet}
      </SessionState>
    {:else}
      <SessionState
        card
        focusSafe
        role="alertdialog"
        icon="shield"
        title={t("fb.cert.title")}
        text={t("fb.cert.intro")}
        backdrop={view.remote.w === 0}
      >
        {#if target}<FingerprintBlock label={t("fb.cert.server")} value={target} />{/if}
        <FingerprintBlock label={t("fb.cert.fingerprint")} value={view.cert.fingerprint} />
        {#snippet actions()}
          <Button data-autofocus onclick={() => controller?.decideCert(false)}>
            {t("fb.cert.cancel")}
          </Button>
          <Button variant="primary" onclick={() => controller?.decideCert(true)}>
            {t("fb.cert.trust")}
          </Button>
        {/snippet}
      </SessionState>
    {/if}
  {:else if view.credential}
    <SessionState icon="key" title={t("session.waitingTitle")} text={t("session.waitingText")}>
      {#if target}<FingerprintBlock value={target} />{/if}
    </SessionState>
  {:else if view.phase === "connecting"}
    <SessionState icon="monitor" busy title={t("fb.connecting")} text={t("fb.connectingText")}>
      {#if target}<FingerprintBlock value={target} />{/if}
    </SessionState>
  {/if}

  {#if view.phase === "closed"}
    <SessionState
      role="alert"
      icon="unplug"
      title={t("fb.closedTitle")}
      text={t("fb.closedText")}
      backdrop={view.remote.w === 0}
    >
      {#if view.closedReason}<FingerprintBlock value={t("fb.closedReason", { reason: view.closedReason })} />{/if}
      {#snippet actions()}
        <Button variant="primary" onclick={() => session.retry()}>{t("fb.reconnect")}</Button>
      {/snippet}
    </SessionState>
  {/if}
</div>

<style>
  /* Letterbox: a dark neutral around the scaled desktop in both themes, so the remote screen
     stands out. */
  .view {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
    height: 100%;
    overflow: hidden;
    background: var(--letterbox-bg);
  }

  .stage {
    flex: none;
    box-shadow: var(--shadow-popover);
  }

  .stage.waiting {
    visibility: hidden;
  }

  .canvas {
    display: block;
    width: 100%;
    height: 100%;
    outline: none;
    touch-action: none;
  }

  .canvas.closed {
    filter: grayscale(1) brightness(0.6);
  }

  .chip-wrap {
    position: absolute;
    inset: auto 0 var(--space-4);
    display: flex;
    justify-content: center;
    padding: 0 var(--space-4);
    pointer-events: none;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    max-width: 100%;
    margin: 0;
    padding: var(--space-1) var(--space-3);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-full);
    background: var(--surface-raised);
    box-shadow: var(--shadow-popover);
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
    transition: opacity var(--duration-slow) var(--ease-standard);
  }

  .lock {
    display: inline-flex;
    color: var(--text-secondary);
  }

  /* Non-blocking: sits at the top, never over the desktop's input. */
  .notice-wrap {
    position: absolute;
    inset: var(--space-4) 0 auto;
    display: flex;
    justify-content: center;
    padding: 0 var(--space-4);
    pointer-events: none;
  }

  .notice-wrap :global(.banner) {
    max-width: 36rem;
    pointer-events: auto;
    box-shadow: var(--shadow-popover);
  }

  .chip.active {
    color: var(--text-primary);
  }

  .chip.faded {
    opacity: 0;
  }

  .hint {
    margin: 0;
    font-size: var(--font-size-md);
    line-height: var(--line-height-relaxed);
  }

  /* The least prominent action sits apart on its own line, before the safe choices. */
  .apart {
    display: flex;
    flex: 1 0 100%;
    justify-content: flex-start;
    padding-bottom: var(--space-1);
  }
</style>
