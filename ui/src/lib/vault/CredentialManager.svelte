<!-- Credential manager: list, add/edit/delete, explicit reveal/copy with auto-hide. -->
<script lang="ts">
  import { onDestroy } from "svelte";

  import { t } from "../i18n/strings";
  import type { CredentialSummary, RevealedSecret, Uuid } from "../ipc";
  import { getApp } from "../state/app";
  import { credentialUsers } from "../tree/model";
  import Button from "../ui/Button.svelte";
  import ConfirmDialog from "../ui/ConfirmDialog.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import EmptyState from "../ui/EmptyState.svelte";
  import ContextMenu from "../ui/ContextMenu.svelte";
  import Icon from "../ui/Icon.svelte";
  import IconButton from "../ui/IconButton.svelte";
  import ListRow from "../ui/ListRow.svelte";
  import SegmentedControl from "../ui/SegmentedControl.svelte";
  import TextArea from "../ui/TextArea.svelte";
  import TextField from "../ui/TextField.svelte";
  import type { MenuItem } from "../ui/menu";
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

  let query = $state("");
  let kindFilter = $state("all");
  let menu = $state<{ x: number; y: number; items: MenuItem[]; label: string } | null>(null);

  const sorted = $derived(
    [...vault.credentials].sort((a, b) =>
      a.name.localeCompare(b.name, undefined, { sensitivity: "base" }),
    ),
  );
  const hasPasswords = $derived(sorted.some((c) => c.kind === "password"));
  const hasKeys = $derived(sorted.some((c) => c.kind === "sshKey"));
  /** The type filter is offered only when both types exist; otherwise it is ignored. */
  const kindActive = $derived(hasPasswords && hasKeys ? kindFilter : "all");
  const needle = $derived(query.trim().toLocaleLowerCase());
  const visible = $derived(
    sorted.filter(
      (c) =>
        (kindActive === "all" || c.kind === kindActive) &&
        (needle === "" ||
          [c.name, c.username, c.domain ?? ""].some((v) => v.toLocaleLowerCase().includes(needle))),
    ),
  );
  const filtered = $derived(needle !== "" || kindActive !== "all");
  const kindOptions = $derived([
    { value: "all", label: t("cred.filter.all") },
    { value: "password", label: t("cred.filter.password") },
    { value: "sshKey", label: t("cred.filter.sshKey") },
  ]);

  function usage(credential: CredentialSummary): string {
    const count = credentialUsers(doc.nodes, credential.id).length;
    if (count === 0) return t("cred.unused");
    return t(count === 1 ? "cred.usedBy.one" : "cred.usedBy.other", { count });
  }

  function clearFilters(): void {
    query = "";
    kindFilter = "all";
  }

  function openMenuAt(event: MouseEvent, items: MenuItem[], label: string): void {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    menu = { x: rect.left, y: rect.bottom, items, label };
  }

  function rowMenu(event: MouseEvent, credential: CredentialSummary): void {
    openMenuAt(
      event,
      [
        { label: t("cred.edit"), icon: "edit", action: () => (editor = { credential }) },
        {
          label: t("cred.delete"),
          icon: "trash",
          danger: true,
          action: () => (deleting = { credential, error: null }),
        },
      ],
      t("cred.rowActions", { name: credential.name }),
    );
  }

  function barMenu(event: MouseEvent): void {
    openMenuAt(
      event,
      [
        { label: t("cred.changePassword"), action: () => (changing = true) },
        { label: t("vault.lock"), icon: "lock", action: () => void lock() },
      ],
      t("cred.moreActions"),
    );
  }

  function onRowDoubleClick(event: MouseEvent, credential: CredentialSummary): void {
    if ((event.target as HTMLElement).closest("button")) return;
    editor = { credential };
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
    {#if vault.credentials.length === 0}
      <div class="bar">
        <span class="spacer"></span>
        <IconButton icon="more" label={t("cred.moreActions")} onclick={barMenu} />
      </div>
      <EmptyState icon="key" title={t("cred.emptyTitle")} text={t("cred.emptyText")}>
        {#snippet action()}
          <Button variant="primary" onclick={() => (editor = { credential: null })}>
            {#snippet icon()}<Icon name="plus" />{/snippet}
            {t("cred.add")}
          </Button>
        {/snippet}
      </EmptyState>
    {:else}
      <div class="bar">
        <div class="search">
          <TextField
            bind:value={query}
            type="search"
            placeholder={t("cred.search")}
            aria-label={t("cred.search")}
            autocomplete="off"
            spellcheck="false"
          >
            {#snippet prefix()}<Icon name="search" size={14} />{/snippet}
          </TextField>
        </div>
        {#if hasPasswords && hasKeys}
          <SegmentedControl label={t("cred.filter")} options={kindOptions} bind:value={kindFilter} />
        {/if}
        <Button variant="primary" onclick={() => (editor = { credential: null })}>
          {#snippet icon()}<Icon name="plus" />{/snippet}
          {t("cred.add")}
        </Button>
        <IconButton icon="more" label={t("cred.moreActions")} onclick={barMenu} />
      </div>
      <p class="count" data-testid="cred-count">
        {filtered
          ? t("cred.countFiltered", { shown: visible.length, total: sorted.length })
          : t(sorted.length === 1 ? "cred.count.one" : "cred.count.other", { count: sorted.length })}
      </p>

      <div class="scroll">
        {#if visible.length === 0}
          <EmptyState icon="search" title={t("cred.noMatch", { query: query.trim() })}>
            {#snippet action()}
              <Button onclick={clearFilters}>{t("cred.clearSearch")}</Button>
            {/snippet}
          </EmptyState>
        {:else}
          <ul class="list" aria-label={t("cred.title")}>
            {#each visible as credential (credential.id)}
              <li>
                <ListRow ondblclick={(e: MouseEvent) => onRowDoubleClick(e, credential)}>
                  {#snippet leading()}
                    <span
                      class="type"
                      role="img"
                      aria-label={t(`cred.kind.${credential.kind}`)}
                      title={t(`cred.kind.${credential.kind}`)}
                    >
                      <Icon name={credential.kind === "sshKey" ? "key" : "lock"} />
                    </span>
                  {/snippet}
                  {credential.name}
                  {#snippet secondary()}
                    {credential.domain ? `${credential.domain}\\` : ""}{credential.username} · {usage(
                      credential,
                    )}
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
                        icon="more"
                        size="compact"
                        label={t("cred.rowActions", { name: credential.name })}
                        onclick={(e: MouseEvent) => rowMenu(e, credential)}
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
      </div>
    {/if}
  {/if}
  {#snippet footer()}
    <Button onclick={onclose}>{t("common.close")}</Button>
  {/snippet}
</Dialog>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} label={menu.label} onclose={() => (menu = null)} />
{/if}

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
  .bar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .search {
    flex: 1;
    min-width: 0;
  }

  .spacer {
    flex: 1;
  }

  .count {
    margin: var(--space-2) 0;
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
  }

  .scroll {
    max-height: min(52vh, 28rem);
    overflow: auto;
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
