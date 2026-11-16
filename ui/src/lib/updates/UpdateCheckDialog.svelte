<!-- "Check for updates…": the running version, the "Automatically check" setting, and the
     answer of a manual check, which always gives feedback (up to date, or the error). When an
     update is found this dialog hands over to the offer dialog. -->
<script lang="ts">
  import { onMount } from "svelte";

  import { t } from "../i18n/strings";
  import type { IpcError } from "../ipc";
  import { getApp } from "../state/app";
  import Banner from "../ui/Banner.svelte";
  import Button from "../ui/Button.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import Spinner from "../ui/Spinner.svelte";
  import Switch from "../ui/Switch.svelte";

  let { onclose }: { onclose: () => void } = $props();

  const { updates } = getApp();

  type Outcome = { state: "checking" } | { state: "upToDate" } | { state: "error"; error: IpcError };
  let outcome = $state<Outcome | null>(null);
  let version = $derived(updates.settings?.currentVersion ?? "");
  let generation = 0;

  async function check(): Promise<void> {
    const mine = ++generation;
    outcome = { state: "checking" };
    try {
      const result = await updates.checkNow();
      if (mine !== generation) return;
      if (result.status === "available") onclose();
      else outcome = { state: "upToDate" };
    } catch (e) {
      if (mine !== generation) return;
      outcome = { state: "error", error: e as IpcError };
    }
  }

  onMount(() => {
    void check();
    return () => {
      generation += 1;
    };
  });

  // The switch edits the stored setting; a failed save puts it back.
  let auto = $state(updates.autoCheck);
  $effect(() => {
    auto = updates.autoCheck;
  });

  async function toggle(): Promise<void> {
    const wanted = auto;
    if (!(await updates.setAutoCheck(wanted))) auto = updates.autoCheck;
  }
</script>

<Dialog title={t("update.checkTitle")} {onclose} size="sm">
  {#if version}<p class="version">{t("update.currentVersion", { version })}</p>{/if}

  <div class="result" aria-live="polite">
    {#if outcome?.state === "checking"}
      <p class="status"><Spinner size={16} /> {t("update.checking")}</p>
    {:else if outcome?.state === "upToDate"}
      <Banner variant="success" title={t("update.upToDate")}>
        {t("update.upToDateDetail", { version })}
      </Banner>
    {:else if outcome?.state === "error"}
      <Banner variant="danger" title={t("update.checkFailed")}>
        {outcome.error.message}
      </Banner>
    {/if}
  </div>

  <Switch
    label={t("update.auto")}
    hint={t("update.autoHint")}
    bind:checked={auto}
    onchange={() => void toggle()}
  />

  {#snippet footer()}
    <Button onclick={onclose}>{t("common.close")}</Button>
    <Button
      variant="primary"
      data-autofocus
      disabled={outcome?.state === "checking"}
      onclick={() => void check()}
    >
      {t("update.checkNow")}
    </Button>
  {/snippet}
</Dialog>

<style>
  .version {
    margin: 0 0 var(--space-3);
    color: var(--text-secondary);
  }

  .result {
    min-height: var(--control-h-large);
    margin-bottom: var(--space-3);
  }

  .status {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
    color: var(--text-secondary);
  }
</style>
