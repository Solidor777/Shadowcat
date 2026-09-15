/**
 * Rewired to read `getAppContext().performance
 * .current.dice3d`/`.reducedMotion`/`.antialias` once `PerformanceSettings` exists.
 * Until then these are the device-local getters this module binds itself — so
 * `DiceOverlay`/`DiceEngine` never take a private duplicate of `PerformanceSettings`; only
 * these three function bodies change at integration, never their call sites.
 */

/** Whether the 3D dice overlay is enabled on this device.
 * @returns `true` while the pre-integration default is on.
 * @example
 * ```ts
 * import { dice3dEnabled } from "@shadowcat/module-dice-3d";
 *
 * dice3dEnabled();
 * ```
 */
export function dice3dEnabled(): boolean {
  return true;
}

/** Whether this device prefers reduced motion (`prefers-reduced-motion: reduce`).
 * @returns `true` when the media query matches, `false` when it does not or `matchMedia`
 * is unavailable.
 * @example
 * ```ts
 * import { reducedMotionPreferred } from "@shadowcat/module-dice-3d";
 *
 * reducedMotionPreferred();
 * ```
 */
export function reducedMotionPreferred(): boolean {
  return typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/** Whether the WebGL context should multisample.
 * @returns `true` while the pre-integration default is on.
 * @example
 * ```ts
 * import { antialiasPreferred } from "@shadowcat/module-dice-3d";
 *
 * antialiasPreferred();
 * ```
 */
export function antialiasPreferred(): boolean {
  return true;
}
