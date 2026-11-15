<script lang="ts">
  import { t } from "../i18n/strings";
  import { getApp } from "../state/app";
  import Banner from "../ui/Banner.svelte";
  import ChartBackdrop from "../ui/ChartBackdrop.svelte";
  import Button from "../ui/Button.svelte";
  import Checkbox from "../ui/Checkbox.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import Logo from "../ui/Logo.svelte";
  import TextField from "../ui/TextField.svelte";
  import StrengthHint from "./StrengthHint.svelte";
  import { MIN_MASTER_PASSWORD_LENGTH } from "./strength";

  let { onclose }: { onclose: (created: boolean) => void } = $props();

  const { vault } = getApp();
  const uid = `cv-${Math.random().toString(36).slice(2, 9)}`;

  let password = $state("");
  let confirmation = $state("");
  let acknowledged = $state(false);
  let error = $state<string | null>(null);
  let busy = $state(false);

  function clearSecrets(): void {
    password = "";
    confirmation = "";
  }

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (busy) return;
    if (password.length < MIN_MASTER_PASSWORD_LENGTH) {
      error = t("vault.tooShort", { min: MIN_MASTER_PASSWORD_LENGTH });
      return;
    }
    if (password !== confirmation) {
      error = t("vault.mismatch");
      return;
    }
    if (!acknowledged) {
      error = t("vault.acknowledge");
      return;
    }
    busy = true;
    error = null;
    const result = await vault.create(password);
    busy = false;
    clearSecrets();
    if (result.ok) onclose(true);
    else error = `${result.error.message} (${result.error.code})`;
  }

  function cancel(): void {
    clearSecrets();
    onclose(false);
  }
</script>

<Dialog title={t("vault.createTitle")} onclose={cancel} size="sm">
  {#snippet hero()}
    <div class="strip">
      <ChartBackdrop />
      <span class="mark"><Logo size={32} /></span>
    </div>
  {/snippet}
  <form id="{uid}-form" onsubmit={submit} novalidate>
    <p class="intro">{t("vault.createIntro")}</p>
    {#if error}
      <Banner variant="danger">{error}</Banner>
    {/if}
    <div class="fields">
      <div class="stack">
        <TextField
          id="{uid}-pw"
          type="password"
          label={t("vault.masterPassword")}
          autocomplete="new-password"
          bind:value={password}
          data-autofocus
        />
        <StrengthHint {password} />
      </div>
      <TextField
        id="{uid}-pw2"
        type="password"
        label={t("vault.confirmPassword")}
        autocomplete="new-password"
        bind:value={confirmation}
      />
    </div>
    <Banner variant="warning" role="note">{t("vault.createWarning")}</Banner>
    <div class="ack">
      <Checkbox bind:checked={acknowledged} label={t("vault.acknowledge")} />
    </div>
  </form>
  {#snippet footer()}
    <Button onclick={cancel}>{t("common.cancel")}</Button>
    <Button type="submit" variant="primary" form="{uid}-form" disabled={busy}>
      {busy ? t("vault.working") : t("common.create")}
    </Button>
  {/snippet}
</Dialog>

<style>
  /* Decoration lives in the dialog's hero strip only, never behind the fields. */
  .strip {
    position: relative;
    display: grid;
    place-items: center;
    block-size: calc(var(--space-12) * 2);
  }

  .mark {
    position: relative;
    display: grid;
    place-items: center;
    inline-size: calc(var(--space-4) * 3);
    block-size: calc(var(--space-4) * 3);
    background: var(--surface-raised);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-panel);
    box-shadow: var(--shadow-popover);
    color: var(--text-primary);
  }

  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding-top: var(--space-1);
  }

  .intro {
    color: var(--text-secondary);
  }

  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .ack {
    padding-bottom: var(--space-1);
  }
</style>
