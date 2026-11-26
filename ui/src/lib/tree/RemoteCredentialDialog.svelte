<!-- "Credential…" on a node of a shared folder. The share file never carries a secret: the
     choice (inherit, none, a vault credential) is stored on this machine only. -->
<script lang="ts">
  import { t } from "../i18n/strings";
  import type { Node } from "../ipc";
  import { getApp } from "../state/app";
  import CredentialField from "../editors/CredentialField.svelte";
  import { credentialToDraft, draftToCredential, type CredentialDraft } from "../editors/form";
  import Banner from "../ui/Banner.svelte";
  import Button from "../ui/Button.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import Hint from "../ui/Hint.svelte";

  let { node, onclose }: { node: Node; onclose: () => void } = $props();

  const { doc } = getApp();
  const uid = `rc-${Math.random().toString(36).slice(2, 9)}`;
  // svelte-ignore state_referenced_locally
  let draft = $state<CredentialDraft>(credentialToDraft(node.credential));
  let error = $state<string | null>(null);
  let saveError = $state<string | null>(null);
  let busy = $state(false);

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (busy) return;
    saveError = null;
    if (draft.mode === "id" && draft.id === "") {
      error = t("validation.credentialRequired");
      return;
    }
    error = null;
    busy = true;
    const result = await doc.setRemoteCredential(node.id, draftToCredential(draft));
    busy = false;
    if (!result.ok) {
      saveError = `${result.error.message} (${result.error.code})`;
      return;
    }
    onclose();
  }
</script>

<Dialog title={t("share.credentialTitle", { name: node.name })} {onclose} width={460}>
  <form id="{uid}-form" onsubmit={submit} novalidate>
    {#if saveError}
      <div class="banner"><Banner variant="danger" role="alert">{saveError}</Banner></div>
    {/if}
    <div class="gap"><Hint>{t("share.credentialHint")}</Hint></div>
    <CredentialField bind:value={draft} nodes={doc.nodes} parentId={node.parentId} {error} idPrefix={uid} />
  </form>

  {#snippet footer()}
    <Button onclick={onclose}>{t("common.cancel")}</Button>
    <Button type="submit" variant="primary" form="{uid}-form" disabled={busy}>
      {t("common.save")}
    </Button>
  {/snippet}
</Dialog>

<style>
  .banner {
    margin-bottom: var(--space-3);
  }

  .gap {
    margin-bottom: var(--space-3);
  }
</style>
