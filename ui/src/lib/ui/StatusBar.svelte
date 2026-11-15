<script lang="ts">
  import { t, type StringKey } from "../i18n/strings";
  import { getApp } from "../state/app";
  import { isConnection } from "../tree/model";

  let { demo = false }: { demo?: boolean } = $props();

  const { vault, tabs, doc } = getApp();

  const vaultKey = $derived(
    (vault.status === "unknown" ? "vault.locked" : `vault.${vault.status}`) as StringKey,
  );
  const activeNode = $derived(tabs.activeTab ? doc.node(tabs.activeTab.connectionId) : undefined);
</script>

<footer class="status" data-testid="statusbar">
  <span>{t("status.vault", { state: t(vaultKey) })}</span>
  <span>{t("status.sessions", { count: tabs.tabs.length })}</span>
  {#if activeNode && isConnection(activeNode)}
    <span class="active">
      {t(`kind.${activeNode.options.type}` as StringKey)}
      {#if activeNode.host}· {activeNode.host}{activeNode.port ? `:${activeNode.port}` : ""}{/if}
    </span>
  {/if}
  {#if demo}<span class="demo">{t("status.ipcMock")}</span>{/if}
</footer>

<style>
  .status {
    display: flex;
    gap: var(--space-4);
    flex: none;
    align-items: center;
    min-height: var(--control-h-compact);
    padding: 0 var(--space-3);
    border-top: 1px solid var(--border-hairline);
    background: var(--surface-window);
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
  }

  .demo {
    margin-left: auto;
  }
</style>
