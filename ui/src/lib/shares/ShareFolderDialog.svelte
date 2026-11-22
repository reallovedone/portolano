<!-- "Share…" on a local folder: publishes it as a signed file. Without an identity the dialog first
     asks for a name and creates one. After the first publish a one-time step offers a backup of
     the identity (password twice). -->
<script lang="ts">
  import { onMount } from "svelte";

  import { t } from "../i18n/strings";
  import { toIpcError, type IdentitySummary } from "../ipc";
  import { getApp } from "../state/app";
  import Banner from "../ui/Banner.svelte";
  import Button from "../ui/Button.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import TextField from "../ui/TextField.svelte";
  import BackupForm from "./BackupForm.svelte";
  import { displayText, shareFileName } from "./display";

  let { folderId, onclose }: { folderId: string; onclose: () => void } = $props();

  const { doc, ipc, files, vault, notices } = getApp();
  const uid = $props.id();

  const folder = $derived(doc.nodes.find((n) => n.id === folderId));
  const folderName = $derived(displayText(folder?.name));

  let identity = $state<IdentitySummary | null>(null);
  let loaded = $state(false);
  let name = $state("");
  let path = $state("");
  let error = $state<string | null>(null);
  let busy = $state(false);
  let step = $state<"publish" | "backup">("publish");

  let locked = $state(false);
  let readFailed = $state(false);

  // `identity_get` needs the vault unlocked: ask first, or an existing identity would look
  // missing and the dialog would try to create a second one.
  async function load(): Promise<void> {
    error = null;
    if (!(await vault.ensureUnlocked(null))) {
      locked = true;
      loaded = true;
      return;
    }
    locked = false;
    readFailed = false;
    try {
      identity = await ipc.identityGet();
    } catch (cause) {
      readFailed = true;
      const e = toIpcError(cause);
      error = `${e.message} (${e.code})`;
    }
    loaded = true;
  }

  onMount(() => {
    void load();
  });

  async function browse(): Promise<void> {
    try {
      const chosen = await files.pickSave({
        title: t("share.saveTitle"),
        defaultName: shareFileName(folder?.name ?? ""),
        filters: [{ name: t("share.filterShare"), extensions: ["portolano-share"] }],
      });
      if (chosen !== null) path = chosen;
    } catch (cause) {
      const e = toIpcError(cause);
      error = `${e.message} (${e.code})`;
    }
  }

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (busy) return;
    error = null;
    if (!identity && name.trim() === "") {
      error = t("share.nameRequired");
      return;
    }
    if (path.trim() === "") {
      error = t("share.fileRequired");
      return;
    }
    busy = true;
    try {
      // Publishing signs with the identity, which lives in the vault.
      if (!(await vault.ensureUnlocked(null))) return;
      // The first share of this installation is when a backup matters.
      const first = !doc.shares.some((s) => s.role === "owner");
      if (!identity) identity = await ipc.identityCreate(name.trim());
      const summary = await ipc.sharePublish(folderId, path.trim());
      await doc.reload();
      notices.push("info", t("share.published"), t("share.publishedDetail", { name: displayText(summary.name), revision: summary.revision }));
      if (first) step = "backup";
      else onclose();
    } catch (cause) {
      const e = toIpcError(cause);
      error = `${t("share.publishFailed")}: ${e.message} (${e.code})`;
    } finally {
      busy = false;
    }
  }
</script>

{#if step === "backup"}
  <Dialog title={t("share.backupTitle")} {onclose} width={460}>
    <p class="hint">{t("share.backupHint")}</p>
    <BackupForm ondone={() => onclose()} cancelLabel={t("share.backupSkip")} />
  </Dialog>
{:else}
  <Dialog title={t("share.shareTitle", { name: folderName })} {onclose} width={500}>
    <form id="{uid}-form" class="form" onsubmit={submit} novalidate>
      <p class="hint">{t("share.shareHint")}</p>
      {#if error}
        <Banner variant="danger">{error}</Banner>
      {/if}
      {#if loaded && locked}
        <p class="hint">{t("share.identityLocked")}</p>
        <div><Button onclick={load}>{t("share.identityUnlock")}</Button></div>
      {:else if loaded && !readFailed}
        {#if identity}
          <p class="hint">
            {t("share.identityAs", { name: displayText(identity.name), fingerprint: identity.fingerprint })}
          </p>
        {:else}
          <TextField
            label={t("share.identityName")}
            hint={t("share.identityNameHint")}
            maxlength={100}
            bind:value={name}
            data-autofocus
          />
        {/if}
        <TextField label={t("share.fileLabel")} monospace bind:value={path}>
          {#snippet suffix()}
            <Button size="compact" onclick={browse}>{t("common.browse")}</Button>
          {/snippet}
        </TextField>
      {/if}
    </form>
    {#snippet footer()}
      <Button onclick={onclose}>{t("common.cancel")}</Button>
      <Button type="submit" variant="primary" form="{uid}-form" disabled={busy || !loaded || locked || readFailed}>
        {t("share.publish")}
      </Button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .hint {
    margin-bottom: var(--space-3);
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
  }
  .form .hint {
    margin-bottom: 0;
  }
</style>
