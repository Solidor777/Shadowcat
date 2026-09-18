// @vitest-environment node
import { describe, it, expect } from "vitest";
import { dice3dEnabled, reducedMotionPreferred, antialiasPreferred } from "./performanceSeam";

/** A minimal fixture matching the `Pick<AppContext, "performance">` slice these getters
 * read — no full `AppContext` construction needed. */
function ctxWith(current: { dice3d: boolean; reducedMotion: boolean; antialias: boolean }) {
  return { performance: { current } } as Parameters<typeof dice3dEnabled>[0];
}

describe("performanceSeam (reads the caller's resolved AppContext)", () => {
  it("dice3dEnabled reads PerformanceSettings.dice3d", () => {
    expect(dice3dEnabled(ctxWith({ dice3d: true, reducedMotion: false, antialias: true }))).toBe(true);
    expect(dice3dEnabled(ctxWith({ dice3d: false, reducedMotion: false, antialias: true }))).toBe(false);
  });

  it("reducedMotionPreferred reads PerformanceSettings.reducedMotion", () => {
    expect(reducedMotionPreferred(ctxWith({ dice3d: true, reducedMotion: true, antialias: true }))).toBe(true);
    expect(reducedMotionPreferred(ctxWith({ dice3d: true, reducedMotion: false, antialias: true }))).toBe(false);
  });

  it("antialiasPreferred reads PerformanceSettings.antialias", () => {
    expect(antialiasPreferred(ctxWith({ dice3d: true, reducedMotion: false, antialias: true }))).toBe(true);
    expect(antialiasPreferred(ctxWith({ dice3d: true, reducedMotion: false, antialias: false }))).toBe(false);
  });
});
