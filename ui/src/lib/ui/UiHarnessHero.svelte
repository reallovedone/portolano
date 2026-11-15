<!-- Test harness: a Dialog with a decorative hero strip, and a TextField whose input is bound. -->
<script lang="ts">
  import type { App } from "../state/app";
  import { provideApp } from "../state/app";
  import Dialog from "./Dialog.svelte";
  import TextField from "./TextField.svelte";

  let {
    app,
    size = "md",
    width,
  }: { app: App; size?: "sm" | "md" | "lg"; width?: number } = $props();

  // svelte-ignore state_referenced_locally
  provideApp(app);

  let search = $state("");
  let input = $state<HTMLInputElement | undefined>();
</script>

<Dialog title="With hero" onclose={() => {}} {size} {width}>
  {#snippet hero()}<div data-testid="hero-strip">decoration</div>{/snippet}
  <TextField label="Query" type="search" bind:value={search} bind:input />
  <button type="button" onclick={() => input?.focus()}>focus-field</button>
</Dialog>
