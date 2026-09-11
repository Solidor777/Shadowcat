// Pure helpers over a `TableEngine.rows` array: every mutation returns a NEW array (the
// caller's `setField` always replaces the whole array — `set_pointer` cannot grow arrays),
// built from a `structuredClone` of the input so the store's own object is never mutated
// in place. No intent dispatch, no reactive reads.
import type { TableRow, TableEntry, DrawRule } from "@shadowcat/core";

/**
 * A fresh, empty row appended to the end. Under `DrawRule::Formula` the row gets a
 * placeholder `range` (`{ lo: 1, hi: 1 }`, since `range: null` is invalid under `formula` —
 * server-side `TableEngine::validate` rejects it); under `weighted`, `range` stays `null`.
 * @param rows The current rows (not mutated).
 * @param draw The table's current draw rule (decides whether the new row needs a `range`).
 * @returns A new array with the row appended.
 * @example
 * ```ts
 * import { addRow } from "@shadowcat/module-sheet-table";
 *
 * addRow([], { kind: "weighted" });
 * // [{ weight: 1, range: null, label: "", results: [] }]
 * ```
 */
export function addRow(rows: TableRow[], draw: DrawRule): TableRow[] {
  const next = structuredClone(rows);
  next.push({
    weight: 1,
    range: draw.kind === "formula" ? { lo: 1, hi: 1 } : null,
    label: "",
    results: [],
  });
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
