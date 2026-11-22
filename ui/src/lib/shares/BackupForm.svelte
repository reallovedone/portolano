<!-- Saves the identity backup: a password entered twice, then the save-file dialog. The password
     is wiped from the fields as soon as it has been used (or the form is left), never logged
     and never kept in a store. -->
<script lang="ts">
  import { t } from "../i18n/strings";
  import { toIpcError } from "../ipc";
  import { getApp } from "../state/app";
  import Banner from "../ui/Banner.svelte";
  import Button from "../ui/Button.svelte";
  import TextField from "../ui/TextField.svelte";

  let {
    /** Called after a saved backup (`true`) or when the user leaves the form (`false`). */
    ondone,
    cancelLabel = t("common.cancel"),
  }: { ondone: (saved: boolean) => void; cancelLabel?: string } = $props();

  const { ipc, files, notices } = getApp();
  const uid = $props.id();

  let password = $state("");
  let confirmation = $state("");
  let error = $state<string | null>(null);
  let busy = $state(false);

  function wipe(): void {
    password = "";
    confirmation = "";
  }

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (busy) return;
    if (password === "") {
      error = t("share.backupEmpty");
      return;
    }
    if (password !== confirmation) {
      error = t("share.backupMismatch");
      return;
    }
    error = null;
    busy = true;
    try {
      const path = await files.pickSave({
        title: t("share.backupSaveTitle"),
        defaultName: "portolano-identity.portolano-id",
        filters: [{ name: t("share.identityTitle"), extensions: ["portolano-id"] }],
      });
      if (path === null) return;
      await ipc.identityExport(path, password);
      notices.push("info", t("share.backupDone"));
      ondone(true);
    } catch (cause) {
      const e = toIpcError(cause);
      error = `${e.message} (${e.code})`;
    } finally {
      // Used, failed or cancelled: the password is never kept.
      wipe();
      busy = false;
    }
  }

  function leave(): void {
    wipe();
    ondone(false);
  }
</script>

<form id="{uid}-form" class="form" onsubmit={submit} novalidate>
  {#if error}
    <Banner variant="danger">{error}</Banner>
  {/if}
  <TextField
    type="password"
    label={t("share.backupPassword")}
    autocomplete="new-password"
    bind:value={password}
  />
  <TextField
    type="password"
    label={t("share.backupConfirm")}
    autocomplete="new-password"
    bind:value={confirmation}
  />
  <div class="actions">
    <Button onclick={leave}>{cancelLabel}</Button>
    <Button type="submit" variant="primary" disabled={busy}>{t("share.backupSave")}</Button>
  </div>
</form>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
  }
</style>
