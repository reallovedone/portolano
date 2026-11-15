<!-- Top row of the sidebar. On macOS it sits in the integrated title bar next to the traffic
     lights (the row itself is a window drag handle; the brand ignores the pointer so a press on
     it falls through to the row). Buttons stay outside the drag handle and remain clickable. -->
<script lang="ts">
  import { t } from "../i18n/strings";
  import IconButton from "../ui/IconButton.svelte";
  import Logo from "../ui/Logo.svelte";
  import Toolbar from "../ui/Toolbar.svelte";

  let {
    onnewconnection,
    onnewfolder,
    onmore,
  }: {
    onnewconnection: () => void;
    onnewfolder: () => void;
    /** Receives the click so the caller can anchor a menu to the button. */
    onmore: (event: MouseEvent) => void;
  } = $props();
</script>

<div class="header" data-tauri-drag-region data-testid="sidebar-header">
  <span class="brand">
    <Logo size={20} />
    <span class="app-name">{t("app.name")}</span>
  </span>
  <div class="actions">
    <Toolbar label={t("tree.toolbar")} flush>
      <IconButton
        icon="plus"
        size="compact"
        label={t("tree.newConnection")}
        onclick={onnewconnection}
      />
      <IconButton icon="folder-plus" size="compact" label={t("tree.newFolder")} onclick={onnewfolder} />
      <IconButton icon="more" size="compact" label={t("tree.more")} onclick={onmore} />
    </Toolbar>
  </div>
</div>

<style>
  .header {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--space-2);
    min-height: max(var(--titlebar-height), calc(var(--control-h-large) + var(--space-2)));
    padding-block: var(--space-1);
    padding-inline: calc(var(--titlebar-safe-left) + var(--space-2)) var(--space-1);
  }

  .brand {
    display: flex;
    flex: 1;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    color: var(--text-primary);
    pointer-events: none;
  }

  .app-name {
    overflow: hidden;
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .actions {
    flex: none;
  }
</style>
