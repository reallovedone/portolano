<script lang="ts">
  import { t } from "../i18n/strings";
  import type { Node } from "../ipc";
  import Banner from "../ui/Banner.svelte";
  import Button from "../ui/Button.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import TreeSelect, { type TreeSelectOption } from "../ui/TreeSelect.svelte";
  import { folderOptions, moveTargets, type RemoteMap } from "./model";

  let {
    node,
    nodes,
    remote,
    onmove,
    oncancel,
  }: {
    node: Node;
    nodes: readonly Node[];
    /** Remote (shared) nodes: they are never offered as a destination. */
    remote?: RemoteMap;
    /** Returns an error message to keep the dialog open, or null when done. */
    onmove: (parentId: string | null) => Promise<string | null>;
    oncancel: () => void;
  } = $props();

  const targets = $derived(moveTargets(nodes, node.id, remote));

  function initialChoice(): string {
    if (node.parentId !== null) return "";
    return moveTargets(nodes, node.id, remote)[0]?.id ?? "";
  }
  let choice = $state(initialChoice());
  let busy = $state(false);
  let error = $state<string | null>(null);
  const formId = `move-${Math.random().toString(36).slice(2, 9)}`;

  // Every folder is listed so the tree stays readable; the ones the node cannot go to
  // (itself, its descendants, the folder it is already in) are shown disabled.
  const allowed = $derived(new Set(targets.map((target) => target.id)));
  const options = $derived<TreeSelectOption[]>([
    ...(node.parentId !== null ? [{ value: "", label: t("tree.moveToRoot"), icon: null }] : []),
    ...folderOptions(nodes).map((folder) => ({
      value: folder.id,
      label: folder.name,
      depth: folder.depth,
      disabled: !allowed.has(folder.id),
    })),
  ]);

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    busy = true;
    error = await onmove(choice === "" ? null : choice);
    busy = false;
  }
</script>

<Dialog title={t("tree.moveDialogTitle", { name: node.name })} onclose={oncancel} size="sm">
  <form id="{formId}-form" class="form" onsubmit={submit}>
    <TreeSelect
      label={t("editor.parent")}
      hint={t("tree.moveInvalid")}
      bind:value={choice}
      {options}
      data-autofocus
    />
    {#if error}
      <Banner variant="danger">{error}</Banner>
    {/if}
  </form>
  {#snippet footer()}
    <Button onclick={oncancel}>{t("common.cancel")}</Button>
    <Button type="submit" variant="primary" form="{formId}-form" disabled={busy}>
      {t("tree.moveTo").replace("…", "")}
    </Button>
  {/snippet}
</Dialog>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
</style>
