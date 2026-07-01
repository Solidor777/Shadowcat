/** Token-motion easing curves. Pure, GL-free, unit-tested. */
export type EasingMode = "easeInOut" | "linear";

/** Standard quadratic ease-in-out (smooth accel/decel). Source: standard easing
 * formula (Penner). Chosen over cubic for a gentle, predictable VTT feel. */
function easeInOutQuad(t: number): number {
  return t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
}

/** Map a normalized progress `t` through `mode`. Input is clamped to [0,1]. */
export function applyEasing(mode: EasingMode, t: number): number {
  const c = t <= 0 ? 0 : t >= 1 ? 1 : t;
  return mode === "linear" ? c : easeInOutQuad(c);
}
