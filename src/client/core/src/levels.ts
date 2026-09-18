import type { SceneLevel, ElevationBand } from "@shadowcat/types";
export type { SceneLevel, ElevationBand };

/** Whether elevation band `band` contains point `e`. Mirrors `scene::elevation::band_contains`
 * exactly — the shared predicate `sceneScopedDocs`'s band-shaped filter and `LevelsEditor`'s
 * range display both call. `band: null` contains every elevation; an absent end is unbounded;
 * a malformed interval (`bottom > top`) or a non-finite endpoint fails closed to containing
 * everything.
 * @param band - The elevation band to test, or `null` for "applies at every elevation".
 * @param e - The elevation to test for membership in the band.
 * @returns Whether the band contains `e`.
 * @example
 * ```
 * bandContains(null, 5); // true — an absent band contains every elevation
 * bandContains({ bottom: 0, top: 10 }, 5); // true
 * bandContains({ bottom: 0, top: 10 }, 15); // false
 * ```
 */
export function bandContains(band: ElevationBand | null, e: number): boolean {
  if (band === null) return true;
  const bottom = band.bottom ?? null;
  const top = band.top ?? null;
  if ((bottom !== null && !Number.isFinite(bottom)) || (top !== null && !Number.isFinite(top))) {
    return true;
  }
  const lo = bottom ?? -Infinity;
  const hi = top ?? Infinity;
  if (lo > hi) return true;
  return lo <= e && e <= hi;
}

/** The level whose `[bottom, top)` band contains `elevation`; mirrors `scene::elevation::level_of`
 * exactly (see that function's doc for the roof/below-every-level fallback rules). Callers pass an
 * already-clamped elevation, never a raw stored value.
 * @param levels - The scene's declared levels (`SceneEngine.levels`).
 * @param elevation - The elevation to resolve to a level.
 * @returns The resolved level, or `null` when `levels` is empty.
 * @example
 * ```
 * const levels = [{ id: "ground", name: "Ground", bottom: 0, top: 10, background: null }];
 * levelOf(levels, 5)?.id; // "ground"
 * levelOf(levels, 99)?.id; // "ground" — a roof is the top floor
 * levelOf([], 5); // null
 * ```
 */
export function levelOf(levels: SceneLevel[], elevation: number): SceneLevel | null {
  if (levels.length === 0) return null;
  const containing = levels.find((l) => l.bottom <= elevation && elevation < l.top);
  if (containing) return containing;
  const below = levels.filter((l) => l.bottom <= elevation);
  if (below.length > 0) {
    return below.reduce((a, b) => (a.bottom > b.bottom ? a : b));
  }
  return levels.reduce((a, b) => (a.bottom < b.bottom ? a : b));
}
