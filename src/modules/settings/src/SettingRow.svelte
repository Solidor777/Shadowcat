<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { SettingField } from "./registerSetting";

  let {
    field,
    labelKey,
  }: {
    /** The value binding rendered as a toggle, select or text input. */
    field: SettingField;
    /** i18n key of the row's label. */
    labelKey: string;
  } = $props();

  const { t } = getAppContext();

  /** Bumped after every write so `value` re-reads `field.get()` even when the getter reads no
   * reactive state of its own. */
  let revision = $state(0);
  const value = $derived.by(() => {
    revision;
    return field.get();
  });

</script>

<label class="setting-row">
  <span>{t(labelKey)}</span>
  {#if field.kind === "toggle"}
    <input
      type="checkbox"
      aria-label={t(labelKey)}
      checked={value as boolean}
      onchange={(e) => {
        field.set(e.currentTarget.checked);
        revision += 1;
      }}
    />
  {:else if field.kind === "select"}
    <select
      aria-label={t(labelKey)}
      value={value as string}
      onchange={(e) => {
        field.set(e.currentTarget.value);
        revision += 1;
      }}
    >
      {#each field.options as option (option.value)}
        <option value={option.value}>{t(option.labelKey)}</option>
      {/each}
    </select>
  {:else}
    <input
      type="text"
      aria-label={t(labelKey)}
      placeholder={field.placeholder}
      value={value as string}
      onchange={(e) => {
        field.set(e.currentTarget.value);
        revision += 1;
      }}
    />
  {/if}
</label>

<style lang="scss">
  .setting-row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  @media (pointer: coarse) {
    .setting-row :is(input, select) {
      min-height: var(--input-height-coarse);
    }
  }
</style>
