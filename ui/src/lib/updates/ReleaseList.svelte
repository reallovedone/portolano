<!-- The scrollable list of versions, newest first, with the note about older ones left out. Shared
     by "What's new" and "Release notes". Screenshots follow the look the page has now. -->
<script lang="ts">
  import { t } from "../i18n/strings";
  import type { ReleaseSection } from "../ipc";
  import Button from "../ui/Button.svelte";
  import ReleaseVersion from "./ReleaseVersion.svelte";
  import { useResolvedTheme } from "./resolvedTheme.svelte";

  let {
    sections,
    truncated,
    moreText,
    moreLabel,
    onmore,
    maxHeight = "50vh",
    testid,
  }: {
    sections: ReleaseSection[];
    truncated: boolean;
    /** The sentence shown when older versions were left out. */
    moreText: string;
    /** The link after it. */
    moreLabel: string;
    onmore: () => void;
    maxHeight?: string;
    testid: string;
  } = $props();

  const theme = useResolvedTheme();
</script>

<!-- A scrollable region must be reachable with the keyboard. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div
  class="list"
  data-testid={testid}
  tabindex="0"
  role="region"
  aria-label={t("update.notesHeading")}
  style:max-height={maxHeight}
>
  {#each sections as section (section.version)}
    <ReleaseVersion {section} theme={theme.current} />
  {/each}
  {#if truncated}
    <p class="more" data-testid="{testid}-more">
      {moreText}
      <Button variant="link" onclick={onmore}>{moreLabel}</Button>
    </p>
  {/if}
</div>

<style>
  .list {
    overflow: auto;
    padding: var(--space-3);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-control);
    background: var(--surface-inset);
  }

  .more {
    margin: var(--space-4) 0 0;
    color: var(--text-secondary);
  }
</style>
