<!-- The icon of a connection or folder in the tree: the one the user chose (Lucide, from the 40 in
     design/icons/nodeIcons.ts), or the kind's own icon when none is set or the name is unknown.
     A chosen icon keeps the kind's colour, so the kind stays readable at a glance. -->
<script lang="ts">
  import { nodeIcon } from "../design/icons/nodeIcons";
  import type { NodeKind } from "../tree/model";
  import KindIcon from "./KindIcon.svelte";

  let {
    kind,
    icon,
    open = false,
    size = 16,
  }: {
    kind: NodeKind;
    /** Stored icon name; empty or unknown falls back to the kind's icon. */
    icon?: string;
    open?: boolean;
    size?: 14 | 16 | 18 | 20 | 24 | 28;
  } = $props();

  const Glyph = $derived(nodeIcon(icon));
</script>

{#if Glyph}
  <Glyph
    class="node-icon kind-{kind}"
    {size}
    strokeWidth={1.75}
    aria-hidden="true"
    data-kind={kind}
    data-node-icon={icon}
  />
{:else}
  <KindIcon {kind} {open} {size} />
{/if}

<style>
  :global(.node-icon) {
    flex: none;
    color: var(--text-secondary);
  }
  :global(.node-icon.kind-folder) {
    color: var(--protocol-folder);
  }
  :global(.node-icon.kind-localShell) {
    color: var(--protocol-local);
  }
  :global(.node-icon.kind-ssh) {
    color: var(--protocol-ssh);
  }
  :global(.node-icon.kind-rdp) {
    color: var(--protocol-rdp);
  }
  :global(.node-icon.kind-vnc) {
    color: var(--protocol-vnc);
  }
  :global(.node-icon.kind-web) {
    color: var(--protocol-web);
  }
</style>
