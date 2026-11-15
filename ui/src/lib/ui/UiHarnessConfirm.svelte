<script lang="ts">
  import type { App } from "../state/app";
  import { provideApp } from "../state/app";
  import ConfirmDialog from "./ConfirmDialog.svelte";

  let { app, onconfirm, oncancel }: { app: App; onconfirm: () => void; oncancel: () => void } = $props();

  // svelte-ignore state_referenced_locally
  provideApp(app);
</script>

<ConfirmDialog title="Delete it?" message="This cannot be undone." error="Failed" {onconfirm} {oncancel} />
