<!-- Create / edit a connection: common fields plus the per-kind options. -->
<script lang="ts">
  import { tick } from "svelte";

  import { detectPlatform } from "../design/platform";
  import { t, type StringKey } from "../i18n/strings";
  import type { Uuid } from "../ipc";
  import { getApp } from "../state/app";
  import { CONNECTION_KINDS, isConnection, type ConnectionKind } from "../tree/model";
  import Banner from "../ui/Banner.svelte";
  import Button from "../ui/Button.svelte";
  import Checkbox from "../ui/Checkbox.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import FormRow from "../ui/FormRow.svelte";
  import FormSection from "../ui/FormSection.svelte";
  import SegmentedControl from "../ui/SegmentedControl.svelte";
  import Select from "../ui/Select.svelte";
  import TextArea from "../ui/TextArea.svelte";
  import TextField from "../ui/TextField.svelte";
  import CredentialField from "./CredentialField.svelte";
  import EditorHeader from "./EditorHeader.svelte";
  import {
    DEFAULT_PORTS,
    draftFromConnection,
    draftToConnection,
    kindUsesHost,
    newConnectionDraft,
    validateConnectionDraft,
    type ConnectionDraft,
    type ConnectionField,
    type FieldErrors,
  } from "./form";
  import LocationField from "./LocationField.svelte";

  let {
    id,
    parentId,
    kind = "ssh",
    onclose,
  }: {
    id: Uuid | null;
    parentId: Uuid | null;
    /** Connection type a new connection starts on (ignored when editing). */
    kind?: ConnectionKind;
    onclose: () => void;
  } = $props();

  const { doc, ui } = getApp();
  // The Mac shortcuts option means something only on a Mac.
  const isMac = detectPlatform() === "macos";
  const uid = `ce-${Math.random().toString(36).slice(2, 9)}`;

  // The editor edits a snapshot taken when it opens.
  // svelte-ignore state_referenced_locally
  const existing = id === null ? undefined : doc.node(id);
  // svelte-ignore state_referenced_locally
  const editing = existing !== undefined && isConnection(existing);
  // svelte-ignore state_referenced_locally
  let draft = $state<ConnectionDraft>(
    existing && isConnection(existing)
      ? draftFromConnection(existing)
      : newConnectionDraft(kind, parentId),
  );
  let errors = $state<FieldErrors<ConnectionField>>({});
  let saveError = $state<string | null>(null);
  let busy = $state(false);
  let form: HTMLFormElement | undefined = $state();

  const sshJumpTargets = $derived(
    doc.nodes.filter((n) => isConnection(n) && n.options.type === "ssh" && n.id !== draft.id),
  );
  const defaultPort = $derived(DEFAULT_PORTS[draft.kind]);

  const errorText = (field: ConnectionField): string | null => {
    const key = errors[field];
    return key ? t(key) : null;
  };

  const kindLabel = (kind: ConnectionKind): string => t(`kind.${kind}` as StringKey);
  const kindOptions = CONNECTION_KINDS.map((kind) => ({ value: kind, label: kindLabel(kind) }));
  const jumpOptions = $derived([
    { value: "", label: t("editor.ssh.jumpNone") },
    ...sshJumpTargets.map((target) => ({ value: target.id, label: target.name })),
  ]);
  const hasOptions = $derived(draft.kind !== "localShell");

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (busy) return;
    errors = validateConnectionDraft(draft);
    saveError = null;
    if (Object.keys(errors).length > 0) {
      await tick();
      form?.querySelector<HTMLElement>('[aria-invalid="true"]')?.focus();
      return;
    }
    busy = true;
    const node = draftToConnection(draft, () => crypto.randomUUID());
    const result = await doc.upsert({ type: "connection", ...node });
    busy = false;
    if (!result.ok) {
      saveError = `${result.error.message} (${result.error.code})`;
      return;
    }
    if (node.parentId !== null) doc.expand(node.parentId);
    ui.selectedId = node.id;
    onclose();
  }
</script>

<Dialog
  title={editing ? t("editor.editConnection") : t("editor.newConnection")}
  {onclose}
  size="lg"
>
  <form bind:this={form} id="{uid}-form" onsubmit={submit} novalidate>
    {#if saveError}
      <div class="form-banner">
        <Banner variant="danger">{t("editor.saveFailed")}: {saveError}</Banner>
      </div>
    {/if}

    <EditorHeader
      kind={draft.kind}
      title={kindLabel(draft.kind)}
      summary={t(`editor.kindSummary.${draft.kind}` as StringKey)}
    />

    <div class="kind">
      <SegmentedControl
        label={t("editor.kind")}
        options={kindOptions}
        bind:value={() => draft.kind, (next) => (draft.kind = next as ConnectionKind)}
        disabled={editing}
      />
    </div>

    <FormSection title={t("editor.section.general")} grouped>
      <FormRow label={t("editor.name")} for="{uid}-name">
        <TextField
          id="{uid}-name"
          bind:value={draft.name}
          data-autofocus
          error={errorText("name")}
        />
      </FormRow>
      <LocationField bind:value={draft.parentId} nodes={doc.nodes} id="{uid}-parent" />
      <FormRow label={t("editor.tags")} for="{uid}-tags">
        <TextField id="{uid}-tags" bind:value={draft.tags} hint={t("editor.tagsHint")} />
      </FormRow>
    </FormSection>

    <FormSection title={t("editor.section.connection")} grouped>
      {#if kindUsesHost(draft.kind)}
        <FormRow label={t("editor.host")} for="{uid}-host">
          <TextField
            id="{uid}-host"
            bind:value={draft.host}
            monospace
            autocomplete="off"
            spellcheck="false"
            error={errorText("host")}
          />
        </FormRow>
        <FormRow label={t("editor.port")} for="{uid}-port">
          <div class="narrow">
            <TextField
              id="{uid}-port"
              inputmode="numeric"
              monospace
              bind:value={draft.port}
              placeholder={defaultPort === null ? "" : t("editor.portDefault", { port: defaultPort })}
              error={errorText("port")}
            />
          </div>
        </FormRow>
        <FormRow label={t("editor.username")} for="{uid}-username">
          <TextField
            id="{uid}-username"
            bind:value={draft.username}
            autocomplete="off"
            hint={t("editor.usernameHint")}
          />
        </FormRow>
      {:else if draft.kind === "localShell"}
        <FormRow label={t("editor.shell.program")} for="{uid}-program">
          <TextField
            id="{uid}-program"
            bind:value={draft.program}
            monospace
            spellcheck="false"
            hint={t("editor.shell.programHint")}
          />
        </FormRow>
        <FormRow label={t("editor.shell.args")} for="{uid}-args">
          <TextArea
            id="{uid}-args"
            rows={3}
            monospace
            bind:value={draft.args}
            spellcheck="false"
            hint={t("editor.shell.argsHint")}
          />
        </FormRow>
        <FormRow label={t("editor.shell.cwd")} for="{uid}-cwd">
          <TextField id="{uid}-cwd" bind:value={draft.cwd} monospace spellcheck="false" />
        </FormRow>
      {:else}
        <FormRow label={t("editor.web.url")} for="{uid}-url">
          <TextField
            id="{uid}-url"
            bind:value={draft.url}
            monospace
            placeholder="https://"
            spellcheck="false"
            error={errorText("url")}
          />
        </FormRow>
      {/if}
    </FormSection>

    <FormSection title={t("editor.section.credentials")} grouped>
      <CredentialField
        bind:value={draft.credential}
        nodes={doc.nodes}
        parentId={draft.parentId}
        error={errorText("credential")}
        idPrefix={uid}
      />
    </FormSection>

    {#if hasOptions}
      <FormSection title={t("editor.section.options")} grouped>
        {#if draft.kind === "ssh"}
          <FormRow label={t("editor.ssh.jumpHost")} for="{uid}-jump">
            <Select id="{uid}-jump" options={jumpOptions} bind:value={draft.jumpHost} />
          </FormRow>
          <FormRow label={t("editor.ssh.keepalive")} for="{uid}-keepalive">
            <div class="narrow">
              <TextField
                id="{uid}-keepalive"
                inputmode="numeric"
                bind:value={draft.keepaliveSecs}
                error={errorText("keepaliveSecs")}
              />
            </div>
          </FormRow>
        {:else if draft.kind === "rdp"}
          <FormRow label={t("editor.rdp.width")} for="{uid}-width">
            <div class="narrow">
              <TextField
                id="{uid}-width"
                inputmode="numeric"
                bind:value={draft.width}
                error={errorText("width")}
              />
            </div>
          </FormRow>
          <FormRow label={t("editor.rdp.height")} for="{uid}-height">
            <div class="narrow">
              <TextField
                id="{uid}-height"
                inputmode="numeric"
                bind:value={draft.height}
                error={errorText("height")}
              />
            </div>
            <p class="size-hint">{t("editor.rdp.sizeHint")}</p>
          </FormRow>
          <FormRow label={t("editor.rdp.domain")} for="{uid}-domain">
            <TextField id="{uid}-domain" bind:value={draft.domain} autocomplete="off" />
          </FormRow>
          <FormRow label={t("editor.rdp.kerberosKdc")} for="{uid}-kdc">
            <TextField
              id="{uid}-kdc"
              bind:value={draft.kerberosKdc}
              autocomplete="off"
              error={errorText("kerberosKdc")}
            />
            <p class="size-hint">{t("editor.rdp.kerberosKdcHint")}</p>
          </FormRow>
          <div class="option">
            <Checkbox bind:checked={draft.ignoreCert} label={t("editor.rdp.ignoreCert")} />
          </div>
          <div class="option">
            <Checkbox bind:checked={draft.allowWithoutNla} label={t("editor.rdp.allowWithoutNla")} />
          </div>
          {#if draft.allowWithoutNla}
            <Banner variant="warning" role="note">{t("editor.rdp.nlaWarning")}</Banner>
          {/if}
          <div class="option">
            <Checkbox bind:checked={draft.shareClipboard} label={t("editor.rdp.shareClipboard")} />
            <p class="size-hint">{t("editor.rdp.shareClipboardHelp")}</p>
          </div>
          {#if isMac}
            <div class="option">
              <Checkbox bind:checked={draft.macShortcuts} label={t("editor.rdp.macShortcuts")} />
              <p class="size-hint">{t("editor.rdp.macShortcutsHelp")}</p>
            </div>
          {/if}
        {:else if draft.kind === "vnc"}
          <div class="option">
            <Checkbox bind:checked={draft.viewOnly} label={t("editor.vnc.viewOnly")} />
          </div>
          <Banner variant="info" role="note">{t("editor.vnc.securityNote")}</Banner>
        {:else}
          <div class="option">
            <Checkbox bind:checked={draft.isolatedProfile} label={t("editor.web.isolated")} />
          </div>
        {/if}
      </FormSection>
    {/if}

    <FormSection title={t("editor.section.notes")}>
      <TextArea aria-label={t("editor.notes")} rows={3} bind:value={draft.notes} />
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

  .kind {
    margin-bottom: var(--space-4);
  }

  .narrow {
    max-inline-size: calc(var(--space-12) * 3);
  }

  .size-hint {
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
  }

  .option {
    padding: var(--space-2) 0;
  }
</style>
