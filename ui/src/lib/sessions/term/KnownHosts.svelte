<!-- Minimal known-hosts list with a remove action. Entries for `host:port` are marked. -->
<script lang="ts">
  import { onMount } from "svelte";

  import { toIpcError, type KnownHostEntry } from "../../ipc";
  import { t } from "../../i18n/strings";
  import { getApp } from "../../state/app";
  import Badge from "../../ui/Badge.svelte";
  import ConfirmDialog from "../../ui/ConfirmDialog.svelte";
  import Icon from "../../ui/Icon.svelte";
  import IconButton from "../../ui/IconButton.svelte";
  import ListRow from "../../ui/ListRow.svelte";
  import FingerprintBlock from "../FingerprintBlock.svelte";

  let {
    host = null,
    port = null,
    onRemoved,
  }: {
    host?: string | null;
    port?: number | null;
    onRemoved?: (entry: KnownHostEntry) => void;
  } = $props();

  const { ipc } = getApp();

  let entries = $state<KnownHostEntry[]>([]);
  let loaded = $state(false);
  let error = $state<string | null>(null);
  let busy = $state(false);
  /** The entry the user asked to remove; the removal itself waits for the confirmation. */
  let pending = $state<KnownHostEntry | null>(null);

  const isThis = (e: KnownHostEntry): boolean =>
    host !== null && port !== null && e.host.toLowerCase() === host.toLowerCase() && e.port === port;

  async function load(): Promise<void> {
    try {
      entries = await ipc.knownHostsList();
      error = null;
    } catch (e) {
      error = t("knownHosts.loadFailed", { message: toIpcError(e).message });
    } finally {
      loaded = true;
    }
  }

  async function remove(entry: KnownHostEntry): Promise<void> {
    pending = null;
    busy = true;
    try {
      await ipc.knownHostsRemove(entry.host, entry.port);
      error = null;
      onRemoved?.(entry);
      await load();
    } catch (e) {
      error = t("knownHosts.removeFailed", { message: toIpcError(e).message });
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    void load();
  });

  // Entries of this connection first.
  const sorted = $derived([...entries].sort((a, b) => Number(isThis(b)) - Number(isThis(a))));
</script>

<section class="known" aria-label={t("knownHosts.title")}>
  <h3 class="heading">{t("knownHosts.title")}</h3>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
  {#if loaded && entries.length === 0 && !error}
    <p class="muted">{t("knownHosts.empty")}</p>
  {/if}
  {#if sorted.length > 0}
    <ul class="list">
      {#each sorted as entry (`${entry.host}:${entry.port}:${entry.algorithm}:${entry.fingerprint}`)}
        <li class="entry" class:match={isThis(entry)} data-testid="known-host">
          <ListRow density="compact">
            {#snippet leading()}<Icon name="server" />{/snippet}
            <span class="where">{entry.host}:{entry.port}</span>
            {#snippet secondary()}{entry.algorithm}{/snippet}
            {#snippet trailing()}
              {#if isThis(entry)}<Badge variant="danger">{t("knownHosts.thisHost")}</Badge>{/if}
              <IconButton
                icon="trash"
                size="compact"
                label={t("knownHosts.removeLabel", { host: entry.host, port: entry.port })}
                disabled={busy}
                onclick={() => (pending = entry)}
              />
            {/snippet}
          </ListRow>
          <code class="print">{entry.fingerprint}</code>
        </li>
      {/each}
    </ul>
  {/if}
</section>

{#if pending}
  {@const entry = pending}
  <ConfirmDialog
    title={t("knownHosts.confirmTitle")}
    message={t("knownHosts.confirmText", { host: entry.host, port: entry.port })}
    confirmLabel={t("knownHosts.remove")}
    onconfirm={() => void remove(entry)}
    oncancel={() => (pending = null)}
  />
{/if}

<style>
  .known {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    width: 100%;
    text-align: left;
  }

  .heading {
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-semibold);
    color: var(--text-primary);
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-height: calc(var(--dialog-width-md) / 2);
    margin: 0;
    padding: 0;
    overflow: auto;
    list-style: none;
  }

  .entry {
    padding: var(--space-1) var(--space-1) var(--space-2);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-control);
  }
  .entry.match {
    border-color: var(--danger-border);
    background: var(--danger-bg);
  }

  .where {
    font-family: var(--font-mono);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
  }

  .print {
    display: block;
    margin: 0 var(--space-2);
    font-family: var(--font-mono);
    font-size: var(--font-size-xs);
    color: var(--text-secondary);
    overflow-wrap: anywhere;
    user-select: all;
  }

  .muted {
    color: var(--text-secondary);
    font-size: var(--font-size-md);
  }

  .error {
    color: var(--danger-text);
    font-size: var(--font-size-md);
  }
</style>
