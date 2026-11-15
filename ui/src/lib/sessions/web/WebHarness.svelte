<!-- Test support: provides the application and session contexts around `WebView`. -->
<script lang="ts">
  import { provideApp, type App } from "../../state/app";
  import { provideSessionContext, type SessionContext } from "../../tabs/session";
  import type { UrlSubscribe } from "./urlEvents";
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

<WebView sessionId={context.sessionId} {standIn} {urlEvents} />
