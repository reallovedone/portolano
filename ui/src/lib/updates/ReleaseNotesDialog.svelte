<!-- "Release notes": every version of the changelog built into the application, newest first,
     with the screenshots of the versions that have them. Opened from Help → "Release Notes…" and
     from Settings, and by the link in "What's new" when it left older versions out. -->
<script lang="ts">
  import { onMount } from "svelte";

  import { t } from "../i18n/strings";
  import { toIpcError, type ReleaseNotesList } from "../ipc";
  import { getApp } from "../state/app";
  import Button from "../ui/Button.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import ReleaseList from "./ReleaseList.svelte";

  let { onclose }: { onclose: () => void } = $props();

  const { ipc, updates } = getApp();

  let list = $state<ReleaseNotesList | null>(null);
  let failure = $state<string | null>(null);

  onMount(() => {
    let alive = true;
    ipc.releaseNotes().then(
      (result) => {
        if (alive) list = result;
      },
      (e: unknown) => {
        if (alive) failure = toIpcError(e).message;
      },
    );
    return () => {
      alive = false;
    };
  });
</script>

<Dialog title={t("releaseNotes.title")} {onclose} size="xl">
  {#if list}
    <ReleaseList
      sections={list.sections}
      truncated={list.truncated}
      moreText={t("releaseNotes.truncated")}
      moreLabel={t("releaseNotes.fullChangelog")}
      onmore={() => void updates.openChangelog()}
      maxHeight="65vh"
      testid="release-notes-list"
    />
  {:else if failure !== null}
    <p class="state" role="alert" data-testid="release-notes-error">
      {t("releaseNotes.loadFailed")} {failure}
    </p>
  {:else}
    <p class="state" role="status">{t("releaseNotes.loading")}</p>
  {/if}
  {#snippet footer()}
    <Button onclick={() => void updates.openChangelog()}>{t("releaseNotes.viewOnGitHub")}</Button>
    <Button variant="primary" data-autofocus onclick={onclose}>{t("common.close")}</Button>
  {/snippet}
</Dialog>

<style>
  .state {
    margin: 0;
    color: var(--text-secondary);
  }
</style>
