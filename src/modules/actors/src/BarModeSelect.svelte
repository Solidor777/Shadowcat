<script lang="ts">
  // One select over the four bar display modes, shared by the bar list, the actor nameplate mode
  // and the per-token nameplate override. When `inheritLabel` is given an extra first option maps
  // to `null` (no value of its own).
  import { getAppContext } from "@shadowcat/ui-kit";
  import { BAR_DISPLAY_MODES, type BarDisplayMode } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  let { value, label, inheritLabel, testId, onChange }: {
    /** The current mode, or `null` for "inherit" (only meaningful with `inheritLabel`). */
    value: BarDisplayMode | null;
    /** The select's accessible name (already translated). */
    label: string;
    /** The translated text of the leading inherit option; absent ⇒ no inherit option. */
    inheritLabel?: string;
    /** Optional `data-testid` for the select. */
    testId?: string;
    /** Called with the chosen mode, or `null` when the inherit option is chosen. */
    onChange: (mode: BarDisplayMode | null) => void;
  } = $props();

  /** Maps the select's raw value to a mode, treating the inherit option as `null`.
   * @param raw The select's value.
   * @example
   * ```
   * // private helper; wired to the select's onchange
   * choose("always");
   * ```
   */
  function choose(raw: string): void {
    onChange(raw === "inherit" ? null : (raw as BarDisplayMode));
  }
</script>

<select aria-label={label} data-testid={testId} value={value ?? "inherit"} onchange={(e) => choose(e.currentTarget.value)}>
  {#if inheritLabel !== undefined}<option value="inherit">{inheritLabel}</option>{/if}
  {#each BAR_DISPLAY_MODES as mode (mode)}
    <option value={mode}>{t(`actors.barMode.${mode}`)}</option>
  {/each}
</select>

<style lang="scss">
  select {
    min-height: 44px;
  }
</style>
