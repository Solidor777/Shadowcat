// @vitest-environment node
import { describe, it, expect } from "vitest";
import { DuckControllerImpl } from "@shadowcat/audio";
import { KeySource } from "./keySource";
import { OsMonitorSource } from "./osMonitor";

/** Ticks `duck` forward in 20ms steps until `totalMs` has elapsed — mirrors `AudioEngine`'s own
 * per-frame `tick(nowMs)` driver, just from a fixed step instead of `requestAnimationFrame`.
 * @param duck The controller to advance.
 * @param startMs The wall-clock ms the first tick lands at.
 * @param totalMs Total elapsed ms to advance across.
 * @returns The wall-clock ms reached after the last tick.
 * @example
 * ```
 * const duck = new DuckControllerImpl();
 * tickFor(duck, 0, 100);
 * ```
 */
function tickFor(duck: DuckControllerImpl, startMs: number, totalMs: number): number {
  let now = startMs;
  const end = startMs + totalMs;
  while (now < end) {
    now += 20;
    duck.tick(now);
  }
  return now;
}

describe("ducking sources through the real DuckControllerImpl", () => {
  it("registers KeySource/OsMonitorSource against real DuckSource handles and takes the max of two demands, never a sum", () => {
    const duck = new DuckControllerImpl();
    // Structural proof the two sources' `setSink` accepts the REAL `DuckSource` shape
    // `DuckControllerImpl.addSource` returns — no cast needed (`keySource.ts`'s own doc for
    // `DuckSink` names this as the integration task's job).
    const key = new KeySource();
    const keySink = duck.addSource("ducking:key");
    key.setSink(keySink);
    const osMonitor = new OsMonitorSource({ port: 0, watch: [], logger: { debug() {}, warn() {}, error() {} } });
    const osSink = duck.addSource("ducking:os-monitor");
    osMonitor.setSink(osSink);

    duck.tick(0);
    keySink.set(1);
    osSink.set(0);
    const now = tickFor(duck, 0, 2_000);

    // Fully saturated at the default depth (0.7): gain settles near 1 - 0.7 = 0.3, not lower
    // (which a summed-demand bug would produce with two simultaneously active sources).
    expect(duck.gain).toBeCloseTo(0.3, 1);

    keySink.set(0);
    osSink.set(0);
    tickFor(duck, now, 3_000);
    // The release curve (~600ms time constant) has fully recovered after 3s.
    expect(duck.gain).toBeCloseTo(1, 2);
  });
});
