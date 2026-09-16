import type { DeviceSignals } from "@shadowcat/core";
import { isCompactViewport } from "@shadowcat/ui-kit";

/** Reads the live `DeviceSignals` `resolveAuto` resolves `"auto"` against. Every probe is
 * guarded on existence, so an environment missing one degrades that field to "unknown" (the
 * field omitted) rather than throwing or synthesizing a false value — which fields are
 * reportable is per-environment: jsdom has no `matchMedia` at all, while Node ≥ 21.5 reports
 * `navigator.hardwareConcurrency` and Chromium alone reports `deviceMemory`. The compact
 * breakpoint itself is NOT re-declared here — it is read through `isCompactViewport`, the one
 * owner of that media query. Called once, pre-mount, in `main.ts` alongside
 * `performanceController.load`.
 * @returns The signals this device/browser can report right now.
 * @example
 * ```ts
 * readDeviceSignals(); // each field present only when its probe exists in this environment
 * ```
 */
export function readDeviceSignals(): DeviceSignals {
  const signals: DeviceSignals = {};
  if (typeof matchMedia === "function") {
    signals.coarsePointer = matchMedia("(pointer: coarse)").matches;
    signals.compact = isCompactViewport();
    signals.reducedMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;
  }
  if (typeof navigator !== "undefined") {
    if (typeof navigator.hardwareConcurrency === "number") {
      signals.hardwareConcurrency = navigator.hardwareConcurrency;
    }
    const deviceMemory = (navigator as unknown as Record<string, unknown>).deviceMemory;
    if (typeof deviceMemory === "number") signals.deviceMemoryGb = deviceMemory;
  }
  return signals;
}
