<!-- Icons for the five connection kinds and folders. Drawn on a 24-unit grid with a single
     round-capped stroke, like the logo. Colour comes from the protocol tokens and is used only
     here, on small icons. -->
<script lang="ts">
  import type { NodeKind } from "../tree/model";

  let {
    kind,
    open = false,
    size = 16,
  }: { kind: NodeKind; open?: boolean; size?: 14 | 16 | 18 | 20 | 24 | 28 } = $props();
</script>

<svg
  class="kind kind-{kind}"
  viewBox="0 0 24 24"
  width={size}
  height={size}
  fill="none"
  stroke="currentColor"
  stroke-width="1.75"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  data-kind={kind}
>
  {#if kind === "folder"}
    {#if open}
      <path d="M3 18.5V6.5a2 2 0 0 1 2-2h3.6a2 2 0 0 1 1.6.8l.9 1.2h6.4a2 2 0 0 1 2 2v1.5" />
      <path d="M3 18.5l2.2-6.3a2 2 0 0 1 1.9-1.4h13a1.2 1.2 0 0 1 1.1 1.6l-1.9 5.4a2 2 0 0 1-1.9 1.3H5a2 2 0 0 1-2-.6z" />
    {:else}
      <path d="M3 7a2 2 0 0 1 2-2h3.6a2 2 0 0 1 1.6.8l.9 1.2H19a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
    {/if}
  {:else if kind === "localShell"}
    <rect x="3" y="4.5" width="18" height="15" rx="3.5" />
    <path d="M7.5 10l3 2.5-3 2.5M13 15h3.5" />
  {:else if kind === "ssh"}
    <rect x="3" y="4.5" width="18" height="15" rx="3.5" />
    <path d="M7.5 10l3 2.5-3 2.5M13 15h3.5" />
    <circle cx="17" cy="9" r="1.1" fill="currentColor" stroke="none" />
  {:else if kind === "rdp"}
    <rect x="3" y="4" width="18" height="12" rx="2.5" />
    <path d="M3 8.5h18M9 20h6M12 16v4" />
  {:else if kind === "vnc"}
    <rect x="3" y="4" width="18" height="12" rx="2.5" />
    <path d="M10 7.5l4.6 2-2 .8-.8 2z" fill="currentColor" />
    <path d="M9 20h6M12 16v4" />
  {:else}
    <circle cx="12" cy="12" r="9" />
    <path d="M3 12h18M12 3c2.6 2.4 2.6 15.6 0 18M12 3c-2.6 2.4-2.6 15.6 0 18" />
  {/if}
</svg>

<style>
  .kind {
    flex: none;
    color: var(--text-secondary);
  }
  .kind-folder {
    color: var(--protocol-folder);
  }
  .kind-localShell {
    color: var(--protocol-local);
  }
  .kind-ssh {
    color: var(--protocol-ssh);
  }
  .kind-rdp {
    color: var(--protocol-rdp);
  }
  .kind-vnc {
    color: var(--protocol-vnc);
  }
  .kind-web {
    color: var(--protocol-web);
  }
</style>
