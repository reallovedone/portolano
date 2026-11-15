<!-- Single text input dialog (rename, new folder name). -->
<script lang="ts">
  import { t } from "../i18n/strings";
  import Button from "./Button.svelte";
  import Dialog from "./Dialog.svelte";
  import TextField from "./TextField.svelte";

  let {
    title,
    label,
    initial = "",
    submitLabel = t("common.save"),
    onsubmit,
    oncancel,
  }: {
    title: string;
    label: string;
    initial?: string;
    submitLabel?: string;
    /** Returns an error message to keep the dialog open, or null when done. */
    onsubmit: (value: string) => Promise<string | null> | string | null;
    oncancel: () => void;
  } = $props();

  // svelte-ignore state_referenced_locally
  let value = $state(initial);
  let error = $state<string | null>(null);
  let busy = $state(false);
  const uid = $props.id();
  const inputId = `prompt-${uid}`;

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (busy) return;
    const trimmed = value.trim();
    if (trimmed === "") {
      error = t("validation.nameRequired");
      return;
    }
    busy = true;
    error = await onsubmit(trimmed);
    busy = false;
  }
</script>

<Dialog {title} onclose={oncancel} size="sm">
  <form onsubmit={submit} id="{inputId}-form">
    <TextField id={inputId} {label} bind:value {error} data-autofocus />
  </form>
  {#snippet footer()}
    <Button onclick={oncancel}>{t("common.cancel")}</Button>
    <Button type="submit" variant="primary" form="{inputId}-form" disabled={busy}>
      {submitLabel}
    </Button>
  {/snippet}
</Dialog>
