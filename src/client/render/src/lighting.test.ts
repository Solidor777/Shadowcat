import { test, expect } from "vitest";
import { Lighting, MockBackend } from "./index";

const bands = [{ name: "bright", min: 0.67 }, { name: "dim", min: 0.34 }, { name: "dark", min: 0 }];

test("resolves band index to darkening alpha and the desaturate hint", () => {
  const backend = new MockBackend();
  const l = new Lighting(backend);
  l.setTarget({ cell: 100, bands, hints: ["desaturate"], cells: [
    { i: 0, j: 0, band: 0, tint: 0, hint: -1 },        // bright → no darkening
    { i: 1, j: 0, band: 2, tint: 0, hint: 0 },         // dark + desaturate
  ] });
  l.tick(1000); // run any fade to completion
  const f = backend.lighting!;
  expect(f.cell).toBe(100);
  expect(f.cells.find((c) => c.i === 0)!.alpha).toBeCloseTo(0);
  expect(f.cells.find((c) => c.i === 1)!.alpha).toBeCloseTo(0.6);
  expect(f.cells.find((c) => c.i === 1)!.desaturate).toBe(true);
});

test("interpolates darkening for cells present before and after (day/night fade)", () => {
  const backend = new MockBackend();
  const l = new Lighting(backend);
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 0, tint: 0, hint: -1 }] }); // bright
  l.tick(1000);
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 2, tint: 0, hint: -1 }] }); // → dark
  l.tick(125); // half of 250ms
  const mid = backend.lighting!.cells[0].alpha;
  expect(mid).toBeGreaterThan(0.2);
  expect(mid).toBeLessThan(0.5); // partway between 0 and 0.6
  l.tick(125);
  expect(backend.lighting!.cells[0].alpha).toBeCloseTo(0.6);
});

test("null target clears the overlay", () => {
  const backend = new MockBackend();
  const l = new Lighting(backend);
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 2, tint: 0, hint: -1 }] });
  l.tick(1000);
  l.setTarget(null);
  l.tick(0);
  expect(backend.lighting!.cells).toEqual([]);
});
