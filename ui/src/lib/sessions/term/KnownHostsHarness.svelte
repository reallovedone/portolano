<!-- Test-only wrapper: provides the app context around `KnownHosts`. -->
<script lang="ts">
  import { provideApp, type App } from "../../state/app";
  import KnownHosts from "./KnownHosts.svelte";

  let { app }: { app: App } = $props();

  // svelte-ignore state_referenced_locally
  provideApp(app);
</script>

<KnownHosts host="changed.example" port={22} />
