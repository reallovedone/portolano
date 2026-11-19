<!-- Help text in two levels: a short line that is always visible and a "Learn more" button that
     reveals the long text in place. Not hover-only, so it works from the keyboard and on touch.
     Escape inside the long text closes it and returns focus to the button. -->
<script lang="ts">
  import { t } from "../i18n/strings";

  let {
    short,
    more,
    open = $bindable(false),
    inset = false,
  }: {
    short: string;
    more: string;
    open?: boolean;
    /** Indents the text to line up with a checkbox label. */
    inset?: boolean;
  } = $props();

  const uid = $props.id();
  const moreId = `help-${uid}`;
  let button: HTMLButtonElement | undefined = $state();

  function onkeydown(event: KeyboardEvent): void {
    if (event.key !== "Escape" || !open) return;
    event.stopPropagation();
    open = false;
    button?.focus();
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="help" class:inset {onkeydown}>
  <p class="short">
    {short}
    <button
      bind:this={button}
      type="button"
      class="toggle"
      aria-expanded={open}
      aria-controls={moreId}
      onclick={() => (open = !open)}>{open ? t("editor.help.showLess") : t("editor.help.learnMore")}</button
    >
  </p>
  <p id={moreId} class="more" hidden={!open}>{more}</p>
</div>

<style>
  .help {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
    line-height: var(--line-height-normal);
  }
  .inset {
    padding-inline-start: calc(var(--checkbox-size) + var(--space-2));
  }
  .toggle {
    all: unset;
    cursor: pointer;
    color: var(--text-link);
    border-radius: var(--radius-xs);
  }
  .toggle:hover {
    text-decoration: underline;
  }
  .toggle:focus-visible {
    outline: var(--focus-ring-width) solid var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
  .more[hidden] {
    display: none;
  }
</style>
