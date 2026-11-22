<!-- "Share info…": who published a shared folder, its fingerprint, revision, last sync and errors,
     and the connections whose host is new or changed and waits for the user's confirmation,
     listed in full (scrollable, never cut to a number of rows). -->
<script lang="ts">
  import { t } from "../i18n/strings";
  import { toIpcError } from "../ipc";
  import { getApp } from "../state/app";
  import Banner from "../ui/Banner.svelte";
  import Button from "../ui/Button.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import { ERROR_MAX, displayText, hostLine } from "./display";

  let { linkId, onclose }: { linkId: string; onclose: () => void } = $props();

  const { doc, ipc, vault, notices } = getApp();

  const share = $derived(doc.shares.find((s) => s.id === linkId));
  let busy = $state(false);
  let error = $state<string | null>(null);
  let note = $state<string | null>(null);

  const dateTime = (iso: string | null): string =>
    iso ? new Date(iso).toLocaleString() : t("share.infoNever");

  async function confirm(): Promise<void> {
    if (!share || busy) return;
    busy = true;
    error = null;
    note = null;
    try {
      // The revision the user is looking at: if the share moved on, the backend says `conflict`.
      await ipc.shareConfirmTargets(linkId, share.revision);
      await doc.reload();
      notices.push("info", t("share.confirmed"));
    } catch (cause) {
      const e = toIpcError(cause);
      if (e.code === "conflict") {
        // Not an error to scare anyone with: show what changed and let them decide again.
        await doc.reload();
        note = t("share.confirmChanged");
      } else {
        error = `${t("share.confirmFailed")}: ${e.message} (${e.code})`;
      }
    } finally {
      busy = false;
    }
  }

  async function publish(): Promise<void> {
    if (!share || busy) return;
    busy = true;
    error = null;
    // Publishing signs with the identity in the vault; a cancelled unlock is not an error.
    if (!(await vault.ensureUnlocked(null))) {
      busy = false;
      return;
    }
    const result = await doc.republish(linkId);
    busy = false;
    if (!result.ok) {
      error = `${t("share.publishFailed")}: ${result.error.message} (${result.error.code})`;
      return;
    }
    const next = doc.shares.find((s) => s.id === linkId);
    notices.push("info", t("share.published"), t("share.publishedDetail", { name: displayText(next?.name), revision: next?.revision ?? 0 }));
  }
</script>

<Dialog title={t("share.infoTitle", { name: displayText(share?.name) })} {onclose} width={520}>
  {#if !share}
    <Banner variant="warning">{t("share.infoGone")}</Banner>
  {:else}
    {#if note}
      <div class="gap"><Banner variant="info">{note}</Banner></div>
    {/if}
    {#if error}
      <div class="gap"><Banner variant="danger">{error}</Banner></div>
    {/if}

    {#if share.role === "owner"}
      <p class="hint">{t("share.infoRoleOwner")}</p>
    {/if}

    <dl class="facts">
      <dt>{t("share.infoOwner")}</dt>
      <dd class="clip" title={displayText(share.ownerName)}>{displayText(share.ownerName)}</dd>
      <dt>{t("share.infoFingerprint")}</dt>
      <dd class="mono">{share.fingerprint}</dd>
      <dt>{t("share.infoRevision")}</dt>
      <dd>{share.revision}</dd>
      {#if share.signerName || share.publishedAt}
        <dt>{t("share.infoSignedBy")}</dt>
        <dd class="clip" title={displayText(share.signerName)}>
          {t("share.infoSignedAt", { name: displayText(share.signerName), when: dateTime(share.publishedAt) })}
        </dd>
      {/if}
      {#if share.role === "subscriber"}
        <dt>{t("share.infoLastSync")}</dt>
        <dd>{dateTime(share.lastSync)}</dd>
      {/if}
      {#if share.lastError}
        <dt>{t("share.infoLastError")}</dt>
        <dd class="error" title={displayText(share.lastError, ERROR_MAX)}>{displayText(share.lastError, ERROR_MAX)}</dd>
      {/if}
      {#if share.location}
        <dt>{t("share.infoLocation")}</dt>
        <dd class="clip mono" title={share.location.value}>{share.location.value}</dd>
      {/if}
      {#if share.role === "owner" && share.publishPath}
        <dt>{t("share.infoPublishPath")}</dt>
        <dd class="clip mono" title={share.publishPath}>{share.publishPath}</dd>
      {/if}
    </dl>

    {#if share.pendingTargets > 0}
      <section class="pending" aria-label={t("share.pendingTitle")}>
        <h4>{t("share.pendingTitle")}</h4>
        <p>{t("share.pendingBody", { count: share.pendingTargets })}</p>
        <ul class="hosts">
          {#each share.pendingHosts as host (host.nodeId)}
            <li class="wrap">{hostLine(host)}</li>
          {/each}
        </ul>
        <Button variant="primary" disabled={busy} onclick={confirm}>{t("share.confirm")}</Button>
      </section>
    {/if}
  {/if}

  {#snippet footer()}
    {#if share?.role === "owner"}
      <Button disabled={busy} onclick={publish}>{t("share.publishUpdate")}</Button>
    {/if}
    <Button variant="primary" data-autofocus onclick={onclose}>{t("common.close")}</Button>
  {/snippet}
</Dialog>

<style>
  .gap {
    margin-bottom: var(--space-3);
  }
  .hint {
    margin-bottom: var(--space-3);
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
  }
  .facts {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: var(--space-2) var(--space-4);
  }
  dt {
    color: var(--text-secondary);
  }
  dd {
    min-width: 0;
  }
  .clip {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .error {
    color: var(--danger-text);
    overflow-wrap: anywhere;
  }
  .wrap {
    overflow-wrap: anywhere;
  }
  .hosts {
    align-self: stretch;
    max-height: 12rem;
    overflow-y: auto;
    padding-left: var(--space-4);
    font-family: var(--font-mono);
    font-size: var(--font-size-sm);
    list-style: disc;
  }
  .pending {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
    margin-top: var(--space-4);
    padding-top: var(--space-4);
    border-top: 1px solid var(--border-hairline);
  }
  h4 {
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-semibold);
  }
</style>
