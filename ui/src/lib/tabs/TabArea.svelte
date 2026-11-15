<script lang="ts">
  import { t } from "../i18n/strings";
  import { detectMac, shortcutLabel } from "../shortcuts";
  import { getApp } from "../state/app";
  import { defaultParentFor } from "../tree/model";
  import ChartBackdrop from "../ui/ChartBackdrop.svelte";
  import Kbd from "../ui/Kbd.svelte";
  import KindTile from "../ui/KindTile.svelte";
  import Logo from "../ui/Logo.svelte";
  import TabBar from "./TabBar.svelte";
  import TabPane from "./TabPane.svelte";
  import { TILE_LABEL, WELCOME_TILES, splitShortcut, type TileKind } from "./welcome";

  const { tabs, ui, doc } = getApp();
  const isMac = detectMac();
  let content: HTMLDivElement | undefined = $state();

  const newKeys = splitShortcut(shortcutLabel("newConnection", isMac));
  const searchKeys = splitShortcut(shortcutLabel("search", isMac));

  function measure(): void {
    if (!content) return;
    const rect = content.getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) {
      tabs.setGeometry({ x: rect.left, y: rect.top, width: rect.width, height: rect.height });
    }
  }

  // A tile opens the regular "new connection" flow on its kind, in the folder that is selected
  // in the tree.
  function startConnection(kind: TileKind): void {
    ui.open({
      type: "connection",
      id: null,
      parentId: defaultParentFor(doc.nodes, ui.selectedId),
      kind,
    });
  }

  $effect(() => {
    if (!content) return;
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(content);
    return () => observer.disconnect();
  });
</script>

<div class="area">
  <TabBar />
  <div bind:this={content} class="content" data-testid="tab-content">
    {#each tabs.tabs as tab (tab.id)}
      <TabPane {tab} />
    {/each}
    {#if tabs.tabs.length === 0}
      <div class="welcome" data-testid="welcome">
        <ChartBackdrop />
        <div class="hero">
          <span class="hero-logo"><Logo size={44} /></span>
          <h1 class="hero-title">{t("welcome.title")}</h1>
          <p class="hero-text">{t("welcome.text")}</p>
          <div class="hero-tiles" role="group" aria-label={t("welcome.tiles")}>
            {#each WELCOME_TILES as tile (tile.kind)}
              <KindTile
                kind={tile.kind}
                label={t(TILE_LABEL[tile.kind])}
                hint={tile.port === null ? t("welcome.localHint") : String(tile.port)}
                data-kind={tile.kind}
                onclick={() => startConnection(tile.kind)}
              />
            {/each}
          </div>
          <p class="hero-keys">
            {#each newKeys as key, i (i)}<Kbd>{key}</Kbd>{/each}
            <span>{t("welcome.newConnection")}</span>
            <span class="hero-sep" aria-hidden="true">·</span>
            {#each searchKeys as key, i (i)}<Kbd>{key}</Kbd>{/each}
            <span>{t("welcome.search")}</span>
          </p>
        </div>
      </div>
    {/if}
  </div>
</div>

<style>
  .area {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    height: 100%;
    background: var(--surface-window);
  }

  .content {
    position: relative;
    flex: 1;
    min-height: 0;
    background: var(--surface-content);
  }

  .welcome {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: auto;
  }

  .hero {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-6);
    text-align: center;
  }

  .hero-logo {
    display: inline-flex;
    color: var(--text-primary);
  }

  .hero-title {
    font-size: var(--font-size-hero);
    font-weight: var(--font-weight-semibold);
    line-height: var(--line-height-tight);
    color: var(--text-primary);
  }

  .hero-text {
    max-width: calc(var(--tooltip-max-width) * 1.5);
    color: var(--text-secondary);
    line-height: var(--line-height-relaxed);
  }

  .hero-tiles {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: var(--space-2);
    margin: var(--space-4) 0 var(--space-3);
  }

  .hero-keys {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: center;
    gap: var(--space-1);
    color: var(--text-tertiary);
    font-size: var(--font-size-sm);
  }

  .hero-sep {
    margin: 0 var(--space-1);
  }
</style>
