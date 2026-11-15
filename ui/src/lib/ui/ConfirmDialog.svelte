<script lang="ts">
  import { t } from "../i18n/strings";
  import Banner from "./Banner.svelte";
  import Button from "./Button.svelte";
  import Dialog from "./Dialog.svelte";

  let {
    title,
    message,
    confirmLabel = t("common.delete"),
    danger = true,
    busy = false,
    error = null,
    onconfirm,
    oncancel,
  }: {
    title: string;
    message: string;
    confirmLabel?: string;
    danger?: boolean;
    busy?: boolean;
    error?: string | null;
    onconfirm: () => void;
    oncancel: () => void;
  } = $props();
</script>

<Dialog {title} onclose={oncancel} width={420}>
  <p class="message">{message}</p>
  {#if error}
    <div class="error"><Banner variant="danger" role="alert">{error}</Banner></div>
  {/if}
  {#snippet footer()}
    <Button data-autofocus onclick={oncancel}>{t("common.cancel")}</Button>
    <Button variant={danger ? "destructive" : "primary"} disabled={busy} onclick={onconfirm}>
      {confirmLabel}
    </Button>
  {/snippet}
</Dialog>

<style>
  .message {
    white-space: pre-wrap;
    line-height: var(--line-height-relaxed);
  }

  .error {
    margin-top: var(--space-3);
  }
</style>
