<!-- Inline message: info, warning, danger or success. Used for security warnings and form-level
     errors. Danger and warning announce themselves (role="alert"). -->
<script lang="ts">
  import type { Snippet } from "svelte";

  import { t } from "../i18n/strings";
  import type { IconName } from "../design/icons/registry";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";

  let {
    variant = "info",
    title,
    icon,
    role,
    ondismiss,
    children,
    actions,
  }: {
    variant?: "info" | "warning" | "danger" | "success";
    title?: string;
    /** Overrides the default icon of the variant. */
    icon?: IconName;
    /** Defaults to "alert" for danger and warning, "status" otherwise. */
    role?: "alert" | "status" | "note";
    ondismiss?: () => void;
    children?: Snippet;
    actions?: Snippet;
  } = $props();

  const ICON_FOR: Record<string, IconName> = {
    info: "info",
    warning: "warning",
    danger: "shield-alert",
    success: "success",
  };
  const iconName = $derived(icon ?? ICON_FOR[variant] ?? "info");
  const liveRole = $derived(role ?? (variant === "danger" || variant === "warning" ? "alert" : "status"));
</script>

<div class="banner {variant}" role={liveRole}>
  <span class="glyph"><Icon name={iconName} /></span>
  <div class="text">
    {#if title}<strong class="title">{title}</strong>{/if}
    {#if children}<div class="body">{@render children()}</div>{/if}
    {#if actions}<div class="actions">{@render actions()}</div>{/if}
  </div>
  {#if ondismiss}
    <IconButton icon="close" label={t("common.dismiss")} size="compact" onclick={ondismiss} />
  {/if}
</div>

<style>
  .banner {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    font-size: var(--font-size-md);
    line-height: var(--line-height-normal);
  }
  .glyph {
    display: inline-flex;
    padding-top: 1px;
  }
  .text {
    flex: 1;
    min-width: 0;
  }
  .title {
    display: block;
    font-weight: var(--font-weight-semibold);
  }
  .body {
    overflow-wrap: anywhere;
    white-space: pre-wrap;
  }
  .actions {
    display: flex;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }
  .info {
    background: var(--info-bg);
    border-color: var(--info-border);
    color: var(--info-text);
  }
  .warning {
    background: var(--warning-bg);
    border-color: var(--warning-border);
    color: var(--warning-text);
  }
  .danger {
    background: var(--danger-bg);
    border-color: var(--danger-border);
    color: var(--danger-text);
  }
  .success {
    background: var(--success-bg);
    border-color: var(--success-border);
    color: var(--success-text);
  }
</style>
