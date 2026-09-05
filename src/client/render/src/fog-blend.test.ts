import { readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";
import { computeFogBlendFactor, fogBlendRtStale, chooseVisionSample, visibilityInputKey } from "./fog-blend";
import type { MoveLightSample, MoveVisionSample, VisibilityInput } from "./types";

describe("computeFogBlendFactor", () => {
  test("is 0 at tCur and 1 at tNext", () => {
    expect(computeFogBlendFactor(0, 0, 500)).toBe(0);
    expect(computeFogBlendFactor(500, 0, 500)).toBe(1);
  });

  test("advances linearly 0→1 across the interval", () => {
    expect(computeFogBlendFactor(125, 0, 500)).toBeCloseTo(0.25);
    expect(computeFogBlendFactor(250, 0, 500)).toBeCloseTo(0.5);
    expect(computeFogBlendFactor(375, 0, 500)).toBeCloseTo(0.75);
  });

  test("clamps outside the interval", () => {
    expect(computeFogBlendFactor(-50, 0, 500)).toBe(0);
    expect(computeFogBlendFactor(600, 0, 500)).toBe(1);
  });

  test("snaps to 1 on a degenerate or inverted span (tNext <= tCur)", () => {
    expect(computeFogBlendFactor(100, 500, 500)).toBe(1);
    expect(computeFogBlendFactor(100, 500, 200)).toBe(1);
  });

  test("fails safe (snaps to 1) on non-finite input", () => {
    expect(computeFogBlendFactor(NaN, 0, 500)).toBe(1);
    expect(computeFogBlendFactor(100, NaN, 500)).toBe(1);
    expect(computeFogBlendFactor(100, 0, Infinity)).toBe(1);
  });
});

describe("fogBlendRtStale", () => {
  test("stales when there is no existing texture", () => {
    expect(fogBlendRtStale(null, 800, 600, 1)).toBe(true);
  });

  test("does not stale when width/height/resolution are unchanged", () => {
    expect(fogBlendRtStale({ width: 800, height: 600, resolution: 1 }, 800, 600, 1)).toBe(false);
  });

  test("stales on a width or height change", () => {
    expect(fogBlendRtStale({ width: 800, height: 600, resolution: 1 }, 1024, 600, 1)).toBe(true);
    expect(fogBlendRtStale({ width: 800, height: 600, resolution: 1 }, 800, 768, 1)).toBe(true);
  });

  test("stales on a resolution change", () => {
    expect(fogBlendRtStale({ width: 800, height: 600, resolution: 1 }, 800, 600, 2)).toBe(true);
  });
});

describe("visibilityInputKey", () => {
  // A positive control (repeats collapse to the same key) proves the cache FIRES; these prove
  // it DISCRIMINATES — a genuinely different input must never fingerprint the same as another,
  // or PixiBackend.setVisibilityBlend would reuse a stale rasterized texture and paint the
  // wrong fog mid-sweep. Fog is the secrecy gate (fail-closed invariant); a cache that silently
  // stops discriminating is exactly the failure that invariant exists to prevent.
  test("differs when visible differs (same explored, same mode)", () => {
    const a: VisibilityInput = { mode: "masked", visible: [{ points: [0, 0, 10, 0, 10, 10] }], explored: [], perceived: [] };
    const b: VisibilityInput = { mode: "masked", visible: [{ points: [0, 0, 20, 0, 20, 20] }], explored: [], perceived: [] };
    expect(visibilityInputKey(a)).not.toBe(visibilityInputKey(b));
  });

  test("differs when explored differs (same visible, same mode)", () => {
    const a: VisibilityInput = { mode: "masked", visible: [{ points: [0, 0, 10, 0, 10, 10] }], explored: [], perceived: [] };
    const b: VisibilityInput = { mode: "masked", visible: [{ points: [0, 0, 10, 0, 10, 10] }], explored: [{ points: [5, 5, 6, 5, 6, 6] }], perceived: [] };
    expect(visibilityInputKey(a)).not.toBe(visibilityInputKey(b));
  });

  test("differs when mode differs (\"all\" vs \"masked\")", () => {
    const a: VisibilityInput = { mode: "all", visible: [], explored: [], perceived: [] };
    const b: VisibilityInput = { mode: "masked", visible: [], explored: [], perceived: [] };
    expect(visibilityInputKey(a)).not.toBe(visibilityInputKey(b));
  });

  test("is stable (equal) for two structurally-identical inputs — the positive control this cache also needs", () => {
    const a: VisibilityInput = { mode: "masked", visible: [{ points: [0, 0, 10, 0, 10, 10] }], explored: [{ points: [1, 1, 2, 1, 2, 2] }], perceived: [] };
    const b: VisibilityInput = { mode: "masked", visible: [{ points: [0, 0, 10, 0, 10, 10] }], explored: [{ points: [1, 1, 2, 1, 2, 2] }], perceived: [] };
    expect(visibilityInputKey(a)).toBe(visibilityInputKey(b));
  });
});

test("chooseVisionSample matches the server's chosen_vision_sample on the shared fixture", () => {
  const raw = JSON.parse(
    readFileSync(new URL("./__fixtures__/chosen-vision-sample.json", import.meta.url), "utf8"),
  ) as { samples: number[]; probes: { elapsed: number; expectIndex: number }[] };
  const samples: MoveVisionSample[] = raw.samples.map((tMs, i) => ({
    tMs,
    polygons: [[[i, 0], [i, 1], [i + 1, 1]]],
  }));
  for (const { elapsed, expectIndex } of raw.probes) {
    expect(chooseVisionSample(samples, elapsed).polygons[0][0][0]).toBe(expectIndex);
  }
});

test("chooseVisionSample selects a light timeline by the same shared-fixture rule (one rule, two sample kinds)", () => {
  const raw = JSON.parse(
    readFileSync(new URL("./__fixtures__/chosen-vision-sample.json", import.meta.url), "utf8"),
  ) as { samples: number[]; probes: { elapsed: number; expectIndex: number }[] };
  const samples: MoveLightSample[] = raw.samples.map((tMs, i) => ({
    tMs,
    pos: [i, 0],
    bright: 1,
    dim: 2,
    color: 0xffcc66,
    intensity: 1,
    falloff: "linear",
    polygons: [[[i, 0], [i, 1], [i + 1, 1]]],
  }));
  for (const { elapsed, expectIndex } of raw.probes) {
    expect(chooseVisionSample(samples, elapsed).pos[0]).toBe(expectIndex);
  }
});
