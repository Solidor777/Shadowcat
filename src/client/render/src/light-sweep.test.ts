import { describe, expect, test } from "vitest";
import { Grid } from "./grid";
import { bandAlpha, MAX_DARK_ALPHA, TINT_ALPHA } from "./lighting";
import { blendLightCells, lightSampleCells, MAX_LIGHT_SWEEP_CELLS } from "./light-sweep";
import type { MoveLightSample, Polygon } from "./types";

const grid = new Grid({ kind: "square", size: 100 });
/** A wide open ring covering the whole neighbourhood. */
const OPEN: [number, number][][] = [[[-1000, -1000], [1000, -1000], [1000, 1000], [-1000, 1000]]];
const LOS_ALL: Polygon[] = [{ points: [-1000, -1000, 1000, -1000, 1000, 1000, -1000, 1000] }];

function torch(over: Partial<MoveLightSample> = {}): MoveLightSample {
  return { tMs: 0, pos: [50, 50], bright: 100, dim: 250, color: 0xffcc66, polygons: OPEN, ...over };
}

describe("lightSampleCells", () => {
  test("lights the cells within the dim disc: bright band inside `bright`, dim band beyond, tinted with the light color", () => {
    const cells = lightSampleCells(torch(), grid, LOS_ALL, 3);
    const at = (i: number, j: number) => cells.find((c) => c.i === i && c.j === j);
    // Center cell (0,0) is within bright reach → brightest band, no darkening.
    expect(at(0, 0)).toMatchObject({ alpha: bandAlpha(0, 3), tint: 0xffcc66, tintAlpha: TINT_ALPHA, desaturate: false });
    // Cell (2,0): center (250,50) is 200 away — dim band.
    expect(at(2, 0)?.alpha).toBeCloseTo(bandAlpha(1, 3));
    // Cell (3,0): center (350,50) is 300 away — beyond dim, absent.
    expect(at(3, 0)).toBeUndefined();
    // Every emitted cell carries the grid's own corner geometry.
    for (const c of cells) expect(c.corners).toEqual(grid.cellVertices(c.i, c.j));
  });

  test("intersects with the viewer's line of sight — a cell outside every LOS polygon is never lit", () => {
    const losLeftHalf: Polygon[] = [{ points: [-1000, -1000, 100, -1000, 100, 1000, -1000, 1000] }];
    const cells = lightSampleCells(torch(), grid, losLeftHalf, 3);
    expect(cells.every((c) => c.i <= 0)).toBe(true);
    expect(cells.some((c) => c.i === 0 && c.j === 0)).toBe(true);
  });

  test("respects the sample's own occlusion polygon", () => {
    // The light's polygon covers only x <= 100 (a wall at x=100 east of the torch).
    const occluded: [number, number][][] = [[[-1000, -1000], [100, -1000], [100, 1000], [-1000, 1000]]];
    const cells = lightSampleCells(torch({ polygons: occluded }), grid, LOS_ALL, 3);
    expect(cells.every((c) => c.i <= 0)).toBe(true);
  });

  test("a single-band gradation lights every reached cell at band 0", () => {
    const cells = lightSampleCells(torch(), grid, LOS_ALL, 1);
    expect(cells.every((c) => c.alpha === 0)).toBe(true);
  });

  test("fails closed on a degenerate sample or an over-cap reach", () => {
    expect(lightSampleCells(torch({ dim: 0 }), grid, LOS_ALL, 3)).toEqual([]);
    expect(lightSampleCells(torch({ dim: NaN }), grid, LOS_ALL, 3)).toEqual([]);
    expect(lightSampleCells(torch({ pos: [NaN, 50] }), grid, LOS_ALL, 3)).toEqual([]);
    // A disc spanning more than MAX_LIGHT_SWEEP_CELLS candidate cells contributes nothing.
    const huge = torch({ dim: 100 * Math.sqrt(MAX_LIGHT_SWEEP_CELLS) });
    expect(lightSampleCells(huge, grid, LOS_ALL, 3)).toEqual([]);
  });

  test("enumerates hex cells through the same axial bounding box", () => {
    const hex = new Grid({ kind: "hex", size: 100 });
    const center = hex.cellCenter(2, 2);
    const cells = lightSampleCells(torch({ pos: [center.x, center.y] }), hex, LOS_ALL, 3);
    // The torch's own hex is lit, and every lit hex center is genuinely within reach.
    expect(cells.some((c) => c.i === 2 && c.j === 2)).toBe(true);
    for (const c of cells) {
      const p = hex.cellCenter(c.i, c.j);
      expect(Math.hypot(p.x - center.x, p.y - center.y)).toBeLessThanOrEqual(250);
    }
  });
});

describe("blendLightCells", () => {
  const cellA = { i: 0, j: 0, alpha: 0.3, tint: 0x111111, tintAlpha: 0.25, desaturate: false, corners: [] };
  const cellB = { i: 1, j: 0, alpha: 0, tint: 0x222222, tintAlpha: 0.25, desaturate: false, corners: [] };
  const cellA2 = { ...cellA, alpha: 0.1, tint: 0x333333 };

  test("a cell in both sets lerps alpha/tintAlpha and takes the incoming tint", () => {
    const out = blendLightCells([cellA], [cellA2], 0.5);
    expect(out).toHaveLength(1);
    expect(out[0].alpha).toBeCloseTo(0.2);
    expect(out[0].tintAlpha).toBeCloseTo(0.25);
    expect(out[0].tint).toBe(0x333333);
  });

  test("a cell only in `from` darkens toward full dark and a cell only in `to` brightens from it", () => {
    const out = blendLightCells([cellA], [cellB], 0.25);
    const a = out.find((c) => c.i === 0)!;
    const b = out.find((c) => c.i === 1)!;
    // Absent = fully dark, so a one-sided cell fades between its own alpha and MAX_DARK_ALPHA.
    expect(a.alpha).toBeCloseTo(0.3 + (MAX_DARK_ALPHA - 0.3) * 0.25);
    expect(a.tintAlpha).toBeCloseTo(0.25 * 0.75);
    expect(b.alpha).toBeCloseTo(MAX_DARK_ALPHA + (0 - MAX_DARK_ALPHA) * 0.25);
    expect(b.tintAlpha).toBeCloseTo(0.25 * 0.25);
  });

  test("a one-sided cell at zero weight is absent, never a fully dark ghost", () => {
    // factor 0: the incoming-only cell does not exist yet; factor 1: the outgoing-only cell is gone.
    expect(blendLightCells([cellA], [cellB], 0).map((c) => c.i)).toEqual([0]);
    expect(blendLightCells([cellA], [cellB], 1).map((c) => c.i)).toEqual([1]);
  });

  test("factor 0 is exactly `from`, factor 1 exactly `to`, and a non-finite factor snaps to `to`", () => {
    expect(blendLightCells([cellA], [cellA2], 0)[0]).toMatchObject({ alpha: 0.3, tintAlpha: 0.25 });
    expect(blendLightCells([cellA], [cellA2], 1)[0]).toMatchObject({ alpha: 0.1, tint: 0x333333 });
    expect(blendLightCells([cellA], [cellA2], NaN)[0].alpha).toBeCloseTo(0.1);
  });
});
