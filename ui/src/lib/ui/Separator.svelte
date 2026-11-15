<script lang="ts">
  let {
    orientation = "horizontal",
    strong = false,
    decorative = true,
  }: {
    orientation?: "horizontal" | "vertical";
    /** Uses the stronger border colour. */
    strong?: boolean;
    /** Decorative separators are hidden from assistive technology. */
    decorative?: boolean;
  } = $props();
</script>

<div
  class="separator {orientation}"
  class:strong
  role={decorative ? "none" : "separator"}
  aria-orientation={decorative ? undefined : orientation}
></div>

<style>
  .separator {
    flex: none;
    background: var(--border-hairline);
  }
  .separator.strong {
    background: var(--border-strong);
  }
  .horizontal {
    width: 100%;
    height: 1px;
  }
  .vertical {
    align-self: stretch;
    width: 1px;
  }
</style>
