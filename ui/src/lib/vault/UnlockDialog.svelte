<script lang="ts">
  import { t } from "../i18n/strings";
  import { getApp } from "../state/app";
  import Button from "../ui/Button.svelte";
  import ChartBackdrop from "../ui/ChartBackdrop.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import Icon from "../ui/Icon.svelte";
  import TextField from "../ui/TextField.svelte";

  let {
    reason = null,
    allowManual = false,
    onmanual,
    onclose,
  }: {
    reason?: string | null;
    /** Offer "enter manually for this session" (the connection's own prompt asks instead). */
    allowManual?: boolean;
    onmanual?: () => void;
    onclose: (unlocked: boolean) => void;
  } = $props();

  const { vault } = getApp();
  const uid = `uv-${Math.random().toString(36).slice(2, 9)}`;

  let password = $state("");
  let error = $state<string | null>(null);
  let busy = $state(false);

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (busy || password === "") return;
    busy = true;
    error = null;
    const result = await vault.unlock(password);
    busy = false;
    password = "";
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
    password = "";
    onclose(false);
  }

  function manual(): void {
    password = "";
    onmanual?.();
  }
</script>

<Dialog title={t("vault.unlockTitle")} onclose={cancel} size="sm">
  {#snippet hero()}
    <div class="strip">
      <ChartBackdrop labels={false} />
      <span class="mark"><Icon name="lock" size={20} /></span>
    </div>
  {/snippet}
  <form id="{uid}-form" onsubmit={submit} novalidate>
    <p class="intro">{reason ? t("vault.unlockWhy", { name: reason }) : t("vault.unlockIntro")}</p>
    <TextField
      id="{uid}-pw"
      type="password"
      label={t("vault.masterPassword")}
      autocomplete="current-password"
      bind:value={password}
      {error}
      data-autofocus
    />
  </form>
  {#snippet footer()}
    {#if allowManual && onmanual}
      <span class="manual"><Button variant="plain" onclick={manual}>{t("vault.enterManually")}</Button></span>
    {/if}
    <Button onclick={cancel}>{t("common.cancel")}</Button>
    <Button
      type="submit"
      variant="primary"
      form="{uid}-form"
      disabled={busy || password === ""}
    >
      {busy ? t("vault.working") : t("vault.unlockButton")}
    </Button>
  {/snippet}
</Dialog>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  /* Decoration stays in the dialog's hero strip, away from the field. */
  .strip {
    position: relative;
    display: grid;
    place-items: center;
    block-size: calc(var(--space-12) * 1.5);
  }

  .mark {
    position: relative;
    display: grid;
    place-items: center;
    inline-size: calc(var(--space-4) * 2.5);
    block-size: calc(var(--space-4) * 2.5);
    background: var(--surface-raised);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-panel);
    box-shadow: var(--shadow-popover);
    color: var(--text-primary);
  }

  .intro {
    color: var(--text-secondary);
    text-align: center;
  }

  .manual {
    margin-right: auto;
  }
</style>
