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
    // Authoritative store steps to the corner vertex (a run endpoint) with a distinct rotation.
    // A naive always-retarget impl would jump to (300,0) and settle at (300,0); the walk must
    // continue to the final goal (300,300) — this discriminates the two behaviors.
    a.setTarget("t1", { x: 300, y: 0, rotation: 1.57 });
    a.tick(0);
    // Still on the smooth walk near `mid`, NOT jumped to the corner.
    expect(a.get("t1")!.x).toBeCloseTo(mid, 0);
    // Drive to completion: must settle at the route's FINAL goal, not the corner.
    a.tick(10_000);
    expect(a.get("t1")).toEqual({ x: 300, y: 300, rotation: 1.57 });
  });

  it("synchronous burst: all run-endpoints arrive at segIndex 0, walk still reaches final goal", () => {
    // Reproduces the real route-commit pattern: dispatchIntent fires V1 then goal synchronously
    // before any tick, so the animator receives setTarget(V1) then setTarget(goal) while segIndex
    // is still 0.  Both must be swallowed by the ignore-scan; the walk must complete to (300,300).
    const a = fresh();
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
    a.animateAlongPath("t1", [[0, 0], [300, 0], [300, 300]], 0);
    // No tick between these — both arrive at segIndex 0.
    a.setTarget("t1", { x: 300, y: 0, rotation: 0 }); // V1 (corner)
    a.setTarget("t1", { x: 300, y: 300, rotation: 0 }); // goal
    a.tick(10_000); // settle
    expect(a.get("t1")).toEqual({ x: 300, y: 300, rotation: 0 });
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

  it("NaN coordinate does not pin the token or produce moved forever", () => {
    // A NaN coordinate makes Math.hypot → NaN, total = NaN, `total < EPSILON` is false, so
    // without the !isFinite guard the anim would live forever re-reporting moved every tick.
    // With the guard: the degenerate branch fires, anim is deleted, tick returns [] afterwards.
    // Use a two-point path where the destination is NaN so the NaN reaches startAnim directly.
    const a = fresh();
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 }); // snap
    // setTarget with a NaN coordinate triggers startAnim([[cur],[NaN,NaN]]) directly.
    a.setTarget("t1", { x: NaN, y: NaN, rotation: 0 });
    // After the degenerate snap the anim map is cleared — no further moved reports.
    const movedAfter = a.tick(16);
    expect(movedAfter).toEqual([]);
    // The rendered position must be finite (not NaN) — cur must not have been overwritten with NaN.
    const pos = a.get("t1")!;
    expect(Number.isFinite(pos.x)).toBe(true);
    expect(Number.isFinite(pos.y)).toBe(true);
  });

  it("remove drops all state", () => {
    const a = fresh();
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
    a.remove("t1");
    expect(a.has("t1")).toBe(false);
    expect(a.get("t1")).toBeUndefined();
  });
});

describe("TokenAnimator.animateSamples", () => {
  it("interpolates position between adjacent samples by tMs", () => {
    const a = fresh();
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
    // serverNow returns startServerMs → initialElapsed = 0.
    a.animateSamples("t1", [{ tMs: 0, pos: [0, 0] }, { tMs: 500, pos: [300, 0] }], 1000, 1000, () => 1000);
    expect(a.get("t1")).toEqual({ x: 0, y: 0, rotation: 0 }); // at t=0
    expect(a.isHidden("t1")).toBe(false);
    a.tick(250); // 250 ms elapsed → 50% of [0→300] = 150
    expect(a.get("t1")!.x).toBeCloseTo(150, 0);
    expect(a.isHidden("t1")).toBe(false);
    a.tick(10_000); // settle at last sample
    expect(a.get("t1")).toEqual({ x: 300, y: 0, rotation: 0 });
    expect(a.isHidden("t1")).toBe(false);
  });

  it("hides the token across an occlusion gap (nominal-interval-based threshold)", () => {
    const a = fresh();
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
    // 3 samples: contiguous run 0→100 (delta=100), then occlusion gap 100→900 (delta=800).
    // minConsecutiveDelta = 100; gapThreshold = 150. Delta 800 > 150 → hidden inside the gap.
    // With only 2 samples (≤1 delta), gapThreshold = Infinity; 3 samples are required for
    // interior gap detection.
    a.animateSamples(
      "t1",
      [{ tMs: 0, pos: [0, 0] }, { tMs: 100, pos: [100, 0] }, { tMs: 900, pos: [900, 0] }],
      1000, 1000, () => 1000,
    );
    expect(a.isHidden("t1")).toBe(false); // visible at first sample (t=0)
    a.tick(1);   // t=1: inside segment 0→100 (gap=100 ≤ threshold=150) → visible
    expect(a.isHidden("t1")).toBe(false);
    a.tick(100); // t=101: inside gap 100→900 (gap=800 > threshold=150) → hidden
    expect(a.isHidden("t1")).toBe(true);
    a.tick(800); // t=901: past tMs=900 → visible at last sample
    expect(a.isHidden("t1")).toBe(false);
    expect(a.get("t1")!.x).toBeCloseTo(900, 0);
  });

  it("partial-occlusion: mid-path gap detected with nominal-interval threshold, contiguous runs stay visible", () => {
    // Spec: samples at tMs 0,100,200,600,700,800 (durationMs 800). Two contiguous runs
    // (0→100→200 and 600→700→800) with an occlusion gap (200→600, delta=400).
    // minConsecutiveDelta = 100 (from the contiguous runs); gapThreshold = 150.
    // The durationMs/2 = 400 heuristic would miss this gap entirely (400 = 400 is not >).
    const a = fresh();
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
    const samples = [
      { tMs:   0, pos: [  0, 0] as [number, number] },
      { tMs: 100, pos: [100, 0] as [number, number] },
      { tMs: 200, pos: [200, 0] as [number, number] },
      { tMs: 600, pos: [600, 0] as [number, number] },
      { tMs: 700, pos: [700, 0] as [number, number] },
      { tMs: 800, pos: [800, 0] as [number, number] },
    ];
    a.animateSamples("t1", samples, 800, 0, () => 0);

    // t=0: visible at first sample.
    expect(a.isHidden("t1")).toBe(false);
    expect(a.get("t1")!.x).toBeCloseTo(0, 0);

    // t=100: still visible, at second sample (segment 0→100, gap=100 ≤ 150).
    a.tick(100);
    expect(a.isHidden("t1")).toBe(false);
    expect(a.get("t1")!.x).toBeCloseTo(100, 0);

    // t=201: inside 200→600 gap (delta=400 > threshold=150) → hidden.
    a.tick(101);
    expect(a.isHidden("t1")).toBe(true);

    // t=601: inside 600→700 run (delta=100 ≤ threshold=150) → visible again.
    a.tick(400);
    expect(a.isHidden("t1")).toBe(false);
    expect(a.get("t1")!.x).toBeCloseTo(601, 0);

    // t=850: settle region past last sample → visible.
    a.tick(249);
    expect(a.isHidden("t1")).toBe(false);
  });

  it("hides the token (not extrapolated) when catch-up elapsed lands before the first sample (leading-occlusion clip)", () => {
    // This observer's clip removed the leading samples (the move started outside their vision),
    // so their earliest visible sample has tMs=200. A fresh-broadcast catch-up landing at
    // elapsed=50 (< samples[0].tMs) must NOT extrapolate backward past samples[0] — the token
    // must be hidden until elapsed reaches samples[0].tMs.
    const a = fresh();
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
    const samples = [
      { tMs: 200, pos: [200, 0] as [number, number] },
      { tMs: 400, pos: [400, 0] as [number, number] },
    ];
    // serverNow = startServerMs + 50 → initialElapsed = 50, well before samples[0].tMs = 200.
    a.animateSamples("t1", samples, 400, 1000, () => 1050);
    expect(a.isHidden("t1")).toBe(true);
    // Must not have been extrapolated backward: still hidden, position pinned at samples[0].
    expect(a.get("t1")!.x).toBeCloseTo(200, 0);
    // Ticking forward but still short of samples[0].tMs keeps it hidden.
    a.tick(100); // elapsed = 150
    expect(a.isHidden("t1")).toBe(true);
    // Reaching samples[0].tMs reveals the token, starting normal interpolation.
    a.tick(50); // elapsed = 200
    expect(a.isHidden("t1")).toBe(false);
    expect(a.get("t1")!.x).toBeCloseTo(200, 0);
  });

  it("setTarget before animateSamples: sample animation takes precedence over ease-to-stop", () => {
    // Reproduces the typical server ordering: the authoritative position Event (→ setTarget via
    // reconcile) arrives before the MoveStream broadcast (→ animateSamples). samplesAnim must
    // cancel the ease entry and drive the token along the sample trajectory (y=500 path), NOT
    // along the straight ease to stop (y=0 path).
    const a = fresh();
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 }); // snap to origin
    // Ease-to-stop registered by reconcile for the authoritative position Event (stop at x=1000, y=0).
    a.setTarget("t1", { x: 1000, y: 0, rotation: 0 });
    // MoveStream arrives: samples along y=500 (perpendicular to the ease path).
    a.animateSamples(
      "t1",
      [{ tMs: 0, pos: [0, 500] }, { tMs: 500, pos: [500, 500] }],
      500, 0, () => 0,
    );
    // Tick to mid-animation.
    a.tick(250);
    // Must match sample interpolation at elapsed=250: 50% of x[0→500]=250, y=500.
    // The ease path would place y=0; sample path places y=500.
    expect(a.get("t1")!.y).toBeCloseTo(500, 0);
    expect(a.get("t1")!.x).toBeCloseTo(250, 0);
  });

  it("catch-up: jumps to the server-aligned position when startServerMs is in the past", () => {
    const a = fresh();
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
    // serverNow = startServerMs + 500 → initialElapsed = 500 → starts at mid-sample
    const samples = [
      { tMs: 0, pos: [0, 0] as [number, number] },
      { tMs: 500, pos: [300, 0] as [number, number] },
      { tMs: 1000, pos: [600, 0] as [number, number] },
    ];
    a.animateSamples("t1", samples, 1000, 1000, () => 1500);
    // elapsed=500 → interpolating between tMs=500 and tMs=1000 at t=500 → should be at (300,0)
    expect(a.get("t1")!.x).toBeCloseTo(300, 0);
  });

  it("settles at last sample position and clears hidden after durationMs", () => {
    const a = fresh();
    a.setTarget("t1", { x: 0, y: 0, rotation: 0 });
    a.animateSamples("t1", [{ tMs: 0, pos: [10, 20] }, { tMs: 500, pos: [100, 200] }], 500, 0, () => 0);
    a.tick(10_000);
    expect(a.get("t1")).toEqual({ x: 100, y: 200, rotation: 0 });
    expect(a.isHidden("t1")).toBe(false);
    expect(a.tick(16)).toEqual([]); // settled — nothing moves
    // No permanent suppression: once samplesAnim settles and clears its entry, a later
    // setTarget retargets normally (the setTarget samplesAnim guard no longer fires).
    a.setTarget("t1", { x: 200, y: 400, rotation: 0 });
    a.tick(50);
    expect(a.get("t1")!.x).toBeGreaterThan(100); // tween toward the new target is running
  });
});
