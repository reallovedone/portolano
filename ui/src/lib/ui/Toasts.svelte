<script lang="ts">
  import { t } from "../i18n/strings";
  import { getApp } from "../state/app";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";

  const { notices } = getApp();
</script>

<div class="toasts" role="region" aria-live="polite" aria-label="Notifications">
  {#each notices.items as notice (notice.id)}
    <div
      class="toast"
      class:error={notice.kind === "error"}
      class:warning={notice.kind === "warning"}
      role={notice.kind === "error" ? "alert" : "status"}
    >
      <span class="glyph">
        <Icon name={notice.kind === "error" ? "error" : notice.kind === "warning" ? "warning" : "info"} />
      </span>
      <div class="text">
        <strong>{notice.title}</strong>
        {#if notice.detail}<div class="detail">{notice.detail}</div>{/if}
      </div>
      <IconButton
        icon="close"
        label={t("common.dismiss")}
        size="compact"
        tooltip=""
        onclick={() => notices.dismiss(notice.id)}
      />
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: var(--space-3);
    bottom: calc(var(--space-8) + var(--space-1));
    z-index: var(--z-toast);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    width: min(var(--toast-width), calc(100vw - var(--space-6)));
  }

  .toast {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-3);
    background: var(--surface-raised);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-panel);
    box-shadow: var(--shadow-popover);
    animation: slide var(--duration-slow) var(--ease-out);
  }

  .glyph {
    display: inline-flex;
    padding-top: 1px;
    color: var(--info-text);
  }

  .toast.error .glyph {
    color: var(--danger-text);
  }

  .toast.warning .glyph {
    color: var(--warning-text);
  }

  .text {
    flex: 1;
    min-width: 0;
  }

  strong {
    font-weight: var(--font-weight-semibold);
  }

  .detail {
    color: var(--text-secondary);
    overflow-wrap: anywhere;
  }

  @keyframes slide {
    from {
      opacity: 0;
      transform: translateY(var(--space-2));
    }
  }
</style>
