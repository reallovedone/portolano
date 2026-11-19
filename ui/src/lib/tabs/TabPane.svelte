<!-- One tab's content: progress, error with retry, or the session component. Inactive panes
     stay mounted (hidden) so terminals and remote desktops keep their state. -->
<script lang="ts">
  import { t } from "../i18n/strings";
  import { getApp } from "../state/app";
  import Banner from "../ui/Banner.svelte";
  import Button from "../ui/Button.svelte";
  import ChartBackdrop from "../ui/ChartBackdrop.svelte";
  import EmptyState from "../ui/EmptyState.svelte";
  import Icon from "../ui/Icon.svelte";
  import Spinner from "../ui/Spinner.svelte";
  import HomeView from "./HomeView.svelte";
  import type { Tab } from "./model";
  import SessionSlot from "./SessionSlot.svelte";

  let { tab }: { tab: Tab } = $props();

  const { tabs } = getApp();
  const active = $derived(tabs.state.activeId === tab.id);
  const locked = $derived(tab.type === "connection" && tab.error?.code === "vault_locked");
</script>

<div
  class="pane"
  class:hidden={!active}
  class:terminal={tab.type === "connection" && (tab.kind === "localShell" || tab.kind === "ssh")}
  role="tabpanel"
  id="tabpanel-{tab.id}"
  aria-labelledby="tab-{tab.id}"
  data-tab-id={tab.id}
  inert={!active}
>
  {#if tab.type === "home"}
    <HomeView testid="home" />
  {:else if tab.status === "opening"}
    <div class="state" role="status">
      <ChartBackdrop />
      <div class="state-body">
        <EmptyState title={t("tabs.opening")}>
          {#snippet graphic()}<Spinner size={20} />{/snippet}
        </EmptyState>
      </div>
    </div>
  {:else if tab.status === "failed"}
    <div class="state failure" role="alert">
      <ChartBackdrop />
      <div class="state-body">
        <EmptyState
          icon="unplug"
          title={t("tabs.failedTitle")}
          text={tab.error?.code === "unsupported" ? t("tabs.unsupported") : undefined}
        >
          {#snippet action()}
            <div class="detail">
              <Banner variant="danger" role="note">{tab.error?.message}</Banner>
              <p class="code">{t("tabs.errorCode", { code: tab.error?.code ?? "" })}</p>
              <div class="buttons">
                {#if locked}
                  <Button variant="primary" onclick={() => tabs.unlockAndRetry(tab.id)}>
                    {#snippet icon()}<Icon name="lock-open" />{/snippet}
                    {t("tabs.unlockAndRetry")}
                  </Button>
                  <Button onclick={() => tabs.enterManuallyAndRetry(tab.id)}>
                    {#snippet icon()}<Icon name="key" />{/snippet}
                    {t("tabs.enterManually")}
                  </Button>
                {/if}
                <Button variant={locked ? "secondary" : "primary"} onclick={() => tabs.retry(tab.id)}>
                  {#snippet icon()}<Icon name="refresh" />{/snippet}
                  {t("tabs.retry")}
                </Button>
                <Button variant="plain" onclick={() => tabs.close(tab.id)}>
                  {t("tabs.close")}
                </Button>
              </div>
            </div>
          {/snippet}
        </EmptyState>
      </div>
    </div>
  {:else if tab.sessionId}
    {#key tab.sessionId}
      <SessionSlot {tab} sessionId={tab.sessionId} />
    {/key}
  {/if}
</div>

<style>
  .pane {
    position: absolute;
    inset: 0;
    overflow: auto;
  }

  /* A terminal fills the pane edge to edge with its own surface, padding included. */
  .pane.terminal {
    background: var(--terminal-bg);
  }

  .pane.hidden {
    visibility: hidden;
  }

  /* Opening and failed states: centred message over the chart decoration. */
  .state {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 100%;
  }

  .state-body {
    position: relative;
    max-width: var(--dialog-width-md);
  }

  .detail {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: center;
    text-align: start;
  }

  .code {
    color: var(--text-tertiary);
    font-family: var(--font-mono);
    font-size: var(--font-size-xs);
  }

  .buttons {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: var(--space-2);
    margin-top: var(--space-1);
  }
</style>
