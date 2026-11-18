<!-- Terminal session view (local shell and SSH): xterm.js on the session bridge.
     The logic lives in `term/controller.ts` (flow control, input order, host key flow) and
     `term/keys.ts` (clipboard and shortcut policy); this component wires them to xterm. -->
<script lang="ts">
  import { onMount } from "svelte";
  import { FitAddon } from "@xterm/addon-fit";
  import { Terminal } from "@xterm/xterm";
  import "@xterm/xterm/css/xterm.css";

  import { t } from "../i18n/strings";
  import type { SessionId } from "../ipc";
  import { detectMac } from "../shortcuts";
  import { getApp } from "../state/app";
  import { getSessionContext } from "../tabs/session";
  import Button from "../ui/Button.svelte";
  import Icon from "../ui/Icon.svelte";
  import Banner from "../ui/Banner.svelte";
  import { TerminalController, type TermState } from "./term/controller";
  import CredentialPromptDialog from "./CredentialPromptDialog.svelte";
  import FingerprintBlock from "./FingerprintBlock.svelte";
  import SessionEndedBar from "./SessionEndedBar.svelte";
  import SessionState from "./SessionState.svelte";
  import KnownHosts from "./term/KnownHosts.svelte";
  import { clipboardAction, keepForTerminal } from "./term/keys";
  import { deriveTerminalMetrics, deriveTerminalTheme, watchTheme } from "./term/theme";
  import { parseMismatch, parseMismatchHost, targetLabel } from "./target";

  let { sessionId = null }: { sessionId?: SessionId | null } = $props();

  const ctx = getSessionContext();
  const app = getApp();
  const isMac = detectMac();

  let container: HTMLDivElement;
  let term: Terminal | null = null;
  let fit: FitAddon | null = null;
  let controller: TerminalController | null = null;
  let ready = $state(false);
  let showKnownHosts = $state(false);
  let ts = $state<TermState>({
    phase: "connecting",
    prompt: null,
    credential: null,
    exitCode: null,
    error: null,
  });

  const connection = $derived.by(() => {
    const node = app.doc.node(ctx.connectionId);
    return node && node.type === "connection" ? node : null;
  });
  // The hop a changed key belongs to (with jump hosts it is not the connection's own host);
  // the connection's own host is the fallback for messages that do not name one.
  const hostPort = $derived.by(() => {
    const named = ts.error ? parseMismatchHost(ts.error.message) : null;
    if (named) return named;
    return {
      host: connection?.host ?? null,
      port: connection ? (connection.port ?? 22) : null,
    };
  });
  const isSsh = $derived(ctx.kind === "ssh");
  const target = $derived(targetLabel(connection));
  const mismatch = $derived(ts.error ? parseMismatch(ts.error.message) : null);

  async function copy(text: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
    } catch (e) {
      app.notices.push("error", t("term.copyFailed", { message: e instanceof Error ? e.message : String(e) }));
    }
  }

  /** Reads the clipboard and feeds it to xterm's paste (bracketed paste and chunking still apply). */
  async function pasteFromClipboard(terminal: Terminal): Promise<void> {
    let text: string;
    try {
      text = (await app.ipc.clipboardReadText()) ?? "";
    } catch (e) {
      app.notices.push("error", t("term.pasteFailed", { message: e instanceof Error ? e.message : String(e) }));
      return;
    }
    if (text !== "") terminal.paste(text);
  }

  /** Windows/Linux right-click: copy and deselect when there is a selection, otherwise paste. */
  function onContextmenu(e: MouseEvent): void {
    if (isMac) return;
    e.preventDefault();
    if (!term) return;
    if (term.hasSelection()) {
      void copy(term.getSelection());
      term.clearSelection();
    } else {
      void pasteFromClipboard(term);
    }
  }

  /** Refits to the container and tells the backend. No-op while the pane has no size. */
  function refit(): void {
    if (!term || !fit || !container) return;
    if (container.clientWidth === 0 || container.clientHeight === 0) return;
    try {
      fit.fit();
    } catch (e) {
      // xterm throws while it is not rendered yet; the next resize retries.
      void app.ipc.uiLog("warn", `terminal fit failed: ${e instanceof Error ? e.message : String(e)}`);
      return;
    }
    controller?.resize(term.cols, term.rows);
  }

  let fitFrame = 0;
  function scheduleFit(): void {
    if (fitFrame !== 0) cancelAnimationFrame(fitFrame);
    fitFrame = requestAnimationFrame(() => {
      fitFrame = 0;
      refit();
    });
  }

  onMount(() => {
    const terminal = new Terminal({
      ...deriveTerminalMetrics(),
      cursorBlink: true,
      scrollback: 10000,
      macOptionIsMeta: false,
      theme: deriveTerminalTheme(),
    });
    const fitAddon = new FitAddon();
    terminal.loadAddon(fitAddon);
    terminal.open(container);
    term = terminal;
    fit = fitAddon;

    controller = new TerminalController({
      ipc: app.ipc,
      sessionId: ctx.sessionId,
      terminal: { write: (data, done) => terminal.write(data, done) },
      onState: (s) => {
        ts = s;
      },
      onFatal: (error) => ctx.fail(error),
      onProblem: (message) => app.notices.push("error", t("term.problem", { message })),
      // The backend saved the typed credential: the connection now points at it.
      onCredentialSaved: () => {
        void app.doc.reload();
        void app.vault.refresh();
      },
    });

    terminal.onData((data) => controller?.input(data));
    terminal.onBinary((data) => controller?.inputBinary(data));
    terminal.onResize(({ cols, rows }) => controller?.resize(cols, rows));
    terminal.attachCustomKeyEventHandler((e) => {
      const action = clipboardAction(e, isMac, terminal.hasSelection());
      if (action === null) return true;
      if (e.type === "keydown" && action === "copy") void copy(terminal.getSelection());
      // Paste happens through the native `paste` event; only the key itself must not be sent.
      return false;
    });

    const unsubscribe = ctx.bridge.subscribe((event) => controller?.handleEvent(event));
    // Light/dark switches recolour the terminal live; the cell size may change with it.
    const stopTheme = watchTheme(() => {
      terminal.options.theme = deriveTerminalTheme();
      scheduleFit();
    });
    const observer = new ResizeObserver(() => scheduleFit());
    observer.observe(container);
    refit();
    ready = true;

    return () => {
      ready = false;
      observer.disconnect();
      stopTheme();
      if (fitFrame !== 0) cancelAnimationFrame(fitFrame);
      unsubscribe();
      controller?.dispose();
      controller = null;
      terminal.dispose();
      term = null;
      fit = null;
    };
  });

  // The tab became visible: its size may have changed while hidden.
  $effect(() => {
    if (ready && ctx.visible) scheduleFit();
  });

  // Keyboard focus follows "visible and nothing on top of it".
  $effect(() => {
    if (ready && ctx.shouldShow && (ts.phase === "connecting" || ts.phase === "running")) {
      term?.focus();
    }
  });

  function onKeydown(e: KeyboardEvent): void {
    // Runs after xterm's own handler (it is on the textarea, deeper in the tree) and before
    // the shell's `window` listener: stopping here keeps Ctrl+W, Ctrl+K... for the remote.
    if (keepForTerminal(e, isMac)) e.stopPropagation();
  }

  async function removeAndReconnect(): Promise<void> {
    const { host, port } = hostPort;
    if (host === null || port === null) return;
    try {
      await app.ipc.knownHostsRemove(host, port);
    } catch (e) {
      app.notices.push("error", t("knownHosts.removeFailed", { message: e instanceof Error ? e.message : String(e) }));
      return;
    }
    ctx.retry();
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="view" data-session={sessionId ?? ctx.sessionId} onkeydown={onKeydown}>
  <div class="term" bind:this={container} oncontextmenu={onContextmenu} aria-label={t("term.label")}></div>

  {#if ts.phase === "connecting" && isSsh}
    <SessionState icon="terminal" busy title={t("term.connecting")} text={t("term.connectingText")}>
      {#if target}<FingerprintBlock value={target} />{/if}
    </SessionState>
  {/if}

  {#if ts.phase === "credentialPrompt" && ts.credential}
    <SessionState icon="key" title={t("session.waitingTitle")} text={t("session.waitingText")}>
      {#if target}<FingerprintBlock value={target} />{/if}
    </SessionState>
  {/if}

  {#if ts.phase === "credentialPrompt" && ts.credential && ctx.visible}
    <CredentialPromptDialog
      prompt={ts.credential}
      connectionName={connection?.name ?? null}
      onsubmit={(answer, save) => void controller?.submitCredential(answer, save)}
      oncancel={() => void controller?.cancelCredential()}
    />
  {/if}

  {#if ts.phase === "cancelled"}
    <SessionState
      data-testid="term-cancelled"
      icon="x-circle"
      title={t("prompt.cancelledTitle")}
      text={t("prompt.cancelledText")}
    >
      {#snippet actions()}
        <Button variant="primary" onclick={() => ctx.retry()}>{t("prompt.retry")}</Button>
        <Button onclick={() => void app.tabs.close(ctx.tabId)}>{t("prompt.closeTab")}</Button>
      {/snippet}
    </SessionState>
  {/if}

  {#if ts.phase === "hostKeyPrompt" && ts.prompt}
    <SessionState
      card
      focusSafe
      role="alertdialog"
      icon="shield"
      title={t("term.hostKeyTitle")}
      text={t("term.hostKeyIntro")}
    >
      {#if ts.prompt.host !== ""}
        <FingerprintBlock label={t("term.hostKeyHost")} value={`${ts.prompt.host}:${ts.prompt.port}`} />
        {#if target && target !== `${ts.prompt.host}:${ts.prompt.port}`}
          <p class="hint">{t("term.hostKeyJump", { target })}</p>
        {/if}
      {:else if target}
        <FingerprintBlock label={t("term.hostKeyHost")} value={target} />
      {/if}
      <FingerprintBlock label={t("term.hostKeyAlgorithm")} value={ts.prompt.algorithm} />
      <FingerprintBlock label={t("term.hostKeyFingerprint")} value={ts.prompt.fingerprint} />
      {#snippet actions()}
        <Button data-autofocus onclick={() => controller?.decideHostKey(false)}>
          {t("term.hostKeyReject")}
        </Button>
        <Button variant="primary" onclick={() => controller?.decideHostKey(true)}>
          {t("term.hostKeyAccept")}
        </Button>
      {/snippet}
    </SessionState>
  {/if}

  {#if ts.phase === "mismatch" && ts.error && mismatch}
    <SessionState
      card
      wide
      focusSafe
      tone="danger"
      role="alertdialog"
      icon="shield-alert"
      title={t("term.mismatchTitle")}
    >
      <Banner variant="danger" role="note">{mismatch.summary}</Banner>
      {#if hostPort.host !== null}
        <FingerprintBlock label={t("term.hostKeyHost")} value={`${hostPort.host}:${hostPort.port}`} />
      {/if}
      {#if mismatch.stored && mismatch.presented}
        <FingerprintBlock label={t("term.mismatchStored")} value={mismatch.stored} />
        <FingerprintBlock label={t("term.mismatchPresented")} value={mismatch.presented} />
      {/if}
      <p class="hint">{t("term.mismatchHint")}</p>
      {#if showKnownHosts}
        <KnownHosts host={hostPort.host} port={hostPort.port} />
      {/if}
      {#snippet actions()}
        <span class="remove">
          <Button variant="plain" onclick={removeAndReconnect}>
            {#snippet icon()}<Icon name="trash" />{/snippet}
            {t("term.mismatchRemove")}
          </Button>
        </span>
        <Button onclick={() => (showKnownHosts = !showKnownHosts)}>
          {t("term.mismatchShowList")}
        </Button>
        <Button onclick={() => ctx.retry()}>{t("term.reconnect")}</Button>
        <Button variant="primary" data-autofocus onclick={() => void app.tabs.close(ctx.tabId)}>
          {t("term.closeTab")}
        </Button>
      {/snippet}
    </SessionState>
  {/if}

  {#if ts.phase === "exited"}
    <SessionEndedBar text={ts.exitCode === null ? t("term.exitedNoCode") : t("term.exited", { code: ts.exitCode })}>
      {#snippet actions()}
        <Button variant="primary" onclick={() => ctx.retry()}>{t("term.reconnect")}</Button>
        <Button variant="plain" onclick={() => void app.tabs.close(ctx.tabId)}>
          {t("term.closeTab")}
        </Button>
      {/snippet}
    </SessionEndedBar>
  {/if}
</div>

<style>
  .view {
    position: relative;
    width: 100%;
    height: 100%;
    background: var(--terminal-bg);
    overflow: hidden;
  }

  .term {
    position: absolute;
    inset: 0;
  }

  /* One surface for the whole pane: xterm's element, its viewport and the padding around the
     text all use the terminal background, so no frame shows between the cells and the edge.
     xterm writes its own colour inline on the viewport, hence the override. */
  .term :global(.xterm),
  .term :global(.xterm-viewport) {
    background-color: var(--terminal-bg) !important;
  }

  /* The padding sits on xterm's own element: the fit addon subtracts it from the size it
     proposes, so the text never touches the edges and the grid still fills the pane. */
  .term :global(.xterm) {
    height: 100%;
    padding: var(--space-2) var(--space-3);
  }

  .hint {
    font-size: var(--font-size-md);
    line-height: var(--line-height-relaxed);
  }

  /* The least prominent action sits apart, on its own line before the safe choices. */
  .remove {
    display: flex;
    flex: 1 0 100%;
    justify-content: flex-start;
    padding-bottom: var(--space-1);
  }
</style>
