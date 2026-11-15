<!-- Test-only wrapper: provides the app and session contexts around `TerminalView`, with
     reactive `visible` / `overlayOpen` that tests can flip through `hooks`. Not used by the app. -->
<script lang="ts">
  import type { AppError } from "../../ipc";
  import type { ConnectionKind } from "../../tree/model";
  import { provideApp, type App } from "../../state/app";
  import { SessionBridge, provideSessionContext } from "../../tabs/session";
  import TerminalView from "../TerminalView.svelte";

  export interface Hooks {
    setVisible?: (visible: boolean) => void;
    setOverlay?: (open: boolean) => void;
    onFail: (error: AppError) => void;
    onRetry: () => void;
  }

  let {
    app,
    bridge,
    hooks,
    connectionId,
    kind = "ssh",
  }: {
    app: App;
    bridge: SessionBridge;
    hooks: Hooks;
    connectionId: string;
    kind?: ConnectionKind;
  } = $props();

  let visible = $state(true);
  let overlayOpen = $state(false);
  // svelte-ignore state_referenced_locally
  hooks.setVisible = (v) => (visible = v);
  // svelte-ignore state_referenced_locally
  hooks.setOverlay = (v) => (overlayOpen = v);

  // svelte-ignore state_referenced_locally
  provideApp(app);
  // svelte-ignore state_referenced_locally
  provideSessionContext({
    tabId: "tab-1",
    get connectionId() {
      return connectionId;
    },
    get kind() {
      return kind;
    },
    sessionId: "sess-1",
    bridge,
    get visible() {
      return visible;
    },
    get overlayOpen() {
      return overlayOpen;
    },
    get shouldShow() {
      return visible && !overlayOpen;
    },
    fail: (error) => hooks.onFail(error),
    retry: () => hooks.onRetry(),
  });
</script>

<TerminalView sessionId="sess-1" />
