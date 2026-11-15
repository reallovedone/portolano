<script lang="ts">
  import { t } from "../i18n/strings";
  import { getApp } from "../state/app";
  import Banner from "../ui/Banner.svelte";
  import Button from "../ui/Button.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import TextField from "../ui/TextField.svelte";
  import StrengthHint from "./StrengthHint.svelte";
  import { MIN_MASTER_PASSWORD_LENGTH } from "./strength";

  let { onclose }: { onclose: (changed: boolean) => void } = $props();

  const { vault } = getApp();
  const uid = `cp-${Math.random().toString(36).slice(2, 9)}`;

  let current = $state("");
  let next = $state("");
  let confirmation = $state("");
  let error = $state<string | null>(null);
  let busy = $state(false);

  function clearSecrets(): void {
    current = "";
    next = "";
    confirmation = "";
  }

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (busy) return;
    if (next.length < MIN_MASTER_PASSWORD_LENGTH) {
      error = t("vault.tooShort", { min: MIN_MASTER_PASSWORD_LENGTH });
      return;
    }
    if (next !== confirmation) {
      error = t("vault.mismatch");
      return;
    }
    busy = true;
    error = null;
    const result = await vault.changePassword(current, next);
    busy = false;
    clearSecrets();
    if (result.ok) {
      onclose(true);
    } else {
      error =
        result.error.code === "auth_failed"
          ? t("vault.wrongPassword")
          : `${result.error.message} (${result.error.code})`;
    }
  }

  function cancel(): void {
    clearSecrets();
    onclose(false);
  }
</script>

<Dialog title={t("cred.changeTitle")} onclose={cancel} size="sm">
  <form id="{uid}-form" onsubmit={submit} novalidate>
    <Banner variant="warning" role="note">{t("vault.createWarning")}</Banner>
    {#if error}
      <Banner variant="danger">{error}</Banner>
    {/if}
    <TextField
      id="{uid}-cur"
      type="password"
      label={t("cred.currentPassword")}
      autocomplete="current-password"
      bind:value={current}
      data-autofocus
    />
    <div class="stack">
      <TextField
        id="{uid}-new"
        type="password"
        label={t("cred.newPassword")}
        autocomplete="new-password"
        bind:value={next}
      />
      <StrengthHint password={next} />
    </div>
    <TextField
      id="{uid}-new2"
      type="password"
      label={t("cred.confirmNewPassword")}
      autocomplete="new-password"
      bind:value={confirmation}
    />
  </form>
  {#snippet footer()}
    <Button onclick={cancel}>{t("common.cancel")}</Button>
    <Button type="submit" variant="primary" form="{uid}-form" disabled={busy}>
      {busy ? t("vault.working") : t("common.save")}
    </Button>
  {/snippet}
</Dialog>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
</style>
