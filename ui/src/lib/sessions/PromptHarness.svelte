<!-- Test-only wrapper: provides the app context around `CredentialPromptDialog`. -->
<script lang="ts">
  import type { CredentialAnswer, CredentialPrompt } from "../ipc";
  import { provideApp, type App } from "../state/app";
  import CredentialPromptDialog from "./CredentialPromptDialog.svelte";

  let {
    app,
    prompt,
    connectionName = null,
    onsubmit,
    oncancel,
  }: {
    app: App;
    prompt: CredentialPrompt;
    connectionName?: string | null;
    onsubmit: (answer: CredentialAnswer, save: boolean) => void;
    oncancel: () => void;
  } = $props();

  // svelte-ignore state_referenced_locally
  provideApp(app);
</script>

<CredentialPromptDialog {prompt} {connectionName} {onsubmit} {oncancel} />
