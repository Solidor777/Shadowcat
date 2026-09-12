import type { DeviceSignals } from "@shadowcat/core";

/** Reads the live `DeviceSignals` `resolveAuto` resolves `"auto"` against. Every probe is
 * guarded on existence — `matchMedia`, `navigator.hardwareConcurrency`, and the Chromium-only
 * `navigator.deviceMemory` are all absent under jsdom/node — so an unsupported environment
 * degrades to "unknown" (the field omitted) rather than throwing or synthesizing a false value.
 * Called once, pre-mount, in `main.ts` alongside `performanceController.load`.
 * @returns The signals this device/browser can report right now.
 * @example
 * ```ts
 * readDeviceSignals(); // {} under jsdom/node; populated fields in a real browser
 * ```
 */
export function readDeviceSignals(): DeviceSignals {
  const signals: DeviceSignals = {};
  if (typeof matchMedia === "function") {
    signals.coarsePointer = matchMedia("(pointer: coarse)").matches;
    signals.compact = !matchMedia("(min-width: 48rem)").matches;
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
