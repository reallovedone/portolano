<!-- Test-only wrapper: provides the app context around the editor dialogs. -->
<script lang="ts">
  import type { Uuid } from "../ipc";
  import type { ConnectionKind } from "../tree/model";
  import { provideApp, type App } from "../state/app";
  import ConnectionEditor from "./ConnectionEditor.svelte";

  let {
    app,
    id,
    parentId = null,
    kind,
  }: { app: App; id: Uuid | null; parentId?: Uuid | null; kind?: ConnectionKind } = $props();

  // svelte-ignore state_referenced_locally
  provideApp(app);
</script>

<ConnectionEditor {id} {parentId} {kind} onclose={() => {}} />
