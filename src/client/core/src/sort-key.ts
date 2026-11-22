// Shared gap-based sibling ordering key for any document tree with a `sort` field (asset
// folders, notes, ...). A fresh sibling gets a key spaced away from its neighbours by
// `SORT_KEY_DENSITY` rather than a dense 0/1/2/... counter, so a later insert BETWEEN two
// existing siblings can take the midpoint without renumbering every sibling after it — the
// classic fractional/gap-key ordering scheme. Keys are integers throughout (both `NoteEngine.sort`
// and `AssetFolderEngine.sort` are server-validated i64 fields): the midpoint of two keys one
// apart has no integer to take, which is exactly the case `allocateSortKey` signals via
// `renumber` rather than silently colliding with a neighbour.

import { buildUpdate } from "./update-op";
import type { WireOperation } from "./wire";

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

/** One sibling in a `computeReorderUpdate` call: its document id and current `sort` key. */
export interface SortSibling {
  /** The sibling document's id. */
  id: string;
  /** The sibling's current `sort` key. */
  sort: number;
}

/** One sibling whose `sort` key changes as a result of a reorder. */
export interface SortUpdate {
  /** The sibling document's id. */
  docId: string;
  /** The key to store. */
  sort: number;
  /** The key the sibling holds now, carried as the optimistic-concurrency pre-image. */
  previous: number;
}

/**
 * The `sort` changes that move the sibling at `from` to index `to` of the list. The single
 * decider for every reorder input (pointer drop and keyboard move), so two input methods can
 * never compute different results for the same logical move. A gap with room yields one update;
 * an exhausted gap renumbers the whole list to `SORT_KEY_DENSITY`-spaced keys in the post-move
 * order and returns every sibling whose key changed (a partial renumber could collide with a key
 * outside the touched range).
 * @param siblings The sibling list in its current display order.
 * @param from The moved sibling's index before the move.
 * @param to The moved sibling's index after the move (post-removal convention, as `moveInOrder`).
 * @returns The key changes, or `null` when `from === to` (a no-op).
 * @throws {RangeError} When `from` or `to` is out of bounds.
 * @example
 * ```ts
 * import { computeReorderUpdate } from "@shadowcat/core";
 *
 * const siblings = [
 *   { id: "a", sort: 1000 },
 *   { id: "b", sort: 2000 },
 *   { id: "c", sort: 3000 },
 * ];
 * computeReorderUpdate(siblings, 2, 0); // [{ docId: "c", sort: 0, previous: 3000 }]
 * ```
 */
export function computeReorderUpdate(siblings: SortSibling[], from: number, to: number): SortUpdate[] | null {
  const n = siblings.length;
  if (from < 0 || from >= n || to < 0 || to >= n) {
    throw new RangeError(`computeReorderUpdate: index out of range (from=${from}, to=${to}, length=${n})`);
  }
  if (from === to) return null;
  const moved = siblings[from];
  const order = siblings.slice();
  order.splice(from, 1);
  order.splice(to, 0, moved);
  const prev = to > 0 ? order[to - 1].sort : null;
  const next = to < n - 1 ? order[to + 1].sort : null;
  const allocation = allocateSortKey(prev, next);
  if (!allocation.renumber) return [{ docId: moved.id, sort: allocation.key, previous: moved.sort }];
  const updates: SortUpdate[] = [];
  order.forEach((s, i) => {
    const sort = (i + 1) * SORT_KEY_DENSITY;
    if (sort !== s.sort) updates.push({ docId: s.id, sort, previous: s.sort });
  });
  return updates;
}

/**
 * The wire operations for `computeReorderUpdate`'s result: one `Update` per changed sibling,
 * each writing `path` with the sibling's current key as the pre-image. Both reorder input
 * methods dispatch this one builder's output as a single intent batch.
 * @param siblings The sibling list in its current display order.
 * @param from The moved sibling's index before the move.
 * @param to The moved sibling's index after the move (post-removal convention).
 * @param path The JSON-pointer path of the sibling's sort field; defaults to `/engine/sort`.
 * @returns The operations, or `null` when the move is a no-op.
 * @example
 * ```ts
 * import { buildReorderOps } from "@shadowcat/core";
 *
 * const ops = buildReorderOps([{ id: "a", sort: 1000 }, { id: "b", sort: 2000 }], 1, 0);
 * ops?.length; // 1
 * ```
 */
export function buildReorderOps(
  siblings: SortSibling[],
  from: number,
  to: number,
  path = "/engine/sort",
): WireOperation[] | null {
  const updates = computeReorderUpdate(siblings, from, to);
  if (!updates) return null;
  return updates.map((u) => buildUpdate(u.docId, [{ path, old: u.previous, value: u.sort }]));
}
