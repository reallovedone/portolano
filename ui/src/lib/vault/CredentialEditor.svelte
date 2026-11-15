<script lang="ts">
  import { onDestroy, tick } from "svelte";

  import { t } from "../i18n/strings";
  import type { CredentialSummary } from "../ipc";
  import { getApp } from "../state/app";
  import Banner from "../ui/Banner.svelte";
  import Button from "../ui/Button.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import SegmentedControl from "../ui/SegmentedControl.svelte";
  import TextArea from "../ui/TextArea.svelte";
  import TextField from "../ui/TextField.svelte";
  import {
    draftFromSummary,
    draftMetadata,
    draftToInput,
    newCredentialDraft,
    secretIsEmpty,
    validateCredentialDraft,
    type CredentialFormDraft,
  } from "./credentialForm";

  let {
    credential = null,
    onclose,
  }: { credential?: CredentialSummary | null; onclose: (saved: boolean) => void } = $props();

  const { vault } = getApp();
  const uid = `ced-${Math.random().toString(36).slice(2, 9)}`;
  // svelte-ignore state_referenced_locally
  const editing = credential !== null;

  // svelte-ignore state_referenced_locally
  let draft = $state<CredentialFormDraft>(
    credential ? draftFromSummary(credential) : newCredentialDraft(),
  );
  let errors = $state<ReturnType<typeof validateCredentialDraft>>({});
  let saveError = $state<string | null>(null);
  let busy = $state(false);
  let form: HTMLFormElement | undefined = $state();

  const kindOptions = [
    { value: "password", label: t("cred.kind.password"), icon: "lock" },
    { value: "sshKey", label: t("cred.kind.sshKey"), icon: "key" },
  ] as const;

  function clearSecrets(): void {
    draft.password = "";
    draft.privateKey = "";
    draft.passphrase = "";
  }

  onDestroy(clearSecrets);

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (busy) return;
    errors = validateCredentialDraft(draft);
    saveError = null;
    if (Object.keys(errors).length > 0) {
      await tick();
      form?.querySelector<HTMLElement>('[aria-invalid="true"]')?.focus();
      return;
    }
    busy = true;
    const keep = draft.id !== null && secretIsEmpty(draft);
    const result =
      keep && draft.id !== null
        ? await vault.upsertKeepingSecret({ ...draftMetadata(draft), id: draft.id })
        : await vault.upsert(draftToInput(draft));
    busy = false;
    clearSecrets();
    if (result.ok) onclose(true);
    else saveError = `${result.error.message} (${result.error.code})`;
  }

  function cancel(): void {
    clearSecrets();
    onclose(false);
  }
</script>

<Dialog title={editing ? t("cred.editTitle") : t("cred.addTitle")} onclose={cancel}>
  <form bind:this={form} id="{uid}-form" onsubmit={submit} novalidate autocomplete="off">
    {#if saveError}
      <Banner variant="danger">{t("cred.saveFailed")}: {saveError}</Banner>
    {/if}

    <div class="kind">
      <SegmentedControl
        label={t("cred.kind")}
        options={[...kindOptions]}
        bind:value={() => draft.kind, (next) => (draft.kind = next as typeof draft.kind)}
        disabled={editing}
      />
    </div>

    <TextField
      id="{uid}-name"
      label={t("cred.name")}
      bind:value={draft.name}
      data-autofocus
      error={errors.name ? t(errors.name) : null}
    />

    <TextField
      id="{uid}-user"
      label={t("cred.username")}
      bind:value={draft.username}
      autocomplete="off"
      spellcheck="false"
    />

    <TextField
      id="{uid}-domain"
      label={t("cred.domain")}
      optionalLabel={t("common.optional")}
      bind:value={draft.domain}
      autocomplete="off"
    />

    {#if draft.kind === "password"}
      <TextField
        id="{uid}-pw"
        type="password"
        label={t("cred.password")}
        autocomplete="new-password"
        bind:value={draft.password}
        error={errors.password ? t(errors.password) : null}
        hint={editing ? t("cred.secretKeep") : undefined}
      />
    {:else}
      <TextArea
        id="{uid}-key"
        label={t("cred.privateKey")}
        rows={6}
        monospace
        spellcheck="false"
        autocomplete="off"
        bind:value={draft.privateKey}
        error={errors.privateKey ? t(errors.privateKey) : null}
        hint={editing ? t("cred.secretKeep") : undefined}
      />
      <TextField
        id="{uid}-pass"
        type="password"
        label={t("cred.passphrase")}
        optionalLabel={t("common.optional")}
        autocomplete="new-password"
        bind:value={draft.passphrase}
      />
    {/if}
  </form>
  {#snippet footer()}
    <Button onclick={cancel}>{t("common.cancel")}</Button>
    <Button type="submit" variant="primary" form="{uid}-form" disabled={busy}>
      {t("common.save")}
    </Button>
  {/snippet}
</Dialog>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  /* The segmented control hugs its content; keep it at the start. */
  .kind {
    align-self: flex-start;
  }
</style>
