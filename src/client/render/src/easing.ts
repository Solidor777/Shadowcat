/** Token-motion easing curves. Pure, GL-free, unit-tested. */
export type EasingMode = "easeInOut" | "linear";

/**
 * Standard quadratic ease-in-out (smooth accel/decel). Source: standard easing formula
 * (Penner). Chosen over cubic for a gentle, predictable VTT feel. Symmetric about the
 * midpoint (`easeInOutQuad(0.5) === 0.5`); does not clamp `t` itself — the caller
 * ({@link applyEasing}) is expected to pass an already-clamped `[0,1]` value.
 * @param t Normalized progress, expected in `[0,1]`.
 * @returns The eased progress, in `[0,1]` for `t` in `[0,1]`.
 * @example
 * ```
 * // not exported from @shadowcat/render; internal to applyEasing
 * easeInOutQuad(0.5); // 0.5 (symmetric at the midpoint)
 * easeInOutQuad(0.25); // < 0.25 (slow start)
 * ```
 */
function easeInOutQuad(t: number): number {
  return t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
}

/**
 * Maps a normalized progress `t` through `mode`, clamping `t` to `[0,1]` first (a value
 * below 0 or above 1 clamps to the nearer bound rather than extrapolating).
 * @param mode Which curve to apply: `"linear"` (identity) or `"easeInOut"`
 * ({@link easeInOutQuad}).
 * @param t Progress; any finite number is accepted, clamped internally.
 * @returns The eased progress, in `[0,1]`.
 * @example
 * ```
 * // `applyEasing` itself is not exported from @shadowcat/render (only the
 * // `EasingMode` type is); internal to TokenAnimator.
 * applyEasing("linear", 0.5); // 0.5
 * applyEasing("easeInOut", 0.25); // < 0.25
 * applyEasing("easeInOut", 5); // 1 (clamped)
 * ```
 */
export function applyEasing(mode: EasingMode, t: number): number {
  const c = t <= 0 ? 0 : t >= 1 ? 1 : t;
  return mode === "linear" ? c : easeInOutQuad(c);
}
