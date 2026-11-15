<!-- "Location" row: the folder a node lives in. -->
<script lang="ts">
  import { t } from "../i18n/strings";
  import type { Node, Uuid } from "../ipc";
  import { folderOptions } from "../tree/model";
  import FormRow from "../ui/FormRow.svelte";
  import TreeSelect, { type TreeSelectOption } from "../ui/TreeSelect.svelte";

  let {
    value = $bindable(),
    nodes,
    excludeSubtreeOf = null,
    id,
  }: {
    value: Uuid | null;
    nodes: readonly Node[];
    excludeSubtreeOf?: Uuid | null;
    id: string;
  } = $props();

  const options = $derived<TreeSelectOption[]>([
    { value: "", label: t("tree.moveToRoot"), icon: null },
    ...folderOptions(nodes, excludeSubtreeOf).map((option) => ({
      value: option.id,
      label: option.name,
      depth: option.depth,
    })),
  ]);
</script>

<FormRow label={t("editor.parent")} for={id}>
  <TreeSelect {id} {options} bind:value={() => value ?? "", (next) => (value = next || null)} />
</FormRow>
