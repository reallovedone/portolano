<!-- Interface icon from the bundled Lucide set (see design/icons/registry.ts).
     Decorative by default (aria-hidden); pass `label` when the icon alone carries meaning. -->
<script lang="ts">
  import { ICONS, type IconName } from "../design/icons/registry";

  let {
    name,
    size = 16,
    strokeWidth = 1.75,
    label,
    class: className,
  }: {
    name: IconName;
    /** Pixel size: 14 (dense rows), 16 (default), 20 (empty states, large controls). */
    size?: 14 | 16 | 20;
    /** Same stroke weight everywhere keeps the set coherent with the logo's single stroke. */
    strokeWidth?: number;
    /** Accessible name; when set the icon is exposed as an image instead of being hidden. */
    label?: string;
    class?: string;
  } = $props();

  const Glyph = $derived(ICONS[name]);
</script>

<Glyph
  {size}
  {strokeWidth}
  class={className ? `ui-icon ${className}` : "ui-icon"}
  data-icon={name}
  {...label ? { role: "img", "aria-label": label } : { "aria-hidden": "true" }}
/>

<style>
  :global(.ui-icon) {
    flex: none;
    display: inline-block;
    vertical-align: middle;
  }
</style>
