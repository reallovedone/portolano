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
  import HelpDisclosure from "../ui/HelpDisclosure.svelte";
  import SegmentedControl from "../ui/SegmentedControl.svelte";
  import Select from "../ui/Select.svelte";
  import TextArea from "../ui/TextArea.svelte";
  import TextField from "../ui/TextField.svelte";
  import AppearancePicker from "./AppearancePicker.svelte";
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

  // RDP sections. Sign-in and Security start closed unless they hold a non-default value, so
  // nothing that matters is hidden; a validation error opens the section that owns the field.
  let displayOpen = $state(true);
  let signInOpen = $state(draft.domain.trim() !== "" || draft.kerberosKdc.trim() !== "");
  let clipboardOpen = $state(true);
  let securityOpen = $state(draft.ignoreCert || draft.allowWithoutNla);
  // Icon and colour start closed unless one is set.
  let appearanceOpen = $state(draft.icon !== undefined || draft.color !== undefined);
  const FIELD_SECTION: Partial<Record<ConnectionField, "display" | "signIn">> = {
    width: "display",
    height: "display",
    kerberosKdc: "signIn",
  };
  function openSectionsWithErrors(): void {
    for (const field of Object.keys(errors) as ConnectionField[]) {
      const section = FIELD_SECTION[field];
      if (section === "display") displayOpen = true;
      if (section === "signIn") signInOpen = true;
    }
  }
  const securitySummary = $derived(
    t(
      draft.ignoreCert && draft.allowWithoutNla
        ? "editor.section.securitySummary.both"
        : draft.ignoreCert
          ? "editor.section.securitySummary.noCert"
          : draft.allowWithoutNla
            ? "editor.section.securitySummary.noNla"
            : "editor.section.securitySummary.default",
    ),
  );

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
      openSectionsWithErrors();
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

    <FormSection
      title={t("editor.section.appearance")}
      summary={t("editor.section.appearanceSummary")}
      grouped
      collapsible
      bind:open={appearanceOpen}
    >
      <AppearancePicker
        kind={draft.kind}
        bind:icon={draft.icon}
        bind:color={draft.color}
        id="{uid}-look"
      />
    </FormSection>

    {#if hasOptions && draft.kind !== "rdp"}
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
        {:else if draft.kind === "vnc"}
          <div class="option">
            <Checkbox bind:checked={draft.viewOnly} label={t("editor.vnc.viewOnly")} />
          </div>
          <Banner variant="info" role="note">
            <HelpDisclosure
              short={t("editor.vnc.securityNote")}
              more={t("editor.vnc.securityNoteMore")}
            />
          </Banner>
        {:else}
          <div class="option">
            <Checkbox bind:checked={draft.isolatedProfile} label={t("editor.web.isolated")} />
          </div>
        {/if}
      </FormSection>
    {/if}

    {#if draft.kind === "rdp"}
      <FormSection title={t("editor.section.display")} grouped collapsible bind:open={displayOpen}>
        <FormRow label={t("editor.rdp.resolution")} hint={t("editor.rdp.sizeHint")}>
          <div class="pair">
            <TextField
              id="{uid}-width"
              aria-label={t("editor.rdp.width")}
              placeholder={t("editor.rdp.width")}
              inputmode="numeric"
              bind:value={draft.width}
              error={errorText("width")}
            />
            <span class="times" aria-hidden="true">×</span>
            <TextField
              id="{uid}-height"
              aria-label={t("editor.rdp.height")}
              placeholder={t("editor.rdp.height")}
              inputmode="numeric"
              bind:value={draft.height}
              error={errorText("height")}
            />
          </div>
        </FormRow>
      </FormSection>

      <FormSection
        title={t("editor.section.signIn")}
        summary={t("editor.section.signInSummary")}
        grouped
        collapsible
        bind:open={signInOpen}
      >
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
          <HelpDisclosure
            short={t("editor.rdp.kerberosKdcHint")}
            more={t("editor.rdp.kerberosKdcMore")}
          />
        </FormRow>
      </FormSection>

      <FormSection
        title={t("editor.section.clipboardKeyboard")}
        grouped
        collapsible
        bind:open={clipboardOpen}
      >
        <div class="option">
          <Checkbox bind:checked={draft.shareClipboard} label={t("editor.rdp.shareClipboard")} />
          <HelpDisclosure
            inset
            short={t("editor.rdp.shareClipboardHelp")}
            more={t("editor.rdp.shareClipboardMore")}
          />
        </div>
        {#if isMac}
          <div class="option">
            <Checkbox bind:checked={draft.macShortcuts} label={t("editor.rdp.macShortcuts")} />
            <HelpDisclosure
              inset
              short={t("editor.rdp.macShortcutsHelp")}
              more={t("editor.rdp.macShortcutsMore")}
            />
          </div>
        {/if}
      </FormSection>

      <FormSection
        title={t("editor.section.security")}
        summary={securitySummary}
        grouped
        collapsible
        bind:open={securityOpen}
      >
        <div class="option">
          <Checkbox bind:checked={draft.ignoreCert} label={t("editor.rdp.ignoreCert")} />
        </div>
        <div class="option">
          <Checkbox bind:checked={draft.allowWithoutNla} label={t("editor.rdp.allowWithoutNla")} />
        </div>
        {#if draft.allowWithoutNla}
          <div class="option">
            <Banner variant="warning" role="note">{t("editor.rdp.nlaWarning")}</Banner>
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

  .pair {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
  }

  .pair > :global(.field) {
    flex: 1 1 0;
    min-inline-size: 0;
  }

  .times {
    display: flex;
    align-items: center;
    min-block-size: var(--control-h-regular);
    color: var(--text-tertiary);
  }

  .option {
    padding: var(--space-2) 0;
  }
</style>
