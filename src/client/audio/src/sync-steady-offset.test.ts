// @vitest-environment node
import { describe, it, expect } from "vitest";
import type { MediaElementLike } from "./context";
import { SYNC_SEEK_THRESHOLD_SECS, syncElementPosition } from "./track-player";
import { stubMediaElement } from "./__fixtures__/stubContext";

/** An element whose reported `currentTime` is its true position plus a constant `bias`, and
 * whose seeks land the true position on the requested time (so the reading after a seek to `t`
 * is `t + bias`).
 * @param bias Seconds the reading leads the true position.
 * @returns The element, its seek count and a handle on its true position.
 * @example
 * ```ts
 * const { el } = biasedElement(1);
 * el.currentTime = 5; // reads 6
 * ```
 */
function biasedElement(bias: number): {
  /** The element under test. */
  el: MediaElementLike;
  /** Hard seeks written to `currentTime`. */
  seeks: () => number;
  /** Moves the true position by `secs` without a seek (playback, a stall or a jump). */
  advance: (secs: number) => void;
} {
  const base = stubMediaElement();
  let position = 0;
  let seeks = 0;
  const el: MediaElementLike = Object.defineProperty(base, "currentTime", {
    get: () => position + bias,
    set: (value: number) => {
      seeks += 1;
      position = value;
    },
  });
  return { el, seeks: () => seeks, advance: (secs) => void (position += secs) };
}

/** Runs `ticks` one-second ticks: the true position advances one second, then the target does.
 * @param h The biased element handle.
 * @param target The server target before the first tick, seconds.
 * @param ticks How many ticks to run.
 * @returns The target after the last tick.
 * @example
 * ```ts
 * runTicks(biasedElement(1), 0, 10);
 * ```
 */
function runTicks(h: ReturnType<typeof biasedElement>, target: number, ticks: number): number {
  let t = target;
  for (let i = 0; i < ticks; i++) {
    t += 1;
    h.advance(1);
    syncElementPosition(h.el, t);
  }
  return t;
}

describe("syncElementPosition against an element that reports a constant lead", () => {
  it("seeks once, not once per call, while the offset holds", () => {
    const h = biasedElement(1);
    runTicks(h, 0, 60);
    expect(h.seeks()).toBe(1);
  });

  it("an element with no lead keeps its single seek and a rate-only correction", () => {
    const h = biasedElement(0);
    syncElementPosition(h.el, 5);
    runTicks(h, 5, 60);
    expect(h.seeks()).toBe(1);
  });

  it("still corrects an element that falls behind after the offset is adopted", () => {
    const h = biasedElement(1);
    const t = runTicks(h, 0, 10);
    expect(h.seeks()).toBe(1);
    h.advance(-(SYNC_SEEK_THRESHOLD_SECS + 0.3));
    syncElementPosition(h.el, t);
    expect(h.seeks()).toBe(2);
    runTicks(h, t, 30);
    expect(h.seeks()).toBe(2);
  });

  it("still corrects a jump after the offset is adopted", () => {
    const h = biasedElement(1);
    const t = runTicks(h, 0, 10);
    h.advance(40);
    syncElementPosition(h.el, t);
    expect(h.seeks()).toBe(2);
    expect(h.el.currentTime - t).toBeCloseTo(1, 5);
    runTicks(h, t, 30);
    expect(h.seeks()).toBe(2);
  });
});
