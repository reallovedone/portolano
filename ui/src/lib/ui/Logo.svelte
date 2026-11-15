<!-- The Portolano mark (brand/logo.svg) inlined so it follows currentColor. -->
<script lang="ts">
  let {
    size = 24,
    label,
  }: {
    /** Width and height in pixels. */
    size?: number;
    /** Accessible name; omit when the logo is next to the product name. */
    label?: string;
  } = $props();
</script>

<svg
  class="logo"
  viewBox="0 0 512 512"
  width={size}
  height={size}
  fill="none"
  role={label ? "img" : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : "true"}
>
  <g transform="translate(-20 -2)">
    <path
      d="M172 424V196A104 104 0 1 1 276 300"
      stroke="currentColor"
      stroke-width="52"
      stroke-linecap="round"
      stroke-linejoin="round"
    />
    <circle cx="276" cy="196" r="34" fill="currentColor" />
  </g>
</svg>

<style>
  .logo {
    flex: none;
    display: inline-block;
    vertical-align: middle;
  }
</style>
