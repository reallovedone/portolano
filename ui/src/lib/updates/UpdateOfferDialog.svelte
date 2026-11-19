<!-- An update is available: current -> new version, a link to its release notes, and the three
     choices.
     "Update now" downloads with visible progress, then the backend verifies the signature,
     installs and restarts. Nothing is ever installed without this click. -->
<script lang="ts">
  import { t } from "../i18n/strings";
  import { getApp } from "../state/app";
  import Banner from "../ui/Banner.svelte";
  import Button from "../ui/Button.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import Spinner from "../ui/Spinner.svelte";

  const { updates, tabs, closeConfirm } = getApp();

  // The dialog only exists while there is an offer.
  const info = $derived(updates.offer?.info);
  const install = $derived(updates.install);
  const percent = $derived(
    install?.phase === "downloading" && install.total
      ? Math.min(100, Math.floor((install.downloaded / install.total) * 100))
      : null,
  );
  const failure = $derived(install?.phase === "failed" ? install.error : null);

  // Installing restarts the app and ends every session: ask first if any is connected.
  async function installNow(): Promise<void> {
    if (await closeConfirm.confirmClose(tabs.tabs.map((x) => x.id))) await updates.installNow();
  }

  function close(): void {
    // The window stays while an installation runs: closing it would hide the progress.
    if (!updates.installing) updates.later();
  }
</script>

{#if info}
  <Dialog title={t("update.offerTitle")} onclose={close} size="md">
    <p class="versions" data-testid="update-versions">
      {t("update.versions", { current: info.currentVersion, version: info.version })}
    </p>

    {#if updates.installing}
      <div class="progress" role="status">
        <p>
          <Spinner size={16} />
          {#if install?.phase === "downloading"}
            {percent === null
              ? t("update.downloading")
              : t("update.downloadingPercent", { percent })}
          {:else if install?.phase === "installing"}
            {t("update.installing")}
          {:else}
            {t("update.restarting")}
          {/if}
        </p>
        {#if install?.phase === "downloading"}
          <progress
            aria-label={t("update.progressLabel")}
            max={install.total ?? undefined}
            value={install.total ? install.downloaded : undefined}
          ></progress>
        {/if}
      </div>
    {:else}
      {#if failure}
        <div class="gap">
          <Banner variant="danger" title={t("update.installFailed")}>
            {failure.message} ({failure.code})
          </Banner>
        </div>
      {/if}
      <p class="notes-link">
        <button
          type="button"
          class="link"
          data-version={info.version}
          onclick={() => void updates.openReleasePage(info.version)}
        >
          {t("update.releaseNotesLink")}
        </button>
      </p>
      {#if tabs.tabs.length > 0}
        <div class="gap"><Banner variant="warning" role="note">{t("update.sessionsWarning")}</Banner></div>
      {/if}
    {/if}

    {#snippet footer()}
      {#if !updates.installing}
        <span class="skip">
          <Button variant="plain" onclick={() => void updates.skip()}>{t("update.skip")}</Button>
        </span>
        <Button onclick={close}>{t("update.later")}</Button>
        {#if failure}
          <Button variant="primary" data-autofocus onclick={() => void installNow()}>
            {t("update.retry")}
          </Button>
        {:else}
          <Button variant="primary" data-autofocus onclick={() => void installNow()}>
            {t("update.now")}
          </Button>
        {/if}
      {/if}
    {/snippet}
  </Dialog>
{/if}

<style>
  .versions {
    margin: 0 0 var(--space-3);
    font-weight: var(--font-weight-semibold);
  }

  .notes-link {
    margin: 0 0 var(--space-3);
  }

  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--text-link);
    font: inherit;
    text-decoration: underline;
    cursor: pointer;
  }

  .gap {
    margin-block: var(--space-3);
  }

  .progress p {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0 0 var(--space-2);
  }

  progress {
    width: 100%;
  }

  .skip {
    margin-right: auto;
  }
</style>
