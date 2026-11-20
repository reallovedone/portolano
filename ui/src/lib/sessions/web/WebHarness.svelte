<!-- Test support: provides the application and session contexts around `WebView`. -->
<script lang="ts">
  import { provideApp, type App } from "../../state/app";
  import { provideSessionContext, type SessionContext } from "../../tabs/session";
  import type { UrlSubscribe } from "./urlEvents";
  import SessionToolbar from "../../tabs/SessionToolbar.svelte";
  import WebView from "../WebView.svelte";

  let {
    app,
    context,
    standIn = false,
    urlEvents,
  }: { app: App; context: SessionContext; standIn?: boolean; urlEvents?: UrlSubscribe } = $props();

  // svelte-ignore state_referenced_locally
  provideApp(app);
  // svelte-ignore state_referenced_locally
  provideSessionContext(context);
</script>

<!-- The address bar and page controls live in the session toolbar. -->
<SessionToolbar
  tab={{
    id: context.tabId,
    type: "connection",
    connectionId: context.connectionId,
    title: "Web",
    kind: "web",
    status: "open",
    sessionId: context.sessionId,
    error: null,
    attempt: 0,
  }}
/>
<WebView sessionId={context.sessionId} {standIn} {urlEvents} />
