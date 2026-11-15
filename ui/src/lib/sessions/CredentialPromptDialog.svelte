<!-- The credential prompt shared by the terminal and framebuffer views (WP11): shown when a
     session has no credential (or the last one did not work). The answer goes to the backend
     and is used for that session only; the secret fields are cleared as soon as it is sent,
     and nothing typed here is kept in any long-lived state.

     Fields follow `prompt.kind`:
       password          a secret only (VNC)
       usernamePassword  user name, password, optional domain (RDP)
       sshKey            user name, then a password or a private key file + passphrase (SSH)
     Enter submits, Escape cancels. Focus starts on the first empty field. -->
<script lang="ts">
  import { t } from "../i18n/strings";
  import type { CredentialAnswer, CredentialPrompt } from "../ipc";
  import { getApp } from "../state/app";
  import type { ConnectionKind } from "../tree/model";
  import Banner from "../ui/Banner.svelte";
  import Button from "../ui/Button.svelte";
  import Checkbox from "../ui/Checkbox.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import KindIcon from "../ui/KindIcon.svelte";
  import SegmentedControl from "../ui/SegmentedControl.svelte";
  import TextField from "../ui/TextField.svelte";

  let {
    prompt,
    connectionName = null,
    onsubmit,
    oncancel,
  }: {
    prompt: CredentialPrompt;
    /** Shown in the intro ("“name” has no credential"). */
    connectionName?: string | null;
    onsubmit: (answer: CredentialAnswer, saveToVault: boolean) => void;
    oncancel: () => void;
  } = $props();

  const { vault } = getApp();
  const uid = `cp-${Math.random().toString(36).slice(2, 9)}`;

  // The dialog is created per prompt: the hints are read once, the fields then belong to the
  // user.
  // svelte-ignore state_referenced_locally
  let username = $state(prompt.usernameHint ?? "");
  // svelte-ignore state_referenced_locally
  let domain = $state(prompt.domainHint ?? "");
  let password = $state("");
  let passphrase = $state("");
  let keyPath = $state("");
  let method = $state<"password" | "key">("password");
  let save = $state(false);
  let problem = $state<string | null>(null);

  const asksUsername = $derived(prompt.kind !== "password");
  const asksDomain = $derived(prompt.kind === "usernamePassword");
  const asksMethod = $derived(prompt.kind === "sshKey");
  const usesKey = $derived(asksMethod && method === "key");
  // Saving needs an unlocked vault; the backend and the vault store both say so.
  const canSave = $derived(prompt.canSaveToVault && vault.unlocked);

  // The first empty field gets the focus (the Dialog focuses `[data-autofocus]` on mount).
  // svelte-ignore state_referenced_locally
  const focusField: "username" | "secret" = asksUsername && username.trim() === "" ? "username" : "secret";

  // The prompt kinds map onto the protocols that use them (SSH, RDP, VNC).
  const KIND_FOR: Record<CredentialPrompt["kind"], ConnectionKind> = {
    sshKey: "ssh",
    usernamePassword: "rdp",
    password: "vnc",
  };
  const kind = $derived(KIND_FOR[prompt.kind]);
  const methodOptions = [
    { value: "password", label: t("prompt.method.password") },
    { value: "key", label: t("prompt.method.key") },
  ];

  function submit(event: SubmitEvent): void {
    event.preventDefault();
    if (asksUsername && username.trim() === "") {
      problem = t("prompt.needUsername");
      return;
    }
    if (usesKey) {
      if (keyPath.trim() === "") {
        problem = t("prompt.needKeyPath");
        return;
      }
    } else if (password === "") {
      problem = t("prompt.needPassword");
      return;
    }
    const answer: CredentialAnswer = {
      username: username.trim(),
      password: usesKey ? null : password,
      keyPath: usesKey ? keyPath.trim() : null,
      passphrase: usesKey && passphrase !== "" ? passphrase : null,
      domain: asksDomain && domain.trim() !== "" ? domain.trim() : null,
    };
    const keep = canSave && save;
    // Clear the secrets before handing the answer over: the inputs never outlive the submit.
    password = "";
    passphrase = "";
    onsubmit(answer, keep);
  }

  function cancel(): void {
    password = "";
    passphrase = "";
    oncancel();
  }
</script>

<Dialog title={t("prompt.title")} onclose={cancel}>
  <form id="{uid}-form" onsubmit={submit} novalidate>
    <div class="target">
      <span class="plate"><KindIcon {kind} size={28} /></span>
      <div class="target-text">
        <p class="lead">
          {connectionName ? t("prompt.introNamed", { name: connectionName }) : t("prompt.introPlain")}
        </p>
        <p class="where">
          <span class="where-label">{t("prompt.targetLabel")}</span>
          <code>{prompt.target}</code>
        </p>
      </div>
    </div>

    {#if prompt.error}
      <Banner variant="danger"><span data-testid="prompt-error">{prompt.error}</span></Banner>
    {/if}
    {#if problem}
      <Banner variant="danger">{problem}</Banner>
    {/if}

    {#if asksUsername}
      <TextField
        id="{uid}-user"
        label={t("prompt.username")}
        autocomplete="off"
        autocapitalize="off"
        spellcheck="false"
        bind:value={username}
        data-autofocus={focusField === "username" ? "" : undefined}
      />
    {/if}

    {#if asksDomain}
      <TextField
        id="{uid}-domain"
        label={t("prompt.domain")}
        autocomplete="off"
        bind:value={domain}
      />
    {/if}

    {#if asksMethod}
      <div class="method">
        <SegmentedControl
          label={t("prompt.method")}
          options={methodOptions}
          bind:value={() => method, (next) => (method = next === "key" ? "key" : "password")}
        />
      </div>
    {/if}

    {#if usesKey}
      <TextField
        id="{uid}-key"
        label={t("prompt.keyPath")}
        autocomplete="off"
        spellcheck="false"
        placeholder="~/.ssh/id_ed25519"
        hint={t("prompt.keyHint")}
        bind:value={keyPath}
        data-autofocus={focusField === "secret" ? "" : undefined}
      />
      <TextField
        id="{uid}-pass"
        type="password"
        label={t("prompt.passphrase")}
        autocomplete="off"
        bind:value={passphrase}
      />
    {:else}
      <TextField
        id="{uid}-pw"
        type="password"
        label={t("prompt.password")}
        autocomplete="off"
        bind:value={password}
        data-autofocus={focusField === "secret" ? "" : undefined}
      />
    {/if}

    {#if canSave}
      <Checkbox bind:checked={save} label={t("prompt.save")} hint={t("prompt.saveHint")} />
    {/if}
  </form>
  {#snippet footer()}
    <Button onclick={cancel}>{t("common.cancel")}</Button>
    <Button type="submit" variant="primary" form="{uid}-form">{t("prompt.connect")}</Button>
  {/snippet}
</Dialog>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .target {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .plate {
    display: grid;
    flex: none;
    place-items: center;
    inline-size: calc(var(--space-4) * 3);
    block-size: calc(var(--space-4) * 3);
    background: var(--surface-inset);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-panel);
  }

  .target-text {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .lead {
    font-weight: var(--font-weight-medium);
  }

  .where {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--space-2);
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
  }

  .where code {
    color: var(--text-primary);
    font-family: var(--font-mono);
    word-break: break-all;
  }

  .method {
    display: flex;
  }
</style>
