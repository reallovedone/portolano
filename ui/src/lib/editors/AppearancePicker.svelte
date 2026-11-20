<!-- Icon and colour pickers of a connection or folder ("General" section of the editors).
     Both are radio groups of real radio inputs (hidden, but they keep focus and the arrow keys),
     so they are announced as groups and work from the keyboard like the Settings window's choices.
     Empty icon = the kind's own icon; empty colour = no colour (both are stored as "absent"). -->
<script lang="ts">
  import { nodeIcon } from "../design/icons/nodeIcons";
  import { NODE_COLORS, NODE_ICON_NAMES } from "../design/nodeAppearance";
  import { t, type StringKey } from "../i18n/strings";
  import ChoiceGroup from "../settings/ChoiceGroup.svelte";
  import type { NodeKind } from "../tree/model";
  import FormRow from "../ui/FormRow.svelte";
  import KindIcon from "../ui/KindIcon.svelte";
  import TextField from "../ui/TextField.svelte";

  let {
    kind,
    icon = $bindable(),
    color = $bindable(),
    id,
  }: {
    /** Kind of the node: its icon is the "Default" choice. */
    kind: NodeKind;
    icon?: string | undefined;
    color?: string | undefined;
    /** Prefix for element ids and radio group names. */
    id: string;
  } = $props();

  let query = $state("");

  /** "hard-drive" -> "Hard drive": the name of an icon, for its tooltip and accessible name. */
  const iconLabel = (name: string): string => {
    const words = name.replace(/-/g, " ");
    return words.charAt(0).toUpperCase() + words.slice(1);
  };

  const matches = $derived.by(() => {
    const needle = query.trim().toLowerCase().replace(/\s+/g, "-");
    return needle === "" ? NODE_ICON_NAMES : NODE_ICON_NAMES.filter((name) => name.includes(needle));
  });

  const colorOptions = $derived([
    { value: "", label: t("editor.colorNone") },
    ...NODE_COLORS.map((name) => ({ value: name as string, label: t(`nodeColor.${name}` as StringKey) })),
  ]);
</script>

<FormRow label={t("editor.icon")}>
  <div class="icons">
    <TextField
      type="search"
      size="compact"
      bind:value={query}
      placeholder={t("editor.iconSearch")}
      aria-label={t("editor.iconSearch")}
      autocomplete="off"
      onkeydown={(e: KeyboardEvent) => {
        // Enter in the search must not save the whole form.
        if (e.key === "Enter") e.preventDefault();
      }}
    />
    <div class="grid" role="radiogroup" aria-label={t("editor.icon")}>
      <label class="cell" class:selected={icon === undefined} title={t("editor.iconDefault")}>
        <input
          type="radio"
          name="{id}-icon"
          value=""
          checked={icon === undefined}
          aria-label={t("editor.iconDefault")}
          onchange={() => (icon = undefined)}
        />
        <KindIcon {kind} size={20} />
      </label>
      {#each matches as name (name)}
        {@const Glyph = nodeIcon(name)}
        <label class="cell" class:selected={icon === name} title={iconLabel(name)}>
          <input
            type="radio"
            name="{id}-icon"
            value={name}
            checked={icon === name}
            aria-label={iconLabel(name)}
            onchange={() => (icon = name)}
          />
          {#if Glyph}<Glyph class="glyph" size={20} strokeWidth={1.75} aria-hidden="true" />{/if}
        </label>
      {/each}
    </div>
    {#if matches.length === 0}
      <p class="empty" role="status">{t("editor.iconNoMatch", { query: query.trim() })}</p>
    {/if}
  </div>
</FormRow>

<FormRow label={t("editor.color")} hint={t("editor.colorHint")}>
  <ChoiceGroup
    label={t("editor.color")}
    variant="swatch"
    captions={false}
    options={colorOptions}
    value={color ?? ""}
    onselect={(value: string) => (color = value === "" ? undefined : value)}
  >
    {#snippet preview(value: string)}
      {#if value === ""}
        <span class="dot none"></span>
      {:else}
        <span class="dot" style:background="var(--node-{value})"></span>
      {/if}
    {/snippet}
  </ChoiceGroup>
</FormRow>

<style>
  .icons {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(var(--control-h-large), 1fr));
    gap: var(--space-1);
    max-height: calc(var(--control-h-large) * 4 + var(--space-1) * 3);
    padding: var(--space-1);
    overflow-y: auto;
    background: var(--surface-field);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-control);
  }

  /* The input covers the whole cell: it is the click target and the focus target. */
  .cell {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: var(--control-h-large);
    color: var(--text-primary);
    border: var(--focus-ring-width) solid transparent;
    border-radius: var(--radius-control);
    cursor: default;
  }
  .cell input {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    margin: 0;
    opacity: 0;
    cursor: default;
  }
  .cell:hover {
    background: var(--surface-hover);
  }
  .cell.selected {
    background: var(--accent-subtle);
    border-color: var(--accent);
  }
  .cell:has(input:focus-visible) {
    outline: var(--focus-ring-width) solid var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
  .cell :global(.glyph) {
    flex: none;
  }

  .empty {
    margin: 0;
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
  }

  .dot {
    display: inline-block;
    width: var(--space-6);
    height: var(--space-6);
    border-radius: var(--radius-full);
  }
  .dot.none {
    border: 1px dashed var(--border-strong);
  }
</style>
