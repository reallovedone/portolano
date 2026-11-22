<!-- "Add Remote Folder…": one field for a share file path or an https address. Preview reads and
     verifies the file (nothing is mounted); the owner's fingerprint is shown large so it can be
     checked with the person who shared the folder; then a destination and Subscribe. -->
<script lang="ts">
  import { t, type StringKey } from "../i18n/strings";
  import { toIpcError, type SharePreview } from "../ipc";
  import { getApp } from "../state/app";
  import { defaultParentFor, effectiveCredential, folderOptions } from "../tree/model";
  import Banner from "../ui/Banner.svelte";
  import Button from "../ui/Button.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import TextField from "../ui/TextField.svelte";
  import TreeSelect, { type TreeSelectOption } from "../ui/TreeSelect.svelte";
  import { SKIPPED_SHOWN, checkLocation, displayText, hostLine } from "./display";

  let { onclose }: { onclose: () => void } = $props();

  const { doc, ipc, files, ui, notices } = getApp();
  const uid = $props.id();

  let input = $state("");
  let preview = $state<SharePreview | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  // The selected folder is the natural place, like "new connection"; shared folders never are.
  let target = $state<string>(defaultParentFor(doc.nodes, ui.selectedId, doc.remoteNodes) ?? "");

  const targetOptions = $derived<TreeSelectOption[]>([
    { value: "", label: t("share.destinationRoot"), icon: null },
    ...folderOptions(doc.nodes, null, doc.remoteNodes).map((f) => ({
      value: f.id,
      label: f.name,
      depth: f.depth,
    })),
  ]);

  // The shared folder starts with "inherit": whatever credential the chosen destination resolves
  // to (its own or one from a folder above) would reach every host listed above.
  const inheritedFrom = $derived.by(() => {
    if (target === "") return null;
    const effective = effectiveCredential(doc.nodes, { type: "inherit" }, target);
    return effective.kind === "id" && !effective.source.own ? effective.source.folderName : null;
  });

  function skippedNames(names: readonly string[]): string {
    const shown = names.slice(0, SKIPPED_SHOWN).map((n) => displayText(n));
    const more = names.length - shown.length;
    return more > 0 ? `${shown.join(", ")} ${t("share.skippedMore", { count: more })}` : shown.join(", ");
  }

  function edit(): void {
    // A different address is a different share: the previous preview no longer applies.
    preview = null;
    error = null;
  }

  async function browse(): Promise<void> {
    try {
      const chosen = await files.pickOpen({
        title: t("share.openTitle"),
        filters: [{ name: t("share.filterShare"), extensions: ["portolano-share"] }],
      });
      if (chosen !== null) {
        input = chosen;
        edit();
      }
    } catch (cause) {
      const e = toIpcError(cause);
      error = `${e.message} (${e.code})`;
    }
  }

  async function runPreview(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (busy) return;
    const checked = checkLocation(input);
    if (!checked.ok) {
      preview = null;
      error = t(`share.location${checked.problem[0]!.toUpperCase()}${checked.problem.slice(1)}` as StringKey);
      return;
    }
    error = null;
    busy = true;
    try {
      preview = await ipc.sharePreview(checked.location);
    } catch (cause) {
      preview = null;
      const e = toIpcError(cause);
      error = `${t("share.previewFailed")}: ${e.message} (${e.code})`;
    } finally {
      busy = false;
    }
  }

  async function subscribe(): Promise<void> {
    if (!preview || busy) return;
    busy = true;
    error = null;
    try {
      const summary = await ipc.shareSubscribe(preview.previewId, target === "" ? null : target);
      await doc.reload();
      if (target !== "") doc.expand(target);
      notices.push("info", t("share.subscribed"), t("share.subscribedDetail", { name: displayText(summary.name) }));
      onclose();
    } catch (cause) {
      const e = toIpcError(cause);
      error = `${e.message} (${e.code})`;
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title={t("share.addRemoteTitle")} {onclose} width={520}>
  <form id="{uid}-form" class="form" onsubmit={runPreview} novalidate>
    <p class="hint">{t("share.addRemoteHint")}</p>
    <TextField
      label={t("share.locationLabel")}
      placeholder={t("share.locationPlaceholder")}
      monospace
      bind:value={input}
      oninput={edit}
      data-autofocus
    >
      {#snippet suffix()}
        <Button size="compact" onclick={browse}>{t("common.browse")}</Button>
      {/snippet}
    </TextField>
    {#if error}
      <Banner variant="danger">{error}</Banner>
    {/if}
  </form>

  {#if preview}
    <section class="preview" aria-label={t("share.preview")}>
      <h3 class="name" title={displayText(preview.name)}>{displayText(preview.name)}</h3>
      <p class="owner" title={displayText(preview.ownerName)}>
        {t("share.previewOwner", { name: displayText(preview.ownerName) })}
      </p>
      <div class="fingerprint" aria-label={t("share.fingerprintLabel")}>{preview.fingerprint}</div>
      <p class="check">{t("share.fingerprintCheck")}</p>
      <p class="counts">
        {t("share.previewCounts", {
          folders: preview.folders,
          connections: preview.connections,
          revision: preview.revision,
        })}
      </p>
      <h4 class="hosts-title">{t("share.previewHosts", { count: preview.hosts.length })}</h4>
      <ul class="hosts" aria-label={t("share.previewHosts", { count: preview.hosts.length })}>
        {#each preview.hosts as host (host.nodeId)}
          <li class="wrap">{hostLine(host)}</li>
        {/each}
      </ul>
      {#if preview.skipped.length > 0}
        <Banner variant="warning">
          {t("share.previewSkipped", {
            count: preview.skipped.length,
            names: skippedNames(preview.skipped),
          })}
        </Banner>
      {/if}
      {#if inheritedFrom !== null}
        <Banner variant="warning">
          {t("share.previewCredentialWarning", { folder: displayText(inheritedFrom) })}
        </Banner>
      {/if}
      <TreeSelect label={t("share.destination")} bind:value={target} options={targetOptions} />
    </section>
  {/if}

  {#snippet footer()}
    <Button onclick={onclose}>{t("common.cancel")}</Button>
    {#if preview}
      <Button variant="primary" disabled={busy} onclick={subscribe}>{t("share.subscribe")}</Button>
    {:else}
      <Button type="submit" variant="primary" form="{uid}-form" disabled={busy}>{t("share.preview")}</Button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .hint {
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
  }
  .preview {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-top: var(--space-4);
    padding-top: var(--space-4);
    border-top: 1px solid var(--border-hairline);
  }
  .name {
    overflow: hidden;
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-semibold);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .owner {
    overflow: hidden;
    color: var(--text-secondary);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fingerprint {
    padding: var(--space-3);
    font-family: var(--font-mono);
    font-size: var(--font-size-xl, 1.5rem);
    letter-spacing: 0.08em;
    text-align: center;
    user-select: all;
    background: var(--surface-raised);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-control);
  }
  .check {
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-semibold);
  }
  .hosts-title {
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-semibold);
  }
  .hosts {
    max-height: 10rem;
    overflow-y: auto;
    padding: var(--space-2) var(--space-3) var(--space-2) var(--space-5);
    font-family: var(--font-mono);
    font-size: var(--font-size-sm);
    list-style: disc;
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-control);
  }
  .wrap {
    overflow-wrap: anywhere;
  }
  .counts {
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
  }
</style>
