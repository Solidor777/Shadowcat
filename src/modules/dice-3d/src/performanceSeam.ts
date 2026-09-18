import type { AppContext } from "@shadowcat/ui-kit";

/** The slice of `AppContext` these getters read — narrowed so a caller need not construct a
 * full fixture just to exercise the performance budget. Every call site invokes these from
 * outside Svelte's component-initialisation window (a document-store subscription callback,
 * fired well after mount), where `getAppContext()`'s own `getContext()` call would throw
 * (`lifecycle_outside_component`); `DiceOverlay.svelte` therefore resolves `AppContext` once,
 * synchronously, at its own top level and threads it through as `ctx`. */
type PerformanceReader = Pick<AppContext, "performance">;

/** Whether the 3D dice overlay is enabled on this device.
 * @param ctx The caller's already-resolved `AppContext`.
 * @returns The device's `PerformanceSettings.dice3d` budget.
 * @example
 * ```ts
 * import { dice3dEnabled } from "@shadowcat/module-dice-3d";
 * import { getAppContext } from "@shadowcat/ui-kit";
 *
 * dice3dEnabled(getAppContext());
 * ```
 */
export function dice3dEnabled(ctx: PerformanceReader): boolean {
  return ctx.performance.current.dice3d;
}

/** Whether this device prefers reduced motion (`prefers-reduced-motion: reduce`).
 * @param ctx The caller's already-resolved `AppContext`.
 * @returns The device's `PerformanceSettings.reducedMotion` budget.
 * @example
 * ```ts
 * import { reducedMotionPreferred } from "@shadowcat/module-dice-3d";
 * import { getAppContext } from "@shadowcat/ui-kit";
 *
 * reducedMotionPreferred(getAppContext());
 * ```
 */
export function reducedMotionPreferred(ctx: PerformanceReader): boolean {
  return ctx.performance.current.reducedMotion;
}

/** Whether the WebGL context should multisample.
 * @param ctx The caller's already-resolved `AppContext`.
 * @returns The device's `PerformanceSettings.antialias` budget.
 * @example
 * ```ts
 * import { antialiasPreferred } from "@shadowcat/module-dice-3d";
 * import { getAppContext } from "@shadowcat/ui-kit";
 *
 * antialiasPreferred(getAppContext());
 * ```
 */
export function antialiasPreferred(ctx: PerformanceReader): boolean {
  return ctx.performance.current.antialias;
}
