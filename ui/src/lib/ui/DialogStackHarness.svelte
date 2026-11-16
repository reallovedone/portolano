<!-- Test-only: three dialogs that can open over each other, to check stacking, focus and Escape.
     The inner ones are rendered BEFORE the outer one in the DOM on purpose (like the app shell
     renders the unlock dialog before the editors): the stack, not the DOM order, decides. -->
<script lang="ts">
  import { provideApp, type App } from "../state/app";
  import Button from "./Button.svelte";
  import Dialog from "./Dialog.svelte";
  import TextField from "./TextField.svelte";
  import TreeSelect from "./TreeSelect.svelte";

  let { app }: { app: App } = $props();
  // svelte-ignore state_referenced_locally
  provideApp(app);

  let inner = $state(false);
  let third = $state(false);
  let typed = $state("");
  let folder = $state("a");
  const options = [
    { value: "a", label: "Folder A" },
    { value: "b", label: "Folder B" },
  ];
</script>

{#if third}
  <Dialog title="Third" onclose={() => (third = false)} size="sm">
    <TextField label="Third field" />
  </Dialog>
{/if}
{#if inner}
  <Dialog title="Inner" onclose={() => (inner = false)} size="sm">
    <TextField label="Inner field" data-autofocus />
    <Button onclick={() => (third = true)}>Open third</Button>
  </Dialog>
{/if}
<Dialog title="Outer" onclose={() => undefined}>
  <TextField label="Outer field" bind:value={typed} />
  <TreeSelect label="Location" {options} bind:value={folder} />
  <Button onclick={() => (inner = true)}>Open inner</Button>
</Dialog>
