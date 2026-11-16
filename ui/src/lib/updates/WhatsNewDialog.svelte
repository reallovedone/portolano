<!-- "What's new in <version>", shown once on the first launch after an update. -->
<script lang="ts">
  import { t } from "../i18n/strings";
  import type { WhatsNew } from "../ipc";
  import { getApp } from "../state/app";
  import Button from "../ui/Button.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import ReleaseNotes from "./ReleaseNotes.svelte";

  let { info, onclose }: { info: WhatsNew; onclose: () => void } = $props();

  const { updates } = getApp();
</script>

<Dialog title={t("whatsNew.title", { version: info.version })} onclose={onclose} size="md">
  <p class="intro">{t("whatsNew.intro", { version: info.version })}</p>
  <ReleaseNotes
    notes={info.notes}
    version={info.version}
    onopen={() => void updates.openReleasePage(info.version)}
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
