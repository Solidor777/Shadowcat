/** Pure cross-fade math for the mover vision-sweep fog (M2 §T7 smoothness enhancement).
 * Extracted from `pixi-backend.ts` so it is unit-testable without a GL/pixi.js context —
 * `pixi-backend.ts` itself is Playwright-covered only (no WebGL in jsdom). */

/**
 * Blend factor for cross-fading between two consecutive vision samples' rasterized fog
 * textures: 0 at `tCur` (fully the outgoing/"from" texture), 1 at `tNext` (fully the
 * incoming/"to" texture), linearly interpolated between and clamped to `[0,1]` outside.
 * A degenerate or non-finite span (`tNext <= tCur`, or any non-finite input) has nothing
 * meaningful to interpolate across and snaps to 1 (immediately "to") — fail-safe toward the
 * newer sample rather than freezing on a stale one.
 */
export function computeFogBlendFactor(clock: number, tCur: number, tNext: number): number {
  if (!Number.isFinite(clock) || !Number.isFinite(tCur) || !Number.isFinite(tNext)) return 1;
  if (tNext <= tCur) return 1;
  const f = (clock - tCur) / (tNext - tCur);
  return Math.min(1, Math.max(0, f));
}
