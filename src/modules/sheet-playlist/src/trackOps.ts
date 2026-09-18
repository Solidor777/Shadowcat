import type { PlaylistTrack } from "@shadowcat/core";

// Pure whole-array helpers over `PlaylistEngine.tracks` — every mutation returns a NEW array
// (a `structuredClone` of the stored one, never a mutation of the store's object), because
// `set_pointer` cannot grow/shrink arrays: the sheet's every track edit is a whole-array
// replace, the same invariant `sheet-table`'s `rowOps` enforces over `TableEngine.rows`.

/** A fresh track's defaults: unity gain, no loop (the playlist's own `mode` governs advancing
 * between tracks; `loop` matters only for `PlaylistMode::Single` or a direct play).
 * @param asset The asset id to play.
 * @returns The new track.
 * @example
 * ```ts
 * import { defaultTrack } from "@shadowcat/module-sheet-playlist";
 * defaultTrack("a1"); // { asset: "a1", name: null, gain: 1, loop: false }
 * ```
 */
export function defaultTrack(asset: string): PlaylistTrack {
  return { asset, name: null, gain: 1, loop: false };
}

/** Append a default track for `asset` to a copy of `tracks`.
 * @param tracks The stored array (never mutated).
 * @param asset The asset id for the new track.
 * @returns The new array with the track appended.
 * @example
 * ```ts
 * import { addTrack } from "@shadowcat/module-sheet-playlist";
 * addTrack([], "a1").length; // 1
 * ```
 */
export function addTrack(tracks: PlaylistTrack[], asset: string): PlaylistTrack[] {
  return [...structuredClone(tracks), defaultTrack(asset)];
}

/** Remove the track at index `i` from a copy of `tracks`.
 * @param tracks The stored array (never mutated).
 * @param i The index to remove.
 * @returns The new array without that track.
 * @example
 * ```ts
 * import { removeTrack } from "@shadowcat/module-sheet-playlist";
 * removeTrack([{ asset: "a", name: null, gain: 1, loop: false }], 0).length; // 0
 * ```
 */
export function removeTrack(tracks: PlaylistTrack[], i: number): PlaylistTrack[] {
  const next = structuredClone(tracks);
  next.splice(i, 1);
  return next;
}

/** Move the track at index `i` one slot in direction `dir` (`-1` up, `1` down), clamped to
 * the array bounds (a no-op at either end).
 * @param tracks The stored array (never mutated).
 * @param i The index to move.
 * @param dir `-1` toward the front, `1` toward the back.
 * @returns The reordered array (unchanged at the bounds).
 * @example
 * ```ts
 * import { moveTrack } from "@shadowcat/module-sheet-playlist";
 * const t = (a: string) => ({ asset: a, name: null, gain: 1, loop: false });
 * moveTrack([t("a"), t("b")], 0, 1)[0].asset; // "b"
 * ```
 */
export function moveTrack(tracks: PlaylistTrack[], i: number, dir: -1 | 1): PlaylistTrack[] {
  const j = i + dir;
  if (i < 0 || i >= tracks.length || j < 0 || j >= tracks.length) return structuredClone(tracks);
  const next = structuredClone(tracks);
  const [moved] = next.splice(i, 1);
  next.splice(j, 0, moved);
  return next;
}

/** Replace the track at index `i` with `track` in a copy of `tracks`.
 * @param tracks The stored array (never mutated).
 * @param i The index to replace.
 * @param track The new track value.
 * @returns The new array with the replacement applied.
 * @example
 * ```ts
 * import { setTrack } from "@shadowcat/module-sheet-playlist";
 * const t = (a: string) => ({ asset: a, name: null, gain: 1, loop: false });
 * setTrack([t("a")], 0, t("b"))[0].asset; // "b"
 * ```
 */
export function setTrack(tracks: PlaylistTrack[], i: number, track: PlaylistTrack): PlaylistTrack[] {
  const next = structuredClone(tracks);
  next[i] = track;
  return next;
}
