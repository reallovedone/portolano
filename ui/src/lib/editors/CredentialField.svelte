<!-- Credential reference editor: Inherit / None / specific credential, plus a line that says
     which credential is effectively used and where it comes from. -->
<script lang="ts">
  import { t } from "../i18n/strings";
  import type { Node, Uuid } from "../ipc";
  import { getApp } from "../state/app";
  import { effectiveCredential } from "../tree/model";
  import Button from "../ui/Button.svelte";
  import FormRow from "../ui/FormRow.svelte";
  import Icon from "../ui/Icon.svelte";
  import RadioGroup from "../ui/RadioGroup.svelte";
  import Select from "../ui/Select.svelte";
  import { draftToCredential, type CredentialDraft } from "./form";

  let {
    value = $bindable(),
    nodes,
    parentId,
    error = null,
    idPrefix,
  }: {
    value: CredentialDraft;
    nodes: readonly Node[];
    /** Parent folder of the node being edited (start of the inheritance walk). */
    parentId: Uuid | null;
    error?: string | null;
    idPrefix: string;
  } = $props();

  const { vault } = getApp();

  const effective = $derived(effectiveCredential(nodes, draftToCredential(value), parentId));

  function credentialLabel(id: Uuid): string {
    return vault.name(id) ?? t("editor.credential.unknown");
  }

  const effectiveText = $derived.by(() => {
    if (effective.kind === "unset") return t("editor.credential.effectiveUnset");
    const own = effective.source.own;
    const folder = effective.source.own ? "" : effective.source.folderName;
    if (effective.kind === "none") {
      return own
        ? t("editor.credential.effectiveNone")
        : t("editor.credential.effectiveNoneFrom", { folder });
    }
    const name = credentialLabel(effective.id);
    return own
      ? t("editor.credential.effective", { name })
      : t("editor.credential.effectiveFrom", { name, folder });
  });

  const modeOptions = [
    { value: "inherit", label: t("editor.credential.inherit"), hint: t("editor.credential.inheritHint") },
    { value: "none", label: t("editor.credential.none"), hint: t("editor.credential.noneHint") },
    { value: "id", label: t("editor.credential.id"), hint: t("editor.credential.idHint") },
  ];

  const credentialOptions = $derived([
    { value: "", label: t("editor.credential.pick") },
    ...vault.credentials.map((c) => ({ value: c.id, label: `${c.name} (${c.username})` })),
    ...(value.id !== "" && !vault.credentials.some((c) => c.id === value.id)
      ? [{ value: value.id, label: t("editor.credential.unknown") }]
      : []),
  ]);
</script>

<FormRow label={t("editor.credential")}>
  <RadioGroup
    label={t("editor.credential")}
    options={modeOptions}
    idPrefix="{idPrefix}-cred"
    name="{idPrefix}-cred-mode"
    bind:value={() => value.mode, (next) => (value = { ...value, mode: next as typeof value.mode })}
  />

  {#if value.mode === "id"}
    {#if vault.unlocked}
      <Select
        id="{idPrefix}-cred-id"
        aria-label={t("editor.credential.pick")}
        options={credentialOptions}
        {error}
        bind:value={() => value.id, (next) => (value = { ...value, id: next })}
      />
    {:else}
      <div class="locked">
        <span class="locked-text">{t("editor.credential.locked")}</span>
        <Button size="compact" onclick={() => vault.ensureUnlocked(null)}>
          {t("editor.credential.unlockNow")}
        </Button>
      </div>
      {#if error}<p class="error" role="alert">{error}</p>{/if}
    {/if}
  {:else if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <p class="effective">
    <Icon name="info" size={14} />
    <span data-testid="effective-credential">{effectiveText}</span>
  </p>
</FormRow>

<style>
  .locked {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .locked-text {
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
  }

  .error {
    color: var(--danger-text);
    font-size: var(--font-size-sm);
  }

  .effective {
    display: flex;
    align-items: flex-start;
    gap: var(--space-1);
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
  }

  .effective :global(svg) {
    flex: none;
    margin-top: 1px;
  }
</style>
