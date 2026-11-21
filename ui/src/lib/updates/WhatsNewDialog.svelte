<!-- "What's new", shown once on the first launch after an update: the notes of every version
     since the one that ran last, newest first, in one scrollable list. -->
<script lang="ts">
  import { t } from "../i18n/strings";
  import type { WhatsNew } from "../ipc";
  import Button from "../ui/Button.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import ReleaseList from "./ReleaseList.svelte";

  let {
    info,
    onclose,
    onfullchangelog,
  }: {
    info: WhatsNew;
    onclose: () => void;
    /** The link shown when older versions were left out: the full list of release notes. */
    onfullchangelog: () => void;
  } = $props();
</script>

<Dialog title={t("whatsNew.title", { version: info.version })} onclose={onclose} size="md">
  <p class="intro">{t("whatsNew.intro", { version: info.version })}</p>
  <ReleaseList
    sections={info.sections}
    truncated={info.truncated}
    moreText={t("whatsNew.truncated")}
    moreLabel={t("whatsNew.fullChangelog")}
    onmore={onfullchangelog}
    testid="whats-new-list"
  />
  {#snippet footer()}
    <Button variant="primary" data-autofocus onclick={onclose}>{t("common.close")}</Button>
  {/snippet}
</Dialog>

<style>
  .intro {
    margin: 0 0 var(--space-3);
    color: var(--text-secondary);
  }
</style>
