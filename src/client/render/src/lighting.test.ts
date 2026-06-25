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
  expect(mid).toBeCloseTo(0.3, 1); // halfway between 0 and 0.6 at t≈0.5
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

test("mid-fade retarget continues from the displayed midpoint without a snap", () => {
  // bright→dark: alpha goes from 0 to 0.6; after 125ms (t≈0.5) alpha≈0.3.
  // Retarget back to bright while mid-fade: new fade starts from ≈0.3 toward 0.
  const backend = new MockBackend();
  const l = new Lighting(backend);
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 0, tint: 0, hint: -1 }] }); // bright
  l.tick(1000); // settle
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 2, tint: 0, hint: -1 }] }); // → dark
  l.tick(125); // mid-fade: alpha≈0.3
  const afterRetarget = backend.lighting!.cells[0].alpha;
  expect(afterRetarget).toBeCloseTo(0.3, 1); // captured midpoint, not 0 or 0.6
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 0, tint: 0, hint: -1 }] }); // back to bright
  l.tick(50); // a few ticks into the new fade: should move toward 0
  const moving = backend.lighting!.cells[0].alpha;
  expect(moving).toBeLessThan(afterRetarget); // heading toward bright from midpoint
  expect(moving).toBeGreaterThan(0); // not yet arrived
});

test("cells only in prev snap gone during fade; new-only cells appear immediately", () => {
  // prev has (0,0); target has (1,0) only — (0,0) must not ghost during the fade.
  const backend = new MockBackend();
  const l = new Lighting(backend);
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 2, tint: 0, hint: -1 }] });
  l.tick(1000); // settle — (0,0) is now prev
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 1, j: 0, band: 2, tint: 0, hint: -1 }] });
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
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 0, tint: 0, hint: -1 }] });
  l.tick(1000);
  l.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 0, tint: 0xFF0000, hint: -1 }] });
  l.tick(125); // mid-fade
  const cellA = backend.lighting!.cells[0];
  expect(cellA.tintAlpha).toBeGreaterThan(0);
  expect(cellA.tintAlpha).toBeLessThan(0.25);
  // Color must be held at target (0xFF0000), NOT lerped toward black.
  expect(cellA.tint).toBe(0xFF0000);

  // Sub-case B: 0xFF0000 → 0x0000FF; mid-fade R between 0–255, B between 0–255.
  const l2 = new Lighting(new MockBackend());
  const b2 = new MockBackend();
  const lb = new Lighting(b2);
  lb.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 0, tint: 0xFF0000, hint: -1 }] });
  lb.tick(1000);
  lb.setTarget({ cell: 100, bands, hints: [], cells: [{ i: 0, j: 0, band: 0, tint: 0x0000FF, hint: -1 }] });
  lb.tick(125); // mid-fade
  const cellB = b2.lighting!.cells[0];
  const r = (cellB.tint >> 16) & 0xff;
  const blue = cellB.tint & 0xff;
  expect(r).toBeGreaterThan(0);
  expect(r).toBeLessThan(255); // lerped toward 0
  expect(blue).toBeGreaterThan(0);
  expect(blue).toBeLessThan(255); // lerped toward 255
  void l2; // unused (only b2/lb used in sub-case B)
});
