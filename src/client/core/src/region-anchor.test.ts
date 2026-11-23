import { describe, expect, test } from "vitest";
import {
  anchoredRegionGeometry,
  buildRegionDoc,
  buildTokenDoc,
  carrierPoseFor,
  offsetRegionGeometry,
  reanchorRegion,
  resolveRegionGeometry,
  type RegionEngine,
  type RegionGeometry,
  type WireDocument,
} from "./scene-docs";
import { DocumentStore } from "./store";

/** A raw token document at `(x, y)` rotated `rotation` degrees on `scene`. */
function token(id: string, scene: string, x: number, y: number, rotation = 0): WireDocument {
  return buildTokenDoc(
    "w1",
    scene,
    { x, y, w: 100, h: 100, rotation, visual: { kind: "image", asset: "a" }, actor_id: null, overrides: null, face: null, elevation: null, lock_rotation: false, identity_hidden: false },
    id,
  );
}

/** A region document on `scene` with `shape`, anchored to `carrier` when given. */
function region(scene: string, shape: RegionGeometry, carrier?: string, followRotation = false): WireDocument {
  const engine: RegionEngine = {
    shape,
    behavior: "terrain",
    cost: 2,
    cost_by_tag: {},
    enabled: true,
    triggers: [],
    elevation: null,
    ambient_override: null,
    occludes: null,
    anchor: carrier ? { token: carrier, include_carrier: false, follow_rotation: followRotation } : null,
  };
  return buildRegionDoc("w1", scene, engine, "r1");
}

/** A store holding `docs`. */
function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}

/** Asserts two flat point lists agree within float noise. */
function expectClose(actual: number[], expected: number[]): void {
  expect(actual).toHaveLength(expected.length);
  actual.forEach((v, i) => expect(v).toBeCloseTo(expected[i], 9));
}

describe("anchoredRegionGeometry (client twin of the server's anchored_parts)", () => {
  const at = { x: 300, y: 200, rotation: 0 };

  test("translates every shape kind and keeps a rect a rect without rotation", () => {
    expect(anchoredRegionGeometry({ kind: "rect", points: [-10, -20, 10, 20] }, at, false)).toEqual({ kind: "rect", points: [290, 180, 310, 220] });
    expect(anchoredRegionGeometry({ kind: "circle", points: [0, 0, 5] }, at, false)).toEqual({ kind: "circle", points: [300, 200, 5] });
    expect(anchoredRegionGeometry({ kind: "polygon", points: [0, 0, 10, 0, 0, 10] }, at, false)).toEqual({ kind: "polygon", points: [300, 200, 310, 200, 300, 210] });
  });

  test("keeps the hole flag of every compound part", () => {
    const out = anchoredRegionGeometry(
      [
        { shape: { kind: "rect", points: [0, 0, 10, 10] }, hole: false },
        { shape: { kind: "circle", points: [0, 0, 2] }, hole: true },
      ],
      at,
      false,
    );
    expect(out).toEqual([
      { shape: { kind: "rect", points: [300, 200, 310, 210] }, hole: false },
      { shape: { kind: "circle", points: [300, 200, 2] }, hole: true },
    ]);
  });

  test("rotates clockwise on the y-down canvas about the carrier centre; a rect becomes a polygon", () => {
    // A 90 degree clockwise turn on a y-down canvas sends the +x axis to +y (down), so the
    // offset (10, 0) lands directly below the carrier. Fails if the rotation direction flips.
    const out = anchoredRegionGeometry({ kind: "rect", points: [0, 0, 10, 10] }, { x: 0, y: 0, rotation: 90 }, true);
    expect(out).toMatchObject({ kind: "polygon" });
    // Corners (0,0) (10,0) (10,10) (0,10) -> (0,0) (0,10) (-10,10) (-10,0).
    expectClose((out as { points: number[] }).points, [0, 0, 0, 10, -10, 10, -10, 0]);
    const circle = anchoredRegionGeometry({ kind: "circle", points: [10, 0, 5] }, { x: 100, y: 100, rotation: 90 }, true) as { kind: string; points: number[] };
    expect(circle.kind).toBe("circle");
    expectClose(circle.points, [100, 110, 5]);
  });

  test("matches the server's anchored_parts vertex for vertex on its rotation fixture", () => {
    // The same input and expected vertices as the server's rotation test; a drift in either
    // twin's direction, corner order or rect-to-polygon rule fails one of the two.
    const out = anchoredRegionGeometry({ kind: "rect", points: [0, -10, 100, 10] }, { x: 500, y: 500, rotation: 90 }, true) as { points: number[] };
    expectClose(out.points, [510, 500, 510, 600, 490, 600, 490, 500]);
  });

  test("a malformed shape passes through unchanged so the outline rule still rejects it", () => {
    const bad = { kind: "rect" as const, points: [0, 0, 10] };
    expect(anchoredRegionGeometry(bad, at, false)).toEqual(bad);
  });

  test("a non-finite pose yields non-finite coordinates (the region draws nothing)", () => {
    const out = anchoredRegionGeometry({ kind: "circle", points: [0, 0, 5] }, { x: Number.NaN, y: 0, rotation: 0 }, false) as { points: number[] };
    expect(Number.isFinite(out.points[0])).toBe(false);
  });

  test("offsetRegionGeometry inverts it, with and without rotation", () => {
    const stored: RegionGeometry = { kind: "polygon", points: [5, 5, 40, 5, 20, 30] };
    for (const [pose, follow] of [
      [{ x: 300, y: 200, rotation: 0 }, false],
      [{ x: 300, y: 200, rotation: 37 }, true],
    ] as const) {
      const absolute = anchoredRegionGeometry(stored, pose, follow);
      expectClose((offsetRegionGeometry(absolute, pose, follow) as { points: number[] }).points, [5, 5, 40, 5, 20, 30]);
    }
  });
});

describe("carrierPoseFor / resolveRegionGeometry (the dormant rule)", () => {
  test("a static region resolves to its stored shape", () => {
    const r = region("s1", { kind: "rect", points: [0, 0, 10, 10] });
    expect(resolveRegionGeometry(r, storeWith(r))).toEqual({ kind: "rect", points: [0, 0, 10, 10] });
  });

  test("an anchored region follows the carrier's committed position", () => {
    const t = token("t1", "s1", 300, 200);
    const r = region("s1", { kind: "circle", points: [0, 0, 50] }, "t1");
    expect(resolveRegionGeometry(r, storeWith(t, r))).toEqual({ kind: "circle", points: [300, 200, 50] });
  });

  test("a live pose replaces the committed one", () => {
    const t = token("t1", "s1", 300, 200);
    const r = region("s1", { kind: "circle", points: [0, 0, 50] }, "t1");
    const out = resolveRegionGeometry(r, storeWith(t, r), () => ({ x: 310, y: 205, rotation: 0 }));
    expect(out).toEqual({ kind: "circle", points: [310, 205, 50] });
  });

  test("dormant when the carrier is missing, is not a token, or is on another scene", () => {
    const r = region("s1", { kind: "circle", points: [0, 0, 50] }, "t1");
    expect(resolveRegionGeometry(r, storeWith(r))).toBeNull();
    const notToken = { ...token("t1", "s1", 0, 0), doc_type: "light" };
    expect(resolveRegionGeometry(r, storeWith(notToken, r))).toBeNull();
    const elsewhere = token("t1", "s2", 0, 0);
    expect(resolveRegionGeometry(r, storeWith(elsewhere, r))).toBeNull();
    expect(carrierPoseFor(r, elsewhere)).toBeNull();
    expect(carrierPoseFor(r, undefined)).toBeNull();
  });
});

describe("reanchorRegion (attach / detach without moving the region)", () => {
  const next = { token: "t1", include_carrier: false, follow_rotation: false };

  test("attaching converts the absolute shape to offsets of the carrier's centre", () => {
    const t = token("t1", "s1", 300, 200);
    const r = region("s1", { kind: "circle", points: [320, 210, 50] });
    const w = reanchorRegion(r, storeWith(t, r), next)!;
    expect(w.shape).toEqual({ kind: "circle", points: [20, 10, 50] });
    expect(w.anchor).toEqual(next);
  });

  test("attach then detach returns the original absolute shape", () => {
    const t = token("t1", "s1", 300, 200, 45);
    const original: RegionGeometry = { kind: "polygon", points: [320, 210, 400, 210, 360, 280] };
    const plain = region("s1", original);
    const attach = reanchorRegion(plain, storeWith(t, plain), { ...next, follow_rotation: true })!;
    const attached = region("s1", attach.shape, "t1", true);
    const detach = reanchorRegion(attached, storeWith(t, attached), null)!;
    expect(detach.anchor).toBeNull();
    expectClose((detach.shape as { points: number[] }).points, [320, 210, 400, 210, 360, 280]);
  });

  test("moving to another carrier keeps the region where it is", () => {
    const a = token("t1", "s1", 300, 200);
    const b = token("t2", "s1", 500, 500);
    const r = region("s1", { kind: "circle", points: [0, 0, 50] }, "t1");
    const w = reanchorRegion(r, storeWith(a, b, r), { ...next, token: "t2" })!;
    // The circle sits at (300, 200); relative to t2 at (500, 500) that is (-200, -300).
    expect(w.shape).toEqual({ kind: "circle", points: [-200, -300, 50] });
  });

  test("refuses a carrier on another scene, and detaching a dormant region", () => {
    const elsewhere = token("t1", "s2", 0, 0);
    const r = region("s1", { kind: "circle", points: [0, 0, 50] });
    expect(reanchorRegion(r, storeWith(elsewhere, r), next)).toBeNull();
    const dormant = region("s1", { kind: "circle", points: [0, 0, 50] }, "gone");
    expect(reanchorRegion(dormant, storeWith(dormant), null)).toBeNull();
  });

  test("a dormant region pointed at a resolving carrier keeps its stored offsets", () => {
    const t = token("t2", "s1", 500, 500);
    const dormant = region("s1", { kind: "circle", points: [20, 10, 50] }, "gone");
    const w = reanchorRegion(dormant, storeWith(t, dormant), { ...next, token: "t2" })!;
    expect(w.shape).toEqual({ kind: "circle", points: [20, 10, 50] });
  });
});
