// @vitest-environment node
// Pure functions plus a shared fixture text; no DOM.
import { describe, expect, it } from "vitest";
import fixtureText from "./__fixtures__/server-bounds.json?raw";
import type { AmbientLightMode } from "@shadowcat/core";
import { MAX_PROXIMITY_CELLS, clampAmbientLevel, isValidProximityWithin, isValidTemplateAngle, isValidTemplateWidth } from "./server-bounds";

/** Accepted and rejected sample values for one bound. */
interface Samples {
  /** Values the server accepts. */
  accepted: number[];
  /** Values the server rejects. */
  rejected: number[];
}

// One corpus read on both sides: this file and the server test `client_bounds_parity`, which
// feeds the same samples to the server's validators. A bound that moves on one side fails the other.
const fixture: {
  maxProximityCells: number;
  proximityWithin: Samples;
  templateAngle: Samples;
  templateWidth: Samples;
  ambientLevel: Record<AmbientLightMode, Samples>;
} = JSON.parse(fixtureText);

describe("server bounds", () => {
  it("copies the server's proximity ceiling and accepts exactly the fixture's distances", () => {
    expect(MAX_PROXIMITY_CELLS).toBe(fixture.maxProximityCells);
    for (const v of fixture.proximityWithin.accepted) expect(isValidProximityWithin(v), String(v)).toBe(true);
    for (const v of fixture.proximityWithin.rejected) expect(isValidProximityWithin(v), String(v)).toBe(false);
  });

  it("template angle agrees with the fixture, and null means no override", () => {
    for (const v of fixture.templateAngle.accepted) expect(isValidTemplateAngle(v), String(v)).toBe(true);
    for (const v of fixture.templateAngle.rejected) expect(isValidTemplateAngle(v), String(v)).toBe(false);
    expect(isValidTemplateAngle(null)).toBe(true);
    expect(isValidTemplateAngle(Number.NaN)).toBe(false);
  });

  it("template width agrees with the fixture, and null means no override", () => {
    for (const v of fixture.templateWidth.accepted) expect(isValidTemplateWidth(v), String(v)).toBe(true);
    for (const v of fixture.templateWidth.rejected) expect(isValidTemplateWidth(v), String(v)).toBe(false);
    expect(isValidTemplateWidth(null)).toBe(true);
    expect(isValidTemplateWidth(Number.POSITIVE_INFINITY)).toBe(false);
  });

  it("clampAmbientLevel returns a level the server accepts, and leaves an accepted one alone", () => {
    for (const mode of ["override", "darken", "brighten"] as const) {
      const samples = fixture.ambientLevel[mode];
      for (const v of samples.accepted) expect(clampAmbientLevel(mode, v), `${mode} ${v}`).toBe(v);
      for (const v of [...samples.rejected, Number.NaN, Number.POSITIVE_INFINITY]) {
        const clamped = clampAmbientLevel(mode, v);
        expect(clamped >= 0 && Number.isFinite(clamped) && (mode !== "override" || clamped <= 1), `${mode} ${v}`).toBe(true);
      }
    }
  });
});
