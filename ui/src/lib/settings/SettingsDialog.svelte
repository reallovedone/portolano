<!-- Settings window. One scrolling page: Appearance (groups of cards: theme, colour scheme,
     accent colour, list density and terminal font size), then Identity, Security (auto-lock and
     quick unlock) and Updates. Every choice applies at once (no Save
     button); a failed save puts the previous value back and says so. Opened from the ⋯ menu and,
     on macOS, "Settings…" (⌘,).

     Each section is a `<section class="page">`; if the page grows further, a sidebar can be
     added around the same sections.

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
  import { detectMac } from "../shortcuts";
  import Button from "../ui/Button.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import Hint from "../ui/Hint.svelte";
  import Select from "../ui/Select.svelte";
  import Icon from "../ui/Icon.svelte";
  import Switch from "../ui/Switch.svelte";
  import IdentitySection from "../shares/IdentitySection.svelte";
  import ChoiceGroup from "./ChoiceGroup.svelte";

  let { onclose }: { onclose: () => void } = $props();

  const { appearance, ui, updates, vault, notices } = getApp();

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
  const securityHeadingId = `settings-security-${uid}`;
  const updatesHeadingId = `settings-updates-${uid}`;
  const sizeHintId = `settings-size-hint-${uid}`;

  function chooseSize(event: Event & { currentTarget: HTMLInputElement }): void {
    const next = Number(event.currentTarget.value);
    dragging = null;
    void appearance.update({ terminalFontSize: next });
  }

  // Quick unlock (Touch ID / Windows Hello). It belongs to the vault, so it is shown only while the
  // vault is unlocked and the system offers it. Opening Settings never asks for the password: the
  // info is read only when the vault is already unlocked, and again when its status changes.
  const quickPlatform = detectMac() ? "mac" : "windows";
  let quickAvailable = $state(false);
  let quickEnabled = $state(false);
  /** Shown after turning it on: the key can only be captured at a password unlock. */
  let quickReady = $state(false);
  let quickBusy = $state(false);

  async function loadQuickInfo(): Promise<void> {
    const result = await vault.quickUnlockInfo();
    if (!result.ok) {
      notices.error(t("security.quickUnlockInfoFailed"), result.error);
      return;
    }
    quickAvailable = result.value.available;
    quickEnabled = result.value.enabled;
    quickReady = result.value.ready;
  }

  $effect(() => {
    if (vault.status === "unlocked") void loadQuickInfo();
    else quickAvailable = false;
  });

  async function changeQuickUnlock(): Promise<void> {
    const wanted = quickEnabled;
    quickBusy = true;
    const result = await vault.setQuickUnlock(wanted);
    quickBusy = false;
    if (result.ok) {
      quickReady = false; // on: needs a password unlock; off: the key is gone
      return;
    }
    quickEnabled = !wanted;
    notices.error(t("security.quickUnlockFailed"), result.error);
  }

  // Auto-lock: reading and saving the setting does not need the vault unlocked, so the section is
  // there whenever a vault exists.
  const AUTO_LOCK_CHOICES = [0, 5, 15, 30, 60] as const;
  const autoLockOptions = AUTO_LOCK_CHOICES.map((minutes) => ({
    value: String(minutes),
    label: t(`security.autoLock.${minutes}` as StringKey),
  }));
  let autoLock = $state<string>("15");
  /** The value last confirmed by the backend; the choice goes back to it when saving fails. */
  let savedAutoLock = "15";
  let hasVault = $derived(vault.status !== "notCreated");

  $effect(() => {
    if (!hasVault) return;
    void (async () => {
      const result = await vault.getAutoLock();
      if (!result.ok) {
        notices.error(t("settings.security"), result.error);
        return;
      }
      savedAutoLock = String(result.value);
      autoLock = savedAutoLock;
    })();
  });

  async function changeAutoLock(): Promise<void> {
    const wanted = autoLock;
    const result = await vault.setAutoLock(Number(wanted));
    if (result.ok) {
      savedAutoLock = wanted;
      return;
    }
    autoLock = savedAutoLock;
    notices.error(t("security.autoLockFailed"), result.error);
  }

  // The switches edit the stored settings; a failed save puts the switch back.
  let auto = $state(updates.autoCheck);
  let prereleases = $state(updates.includePrereleases);
  $effect(() => {
    auto = updates.autoCheck;
    prereleases = updates.includePrereleases;
  });

  async function toggleAuto(): Promise<void> {
    if (!(await updates.setAutoCheck(auto))) auto = updates.autoCheck;
  }

  async function togglePrereleases(): Promise<void> {
    if (!(await updates.setIncludePrereleases(prereleases))) prereleases = updates.includePrereleases;
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
      <div class="lift"><Hint>{t("appearance.themeHint")}</Hint></div>
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
      <div class="lift"><Hint>{t("appearance.schemeHint")}</Hint></div>
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
                <span class="scheme-text scheme-bar"></span>
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
      <div class="lift"><Hint>{t("appearance.accentHint")}</Hint></div>
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
      <div class="lift"><Hint>{t("appearance.densityHint")}</Hint></div>
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
      <div class="lift"><Hint id={sizeHintId}>{t("appearance.terminalFontSizeHint")}</Hint></div>
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

  <IdentitySection />

  {#if hasVault}
    <section class="page security" aria-labelledby={securityHeadingId} data-testid="settings-security">
      <header class="page-head">
        <h3 id={securityHeadingId}>{t("settings.security")}</h3>
        <p>{t("settings.securityHint")}</p>
      </header>
      <Select
        label={t("security.autoLock")}
        hint={t("security.autoLockHint")}
        options={autoLockOptions}
        bind:value={autoLock}
        onchange={changeAutoLock}
      />
      {#if vault.status === "unlocked" && quickAvailable}
        <Switch
          label={t(`security.quickUnlock.${quickPlatform}`)}
          hint={quickEnabled && !quickReady ? t("security.quickUnlockHint") : undefined}
          bind:checked={quickEnabled}
          disabled={quickBusy}
          onchange={changeQuickUnlock}
        />
      {/if}
    </section>
  {/if}

  <section class="page updates" aria-labelledby={updatesHeadingId} data-testid="settings-updates">
    <header class="page-head">
      <h3 id={updatesHeadingId}>{t("settings.updates")}</h3>
      <p>{t("settings.updatesHint")}</p>
    </header>

    <Switch label={t("update.auto")} hint={t("update.autoHint")} bind:checked={auto} onchange={() => void toggleAuto()} />
    <Switch
      label={t("update.prereleases")}
      hint={t("update.prereleasesHint")}
      bind:checked={prereleases}
      onchange={() => void togglePrereleases()}
    />

    <div class="about">
      {#if updates.settings?.currentVersion}
        <span class="version">{t("update.currentVersion", { version: updates.settings.currentVersion })}</span>
      {/if}
      <span class="actions">
        <Button size="compact" onclick={() => ui.open({ type: "releaseNotes" })}>{t("settings.releaseNotes")}</Button>
        <Button size="compact" onclick={() => ui.open({ type: "updates" })}>{t("update.menu")}</Button>
      </span>
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

  /* Security: the one line between the identity above and the updates below is this section's
     bottom border (or the identity's own when this section is absent). */
  .security {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding-bottom: var(--space-4);
    margin-bottom: var(--space-4);
    border-bottom: 1px solid var(--border-hairline);
  }
  .security .page-head {
    margin-bottom: 0;
  }

  /* Updates: the two switches, then the version with the ways to its notes and to a check. */
  .updates {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .updates .page-head {
    margin-bottom: 0;
  }
  .actions {
    display: flex;
    gap: var(--space-2);
  }
  .about {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }
  .version {
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
  .lift {
    margin-top: calc(-1 * var(--space-1));
  }

  /* ---- Theme: a mini window per option; System shows the two looks side by side ---- */
  .mini-row {
    display: grid;
    grid-template-columns: 1fr;
    aspect-ratio: 16 / 10;
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
    height: 100%;
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
  .scheme-text.scheme-bar {
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
