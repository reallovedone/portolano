<!-- One screenshot with its caption, in the current look (light or dark). If the picture cannot be
     loaded (not published yet, offline), the whole figure disappears: no broken-image icon and no
     empty frame. A look that failed is remembered, so a switch to the other look tries again. -->
<script lang="ts">
  import {
    SCREENSHOT_HEIGHT,
    SCREENSHOT_WIDTH,
    screenshotUrl,
    type Screenshot,
    type ShotTheme,
  } from "./screenshots";

  let { version, shot, theme }: { version: string; shot: Screenshot; theme: ShotTheme } = $props();

  let failed = $state<ShotTheme[]>([]);
  const src = $derived(failed.includes(theme) ? null : screenshotUrl(version, shot.name, theme));
</script>

{#if src}
  <figure data-testid="release-shot" data-name={shot.name}>
    <img
      {src}
      alt={shot.alt}
      width={SCREENSHOT_WIDTH}
      height={SCREENSHOT_HEIGHT}
      loading="lazy"
      decoding="async"
      referrerpolicy="no-referrer"
      onerror={() => (failed = [...failed, theme])}
    />
    <figcaption>{shot.alt}</figcaption>
  </figure>
{/if}

<style>
  figure {
    margin: var(--space-3) 0 0;
  }

  img {
    display: block;
    width: 100%;
    height: auto;
    aspect-ratio: 16 / 10;
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-panel);
    background: var(--surface-inset);
  }

  figcaption {
    margin-top: var(--space-1);
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
  }
</style>
