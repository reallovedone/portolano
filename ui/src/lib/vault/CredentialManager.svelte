<!-- Credential manager: list, add/edit/delete, explicit reveal/copy with auto-hide. -->
<script lang="ts">
  import { onDestroy, onMount } from "svelte";

  import { t, type StringKey } from "../i18n/strings";
  import type { CredentialSummary, RevealedSecret, Uuid } from "../ipc";
  import { getApp } from "../state/app";
  import { credentialUsers } from "../tree/model";
  import Button from "../ui/Button.svelte";
  import ConfirmDialog from "../ui/ConfirmDialog.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import EmptyState from "../ui/EmptyState.svelte";
  import FormSection from "../ui/FormSection.svelte";
  import Icon from "../ui/Icon.svelte";
  import IconButton from "../ui/IconButton.svelte";
  import ListRow from "../ui/ListRow.svelte";
  import Select from "../ui/Select.svelte";
  import TextArea from "../ui/TextArea.svelte";
  import ChangePasswordDialog from "./ChangePasswordDialog.svelte";
  import { CLIPBOARD_CLEAR_SECONDS, copySecret } from "./clipboard";
  import CredentialEditor from "./CredentialEditor.svelte";

  let { onclose }: { onclose: () => void } = $props();

  const { vault, notices, doc } = getApp();
  /** At most this many names of the connections and folders that use a credential are listed. */
  const MAX_NAMES = 3;
  const REVEAL_SECONDS = 15;

  let editor = $state<{ credential: CredentialSummary | null } | null>(null);
  let deleting = $state<{ credential: CredentialSummary; error: string | null } | null>(null);
  let changing = $state(false);
  let revealed = $state<{ id: Uuid; secret: RevealedSecret } | null>(null);
  let timer: ReturnType<typeof setTimeout> | null = null;

  function hideSecret(): void {
    if (timer !== null) clearTimeout(timer);
    timer = null;
    revealed = null;
  }

  onDestroy(hideSecret);

  // The secret must not outlive the unlocked state.
  $effect(() => {
    if (!vault.unlocked) hideSecret();
  });

  async function reveal(credential: CredentialSummary): Promise<void> {
    hideSecret();
    const result = await vault.reveal(credential.id);
    if (!result.ok) {
      notices.error(t("cred.reveal"), result.error);
      return;
    }
    revealed = { id: credential.id, secret: result.value };
    timer = setTimeout(hideSecret, REVEAL_SECONDS * 1000);
  }

  async function copy(credential: CredentialSummary): Promise<void> {
    const result = await vault.reveal(credential.id);
    if (!result.ok) {
      notices.error(t("cred.copySecret"), result.error);
      return;
    }
    const text =
      result.value.type === "password" ? result.value.password : result.value.privateKeyPem;
    try {
      await copySecret(text, {
        onNotCleared: () => notices.push("warning", t("cred.clipboardNotCleared")),
      });
      notices.push("info", t("cred.copied", { seconds: CLIPBOARD_CLEAR_SECONDS }));
    } catch (e) {
      notices.push("error", t("cred.copySecret"), e instanceof Error ? e.message : "clipboard error");
    }
  }

  async function confirmDelete(): Promise<void> {
    if (!deleting) return;
    const target = deleting.credential;
    const result = await vault.remove(target.id);
    if (result.ok) {
      if (revealed?.id === target.id) hideSecret();
      deleting = null;
      // The backend turned every reference to the credential into "none": show that.
      await doc.reload();
    } else {
      deleting = { credential: target, error: `${result.error.message} (${result.error.code})` };
    }
  }

  /** The delete confirmation: who uses the credential directly and what happens to them. */
  function deleteMessage(credential: CredentialSummary): string {
    const base = t("cred.deleteBody", { name: credential.name });
    const users = credentialUsers(doc.nodes, credential.id);
    if (users.length === 0) return base;
    const names = users.slice(0, MAX_NAMES).map((n) => `“${n.name}”`);
    if (users.length > MAX_NAMES) {
      names.push(t("cred.deleteUsedMore", { count: users.length - MAX_NAMES }));
    }
    const detail = t(users.length === 1 ? "cred.deleteUsed.one" : "cred.deleteUsed.other", {
      count: users.length,
      names: names.join(", "),
    });
    return `${base}\n\n${detail}`;
  }

  /** Minutes the user can pick for the vault auto-lock (0 = never), as the backend accepts them. */
  const AUTO_LOCK_CHOICES = [0, 5, 15, 30, 60] as const;
  const autoLockOptions = AUTO_LOCK_CHOICES.map((minutes) => ({
    value: String(minutes),
    label: t(`security.autoLock.${minutes}` as StringKey),
  }));
  let autoLock = $state<string>("15");
  /** The value last confirmed by the backend; the choice goes back to it when saving fails. */
  let savedAutoLock = "15";

  onMount(async () => {
    const result = await vault.getAutoLock();
    if (!result.ok) {
      notices.error(t("security.title"), result.error);
      return;
    }
    savedAutoLock = String(result.value);
    autoLock = savedAutoLock;
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

  async function lock(): Promise<void> {
    const result = await vault.lock();
    if (!result.ok) notices.error(t("vault.lock"), result.error);
  }
</script>

<Dialog title={t("cred.title")} {onclose} size="lg">
  {#if !vault.unlocked}
    <EmptyState
      icon="lock"
      title={vault.status === "notCreated" ? t("cred.noVaultTitle") : t("cred.lockedTitle")}
      text={vault.status === "notCreated" ? t("vault.noVaultYet") : t("cred.lockedHint")}
    />
  {:else}
    <div class="toolbar">
      {#if vault.credentials.length > 0}
        <Button variant="primary" onclick={() => (editor = { credential: null })}>
          {#snippet icon()}<Icon name="plus" />{/snippet}
          {t("cred.add")}
        </Button>
      {/if}
      <span class="spacer"></span>
      <Button variant="plain" onclick={() => (changing = true)}>{t("cred.changePassword")}</Button>
      <Button variant="plain" onclick={lock}>{t("vault.lock")}</Button>
    </div>

    {#if vault.credentials.length === 0}
      <EmptyState icon="key" title={t("cred.emptyTitle")} text={t("cred.emptyText")}>
        {#snippet action()}
          <Button variant="primary" onclick={() => (editor = { credential: null })}>
            {#snippet icon()}<Icon name="plus" />{/snippet}
            {t("cred.add")}
          </Button>
        {/snippet}
      </EmptyState>
    {:else}
      <ul class="list" aria-label={t("cred.title")}>
        {#each vault.credentials as credential (credential.id)}
          <li>
            <ListRow>
              {#snippet leading()}
                <span class="type"><Icon name={credential.kind === "sshKey" ? "key" : "lock"} /></span>
              {/snippet}
              {credential.name}
              {#snippet secondary()}
                {t(`cred.kind.${credential.kind}` as StringKey)} · {credential.domain
                  ? `${credential.domain}\\`
                  : ""}{credential.username}
              {/snippet}
              {#snippet trailing()}
                <span class="buttons">
                  {#if revealed?.id === credential.id}
                    <IconButton
                      icon="eye-off"
                      size="compact"
                      label={t("cred.hideSecret")}
                      onclick={hideSecret}
                    />
                  {:else}
                    <IconButton
                      icon="eye"
                      size="compact"
                      label={t("cred.reveal")}
                      onclick={() => reveal(credential)}
                    />
                  {/if}
                  <IconButton
                    icon="copy"
                    size="compact"
                    label={t("cred.copySecret")}
                    onclick={() => copy(credential)}
                  />
                  <IconButton
                    icon="edit"
                    size="compact"
                    label={t("common.edit")}
                    onclick={() => (editor = { credential })}
                  />
                  <IconButton
                    icon="trash"
                    size="compact"
                    label={t("common.delete")}
                    onclick={() => (deleting = { credential, error: null })}
                  />
                </span>
              {/snippet}
            </ListRow>
            {#if revealed?.id === credential.id}
              <div class="secret" role="status" data-testid="revealed-secret">
                <span class="secret-note">
                  <Icon name="clock" size={14} />
                  {t("cred.revealWarning", { seconds: REVEAL_SECONDS })}
                </span>
                {#if revealed.secret.type === "password"}
                  <code>{revealed.secret.password}</code>
                {:else}
                  <TextArea
                    readonly
                    monospace
                    rows={5}
                    aria-label={t("cred.privateKey")}
                    spellcheck="false"
                    value={revealed.secret.privateKeyPem}
                  />
                  {#if revealed.secret.passphrase}
                    <code>{revealed.secret.passphrase}</code>
                  {/if}
                {/if}
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}

    <FormSection title={t("security.title")}>
      <Select
        label={t("security.autoLock")}
        hint={t("security.autoLockHint")}
        options={autoLockOptions}
        bind:value={autoLock}
        onchange={changeAutoLock}
      />
    </FormSection>
  {/if}
  {#snippet footer()}
    <Button onclick={onclose}>{t("common.close")}</Button>
  {/snippet}
</Dialog>

{#if editor}
  <CredentialEditor credential={editor.credential} onclose={() => (editor = null)} />
{/if}

{#if deleting}
  <ConfirmDialog
    title={t("cred.deleteTitle")}
    message={deleteMessage(deleting.credential)}
    error={deleting.error}
    onconfirm={confirmDelete}
    oncancel={() => (deleting = null)}
  />
{/if}

{#if changing}
  <ChangePasswordDialog
    onclose={(changed) => {
      changing = false;
      if (changed) notices.push("info", t("cred.passwordChanged"));
    }}
  />
{/if}

<style>
  .toolbar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-3);
  }

  .spacer {
    flex: 1;
  }

  .list {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .list li + li {
    margin-top: var(--space-1);
    padding-top: var(--space-1);
    border-top: 1px solid var(--border-hairline);
  }

  .type {
    display: inline-flex;
    color: var(--text-secondary);
  }

  .buttons {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }

  .secret {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: var(--space-1) var(--space-2) var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--surface-inset);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-control);
  }

  .secret-note {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
  }

  .secret code {
    font-family: var(--font-mono);
    user-select: all;
    word-break: break-all;
  }
</style>
