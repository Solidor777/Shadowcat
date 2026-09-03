import { test, expect } from "vitest";
import { Lighting, MockBackend, mergeSweepCells, bandAlpha, unionLightingInputs, holdLightingCells } from "./index";

const bands = [{ name: "bright", min: 0.67 }, { name: "dim", min: 0.34 }, { name: "dark", min: 0 }];

test("resolves band index to darkening alpha and the desaturate hint", () => {
  const backend = new MockBackend();
  const l = new Lighting(backend);
  l.setTarget({ cell: 100, bands, hints: ["desaturate"], cells: [
    { i: 0, j: 0, band: 0, tint: 0, hint: -1, corners: [] },        // bright → no darkening
    { i: 1, j: 0, band: 2, tint: 0, hint: 0, corners: [] },         // dark + desaturate
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
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 0, tint: 0, hint: -1, corners: [] }] }); // bright
  l.tick(1000);
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 2, tint: 0, hint: -1, corners: [] }] }); // → dark
  l.tick(125); // half of 250ms
  const mid = backend.lighting!.cells[0].alpha;
  expect(mid).toBeCloseTo(0.3, 1); // halfway between 0 and 0.6 at t≈0.5
  l.tick(125);
  expect(backend.lighting!.cells[0].alpha).toBeCloseTo(0.6);
});

test("null target clears the overlay", () => {
  const backend = new MockBackend();
  const l = new Lighting(backend);
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 2, tint: 0, hint: -1, corners: [] }] });
  l.tick(1000);
  l.setTarget(null);
  l.tick(0);
  expect(backend.lighting!.cells).toEqual([]);
});

test("mid-fade retarget continues from the displayed midpoint without a snap", () => {
  // bright→dark: alpha goes from 0 to 0.6; after 125ms (t≈0.5) alpha≈0.3.
  // Retarget back to bright while mid-fade: new fade starts from ≈0.3 toward 0.
  const backend = new MockBackend();
  const l = new Lighting(backend);
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 0, tint: 0, hint: -1, corners: [] }] }); // bright
  l.tick(1000); // settle
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 2, tint: 0, hint: -1, corners: [] }] }); // → dark
  l.tick(125); // mid-fade: alpha≈0.3
  const afterRetarget = backend.lighting!.cells[0].alpha;
  expect(afterRetarget).toBeCloseTo(0.3, 1); // captured midpoint, not 0 or 0.6
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 0, tint: 0, hint: -1, corners: [] }] }); // back to bright
  l.tick(50); // a few ticks into the new fade: should move toward 0
  const moving = backend.lighting!.cells[0].alpha;
  expect(moving).toBeLessThan(afterRetarget); // heading toward bright from midpoint
  expect(moving).toBeGreaterThan(0); // not yet arrived
});

test("cells only in prev snap gone during fade; new-only cells appear immediately", () => {
  // prev has (0,0); target has (1,0) only — (0,0) must not ghost during the fade.
  const backend = new MockBackend();
  const l = new Lighting(backend);
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 2, tint: 0, hint: -1, corners: [] }] });
  l.tick(1000); // settle — (0,0) is now prev
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 1, j: 0, band: 2, tint: 0, hint: -1, corners: [] }] });
  l.tick(50); // mid-fade
  const keys = backend.lighting!.cells.map((c) => `${c.i},${c.j}`);
  // prev-only cell (0,0) must be absent — snap-gone, not ghosted
  expect(keys).not.toContain("0,0");
  // new-only cell (1,0) must be present immediately (snapped in)
  expect(keys).toContain("1,0");
});

test("tint: alpha lerps; color held when one side is untinted, channel-blended when both tinted", () => {
  const backend = new MockBackend();
  const l = new Lighting(backend);

  // Sub-case A: tintAlpha 0 → 0.25; mid-fade tintAlpha is between, tint color held at target.
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 0, tint: 0, hint: -1, corners: [] }] });
  l.tick(1000);
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 0, tint: 0xFF0000, hint: -1, corners: [] }] });
  l.tick(125); // mid-fade
  const cellA = backend.lighting!.cells[0];
  expect(cellA.tintAlpha).toBeGreaterThan(0);
  expect(cellA.tintAlpha).toBeLessThan(0.25);
  // Color must be held at target (0xFF0000), NOT lerped toward black.
  expect(cellA.tint).toBe(0xFF0000);

  // Sub-case B: 0xFF0000 → 0x0000FF; mid-fade R between 0–255, B between 0–255.
  const b2 = new MockBackend();
  const lb = new Lighting(b2);
  lb.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 0, tint: 0xFF0000, hint: -1, corners: [] }] });
  lb.tick(1000);
  lb.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 0, tint: 0x0000FF, hint: -1, corners: [] }] });
  lb.tick(125); // mid-fade
  const cellB = b2.lighting!.cells[0];
  const r = (cellB.tint >> 16) & 0xff;
  const blue = cellB.tint & 0xff;
  expect(r).toBeGreaterThan(0);
  expect(r).toBeLessThan(255); // lerped toward 0
  expect(blue).toBeGreaterThan(0);
  expect(blue).toBeLessThan(255); // lerped toward 255
});

test("corners carry through unchanged across a fade — cell geometry is not something a day/night fade interpolates", () => {
  const backend = new MockBackend();
  const l = new Lighting(backend);
  const hexCorners = [{ x: 1, y: 2 }, { x: 3, y: 4 }, { x: 5, y: 6 }];
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 0, tint: 0, hint: -1, corners: hexCorners }] });
  l.tick(1000); // settle
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 2, tint: 0, hint: -1, corners: hexCorners }] });
  l.tick(125); // mid-fade
  expect(backend.lighting!.cells[0].corners).toEqual(hexCorners);
});

test("bandAlpha maps band 0 to no darkening and the last band to the maximum", () => {
  expect(bandAlpha(0, 3)).toBe(0);
  expect(bandAlpha(2, 3)).toBeCloseTo(0.6);
  expect(bandAlpha(1, 3)).toBeCloseTo(0.3);
  expect(bandAlpha(0, 1)).toBe(0);
});

test("mergeSweepCells replaces a same-key base cell and appends new ones; null/empty is identity", () => {
  const darkness = [{ points: [0, 0, 100, 0, 100, 100] }];
  const base = { cell: 100, cells: [{ i: 0, j: 0, alpha: 0.6, tint: 0, tintAlpha: 0, desaturate: false, corners: [] }], darkness };
  expect(mergeSweepCells(base, null)).toBe(base);
  expect(mergeSweepCells(base, [])).toBe(base);
  const lit = { i: 0, j: 0, alpha: 0, tint: 0xffcc66, tintAlpha: 0.25, desaturate: false, corners: [] };
  const fresh = { i: 1, j: 0, alpha: 0.3, tint: 0xffcc66, tintAlpha: 0.25, desaturate: false, corners: [] };
  const out = mergeSweepCells(base, [lit, fresh]);
  expect(out.cells).toEqual([lit, fresh]);
  expect(out.darkness).toBe(darkness); // the base's darkness rides through
  expect(base.cells[0].alpha).toBe(0.6); // base untouched
});

test("unionLightingInputs keeps prev cells next does not restate; next wins a shared key and supplies cell/bands/hints", () => {
  const prev = { cell: 100, bands: [{ name: "bright", min: 0.67 }], hints: [], cells: [
    { i: 0, j: 0, band: 0, tint: 0, hint: -1, corners: [] },
    { i: 1, j: 0, band: 1, tint: 0, hint: -1, corners: [] },
  ] };
  const next = { cell: 50, bands, hints: ["desaturate"], cells: [
    { i: 1, j: 0, band: 2, tint: 0xffcc66, hint: 0, corners: [] },
    { i: 3, j: 0, band: 0, tint: 0, hint: -1, corners: [] },
  ] };
  const out = unionLightingInputs(prev, next);
  expect(out.cell).toBe(50);
  expect(out.bands).toBe(bands);
  expect(out.hints).toEqual(["desaturate"]);
  expect(out.cells.map((c) => [c.i, c.j, c.band])).toEqual([[1, 0, 2], [3, 0, 0], [0, 0, 0]]);
  expect(prev.cells).toHaveLength(2); // pure
  expect(next.cells).toHaveLength(2);
});

test("holdLightingCells keeps held keys at prev's values (absent when prev lacks them) and takes everything else from next", () => {
  const prev = { cell: 100, bands: [{ name: "bright", min: 0.67 }], hints: [], cells: [
    { i: 3, j: 0, band: 1, tint: 0, hint: -1, corners: [] },
  ] };
  const next = { cell: 50, bands, hints: ["desaturate"], cells: [
    { i: 3, j: 0, band: 0, tint: 0xffcc66, hint: 0, corners: [] }, // held: prev's band-1 cell stands in
    { i: 4, j: 0, band: 0, tint: 0xffcc66, hint: 0, corners: [] }, // held: prev has none → absent
    { i: 7, j: 7, band: 0, tint: 0, hint: -1, corners: [] },       // applies at once
  ] };
  const out = holdLightingCells(prev, next, new Set(["3,0", "4,0"]));
  expect(out.cell).toBe(50);
  expect(out.bands).toBe(bands);
  expect(out.cells.map((c) => [c.i, c.j, c.band, c.tint])).toEqual([[7, 7, 0, 0], [3, 0, 1, 0]]);
  expect(holdLightingCells(prev, next, new Set())).toBe(next); // nothing held → next itself
  expect(prev.cells).toHaveLength(1); // pure
  expect(next.cells).toHaveLength(3);
});

test("darkness regions paint only while a lighting model is in force, and a repeated reference does not repaint", () => {
  const backend = new MockBackend();
  const painted: number[] = [];
  const l = new Lighting(backend, (f) => painted.push(f.darkness.length));
  const los = [{ points: [0, 0, 300, 0, 300, 300, 0, 300] }];
  // No model yet: the regions are held, not painted.
  l.setDarkness(los);
  expect(backend.lighting!.darkness).toEqual([]);
  // A model arrives: the held regions paint with it, and ride every later frame unchanged.
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 1, tint: 0, hint: -1, corners: [] }] });
  expect(backend.lighting!.darkness).toBe(los);
  l.tick(125);
  expect(backend.lighting!.darkness).toBe(los);
  expect(backend.lighting!.cells[0].alpha).toBeGreaterThan(0);
  l.setSweep([{ i: 1, j: 0, alpha: 0, tint: 0xffcc66, tintAlpha: 0.25, desaturate: false, corners: [] }]);
  expect(backend.lighting!.darkness).toBe(los);
  // The same reference is a no-op; a new one repaints at once.
  const paints = painted.length;
  l.setDarkness(los);
  expect(painted.length).toBe(paints);
  const narrower = [{ points: [0, 0, 100, 0, 100, 100] }];
  l.setDarkness(narrower);
  expect(backend.lighting!.darkness).toBe(narrower);
  expect(painted.length).toBe(paints + 1);
  // The model is withdrawn (GM `mode:"all"`): the regions are withheld again.
  l.setSweep(null);
  l.setTarget(null);
  expect(backend.lighting!.darkness).toEqual([]);
  expect(backend.lighting!.cells).toEqual([]);
});

test("setSweep paints the committed frame unioned with the sweep at once, and clearing it restores the frame", () => {
  const backend = new MockBackend();
  const painted: number[] = [];
  const l = new Lighting(backend, (f) => painted.push(f.cells.length));
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 2, tint: 0, hint: -1, corners: [] }] });
  l.tick(1000);
  expect(backend.lighting!.cells[0].alpha).toBeCloseTo(0.6);
  // The sweep lifts the dark cell and adds a neighbour — no fade, immediate.
  l.setSweep([
    { i: 0, j: 0, alpha: 0, tint: 0xffcc66, tintAlpha: 0.25, desaturate: false, corners: [] },
    { i: 1, j: 0, alpha: 0.3, tint: 0xffcc66, tintAlpha: 0.25, desaturate: false, corners: [] },
  ]);
  expect(backend.lighting!.cells.map((c) => [c.i, c.alpha])).toEqual([[0, 0], [1, 0.3]]);
  expect(l.current()).toBe(backend.lighting);
  l.setSweep(null);
  expect(backend.lighting!.cells.map((c) => [c.i, c.alpha])).toEqual([[0, 0.6]]);
  expect(painted).toEqual([1, 1, 2, 1]); // setTarget, settle tick, sweep on, sweep off
});
