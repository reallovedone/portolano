<!-- Advisory password strength: a slim four-step meter with a text label (the text, not the
     colour, carries the meaning). The label is a polite live region. -->
<script lang="ts">
  import { t, type StringKey } from "../i18n/strings";
  import { passwordStrength, type Strength } from "./strength";

  let { password }: { password: string } = $props();
  const strength = $derived(passwordStrength(password));

  const STEPS: Record<Strength, number> = { empty: 0, weak: 1, fair: 2, good: 3, strong: 4 };
  const filled = $derived(STEPS[strength]);
</script>

<div class="strength" data-strength={strength}>
  <div class="meter" aria-hidden="true">
    {#each [1, 2, 3, 4] as step (step)}
      <span class="step" class:on={step <= filled}></span>
    {/each}
  </div>
  <span class="label" aria-live="polite">{t(`vault.strength.${strength}` as StringKey)}</span>
</div>

<style>
  .strength {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-height: var(--space-4);
  }

  .meter {
    display: flex;
    flex: 1;
    gap: var(--space-1);
  }

  .step {
    flex: 1;
    block-size: var(--space-1);
    background: var(--surface-inset);
    border: 1px solid var(--border-hairline);
    border-radius: var(--radius-full);
    transition: background var(--duration-base) var(--ease-standard);
  }

  .label {
    min-width: calc(var(--space-12) * 2);
    color: var(--text-secondary);
    font-size: var(--font-size-sm);
    text-align: right;
  }

  [data-strength="weak"] .step.on {
    background: var(--danger-text);
  }

  [data-strength="fair"] .step.on {
    background: var(--warning-text);
  }

  [data-strength="good"] .step.on,
  [data-strength="strong"] .step.on {
    background: var(--success-text);
  }
</style>
