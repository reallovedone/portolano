<script lang="ts">
  import type { App } from "../state/app";
  import { provideApp } from "../state/app";
  import PromptDialog from "./PromptDialog.svelte";

  let {
    app,
    onsubmit,
    oncancel,
  }: { app: App; onsubmit: (value: string) => string | null; oncancel: () => void } = $props();

  // svelte-ignore state_referenced_locally
  provideApp(app);
</script>

<PromptDialog title="New folder" label="Name" {onsubmit} {oncancel} />
