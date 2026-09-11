// Pure helpers over a `TableEngine.rows` array: every mutation returns a NEW array (the
// caller's `setField` always replaces the whole array — `set_pointer` cannot grow arrays),
// built from a `structuredClone` of the input so the store's own object is never mutated
// in place. No intent dispatch, no reactive reads.
import type { TableRow, TableEntry, DrawRule, RowRange } from "@shadowcat/core";

/**
 * The next range slot free of every ranged row in `rows`: `{ lo: m+1, hi: m+1 }` where `m` is
 * the max `hi` over rows that carry a range, or `{ lo: 1, hi: 1 }` when none do. This is the
 * client mirror of `TableEngine::validate`'s pairwise-non-overlap rule for a `Formula` table's
 * rows — every row-shaping site that needs to seed a `Formula` row's range (a fresh row, or an
 * existing row carried over with no range of its own) reads this ONE helper rather than
 * restating a literal range that risks colliding with another row's.
 * @param rows The rows to compute a free slot against.
 * @returns A range disjoint from every ranged row in `rows`.
 * @example
 * ```ts
 * import { nextFreeRange } from "@shadowcat/module-sheet-table";
 *
 * nextFreeRange([{ weight: 1, range: { lo: 1, hi: 3 }, label: "a", results: [] }]);
 * // { lo: 4, hi: 4 }
 * ```
 */
export function nextFreeRange(rows: TableRow[]): RowRange {
  const maxHi = rows.reduce((m, r) => (r.range ? Math.max(m, r.range.hi) : m), 0);
  return { lo: maxHi + 1, hi: maxHi + 1 };
}

/**
 * A fresh row appended to the end, labeled `defaultLabel` — server-side `TableEngine::validate`
 * rejects an empty/whitespace-only label on every row (a row must be selectable and
 * distinguishable in the drawn-from list), so appending with a placeholder that still commits
 * is the only shape that round-trips: the author edits it afterward. Under `DrawRule::Formula`
 * the row gets {@link nextFreeRange} computed against the existing rows, since `range: null` is
 * invalid under `formula`, and a fixed placeholder range would collide with any existing row
 * already covering that slot; under `weighted`, `range` stays `null`.
 * @param rows The current rows (not mutated).
 * @param draw The table's current draw rule (decides whether the new row needs a `range`).
 * @param defaultLabel The new row's initial label (never empty — the caller passes a
 * localized placeholder, e.g. `t("sheetTable.newRowLabel")`).
 * @returns A new array with the row appended.
 * @example
 * ```ts
 * import { addRow } from "@shadowcat/module-sheet-table";
 *
 * addRow([], { kind: "weighted" }, "New row");
 * // [{ weight: 1, range: null, label: "New row", results: [] }]
 * ```
 */
export function addRow(rows: TableRow[], draw: DrawRule, defaultLabel: string): TableRow[] {
  const next = structuredClone(rows);
  const range = draw.kind === "formula" ? nextFreeRange(next) : null;
  next.push({ weight: 1, range, label: defaultLabel, results: [] });
  return next;
}

/**
 * Reshapes every row's `range` to match `draw`'s requirement, so switching a table's draw rule
 * never produces a post-image `TableEngine::validate` rejects: `TableEngine::validate` requires
 * `range: None` under `Weighted` and `range: Some` under `Formula` for EVERY row, and every
 * `Formula` row's range pairwise disjoint from every other's. Under `Formula`, a row that
 * already carries a valid range keeps it; a row with none is assigned {@link nextFreeRange}
 * computed sequentially against the rows already processed (so two rows both missing a range
 * land in disjoint slots, never the same placeholder). Under `Weighted`, every row's `range` is
 * cleared to `null`.
 * @param rows The current rows (not mutated).
 * @param draw The draw rule the rows are being reshaped for.
 * @returns A new array with every row's `range` normalized for `draw`.
 * @example
 * ```ts
 * import { normalizeRowsForDraw } from "@shadowcat/module-sheet-table";
 *
 * normalizeRowsForDraw([{ weight: 1, range: null, label: "", results: [] }], { kind: "formula", notation: "1d20" });
 * // [{ weight: 1, range: { lo: 1, hi: 1 }, label: "", results: [] }]
 * ```
 */
export function normalizeRowsForDraw(rows: TableRow[], draw: DrawRule): TableRow[] {
  const next = structuredClone(rows);
  if (draw.kind === "formula") {
    for (const row of next) {
      if (!row.range) row.range = nextFreeRange(next.filter((r) => r.range !== null));
    }
  } else {
    for (const row of next) row.range = null;
  }
  return next;
}

/**
 * Removes the row at `index`.
 * @param rows The current rows (not mutated).
 * @param index The index to remove.
 * @returns A new array without that row.
 * @throws {RangeError} When `index` is out of bounds.
 * @example
 * ```ts
 * import { removeRow } from "@shadowcat/module-sheet-table";
 * import type { TableRow } from "@shadowcat/core";
 *
 * declare const rows: TableRow[];
 * removeRow(rows, 0);
 * ```
 */
export function removeRow(rows: TableRow[], index: number): TableRow[] {
  if (index < 0 || index >= rows.length) {
    throw new RangeError(`removeRow: index out of range (index=${index}, length=${rows.length})`);
  }
  const next = structuredClone(rows);
  next.splice(index, 1);
  return next;
}

/**
 * Moves the row at `index` one place toward `dir`. A no-op-shaped clamp: moving the first row
 * up (or the last row down) returns an unchanged (but still freshly cloned) array, matching
 * the "disable the button at the boundary" UI affordance without requiring the caller to
 * guard the call.
 * @param rows The current rows (not mutated).
 * @param index The row to move.
 * @param dir `-1` to move up, `1` to move down.
 * @returns A new array with the row relocated (or an equal clone at a boundary).
 * @example
 * ```ts
 * import { moveRow } from "@shadowcat/module-sheet-table";
 * import type { TableRow } from "@shadowcat/core";
 *
 * declare const rows: TableRow[];
 * moveRow(rows, 1, -1);
 * ```
 */
export function moveRow(rows: TableRow[], index: number, dir: -1 | 1): TableRow[] {
  const next = structuredClone(rows);
  const target = index + dir;
  if (target < 0 || target >= next.length) return next;
  const [moved] = next.splice(index, 1);
  next.splice(target, 0, moved);
  return next;
}

/**
 * Replaces the row at `index` with `row` (whole-row replacement — a field-level edit inside
 * `RowEditor` builds the replacement row and calls this rather than mutating in place).
 * @param rows The current rows (not mutated).
 * @param index The row to replace.
 * @param row The replacement row.
 * @returns A new array with that row replaced.
 * @throws {RangeError} When `index` is out of bounds.
 * @example
 * ```ts
 * import { setRow } from "@shadowcat/module-sheet-table";
 * import type { TableRow } from "@shadowcat/core";
 *
 * declare const rows: TableRow[];
 * declare const edited: TableRow;
 * setRow(rows, 0, edited);
 * ```
 */
export function setRow(rows: TableRow[], index: number, row: TableRow): TableRow[] {
  if (index < 0 || index >= rows.length) {
    throw new RangeError(`setRow: index out of range (index=${index}, length=${rows.length})`);
  }
  const next = structuredClone(rows);
  next[index] = structuredClone(row);
  return next;
}

/**
 * A fresh, empty entry of `kind`, for a row's "Add entry" affordance — every field present
 * with a sane, ingress-valid default (an empty `asset_id`/`doc_id`/`table_id` is rejected
 * server-side once picked, which the sheet's own toast surfaces).
 * @param kind The entry kind to build a default payload for.
 * @returns A fresh default payload per call (no aliasing between entries).
 * @example
 * ```ts
 * import { defaultEntry } from "@shadowcat/module-sheet-table";
 *
 * defaultEntry("text"); // { kind: "text", text: "" }
 * ```
 */
export function defaultEntry(kind: TableEntry["kind"]): TableEntry {
  switch (kind) {
    case "text":
      return { kind: "text", text: "" };
    case "doc":
      return { kind: "doc", target: { kind: "doc", doc_id: "", embedded_path: null }, label: "" };
    case "image":
      return { kind: "image", asset_id: "", alt: "" };
    case "draw":
      return { kind: "draw", table_id: "", count: 1 };
  }
}
