<script lang="ts">
  let {
    size = 16,
    label,
  }: {
    size?: 14 | 16 | 20;
    /** Accessible name. Omit when the surrounding control already announces the busy state. */
    label?: string;
  } = $props();
</script>

<span
  class="spinner"
  style:--spinner-size="{size}px"
  role={label ? "status" : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : "true"}
></span>

<style>
  .spinner {
    flex: none;
    display: inline-block;
    width: var(--spinner-size);
    height: var(--spinner-size);
    border: var(--spinner-width) solid var(--border-strong);
    border-top-color: currentColor;
    border-radius: var(--radius-full);
    animation: spin var(--duration-spin) linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
