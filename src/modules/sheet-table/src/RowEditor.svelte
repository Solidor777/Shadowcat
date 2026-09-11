<script lang="ts">
  import { getAppContext } from "@shadowcat/ui-kit";
  import type { TableRow, DrawRule, TableEntry } from "@shadowcat/core";
  import { defaultEntry } from "./rowOps";
  import EntryEditor from "./EntryEditor.svelte";

  // One row inside a `TableSheet`. Every field edit builds the WHOLE replacement `TableRow`
  // (label/weight/range/results) and hands it to `onChange`; the sheet owns the actual
  // whole-array `setField` write over `enginePrefix + "/rows"`.
  let {
    row,
    draw,
    onChange,
    onRemove,
    onMoveUp,
    onMoveDown,
    disabled = false,
  }: {
    /** The row this editor renders. */
    row: TableRow;
    /** The table's current draw rule — decides whether `lo`/`hi` render. */
    draw: DrawRule;
    /** Called with the replacement row on every field edit. */
    onChange: (next: TableRow) => void;
    /** Removes this row. */
    onRemove: () => void;
    /** Moves this row up one place; `undefined` at the top boundary (button disabled). */
    onMoveUp?: () => void;
    /** Moves this row down one place; `undefined` at the bottom boundary (button disabled). */
    onMoveDown?: () => void;
    /** Disables every control. */
    disabled?: boolean;
  } = $props();

  const ctx = getAppContext();
  const t = ctx.t;

  /** Replaces the entry at `index` with `entry`.
   * @param index The entry's index within `row.results`.
   * @param entry The replacement entry.
   * @example
   * ```
   * // private function; not part of the public API — wired to each EntryEditor's onChange
   * declare const entry: TableEntry;
   * setEntry(0, entry);
   * ```
   */
  function setEntry(index: number, entry: TableEntry): void {
    const results = row.results.slice();
    results[index] = entry;
    onChange({ ...row, results });
  }

  /** Removes the entry at `index`.
   * @param index The entry's index within `row.results`.
   * @example
   * ```
   * // private function; not part of the public API — wired to each EntryEditor's onRemove
   * removeEntry(0);
   * ```
   */
  function removeEntry(index: number): void {
    const results = row.results.slice();
    results.splice(index, 1);
    onChange({ ...row, results });
  }

  /** Appends a fresh default `text` entry.
   * @example
   * ```
   * // private function; not part of the public API — wired to "Add entry"
   * addEntry();
   * ```
   */
  function addEntry(): void {
    onChange({ ...row, results: [...row.results, defaultEntry("text")] });
  }
</script>

<div class="row" data-testid="table-row">
  <label>{t("sheetTable.label")}
    <input data-testid="row-label" aria-label={t("sheetTable.label")} {disabled}
      value={row.label} onchange={(e) => onChange({ ...row, label: (e.currentTarget as HTMLInputElement).value })} /></label>
  <label>{t("sheetTable.weight")}
    <input data-testid="row-weight" type="number" min="1" aria-label={t("sheetTable.weight")} {disabled}
      value={row.weight} onchange={(e) => {
        const v = Number((e.currentTarget as HTMLInputElement).value);
        if (Number.isNaN(v)) return;
        onChange({ ...row, weight: v });
      }} /></label>

  {#if draw.kind === "formula"}
    <label>{t("sheetTable.rangeLo")}
      <input data-testid="row-lo" type="number" aria-label={t("sheetTable.rangeLo")} {disabled}
        value={row.range?.lo ?? 1} onchange={(e) => {
          const v = Number((e.currentTarget as HTMLInputElement).value);
          if (Number.isNaN(v)) return;
          onChange({ ...row, range: { lo: v, hi: row.range?.hi ?? v } });
        }} /></label>
    <label>{t("sheetTable.rangeHi")}
      <input data-testid="row-hi" type="number" aria-label={t("sheetTable.rangeHi")} {disabled}
        value={row.range?.hi ?? 1} onchange={(e) => {
          const v = Number((e.currentTarget as HTMLInputElement).value);
          if (Number.isNaN(v)) return;
          onChange({ ...row, range: { lo: row.range?.lo ?? v, hi: v } });
        }} /></label>
  {/if}

  <button type="button" data-testid="row-remove" {disabled} onclick={onRemove}>{t("sheetTable.removeRow")}</button>
  <button type="button" data-testid="row-up" disabled={disabled || !onMoveUp} onclick={onMoveUp}>{t("sheetTable.moveUp")}</button>
  <button type="button" data-testid="row-down" disabled={disabled || !onMoveDown} onclick={onMoveDown}>{t("sheetTable.moveDown")}</button>

  <h4>{t("sheetTable.results")}</h4>
  {#each row.results as entry, i (i)}
    <EntryEditor {entry} {disabled} onChange={(next) => setEntry(i, next)} onRemove={() => removeEntry(i)} />
  {/each}
  <button type="button" data-testid="row-add-entry" {disabled} onclick={addEntry}>{t("sheetTable.addEntry")}</button>
</div>

<style lang="scss">
  .row { display: flex; flex-direction: column; gap: var(--space-1); padding: var(--space-1); border: 1px solid var(--border); border-radius: var(--radius-1); }
</style>
