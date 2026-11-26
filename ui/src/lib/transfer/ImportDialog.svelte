<!-- Import preview and confirmation: format of the chosen file (detected, can be changed), what
     it contains, the warnings, and the folder to import into. The file is read by the backend
     (`import_preview`); nothing is written until "Import" (`import_apply`, all or nothing). -->
<script lang="ts">
  import { untrack } from "svelte";

  import { t, type StringKey } from "../i18n/strings";
  import { toIpcError, type IpcError, type ImportFormat, type ImportPreview } from "../ipc";
  import { getApp } from "../state/app";
  import { defaultParentFor, folderOptions, isConnection } from "../tree/model";
  import Badge from "../ui/Badge.svelte";
  import Banner from "../ui/Banner.svelte";
  import Button from "../ui/Button.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import EmptyState from "../ui/EmptyState.svelte";
  import FormSection from "../ui/FormSection.svelte";
  import Hint from "../ui/Hint.svelte";
  import KindIcon from "../ui/KindIcon.svelte";
  import ListRow from "../ui/ListRow.svelte";
  import Select, { type SelectOption } from "../ui/Select.svelte";
  import TreeSelect, { type TreeSelectOption } from "../ui/TreeSelect.svelte";
  import Spinner from "../ui/Spinner.svelte";
  import { IMPORT_FORMATS, baseName, countByKind, detectImportFormat, previewRows, summarizeWarnings } from "./formats";

  /** The preview list shows at most this many rows (a big import stays responsive). */
  const MAX_ROWS = 300;

  let {
    path,
    onclose,
  }: {
    path: string;
    /** `count` is the number of imported items, or `null` when nothing was imported. */
    onclose: (count: number | null) => void;
  } = $props();

  const { doc, ipc, ui, notices } = getApp();
  const uid = $props.id();

  // The dialog is opened for one file: the path does not change while it is mounted.
  // svelte-ignore state_referenced_locally
  let format = $state<ImportFormat | "">(detectImportFormat(path) ?? "");
  let preview = $state<ImportPreview | null>(null);
  let loading = $state(false);
  let error = $state<{ title: string; message: string } | null>(null);
  let busy = $state(false);
  // The selected folder is the natural place to import into, like "new connection".
  let target = $state<string>(defaultParentFor(doc.nodes, ui.selectedId, doc.remoteNodes) ?? "");
  let generation = 0;

  const formatOptions = $derived<SelectOption[]>([
    ...(format === "" ? [{ value: "", label: t("import.formatChoose"), disabled: true }] : []),
    ...IMPORT_FORMATS.map((f) => ({ value: f, label: t(`import.format.${f}` as StringKey) })),
  ]);
  const targetOptions = $derived<TreeSelectOption[]>([
    { value: "", label: t("import.targetRoot"), icon: null },
    ...folderOptions(doc.nodes, null, doc.remoteNodes).map((f) => ({
      value: f.id,
      label: f.name,
      depth: f.depth,
    })),
  ]);
  const counts = $derived(preview ? countByKind(preview.nodes) : []);
  const rows = $derived(preview ? previewRows(preview.nodes) : []);
  const shownRows = $derived(rows.slice(0, MAX_ROWS));
  const warnings = $derived(preview ? summarizeWarnings(preview.warnings) : null);
  const canImport = $derived(preview !== null && preview.nodes.length > 0 && !busy);

  function describe(cause: unknown): { title: string; message: string } {
    const e: IpcError = toIpcError(cause);
    const key = `import.error.${e.code}` as StringKey;
    const known = e.code === "invalid_input" || e.code === "unsupported" || e.code === "io";
    return { title: t(known ? key : "import.error.other"), message: e.message };
  }

  async function load(chosen: ImportFormat | ""): Promise<void> {
    const mine = ++generation;
    preview = null;
    error = null;
    if (chosen === "") return;
    loading = true;
    try {
      const result = await ipc.importPreview(chosen, path);
      if (mine === generation) preview = result;
    } catch (e) {
      if (mine === generation) error = describe(e);
    } finally {
      if (mine === generation) loading = false;
    }
  }

  // Previews on open and again whenever the format is changed by hand.
  $effect(() => {
    const chosen = format;
    untrack(() => void load(chosen));
  });

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!preview || busy) return;
    busy = true;
    error = null;
    const folder = target === "" ? null : target;
    try {
      const count = await ipc.importApply(preview.previewId, folder);
      await doc.reload();
      if (folder !== null) doc.expand(folder);
      notices.push("info", t(count === 1 ? "import.done.one" : "import.done.other", { count }));
      onclose(count);
    } catch (e) {
      const described = describe(e);
      error = { title: t("import.applyFailed"), message: described.message };
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title={t("import.title")} onclose={() => onclose(null)} size="lg">
  <form id="{uid}-form" class="form" onsubmit={submit} novalidate>
    <FormSection title={t("import.file")}>
      <p class="file" title={path}>{baseName(path)}</p>
      <Select
        label={t("import.format")}
        bind:value={() => format, (next) => (format = next as ImportFormat | "")}
        options={formatOptions}
        disabled={busy}
        data-autofocus
      />
      {#if format === ""}
        <Banner variant="info">{t("import.formatUnknown")}</Banner>
      {/if}
    </FormSection>

    {#if error}
      <Banner variant="danger" title={error.title}>{error.message}</Banner>
    {/if}

    {#if loading}
      <Spinner size={14}>{t("import.loading")}</Spinner>
    {:else if preview && warnings}
      {#if preview.nodes.length === 0}
        <EmptyState icon="info" title={t("import.nothing")} />
      {:else}
        <FormSection title={t("import.summary")}>
          <div class="counts">
            {#each counts as entry (entry.kind)}
              <Badge variant={entry.kind === "folder" ? "neutral" : "accent"}>
                {t("import.countBadge", { kind: t(`kind.${entry.kind}` as StringKey), count: entry.count })}
              </Badge>
            {/each}
          </div>
        </FormSection>

        <FormSection title={t("import.contents")}>
          <ul class="list tree" aria-label={t("import.contents")}>
            {#each shownRows as row (row.node.id)}
              <li>
                <ListRow depth={row.depth}>
                  {#snippet leading()}
                    <KindIcon
                      kind={row.node.type === "folder" ? "folder" : row.node.options.type}
                      size={18}
                    />
                  {/snippet}
                  {row.node.name}
                  {#snippet secondary()}
                    {#if isConnection(row.node)}{row.node.host}{/if}
                  {/snippet}
                </ListRow>
              </li>
            {/each}
          </ul>
          {#if rows.length > MAX_ROWS}
            <Hint>{t("import.more", { count: rows.length - MAX_ROWS })}</Hint>
          {/if}
        </FormSection>
      {/if}

      {#if warnings.ignoredSecrets > 0}
        <Banner variant="info" title={t("import.secretsTitle")}>
          {t(warnings.ignoredSecrets === 1 ? "import.secretsBody.one" : "import.secretsBody.other", {
            count: warnings.ignoredSecrets,
          })}
        </Banner>
      {/if}

      {#if warnings.others.length > 0}
        <FormSection title={t("import.warnings", { count: warnings.others.length })}>
          <ul class="list warnings" aria-label={t("import.warnings", { count: warnings.others.length })}>
            {#each warnings.others as warning, index (index)}
              <li class="warning">
                {#if warning.line !== null}
                  <Badge variant="warning">{t("import.warningLine", { line: warning.line })}</Badge>
                {/if}
                <span class="message">{warning.message}</span>
              </li>
            {/each}
          </ul>
        </FormSection>
      {/if}

      {#if preview.nodes.length > 0}
        <FormSection>
          <TreeSelect
            label={t("import.target")}
            bind:value={target}
            options={targetOptions}
            disabled={busy}
          />
        </FormSection>
      {/if}
    {/if}
  </form>
  {#snippet footer()}
    <Button onclick={() => onclose(null)}>{t("common.cancel")}</Button>
    <Button type="submit" variant="primary" form="{uid}-form" disabled={!canImport}>
      {t("import.confirm")}
    </Button>
  {/snippet}
</Dialog>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .file {
    overflow: hidden;
    color: var(--text-secondary);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .counts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .list {
    margin: 0;
    padding: 0;
    overflow: auto;
    list-style: none;
    background: var(--surface-inset);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-control);
  }

  .tree {
    max-height: calc(var(--space-12) * 5);
  }

  .warnings {
    max-height: calc(var(--space-12) * 3);
  }

  .warning {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    font-size: var(--font-size-sm);
  }

  .warning + .warning {
    border-top: 1px solid var(--border-hairline);
  }

  .warning :global(.badge) {
    flex: none;
  }

  .message {
    min-width: 0;
    overflow-wrap: anywhere;
  }
</style>
