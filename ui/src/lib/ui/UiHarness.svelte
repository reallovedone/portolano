<!-- Test harness: provides the app context that Dialog, ContextMenu and Toasts need. -->
<script lang="ts">
  import type { App } from "../state/app";
  import { provideApp } from "../state/app";
  import Button from "./Button.svelte";
  import ContextMenu from "./ContextMenu.svelte";
  import Dialog from "./Dialog.svelte";
  import TextField from "./TextField.svelte";
  import Toasts from "./Toasts.svelte";
  import type { MenuItem } from "./menu";

  let {
    app,
    mode,
    items = [],
    onclose = () => {},
  }: { app: App; mode: "dialog" | "menu" | "toasts"; items?: MenuItem[]; onclose?: () => void } =
    $props();

  // svelte-ignore state_referenced_locally
  provideApp(app);
</script>

<button type="button" data-testid="opener">opener</button>

{#if mode === "dialog"}
  <Dialog title="Sample" {onclose} size="sm">
    <TextField label="First" />
    <TextField label="Second" />
    {#snippet footer()}
      <Button onclick={onclose}>Cancel</Button>
      <Button variant="primary">Save</Button>
    {/snippet}
  </Dialog>
{:else if mode === "menu"}
  <ContextMenu x={10} y={10} {items} {onclose} label="Sample menu" />
{:else}
  <Toasts />
{/if}
