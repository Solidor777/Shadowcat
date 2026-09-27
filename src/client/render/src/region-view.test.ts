import { test, expect } from "vitest";
import { DocumentStore, buildSceneDoc, type WireDocument, type WireOperation } from "@shadowcat/core";
import { MockBackend, RegionView } from "./index";

function regionDoc(
  id: string,
  shape: unknown,
  behavior: string,
  elevation: { bottom: number | null; top: number | null } | null = null,
): WireDocument {
  return {
    id, scope: { kind: "world", world_id: "w1" }, doc_type: "region", schema_version: 1,
    name: null, source: null, owner: null,
    permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    embedded: {}, parent_id: "s1",
    engine: { shape, behavior, cost: 1, enabled: true, elevation },
    system: {},
    created_at: 0, updated_at: 0,
  };
}
const cmd = (seq: number, ops: WireOperation[]) => ({ seq, world_id: "w1", author: "a", ts: 0, ops });

test("a region reconciles to a tinted shape in the regions layer", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  store.applyCommand(cmd(1, [{ op: "create", doc: regionDoc("r1", { kind: "rect", points: [0, 0, 100, 100] }, "terrain") }]));
  new RegionView(store, backend).reconcile();
  const s = backend.shapes.get("r1")!;
  expect(s.layer).toBe("regions");
  expect(s.closed).toBe(true);
  expect(s.fill).not.toBeNull();
});

test("each region behavior carries its own fill tint", () => {
  // An unmapped behavior falls back to the terrain tint, so a behavior missing from the tint
  // map renders indistinguishably from terrain — this pins every behavior as distinct.
  const tint = (behavior: string): number => {
    const store = new DocumentStore();
    const backend = new MockBackend();
    store.applyCommand(cmd(1, [{ op: "create", doc: regionDoc("r1", { kind: "rect", points: [0, 0, 10, 10] }, behavior) }]));
    new RegionView(store, backend).reconcile();
    return backend.shapes.get("r1")!.fill!.color;
  };
  const tints = ["terrain", "impassable", "arrest", "ambient_light"].map(tint);
  expect(new Set(tints).size).toBe(tints.length);
});

test("a circular region tessellates to a closed polygon", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  store.applyCommand(cmd(1, [{ op: "create", doc: regionDoc("r1", { kind: "circle", points: [50, 50, 25] }, "impassable") }]));
  new RegionView(store, backend).reconcile();
  const s = backend.shapes.get("r1")!;
  expect(s.layer).toBe("regions");
  expect(s.points.length).toBeGreaterThan(6);
});

test("a deleted region removes its shape", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const view = new RegionView(store, backend);
  const doc = regionDoc("r1", { kind: "circle", points: [50, 50, 25] }, "impassable");
  store.applyCommand(cmd(1, [{ op: "create", doc }]));
  view.reconcile();
  expect(backend.shapes.has("r1")).toBe(true);
  store.applyCommand(cmd(2, [{ op: "delete", doc }]));
  view.reconcile();
  expect(backend.shapes.has("r1")).toBe(false);
});

test("skips a doc with malformed shape geometry rather than pushing NaN", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  store.applyCommand(cmd(1, [{ op: "create", doc: regionDoc("r1", { kind: "rect", points: [0, 0] }, "terrain") }]));
  new RegionView(store, backend).reconcile();
  expect(backend.shapes.has("r1")).toBe(false);
});

test("a compound region reconciles to one node: additive rings plus hole rings", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  // Additive bar [0,0]-[30,10] in two disjoint rects, with a hole punched out of the first.
  const compound = regionDoc(
    "r1",
    [
      { shape: { kind: "rect", points: [0, 0, 10, 10] }, hole: false },
      { shape: { kind: "rect", points: [20, 0, 30, 10] }, hole: false },
      { shape: { kind: "rect", points: [2, 2, 4, 4] }, hole: true },
    ],
    "impassable",
  );
  store.applyCommand(cmd(1, [{ op: "create", doc: compound }]));
  new RegionView(store, backend).reconcile();
  const s = backend.shapes.get("r1")!;
  expect(s.layer).toBe("regions");
  expect(s.closed).toBe(true);
  expect(s.points).toEqual([0, 0, 10, 0, 10, 10, 0, 10]);
  expect(s.extraParts).toEqual([[20, 0, 30, 0, 30, 10, 20, 10]]);
  expect(s.holes).toEqual([[2, 2, 4, 2, 4, 4, 2, 4]]);
});

test("a legacy singular-shape region carries no extraParts or holes", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  store.applyCommand(cmd(1, [{ op: "create", doc: regionDoc("r1", { kind: "rect", points: [0, 0, 10, 10] }, "terrain") }]));
  new RegionView(store, backend).reconcile();
  const s = backend.shapes.get("r1")!;
  expect(s.extraParts).toBeUndefined();
  expect(s.holes).toBeUndefined();
});

test("one malformed part drops the whole compound spec; a holes-only region renders nothing", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  store.applyCommand(cmd(1, [
    { op: "create", doc: regionDoc("bad", [
      { shape: { kind: "rect", points: [0, 0, 10, 10] }, hole: false },
      { shape: { kind: "hexagon", points: [] }, hole: true },
    ], "terrain") },
    { op: "create", doc: regionDoc("holes-only", [
      { shape: { kind: "rect", points: [0, 0, 10, 10] }, hole: true },
    ], "terrain") },
  ]));
  new RegionView(store, backend).reconcile();
  expect(backend.shapes.has("bad")).toBe(false);
  expect(backend.shapes.has("holes-only")).toBe(false);
});

test("a null part element in unvalidated engine data renders nothing instead of throwing", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  // The optimistic store can hold unvalidated engine data pre-rejection; a null array
  // element must fail the whole spec closed, never raise inside the reconciler.
  store.applyCommand(cmd(1, [
    { op: "create", doc: regionDoc("r1", [
      { shape: { kind: "rect", points: [0, 0, 10, 10] }, hole: false },
      null,
    ], "terrain") },
  ]));
  expect(() => new RegionView(store, backend).reconcile()).not.toThrow();
  expect(backend.shapes.has("r1")).toBe(false);
});

test("a viewedLevel function scopes reconcile() to that level's band", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const scene = buildSceneDoc(
    "w1",
    { levels: [{ id: "l1", name: "Ground", bottom: 0, top: 10, background: null }, { id: "l2", name: "Upper", bottom: 10, top: 20, background: null }] },
    "s1",
  );
  const ground = regionDoc("r-ground", { kind: "rect", points: [0, 0, 10, 10] }, "terrain", { bottom: 0, top: 9 });
  const upper = regionDoc("r-upper", { kind: "rect", points: [0, 0, 10, 10] }, "terrain", { bottom: 10, top: 20 });
  store.applyCommand(cmd(1, [{ op: "create", doc: scene }, { op: "create", doc: ground }, { op: "create", doc: upper }]));
  new RegionView(store, backend, () => "s1", () => "l1").reconcile();
  expect(backend.shapes.has("r-ground")).toBe(true);
  expect(backend.shapes.has("r-upper")).toBe(false);
});
