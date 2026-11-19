<script lang="ts">
  import { getApp } from "../state/app";
  import HomeView from "./HomeView.svelte";
  import TabBar from "./TabBar.svelte";
  import TabPane from "./TabPane.svelte";

  const { tabs } = getApp();
  let content: HTMLDivElement | undefined = $state();

  function measure(): void {
    if (!content) return;
    const rect = content.getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) {
      tabs.setGeometry({ x: rect.left, y: rect.top, width: rect.width, height: rect.height });
    }
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
      <HomeView />
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
</style>
