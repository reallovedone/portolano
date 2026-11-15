<!-- Vault status in the sidebar footer: a quiet status line (icon + state) with the actions
     that apply to the current state as small buttons. -->
<script lang="ts">
  import { t, type StringKey } from "../i18n/strings";
  import { getApp } from "../state/app";
  import Button from "../ui/Button.svelte";
  import Icon from "../ui/Icon.svelte";
  import IconButton from "../ui/IconButton.svelte";

  const { vault, ui, notices } = getApp();

  const stateKey = $derived(
    (vault.status === "unknown" ? "vault.locked" : `vault.${vault.status}`) as StringKey,
  );

  async function lock(): Promise<void> {
    const result = await vault.lock();
    if (!result.ok) notices.error(t("vault.lock"), result.error);
  }
</script>

<div class="vault">
  <span
    class="status"
    data-state={vault.status}
    role="status"
    aria-label={t("vault.badgeLabel", { state: t(stateKey) })}
  >
    <span class="glyph">
      <Icon name={vault.status === "unlocked" ? "lock-open" : vault.status === "notCreated" ? "shield" : "lock"} />
    </span>
    <span class="text">{t(stateKey)}</span>
  </span>
  <span class="actions">
    {#if vault.status === "notCreated"}
      <Button variant="plain" size="compact" onclick={() => ui.open({ type: "createVault" })}>
        {t("vault.create")}
      </Button>
    {:else if vault.status === "locked"}
      <Button variant="plain" size="compact" onclick={() => vault.ensureUnlocked(null)}>
        {t("vault.unlock")}
      </Button>
    {:else if vault.status === "unlocked"}
      <IconButton
        icon="key"
        size="compact"
        label={t("vault.credentials")}
        onclick={() => ui.open({ type: "credentials" })}
      />
      <IconButton icon="lock" size="compact" label={t("vault.lock")} onclick={lock} />
    {/if}
  </span>
</div>

<style>
  .vault {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    min-height: var(--control-h-large);
    padding: var(--space-1) var(--space-2);
    border-top: 1px solid var(--border-hairline);
  }

  .status {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    color: var(--text-secondary);
  }

  .glyph {
    display: inline-flex;
    color: var(--text-tertiary);
  }

  .status[data-state="unlocked"] .glyph {
    color: var(--success-text);
  }

  .status[data-state="locked"] .glyph,
  .status[data-state="unknown"] .glyph {
    color: var(--warning-text);
  }

  .text {
    overflow: hidden;
    font-size: var(--font-size-sm);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .actions {
    display: inline-flex;
    flex: none;
    align-items: center;
    gap: var(--space-1);
  }
</style>
