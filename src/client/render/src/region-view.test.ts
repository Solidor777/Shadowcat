import { test, expect } from "vitest";
import { DocumentStore, type WireDocument, type WireOperation } from "@shadowcat/core";
import { MockBackend, RegionView } from "./index";

function regionDoc(id: string, shape: unknown, behavior: string): WireDocument {
  return {
    id, scope: { kind: "world", world_id: "w1" }, doc_type: "region", schema_version: 1,
    source: null, owner: null,
    permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    embedded: {}, parent_id: "s1",
    system: { shape, behavior, cost: 1, enabled: true },
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
