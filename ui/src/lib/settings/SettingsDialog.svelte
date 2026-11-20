<!-- Settings window. One page for now, Appearance, in groups of cards: theme, colour scheme,
     accent colour, list density and terminal font size. Every choice applies at once (no Save
     button); a failed save puts the previous look back and says so. Opened from the ⋯ menu and,
     on macOS, "Settings…" (⌘,).

     Why a page and not a sidebar: with one section a sidebar would be a lonely item. Another
     section (Security, Updates) is one more `<section class="page">` below Appearance; when there
     are two or three, the sidebar of the layout can be added around the same sections.

     Each group is a radio group (ChoiceGroup): arrow keys move between options, the pictures
     are decoration, the names come from strings.ts. -->
<script lang="ts">
  import { t, type StringKey } from "../i18n/strings";
  import {
    ACCENT_CHOICES,
    DENSITY_CHOICES,
    SCHEME_CHOICES,
    TERMINAL_FONT_SIZE_MAX,
    TERMINAL_FONT_SIZE_MIN,
    THEME_CHOICES,
    type AccentChoice,
    type DensityChoice,
    type SchemeChoice,
    type ThemeChoice,
  } from "../ipc";
  import { getApp } from "../state/app";
  import Button from "../ui/Button.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import Icon from "../ui/Icon.svelte";
  import ChoiceGroup from "./ChoiceGroup.svelte";

  let { onclose }: { onclose: () => void } = $props();

  const { appearance } = getApp();

  const options = <T extends string>(values: readonly T[], prefix: string): { value: T; label: string }[] =>
    values.map((value) => ({ value, label: t(`${prefix}.${value}` as StringKey) }));
  const themeOptions = options(THEME_CHOICES, "appearance.theme");
  const schemeOptions = options(SCHEME_CHOICES, "appearance.scheme");
  const accentOptions = options(ACCENT_CHOICES, "appearance.accent");
  const densityOptions = options(DENSITY_CHOICES, "appearance.density");

  // While the slider is dragged the preview follows it at once; the choice is saved on release.
  let dragging = $state<number | null>(null);
  const size = $derived(dragging ?? appearance.value.terminalFontSize);

  const uid = $props.id();
  const headingId = `settings-appearance-${uid}`;
  const sizeHintId = `settings-size-hint-${uid}`;

  function chooseSize(event: Event & { currentTarget: HTMLInputElement }): void {
    const next = Number(event.currentTarget.value);
    dragging = null;
    void appearance.update({ terminalFontSize: next });
  }
</script>

<Dialog title={t("settings.title")} {onclose} size="lg">
  <section class="page" aria-labelledby={headingId}>
    <header class="page-head">
      <h3 id={headingId}>{t("settings.appearance")}</h3>
      <p>{t("settings.appearanceHint")}</p>
    </header>

    <div class="group">
      <h4>{t("appearance.theme")}</h4>
      <p class="hint">{t("appearance.themeHint")}</p>
      <ChoiceGroup
        label={t("appearance.theme")}
        options={themeOptions}
        value={appearance.value.theme}
        columns={3}
        onselect={(value: ThemeChoice) => void appearance.update({ theme: value })}
      >
        {#snippet preview(value: ThemeChoice)}
          <span class="mini-row" class:split={value === "system"}>
            {#if value !== "dark"}{@render mini("light")}{/if}
            {#if value !== "light"}{@render mini("dark")}{/if}
          </span>
        {/snippet}
      </ChoiceGroup>
    </div>

    <div class="group">
      <h4>{t("appearance.scheme")}</h4>
      <p class="hint">{t("appearance.schemeHint")}</p>
      <ChoiceGroup
        label={t("appearance.scheme")}
        options={schemeOptions}
        value={appearance.value.scheme}
        onselect={(value: SchemeChoice) => void appearance.update({ scheme: value })}
      >
        {#snippet preview(value: SchemeChoice)}
          <!-- The attribute makes every token inside resolve to this scheme, in the active theme. -->
          <span class="scheme-preview" data-scheme={value}>
            <span class="scheme-side"></span>
            <span class="scheme-main">
              <span class="scheme-line">
                <span style:color="var(--ansi-green)">$</span>
                <span class="scheme-text">ls</span>
              </span>
              <span class="scheme-line">
                <span class="scheme-text muted"></span>
              </span>
              <span class="scheme-dots">
                {#each ["red", "green", "yellow", "blue", "magenta", "cyan"] as ansi (ansi)}
                  <span class="scheme-dot" style:background="var(--ansi-{ansi})"></span>
                {/each}
              </span>
            </span>
          </span>
        {/snippet}
      </ChoiceGroup>
    </div>

    <div class="group">
      <h4>{t("appearance.accent")}</h4>
      <p class="hint">{t("appearance.accentHint")}</p>
      <ChoiceGroup
        label={t("appearance.accent")}
        variant="swatch"
        options={accentOptions}
        value={appearance.value.accent}
        onselect={(value: AccentChoice) => void appearance.update({ accent: value })}
      >
        {#snippet preview(value: AccentChoice, selected: boolean)}
          <span class="dot" style:background="var(--swatch-{value})">
            {#if selected}<Icon name="check" size={16} strokeWidth={2.5} />{/if}
          </span>
        {/snippet}
      </ChoiceGroup>
    </div>

    <div class="group">
      <h4>{t("appearance.density")}</h4>
      <p class="hint">{t("appearance.densityHint")}</p>
      <ChoiceGroup
        label={t("appearance.density")}
        options={densityOptions}
        value={appearance.value.density}
        columns={2}
        onselect={(value: DensityChoice) => void appearance.update({ density: value })}
      >
        {#snippet preview(value: DensityChoice)}
          <span class="sketch {value}">
            {#each Array.from({ length: value === "compact" ? 5 : 3 }, (_, index) => index) as row (row)}
              <span class="sketch-row" class:active={row === 1}></span>
            {/each}
          </span>
        {/snippet}
      </ChoiceGroup>
    </div>

    <div class="group">
      <h4>{t("appearance.terminalFontSize")}</h4>
      <p class="hint" id={sizeHintId}>{t("appearance.terminalFontSizeHint")}</p>
      <div class="size">
        <input
          class="slider"
          type="range"
          min={TERMINAL_FONT_SIZE_MIN}
          max={TERMINAL_FONT_SIZE_MAX}
          step="1"
          value={size}
          aria-label={t("appearance.terminalFontSize")}
          aria-describedby={sizeHintId}
          aria-valuetext={t("appearance.terminalFontSizeOption", { size })}
          oninput={(event) => (dragging = Number(event.currentTarget.value))}
          onchange={chooseSize}
        />
        <output class="size-value">{t("appearance.terminalFontSizeOption", { size })}</output>
      </div>
      <div class="terminal" role="img" aria-label={t("appearance.terminalPreview")} style:font-size="{size}px">
        <span style:color="var(--ansi-green)">{t("appearance.previewUser")}</span><span>:</span><span style:color="var(--ansi-blue)">{t("appearance.previewPath")}</span><span>$ {t("appearance.previewCommand")}</span>
      </div>
    </div>
  </section>

  {#snippet footer()}
    <Button variant="primary" data-autofocus onclick={onclose}>{t("common.close")}</Button>
  {/snippet}
</Dialog>

<!-- A little window in the light or the dark look, drawn with the fixed preview tokens. -->
{#snippet mini(mode: "light" | "dark")}
  <span class="mini {mode}">
    <span class="mini-bar"></span>
    <span class="mini-body">
      <span class="mini-line long"></span>
      <span class="mini-line short"></span>
    </span>
  </span>
{/snippet}

<style>
  .page-head {
    margin-bottom: var(--space-4);
  }
  h3 {
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-semibold);
  }
  .page-head p {
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
  }

  /* One group = a title, one line of help and the choices. */
  .group {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding-bottom: var(--space-4);
    margin-bottom: var(--space-4);
    border-bottom: 1px solid var(--border-hairline);
  }
  .group:last-child {
    margin-bottom: 0;
    border-bottom: 0;
  }
  h4 {
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-semibold);
  }
  .hint {
    margin-top: calc(-1 * var(--space-1));
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
  }

  /* ---- Theme: a mini window per option; System shows the two looks side by side ---- */
  .mini-row {
    display: grid;
    grid-template-columns: 1fr;
    overflow: hidden;
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-control);
  }
  .mini-row.split {
    grid-template-columns: 1fr 1fr;
  }
  .mini {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    aspect-ratio: 16 / 10;
    padding-bottom: var(--space-2);
  }
  .mini.light {
    background: var(--preview-light-bg);
  }
  .mini.dark {
    background: var(--preview-dark-bg);
  }
  .mini-bar {
    height: var(--space-2);
  }
  .light .mini-bar {
    background: var(--preview-light-bar);
  }
  .dark .mini-bar {
    background: var(--preview-dark-bar);
  }
  .mini-body {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding-inline: var(--space-2);
  }
  .mini-line {
    height: var(--space-1);
    border-radius: var(--radius-full);
  }
  .light .mini-line {
    background: var(--preview-light-line);
  }
  .dark .mini-line {
    background: var(--preview-dark-line);
  }
  .mini-line.long {
    width: 80%;
  }
  .mini-line.short {
    width: 50%;
  }

  /* ---- Colour scheme: sidebar, a prompt and the ANSI colours, all from the scheme's tokens ---- */
  .scheme-preview {
    display: flex;
    min-height: calc(var(--space-12) + var(--space-4));
    overflow: hidden;
    background: var(--surface-content);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-control);
  }
  .scheme-side {
    flex: none;
    width: var(--space-4);
    background: var(--surface-sidebar);
    border-right: 1px solid var(--border-hairline);
  }
  .scheme-main {
    display: flex;
    flex: 1;
    flex-direction: column;
    justify-content: center;
    gap: var(--space-1);
    padding: var(--space-2);
    background: var(--terminal-bg);
  }
  .scheme-line {
    display: flex;
    gap: var(--space-1);
    font-family: var(--font-mono);
    font-size: var(--font-size-xs);
    line-height: var(--line-height-tight);
  }
  .scheme-text {
    color: var(--text-primary);
  }
  .scheme-text.muted {
    width: 60%;
    height: var(--space-1);
    margin-block: var(--space-1);
    background: var(--text-tertiary);
    border-radius: var(--radius-full);
  }
  .scheme-dots {
    display: flex;
    gap: var(--space-1);
    margin-top: var(--space-1);
  }
  .scheme-dot {
    width: var(--space-2);
    height: var(--space-2);
    border-radius: var(--radius-full);
  }

  /* ---- Accent: round swatch with a check on the chosen one ---- */
  .dot {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--space-8);
    height: var(--space-8);
    color: var(--text-on-accent);
    border-radius: var(--radius-full);
  }

  /* ---- Density: a sketch of the rows of a list ---- */
  .sketch {
    display: flex;
    flex-direction: column;
    justify-content: center;
    min-height: calc(var(--space-12) + var(--space-4));
    padding: var(--space-2) var(--space-3);
    background: var(--surface-content);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-control);
  }
  .sketch.comfortable {
    gap: var(--space-4);
  }
  .sketch.compact {
    gap: var(--space-1);
  }
  .sketch-row {
    height: var(--space-2);
    background: var(--decor-line-strong);
    border-radius: var(--radius-full);
  }
  .sketch-row.active {
    background: var(--accent);
  }

  /* ---- Terminal font size: slider and a one-line terminal in the chosen size and scheme ---- */
  .size {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .slider {
    flex: 1;
    min-width: 0;
    accent-color: var(--accent);
  }
  .slider:focus-visible {
    outline: var(--focus-ring-width) solid var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }
  .size-value {
    min-width: var(--space-12);
    text-align: right;
    color: var(--text-secondary);
    font-size: var(--font-size-md);
    font-variant-numeric: tabular-nums;
  }
  .terminal {
    padding: var(--space-3);
    overflow: hidden;
    white-space: nowrap;
    background: var(--terminal-bg);
    color: var(--text-primary);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-control);
    font-family: var(--font-mono);
    line-height: var(--terminal-line-height);
  }
</style>
