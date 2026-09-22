// Shared gap-based sibling ordering key for any document tree with a `sort` field (asset
// folders, notes, ...). A fresh sibling gets a key spaced away from its neighbours by
// `SORT_KEY_DENSITY` rather than a dense 0/1/2/... counter, so a later insert BETWEEN two
// existing siblings can take the midpoint without renumbering every sibling after it — the
// classic fractional/gap-key ordering scheme. Keys are integers throughout (both `NoteEngine.sort`
// and `AssetFolderEngine.sort` are server-validated i64 fields): the midpoint of two keys one
// apart has no integer to take, which is exactly the case `allocateSortKey` signals via
// `renumber` rather than silently colliding with a neighbour.

/** The default spacing between adjacent siblings' `sort` keys, and the step a fresh append
 * advances by. Large enough that many ordinary inserts-between exhaust no gap before a renumber
 * is ever needed. */
export const SORT_KEY_DENSITY = 1000;

/** The result of `allocateSortKey`. */
export interface SortKeyAllocation {
  /** The key to store on the new/moved sibling. When `renumber` is `true`, this is a
   * best-effort value only (it may equal `prev`) — the caller must renumber the sibling list via
   * fresh, evenly-spaced keys before persisting the insertion, and should not store this value
   * as-is. */
  key: number;
  /** `true` when `prev` and `next` left no integer strictly between them — the gap is
   * exhausted and the caller must renumber the sibling list before inserting. */
  renumber: boolean;
}

/**
 * Allocates a `sort` key for a sibling placed between `prev` and `next` (either or both `null`
 * for an append/prepend/first-sibling placement).
 * @param prev The immediately preceding sibling's key, or `null` to insert at the very start.
 * @param next The immediately following sibling's key, or `null` to insert at the very end
 * (append) — the common case for a newly created document with no explicit position.
 * @returns The allocated key, and whether the gap between `prev`/`next` is exhausted and the
 * caller must renumber siblings before inserting.
 * @example
 * ```ts
 * import { allocateSortKey } from "@shadowcat/core";
 *
 * allocateSortKey(null, null); // { key: 1000, renumber: false } — first sibling
 * allocateSortKey(1000, null); // { key: 2000, renumber: false } — append
 * allocateSortKey(1000, 2000); // { key: 1500, renumber: false } — insert between
 * allocateSortKey(1000, 1001); // { key: 1000, renumber: true } — gap exhausted
 * ```
 */
export function allocateSortKey(prev: number | null, next: number | null): SortKeyAllocation {
  if (prev === null && next === null) return { key: SORT_KEY_DENSITY, renumber: false };
  if (prev === null) return { key: (next as number) - SORT_KEY_DENSITY, renumber: false };
  if (next === null) return { key: prev + SORT_KEY_DENSITY, renumber: false };
  const gap = next - prev;
  if (gap <= 1) return { key: prev, renumber: true };
  return { key: prev + Math.floor(gap / 2), renumber: false };
}

/**
 * The `sort` key a new sibling appended to the end of a list should take — one density step
 * past the highest existing sibling's key, or `SORT_KEY_DENSITY` for the first child. The shared
 * "highest sibling → append" step behind `appendNoteSortKey`/`appendFolderSortKey`: each caller
 * still does its own sibling FILTERING (by `parent_id`/doc type), but hands the resulting sibling
 * `sort` values here rather than re-deriving `allocateSortKey`'s append case (`prev` = the max,
 * `next` = `null`) itself — a forked "highest sibling → append" computation is exactly the defect
 * class two independent copies of this logic would reintroduce.
 * @param siblingSortValues Every existing sibling's `sort` value, in any order.
 * @returns The allocated append key.
 * @example
 * ```ts
 * import { appendSortKey } from "@shadowcat/core";
 *
 * appendSortKey([]); // 1000 — first sibling
 * appendSortKey([1000, 2000]); // 3000 — past the highest
 * ```
 */
export function appendSortKey(siblingSortValues: number[]): number {
  const prev = siblingSortValues.length > 0 ? Math.max(...siblingSortValues) : null;
  return allocateSortKey(prev, null).key;
}
