<script lang="ts">
  // Shared vision-assignment list editor: one row per `VisionAssignment` (mode select + range
  // override + remove), plus an add row. Deliberately value-only — the OWNING surface (actor
  // row, actor sheet, token-override control) decides what an empty list means around it
  // (no senses vs. inherit) and owns the dispatch, so the raw-`old` OCC pre-image read stays
  // at the call site that knows the document path.
  import { getAppContext } from "./appContext";
  import type { VisionAssignment, VisionMode } from "@shadowcat/core";

  const ctx = getAppContext();
  const t = ctx.t;

  let { value, modes, disabled = false, onCommit }: {
    /** The assignments being edited (never `null` — the parent normalizes its stored
     * `vision: VisionAssignment[] | null` before rendering and interprets the committed
     * empty list itself). */
    value: VisionAssignment[];
    /** The resolved vision-mode registry entries the mode select offers
     * (`resolveVisionModes`'s values). */
    modes: VisionMode[];
    /** Read-only mode (a sheet the viewer may not edit). */
    disabled?: boolean;
    /** Commit callback: receives the WHOLE updated assignment list (whole-payload
     * replacement — the list is one nested value, so a single write carries every row's
     * change). The list is passed through verbatim, INCLUDING when empty. */
    onCommit: (next: VisionAssignment[]) => void;
  } = $props();

  /** The mode ids the select offers — plus, per row, the row's own id when it is dangling
   * (a mode removed from the registry after the assignment was authored). Rendering the raw
   * dangling id keeps the stored value visible and the OCC pre-image intact instead of
   * silently displaying a different mode. */
  const modeOptions = $derived(modes.map((m) => m.id));

  /**
   * Commit the list with one row replaced.
   * @param i The row index.
   * @param patch The field(s) to change on that row.
   * @example
   * ```
   * // private helper; wired to each row control's onchange
   * commitRow(0, { mode: "tremorsense" });
   * ```
   */
  function commitRow(i: number, patch: Partial<VisionAssignment>): void {
    onCommit(value.map((a, j) => (j === i ? { ...a, ...patch } : a)));
  }

  /**
   * Commit a row's range from the raw input string: empty inherits the mode's own default
   * range (`null`), a finite number overrides it, anything else is ignored.
   * @param i The row index.
   * @param raw The input's raw string value.
   * @example
   * ```
   * // private helper; wired to each range input's onchange
   * commitRange(0, ""); // → range: null (inherit the mode default)
   * ```
   */
  function commitRange(i: number, raw: string): void {
    if (raw.trim() === "") {
      commitRow(i, { range: null });
      return;
    }
    const n = Number(raw);
    if (!Number.isFinite(n)) return;
    commitRow(i, { range: n });
  }

  /**
   * Append a row for the first offered mode with an inherited (`null`) range.
   * @example
   * ```
   * // private helper; wired to the add button's onclick
   * addRow();
   * ```
   */
  function addRow(): void {
    const first = modeOptions[0];
    if (first === undefined) return;
    onCommit([...value, { mode: first, range: null }]);
  }

  /**
   * Drop one row.
   * @param i The row index to remove.
   * @example
   * ```
   * // private helper; wired to each row's remove button
   * removeRow(0);
   * ```
   */
  function removeRow(i: number): void {
    onCommit(value.filter((_, j) => j !== i));
  }
</script>

<div class="vision-assignments">
  {#each value as assignment, i (i)}
    <div class="vision-row" data-testid="vision-row">
      <select
        data-testid="vision-mode-{i}"
        aria-label={t("actors.visionMode")}
        value={assignment.mode}
        {disabled}
        onchange={(e) => commitRow(i, { mode: e.currentTarget.value })}
      >
        {#each modes as m (m.id)}<option value={m.id}>{m.name}</option>{/each}
        {#if !modeOptions.includes(assignment.mode)}
          <option value={assignment.mode}>{assignment.mode}</option>
        {/if}
      </select>
      <input
        type="number"
        min="0"
        step="1"
        data-testid="vision-range-{i}"
        aria-label={t("actors.visionRange")}
        title={t("actors.visionRange")}
        value={assignment.range ?? ""}
        {disabled}
        onchange={(e) => commitRange(i, e.currentTarget.value)}
      />
      <button type="button" data-testid="vision-remove-{i}" {disabled} onclick={() => removeRow(i)}>
        {t("actors.visionRemove")}
      </button>
    </div>
  {/each}
  <button type="button" data-testid="vision-add" disabled={disabled || modeOptions.length === 0} onclick={addRow}>
    {t("actors.visionAdd")}
  </button>
</div>

<style lang="scss">
  .vision-assignments {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .vision-row {
    display: flex;
    flex-direction: row;
    gap: var(--space-1);
    align-items: center;
  }
  .vision-assignments input,
  .vision-assignments select,
  .vision-assignments button {
    min-height: 32px;

    @media (pointer: coarse) {
      min-height: 44px;
    }
  }
</style>
