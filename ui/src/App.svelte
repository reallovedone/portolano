<script lang="ts">
  import { onMount } from "svelte";

  import ConnectionEditor from "./lib/editors/ConnectionEditor.svelte";
  import FolderEditor from "./lib/editors/FolderEditor.svelte";
  import { t } from "./lib/i18n/strings";
  import { ipc, ipcMode } from "./lib/ipc";
  import { detectMac, matchShortcut } from "./lib/shortcuts";
  import { createApp, provideApp, type App } from "./lib/state/app";
  import { connectBackendEvents } from "./lib/state/backend-events";
  import { isNumber, loadJson, saveJson } from "./lib/state/storage";
  import TabArea from "./lib/tabs/TabArea.svelte";
  import ConnectionTree from "./lib/tree/ConnectionTree.svelte";
  import ImportDialog from "./lib/transfer/ImportDialog.svelte";
  import { defaultParentFor } from "./lib/tree/model";
  import ConfirmDialog from "./lib/ui/ConfirmDialog.svelte";
  import Splitter from "./lib/ui/Splitter.svelte";
  import StatusBar from "./lib/ui/StatusBar.svelte";
  import Toasts from "./lib/ui/Toasts.svelte";
  import SettingsDialog from "./lib/settings/SettingsDialog.svelte";
  import UpdateCheckDialog from "./lib/updates/UpdateCheckDialog.svelte";
  import UpdateOfferDialog from "./lib/updates/UpdateOfferDialog.svelte";
  import WhatsNewDialog from "./lib/updates/WhatsNewDialog.svelte";
  import CreateVaultDialog from "./lib/vault/CreateVaultDialog.svelte";
  import CredentialManager from "./lib/vault/CredentialManager.svelte";
  import UnlockDialog from "./lib/vault/UnlockDialog.svelte";
  import VaultBadge from "./lib/vault/VaultBadge.svelte";

  let {
    app = createApp(ipc),
    demo = ipcMode === "mock",
  }: { app?: App; demo?: boolean } = $props();

  // The app object is fixed for the lifetime of the shell.
  // svelte-ignore state_referenced_locally
  const { doc, vault, tabs, closeConfirm, ui, overlay, notices, updates } = app;
  // svelte-ignore state_referenced_locally
  provideApp(app);

  const isMac = detectMac();
  const SIDEBAR_KEY = "portolano.ui.sidebarWidth";
  let sidebarWidth = $state(loadJson(SIDEBAR_KEY, 280, isNumber));

  $effect(() => {
    saveJson(SIDEBAR_KEY, sidebarWidth);
  });

  // Notices the backend sends on its own: the vault locking itself, a failed "save to vault".
  onMount(() => connectBackendEvents(app));
  // The saved appearance replaces the copy the first paint used (a change made on another run).
  onMount(() => void app.appearance.load());
  // A reload starts with the native Edit menu in place, whatever an earlier page left behind.
  onMount(() => {
    app.ipc
      .menuSetSessionKeys(false)
      .catch((e: unknown) =>
        app.ipc
          .uiLog("warn", `menu keys failed: ${e instanceof Error ? e.message : String(e)}`)
          .catch(() => undefined),
      );
  });

  onMount(() => {
    void (async () => {
      const [, status] = await Promise.all([doc.load(), vault.refresh()]);
      if (!status.ok) notices.error(t("app.vaultStatusFailed"), status.error);
      try {
        await app.ipc.uiLog(
          "info",
          `shell mounted (ipc=${ipcMode}) vault_status=${vault.status} doc_nodes=${doc.nodes.length}`,
        );
      } catch (e) {
        notices.error(t("app.logFailed"), e);
      }
    })();
  });

  // "What's new" (after an update), then the automatic update check, never blocking start-up.
  onMount(() => updates.start());

  // An update found by a scheduled check waits until "What's new" is closed and no other dialog
  // or menu is open; one asked for with "Check for updates…" is shown at once.
  let offerVisible = $state(false);
  $effect(() => {
    const offer = updates.offer;
    if (!offer) {
      offerVisible = false;
    } else if (!offerVisible && updates.whatsNew === null && (offer.manual || !overlay.open)) {
      offerVisible = true;
    }
  });

  function onKeydown(event: KeyboardEvent): void {
    if (event.defaultPrevented || overlay.open) return;
    const action = matchShortcut(event, isMac);
    if (!action) return;
    event.preventDefault();
    switch (action) {
      case "newConnection":
        ui.open({
          type: "connection",
          id: null,
          parentId: defaultParentFor(doc.nodes, ui.selectedId),
        });
        break;
      case "search":
        ui.focusSearch();
        break;
      case "closeTab":
        if (tabs.state.activeId) void closeConfirm.closeTab(tabs.state.activeId);
        break;
      case "reconnectTab":
        if (!event.repeat && tabs.state.activeId) void tabs.reconnect(tabs.state.activeId);
        break;
      case "openHome":
        if (!event.repeat) tabs.openHome();
        break;
      case "nextTab":
        tabs.next();
        break;
      case "prevTab":
        tabs.previous();
        break;
      case "lockVault":
        if (vault.unlocked) {
          void vault.lock().then((result) => {
            if (!result.ok) notices.error(t("vault.lock"), result.error);
          });
        }
        break;
    }
  }

  // The backend can lock the vault on inactivity: re-read the status when the window returns.
  function onFocus(): void {
    void vault.refresh();
  }
</script>

<svelte:window onkeydown={onKeydown} onfocus={onFocus} />

<div class="shell" style:--sidebar-width="{sidebarWidth}px">
  <aside class="sidebar" data-testid="sidebar" aria-label={t("sidebar.title")}>
    <!-- The tree's header row is the title-bar strip next to the traffic lights. -->
    <ConnectionTree />
    <VaultBadge />
  </aside>
  <Splitter bind:width={sidebarWidth} />
  <main class="main" data-testid="tab-area">
    <TabArea />
  </main>
  <div class="statusrow"><StatusBar {demo} /></div>
</div>

<Toasts />

{#if vault.request}
  {#if vault.status === "notCreated"}
    <CreateVaultDialog onclose={(created) => vault.finishRequest(created)} />
  {:else}
    <UnlockDialog
      reason={vault.request.reason}
      allowManual={vault.manualAllowed}
      onmanual={() => vault.finishRequestManual()}
      onclose={(unlocked) => vault.finishRequest(unlocked)}
    />
  {/if}
{:else if ui.dialog?.type === "createVault"}
  <CreateVaultDialog onclose={() => ui.closeDialog()} />
{/if}

{#if ui.dialog?.type === "connection"}
  <ConnectionEditor
    id={ui.dialog.id}
    parentId={ui.dialog.parentId}
    kind={ui.dialog.kind}
    onclose={() => ui.closeDialog()}
  />
{:else if ui.dialog?.type === "folder"}
  <FolderEditor id={ui.dialog.id} parentId={ui.dialog.parentId} onclose={() => ui.closeDialog()} />
{:else if ui.dialog?.type === "credentials"}
  <CredentialManager onclose={() => ui.closeDialog()} />
{:else if ui.dialog?.type === "import"}
  <ImportDialog path={ui.dialog.path} onclose={() => ui.closeDialog()} />
{:else if ui.dialog?.type === "updates"}
  <UpdateCheckDialog onclose={() => ui.closeDialog()} />
{:else if ui.dialog?.type === "settings"}
  <SettingsDialog onclose={() => ui.closeDialog()} />
{/if}

{#if updates.whatsNew}
  <WhatsNewDialog info={updates.whatsNew} onclose={() => updates.dismissWhatsNew()} />
{:else if offerVisible}
  <UpdateOfferDialog />
{/if}

{#if closeConfirm.pending}
  <ConfirmDialog
    title={closeConfirm.title}
    message={t("closeConfirm.message")}
    confirmLabel={t("closeConfirm.confirm")}
    onconfirm={() => closeConfirm.answer(true)}
    oncancel={() => closeConfirm.answer(false)}
  />
{/if}

<style>
  .shell {
    display: grid;
    grid-template-columns: var(--sidebar-width) 0 minmax(0, 1fr);
    grid-template-rows: minmax(0, 1fr) auto;
    height: 100%;
    background: var(--surface-window);
  }

  .sidebar {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--surface-sidebar);
    border-right: 1px solid var(--border-hairline);
  }

  .main {
    min-width: 0;
    min-height: 0;
  }

  .statusrow {
    grid-column: 1 / -1;
  }
</style>
