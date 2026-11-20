<!-- Mounts the session component for an open tab and provides the session context. -->
<script lang="ts">
  import { getApp } from "../state/app";
  import FramebufferView from "../sessions/FramebufferView.svelte";
  import TerminalView from "../sessions/TerminalView.svelte";
  import WebView from "../sessions/WebView.svelte";
  import type { ConnectionTab } from "./model";
  import { provideSessionContext } from "./session";

  let { tab, sessionId }: { tab: ConnectionTab; sessionId: string } = $props();

  const { tabs, overlay, toolbar } = getApp();

  // The context reads `tab` lazily through getters, so it stays reactive.
  // svelte-ignore state_referenced_locally
  provideSessionContext({
    get tabId() {
      return tab.id;
    },
    get connectionId() {
      return tab.connectionId;
    },
    get kind() {
      return tab.kind;
    },
    get sessionId() {
      return sessionId;
    },
    bridge: tabs.bridge(tab.id),
    get visible() {
      return tabs.state.activeId === tab.id;
    },
    get overlayOpen() {
      return overlay.open;
    },
    get shouldShow() {
      return tabs.state.activeId === tab.id && !overlay.open;
    },
    fail: (error) => void tabs.fail(tab.id, error),
    retry: () => void tabs.retry(tab.id),
    publishToolbar: (model) => toolbar.publish(tab.id, model),
  });
</script>

{#if tab.kind === "localShell" || tab.kind === "ssh"}
  <TerminalView {sessionId} />
{:else if tab.kind === "rdp" || tab.kind === "vnc"}
  <FramebufferView {sessionId} />
{:else}
  <WebView {sessionId} />
{/if}
