import { describe, it, expect } from "vitest";
import { TokenAnimator } from "./token-animator";

const cfg = { speedCellsPerSec: 6, easing: "linear" as const, cellSize: 100 };

function fresh(): TokenAnimator {
  const a = new TokenAnimator();
  a.setConfig(cfg);
  return a;
}

describe("TokenAnimator duration model", () => {
  it("a brand-new id snaps to its target", () => {
    const a = fresh();
    a.setTarget("t1", { x: 100, y: 50, rotation: 0 });
    expect(a.get("t1")).toEqual({ x: 100, y: 50, rotation: 0 });
    expect(a.tick(16)).toEqual([]); // already there → nothing moves
  });

  it("duration = distanceCells / speed; reaches target exactly at duration (linear)", () => {
    const a = fresh();
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 }); // snap
    a.setTarget("t1", { x: 600, y: 0, rotation: 0 }); // 6 cells @ 6 c/s → 1000ms
    a.tick(500); // half time, linear → halfway
    expect(a.get("t1")!.x).toBeCloseTo(300, 0);
    a.tick(500); // remaining → settle exactly
    expect(a.get("t1")).toEqual({ x: 600, y: 0, rotation: 0 });
    expect(a.tick(16)).toEqual([]); // settled
  });

  it("easeInOut is slower than linear at the first quarter of the duration", () => {
    const a = new TokenAnimator();
    a.setConfig({ ...cfg, easing: "easeInOut" });
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
    a.setTarget("t1", { x: 600, y: 0, rotation: 0 }); // 1000ms
    a.tick(250); // quarter time
    expect(a.get("t1")!.x).toBeLessThan(150); // < linear's 150
  });

  it("animateAlongPath traverses waypoints (L-route bends at the corner)", () => {
    const a = fresh();
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 }); // snap to start
    // start (0,0) → (300,0) → (300,300): total 6 cells → 1000ms.
    a.animateAlongPath("t1", [[0, 0], [300, 0], [300, 300]], 0);
    a.tick(500); // half distance (3 cells) → end of first leg, at the corner
    expect(a.get("t1")!.x).toBeCloseTo(300, 0);
    expect(a.get("t1")!.y).toBeCloseTo(0, 0);
    a.tick(500);
    expect(a.get("t1")).toEqual({ x: 300, y: 300, rotation: 0 });
  });

  it("optimistic route-vertex setTarget does NOT clobber an active path walk", () => {
    const a = fresh();
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
    a.animateAlongPath("t1", [[0, 0], [300, 0], [300, 300]], 0);
    a.tick(100); // partway along leg 1
    const mid = a.get("t1")!.x;
    // Authoritative store steps to the corner vertex (a run endpoint).
    a.setTarget("t1", { x: 300, y: 0, rotation: 0 });
    a.tick(0);
    // Still on the smooth walk near `mid`, NOT jumped to the corner.
    expect(a.get("t1")!.x).toBeCloseTo(mid, 0);
  });

  it("a foreign authoritative position interrupts the path walk and retargets", () => {
    const a = fresh();
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
    a.animateAlongPath("t1", [[0, 0], [300, 0]], 0);
    a.tick(100);
    a.setTarget("t1", { x: -600, y: 0, rotation: 0 }); // off-path (another mover)
    a.tick(10_000); // settle
    expect(a.get("t1")).toEqual({ x: -600, y: 0, rotation: 0 });
  });

  it("interrupting a tween retargets from the CURRENT position (no stacking)", () => {
    const a = fresh();
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
    a.setTarget("t1", { x: 1200, y: 0, rotation: 0 }); // 2000ms
    a.tick(500); // ~quarter → x≈300
    const here = a.get("t1")!.x;
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 }); // reverse from `here`
    a.tick(10_000);
    expect(a.get("t1")).toEqual({ x: 0, y: 0, rotation: 0 });
    expect(here).toBeGreaterThan(0);
  });

  it("zero-distance / degenerate config snaps", () => {
    const a = new TokenAnimator();
    a.setConfig({ speedCellsPerSec: 0, easing: "linear", cellSize: 100 });
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
    a.setTarget("t1", { x: 500, y: 0, rotation: 0 });
    expect(a.get("t1")!.x).toBe(500); // speed 0 → snap, never freeze
  });

  it("remove drops all state", () => {
    const a = fresh();
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
    a.remove("t1");
    expect(a.has("t1")).toBe(false);
    expect(a.get("t1")).toBeUndefined();
  });
});
