<!-- Test support: provides the application and session contexts around `FramebufferView`. -->
<script lang="ts">
  import { provideApp, type App } from "../../state/app";
  import { provideSessionContext, type SessionContext } from "../../tabs/session";
  import FramebufferView from "../FramebufferView.svelte";

  let { app, context }: { app: App; context: SessionContext } = $props();

  // svelte-ignore state_referenced_locally
  provideApp(app);
  // svelte-ignore state_referenced_locally
  provideSessionContext(context);
</script>

<div data-testid="outer">
  <FramebufferView sessionId={context.sessionId} />
</div>
