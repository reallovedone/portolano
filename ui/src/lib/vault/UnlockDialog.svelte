<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { t } from "../i18n/strings";
  import { detectMac } from "../shortcuts";
  import { getApp } from "../state/app";
  import Button from "../ui/Button.svelte";
  import ChartBackdrop from "../ui/ChartBackdrop.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import Hint from "../ui/Hint.svelte";
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

  const { vault, notices } = getApp();
  const uid = `uv-${Math.random().toString(36).slice(2, 9)}`;

  let password = $state("");
  let error = $state<string | null>(null);
  let busy = $state(false);

  const platform = detectMac() ? "mac" : "windows";
  /** A quick-unlock key is held: offer the biometric button. */
  let quickReady = $state(false);
  let quickBusy = $state(false);
  let quickNote = $state<string | null>(null);
  /** Set once the dialog is done (unlocked, cancelled or destroyed): later biometric answers are ignored. */
  let finished = false;

  onMount(async () => {
    const info = await vault.quickUnlockInfo();
    if (finished) return;
    if (!info.ok) {
      notices.error(t("security.quickUnlockInfoFailed"), info.error);
      return;
    }
    if (!info.value.ready) return;
    quickReady = true;
    // One automatic attempt per opening; retrying is the user's choice.
    await useQuickUnlock();
  });

  onDestroy(() => {
    finished = true;
  });

  async function useQuickUnlock(): Promise<void> {
    if (finished || quickBusy) return;
    quickBusy = true;
    quickNote = null;
    const result = await vault.quickUnlock();
    quickBusy = false;
    if (finished) return;
    if (result.ok) {
      finished = true;
      password = "";
      onclose(true);
      return;
    }
    if (result.error.code === "quick_unlock_not_ready") quickReady = false;
    else if (result.error.code !== "quick_unlock_cancelled") {
      // (A second request while an OS prompt is open comes back as cancelled: stays silent.)
      quickNote = t(`vault.biometricFailed.${platform}`);
    }
    // The system prompt takes the focus: give it back to the password field.
    document.getElementById(`${uid}-pw`)?.focus();
  }

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (busy || password === "") return;
    busy = true;
    error = null;
    const result = await vault.unlock(password);
    busy = false;
    password = "";
    // The biometric check may have closed the dialog while this was in flight.
    if (finished) return;
    if (result.ok) {
      finished = true;
      onclose(true);
    } else {
      error =
        result.error.code === "auth_failed"
          ? t("vault.wrongPassword")
          : `${result.error.message} (${result.error.code})`;
    }
  }

  function cancel(): void {
    finished = true;
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
    {#if quickReady}
      <div class="quick">
        <Button variant="plain" onclick={useQuickUnlock} disabled={quickBusy}>
          {t(`vault.useBiometric.${platform}`)}
        </Button>
        {#if quickNote}<Hint role="status">{quickNote}</Hint>{/if}
      </div>
    {/if}
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

  .quick {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-1);
    text-align: center;
  }

  .manual {
    margin-right: auto;
  }
</style>
