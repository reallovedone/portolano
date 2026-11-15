<script lang="ts">
  import { tick } from "svelte";

  import { t } from "../i18n/strings";
  import type { Uuid } from "../ipc";
  import { getApp } from "../state/app";
  import { isFolder } from "../tree/model";
  import Banner from "../ui/Banner.svelte";
  import Button from "../ui/Button.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import FormRow from "../ui/FormRow.svelte";
  import FormSection from "../ui/FormSection.svelte";
  import TextField from "../ui/TextField.svelte";
  import CredentialField from "./CredentialField.svelte";
  import EditorHeader from "./EditorHeader.svelte";
  import {
    draftFromFolder,
    draftToFolder,
    newFolderDraft,
    validateFolderDraft,
    type FolderDraft,
  } from "./form";
  import LocationField from "./LocationField.svelte";

  let {
    id,
    parentId,
    onclose,
  }: { id: Uuid | null; parentId: Uuid | null; onclose: () => void } = $props();

  const { doc, ui } = getApp();
  const uid = `fe-${Math.random().toString(36).slice(2, 9)}`;
  // svelte-ignore state_referenced_locally
  const existing = id === null ? undefined : doc.node(id);
  // svelte-ignore state_referenced_locally
  const editing = existing !== undefined && isFolder(existing);

  // svelte-ignore state_referenced_locally
  let draft = $state<FolderDraft>(
    existing && isFolder(existing) ? draftFromFolder(existing) : newFolderDraft(parentId),
  );
  let errors = $state<ReturnType<typeof validateFolderDraft>>({});
  let saveError = $state<string | null>(null);
  let busy = $state(false);
  let form: HTMLFormElement | undefined = $state();

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (busy) return;
    errors = validateFolderDraft(draft);
    saveError = null;
    if (Object.keys(errors).length > 0) {
      await tick();
      form?.querySelector<HTMLElement>('[aria-invalid="true"]')?.focus();
      return;
    }
    busy = true;
    const folder = draftToFolder(draft, () => crypto.randomUUID());
    const result = await doc.upsert({ type: "folder", ...folder });
    busy = false;
    if (!result.ok) {
      saveError = `${result.error.message} (${result.error.code})`;
      return;
    }
    if (folder.parentId !== null) doc.expand(folder.parentId);
    ui.selectedId = folder.id;
    onclose();
  }
</script>

<Dialog title={editing ? t("editor.editFolder") : t("editor.newFolder")} {onclose}>
  <form bind:this={form} id="{uid}-form" onsubmit={submit} novalidate>
    {#if saveError}
      <div class="form-banner">
        <Banner variant="danger">{t("editor.saveFailed")}: {saveError}</Banner>
      </div>
    {/if}

    <EditorHeader kind="folder" title={t("kind.folder")} summary={t("editor.folderSummary")} />

    <FormSection title={t("editor.section.general")} grouped>
      <FormRow label={t("editor.name")} for="{uid}-name">
        <TextField
          id="{uid}-name"
          bind:value={draft.name}
          data-autofocus
          error={errors.name ? t(errors.name) : null}
        />
      </FormRow>
      <LocationField
        bind:value={draft.parentId}
        nodes={doc.nodes}
        excludeSubtreeOf={draft.id}
        id="{uid}-parent"
      />
    </FormSection>

    <FormSection title={t("editor.section.credentials")} grouped>
      <CredentialField
        bind:value={draft.credential}
        nodes={doc.nodes}
        parentId={draft.parentId}
        error={errors.credential ? t(errors.credential) : null}
        idPrefix={uid}
      />
    </FormSection>
  </form>

  {#snippet footer()}
    <Button onclick={onclose}>{t("common.cancel")}</Button>
    <Button type="submit" variant="primary" form="{uid}-form" disabled={busy}>
      {t("common.save")}
    </Button>
  {/snippet}
</Dialog>

<style>
  .form-banner {
    margin-bottom: var(--space-4);
  }
</style>
