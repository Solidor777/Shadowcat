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
