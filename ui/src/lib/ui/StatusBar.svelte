<script lang="ts">
  import { t, type StringKey } from "../i18n/strings";
  import { getApp } from "../state/app";
  import { isConnection } from "../tree/model";

  let { demo = false }: { demo?: boolean } = $props();

  const { vault, tabs, doc, ui, updates } = getApp();

  const version = $derived(updates.settings?.currentVersion ?? "");
  const freshnessKey = $derived(`status.update.${updates.freshness}` as StringKey);

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
  <span class="version">
    {#if version}<span>{t("status.version", { version })}</span>{/if}
    <button
      type="button"
      class="dot {updates.freshness}"
      aria-label={t(freshnessKey)}
      title={t(freshnessKey)}
      onclick={() => ui.open({ type: "updates" })}
    ></button>
  </span>
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

  .version {
    display: flex;
    gap: var(--space-1);
    align-items: center;
    margin-left: auto;
  }

  .dot {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--icon-size-md);
    height: var(--icon-size-md);
    padding: 0;
    border: none;
    border-radius: var(--radius-full);
    background: transparent;
    cursor: pointer;
  }

  .dot::before {
    width: var(--space-2);
    height: var(--space-2);
    border-radius: var(--radius-full);
    content: "";
  }

  .dot.upToDate::before {
    background: var(--success-text);
  }

  .dot.available::before {
    background: var(--warning-text);
  }

  .dot.unknown::before {
    background: var(--text-tertiary);
  }

  .dot:focus-visible {
    outline: var(--focus-ring-width) solid var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
</style>
