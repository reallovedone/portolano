<!-- "What's new", shown once on the first launch after an update: the notes of every version
     since the one that ran last, newest first, in one scrollable list. -->
<script lang="ts">
  import { t } from "../i18n/strings";
  import type { WhatsNew } from "../ipc";
  import { getApp } from "../state/app";
  import Button from "../ui/Button.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import ReleaseNotes from "./ReleaseNotes.svelte";

  let { info, onclose }: { info: WhatsNew; onclose: () => void } = $props();

  const { updates } = getApp();

  const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

  /** "0.3.1 — 18 Nov 2026"; just the version while it has no release date. */
  function heading(version: string, date: string | null): string {
    const match = date === null ? null : /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
    const month = match ? MONTHS[Number(match[2]) - 1] : undefined;
    if (!match || month === undefined) return version;
    return `${version} — ${Number(match[3])} ${month} ${match[1]}`;
  }
</script>

<Dialog title={t("whatsNew.title", { version: info.version })} onclose={onclose} size="md">
  <p class="intro">{t("whatsNew.intro", { version: info.version })}</p>
  <!-- A scrollable region must be reachable with the keyboard. -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div class="list" data-testid="whats-new-list" tabindex="0" role="region" aria-label={t("update.notesHeading")}>
    {#each info.sections as section (section.version)}
      <section data-testid="whats-new-section" data-version={section.version}>
        <h3>{heading(section.version, section.date)}</h3>
        <ReleaseNotes notes={section.notes} bounded={false} />
      </section>
    {/each}
    {#if info.truncated}
      <p class="more" data-testid="whats-new-more">
        {t("whatsNew.truncated")}
        <button type="button" class="link" onclick={() => void updates.openChangelog()}>
          {t("whatsNew.fullChangelog")}
        </button>
      </p>
    {/if}
  </div>
  {#snippet footer()}
    <Button variant="primary" data-autofocus onclick={onclose}>{t("common.close")}</Button>
  {/snippet}
</Dialog>

<style>
  .intro {
    margin: 0 0 var(--space-3);
    color: var(--text-secondary);
  }

  .list {
    max-height: 50vh;
    overflow: auto;
    padding: var(--space-3);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-control);
    background: var(--surface-inset);
  }

  section + section {
    margin-top: var(--space-4);
  }

  h3 {
    margin: 0;
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
  }

  /* The list already has the frame: the notes inside only add their text. */
  section :global(.notes) {
    padding: 0;
    border: 0;
    background: none;
  }

  .more {
    margin: var(--space-4) 0 0;
    color: var(--text-secondary);
  }

  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--text-link);
    font: inherit;
    text-decoration: underline;
    cursor: pointer;
  }
</style>
