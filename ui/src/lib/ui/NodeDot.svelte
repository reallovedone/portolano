<!-- The small dot that shows a node's colour next to its name. Decorative: the colour is never the
     only carrier of meaning and never tints text. Renders nothing for an empty or unknown colour. -->
<script lang="ts">
  import { nodeColorVar } from "../design/icons/nodeIcons";

  let { color }: { color: string | undefined } = $props();

  const fill = $derived(nodeColorVar(color));
</script>

{#if fill}
  <span class="node-dot" style:background={fill} aria-hidden="true" data-node-color={color}></span>
{/if}

<style>
  .node-dot {
    display: inline-block;
    flex: none;
    width: var(--space-2);
    height: var(--space-2);
    border-radius: var(--radius-full);
  }
</style>
