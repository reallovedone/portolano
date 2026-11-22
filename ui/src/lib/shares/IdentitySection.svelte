<!-- Settings → Identity: the name and fingerprint that sign the shared folders, the public code to
     send to others, a password-protected backup, and (only while there is no identity) a
     restore from a backup. Backup passwords are wiped from the fields as soon as they are used. -->
<script lang="ts">
  import { onMount } from "svelte";

  import { t } from "../i18n/strings";
  import { toIpcError, type IdentitySummary } from "../ipc";
  import { getApp } from "../state/app";
  import Banner from "../ui/Banner.svelte";
  import Button from "../ui/Button.svelte";
  import TextField from "../ui/TextField.svelte";
  import BackupForm from "./BackupForm.svelte";
  import { displayText } from "./display";

  const { ipc, files, vault, notices } = getApp();
  const uid = $props.id();
  const headingId = `identity-${uid}`;

  let identity = $state<IdentitySummary | null>(null);
  let loaded = $state(false);
  let mode = $state<"idle" | "export" | "import">("idle");
  let name = $state("");
  let importPath = $state<string | null>(null);
  let importPassword = $state("");
  let error = $state<string | null>(null);
  let busy = $state(false);

  function fail(cause: unknown): void {
    const e = toIpcError(cause);
    error = `${t("share.identityFailed")}: ${e.message} (${e.code})`;
  }

  // The identity lives in the vault. Opening Settings never asks for the password: the identity
  // is read only while the vault is already unlocked, and again whenever its status changes.
  // While it is locked (or missing) we cannot tell whether an identity exists, so neither
  // Create nor Import is offered (that would clash with the identity we cannot see).
  // Reading the identity failed: it may exist, so Create and Import stay hidden.
  let readFailed = $state(false);
  let reading = 0;

  async function loadIdentity(): Promise<void> {
    const mine = ++reading;
    error = null;
    readFailed = false;
    try {
      const found = await ipc.identityGet();
      if (mine === reading) identity = found;
    } catch (cause) {
      if (mine !== reading) return;
      readFailed = true;
      fail(cause);
    }
    if (mine === reading) loaded = true;
  }

  // Reading the vault status failed while it is still unknown: show why, with a retry.
  let statusFailed = $state(false);

  async function readStatus(): Promise<void> {
    statusFailed = false;
    error = null;
    const result = await vault.refresh();
    if (!result.ok) {
      statusFailed = true;
      fail(result.error);
    }
  }

  onMount(() => {
    // Only reads the status (no prompt); a store that already knows it makes this a no-op.
    if (vault.status === "unknown") void readStatus();
  });

  $effect(() => {
    const status = vault.status;
    if (status === "unlocked") {
      // Hide Create/Import until the identity is known again (lock -> unlock transition).
      loaded = false;
      statusFailed = false;
      void loadIdentity();
    } else {
      // Locked, not created, or not known yet: what was shown is stale.
      reading++;
      identity = null;
      readFailed = false;
      mode = "idle";
      // A banner from the unlocked session (or a stale read) no longer applies.
      if (status !== "unknown") error = null;
      loaded = status !== "unknown";
    }
  });

  /** The user asks for it: the unlock (or create) screen. Loading follows the status change. */
  async function unlock(): Promise<void> {
    await vault.ensureUnlocked(null);
  }

  async function create(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (busy) return;
    if (name.trim() === "") {
      error = t("share.nameRequired");
      return;
    }
    busy = true;
    error = null;
    try {
      if (!(await vault.ensureUnlocked(null))) return;
      identity = await ipc.identityCreate(name.trim());
      name = "";
      notices.push("info", t("share.identityCreated"));
    } catch (cause) {
      fail(cause);
    } finally {
      busy = false;
    }
  }

  async function copyCode(): Promise<void> {
    if (!identity) return;
    try {
      // The public code holds the public name and key only: it is meant to be sent around.
      await navigator.clipboard.writeText(identity.publicCode);
      notices.push("info", t("share.identityCopied"));
    } catch {
      notices.push("warning", t("share.identityCopyFailed"));
    }
  }

  async function chooseImport(): Promise<void> {
    error = null;
    try {
      const chosen = await files.pickOpen({
        title: t("share.identityImportTitle"),
        filters: [{ name: t("share.identityTitle"), extensions: ["portolano-id"] }],
      });
      if (chosen !== null) {
        importPath = chosen;
        mode = "import";
      }
    } catch (cause) {
      fail(cause);
    }
  }

  function leaveImport(): void {
    importPassword = "";
    importPath = null;
    mode = "idle";
  }

  async function runImport(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (busy || importPath === null) return;
    busy = true;
    error = null;
    try {
      if (!(await vault.ensureUnlocked(null))) return;
      identity = await ipc.identityImport(importPath, importPassword);
      notices.push("info", t("share.identityImported"));
      leaveImport();
    } catch (cause) {
      fail(cause);
    } finally {
      // Used or failed, the password is not kept.
      importPassword = "";
      busy = false;
    }
  }
</script>

<section class="group" aria-labelledby={headingId}>
  <h4 id={headingId}>{t("share.identityTitle")}</h4>
  <p class="hint">{t("share.identityHint")}</p>

  {#if error}
    <Banner variant="danger">{error}</Banner>
  {/if}

  {#if statusFailed && vault.status === "unknown"}
    <div class="row">
      <Button size="compact" onclick={readStatus}>{t("app.retry")}</Button>
    </div>
  {:else if !loaded}
    <!-- Nothing until the identity is known, so the buttons do not jump. -->
  {:else if vault.status === "notCreated"}
    <p class="hint">{t("share.identityNoVault")}</p>
    <div class="row">
      <Button size="compact" onclick={unlock}>{t("share.identityCreateVault")}</Button>
    </div>
  {:else if vault.status !== "unlocked"}
    <p class="hint">{t("share.identityLockedSection")}</p>
    <div class="row">
      <Button size="compact" onclick={unlock}>{t("share.identityUnlock")}</Button>
    </div>
  {:else if readFailed}
    <!-- The identity could not be read: do not offer to create another one. -->
  {:else if identity}
    <div class="who">
      <span class="clip" title={displayText(identity.name)}>{displayText(identity.name)}</span>
      <span class="mono">{identity.fingerprint}</span>
    </div>
    {#if mode === "export"}
      <BackupForm ondone={() => (mode = "idle")} />
    {:else}
      <div class="row">
        <Button size="compact" onclick={copyCode}>{t("share.identityCopy")}</Button>
        <Button size="compact" onclick={() => (mode = "export")}>{t("share.identityExport")}</Button>
      </div>
    {/if}
  {:else if mode === "import"}
    <form class="form" onsubmit={runImport} novalidate>
      <TextField
        type="password"
        label={t("share.identityImportPassword")}
        autocomplete="off"
        bind:value={importPassword}
        data-autofocus
      />
      <div class="row">
        <Button onclick={leaveImport}>{t("common.cancel")}</Button>
        <Button type="submit" variant="primary" disabled={busy}>{t("share.identityImportRun")}</Button>
      </div>
    </form>
  {:else}
    <p class="hint">{t("share.identityNone")}</p>
    <form class="form" onsubmit={create} novalidate>
      <TextField label={t("share.identityName")} maxlength={100} bind:value={name} />
      <div class="row">
        <Button type="submit" variant="primary" size="compact" disabled={busy}>
          {t("share.identityCreate")}
        </Button>
        <Button size="compact" onclick={chooseImport}>{t("share.identityImport")}</Button>
      </div>
    </form>
  {/if}
</section>

<style>
  .group {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding-bottom: var(--space-4);
    margin-bottom: var(--space-4);
    border-bottom: 1px solid var(--border-hairline);
  }
  h4 {
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-semibold);
  }
  .hint {
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
  }
  .who {
    display: flex;
    align-items: baseline;
    gap: var(--space-3);
    min-width: 0;
  }
  .clip {
    min-width: 0;
    overflow: hidden;
    font-weight: var(--font-weight-semibold);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mono {
    flex: none;
    font-family: var(--font-mono);
    color: var(--text-secondary);
  }
  .row {
    display: flex;
    gap: var(--space-2);
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
</style>
