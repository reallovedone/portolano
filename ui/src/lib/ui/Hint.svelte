<!-- Secondary text under a field, heading or dialog title: a hint or a short note. Spacing belongs
     to the parent. `role` lets a note announce itself, for example role="status" after an action. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";

  type Props = Omit<HTMLAttributes<HTMLParagraphElement>, "children" | "class" | "role"> & {
    role?: "status" | "alert" | "note";
    children?: Snippet;
  };

  let { role, children, ...rest }: Props = $props();
</script>

<p {...rest} {role} class="hint">{@render children?.()}</p>

<style>
  .hint {
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
  }
</style>
