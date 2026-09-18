import { test, expect } from "vitest";
import { DocumentStore, buildSceneDoc, type WireDocument, type WireOperation } from "@shadowcat/core";
import { MockBackend, WallView } from "./index";

function wallDoc(
  id: string,
  seg: { x1: number; y1: number; x2: number; y2: number },
  elevation: { bottom: number | null; top: number | null } | null = null,
): WireDocument {
  return {
    id, scope: { kind: "world", world_id: "w1" }, doc_type: "wall", schema_version: 1,
    name: null, source: null, owner: null,
    permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    embedded: {}, parent_id: "s1",
    engine: { seg, blocksSight: true, blocksMove: true, blocksLight: true, elevation },
    system: {},
    created_at: 0, updated_at: 0,
  };
}
const cmd = (seq: number, ops: WireOperation[]) => ({ seq, world_id: "w1", author: "a", ts: 0, ops });

test("a wall reconciles to a stroked segment in the walls layer", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  store.applyCommand(cmd(1, [{ op: "create", doc: wallDoc("wl1", { x1: 0, y1: 0, x2: 100, y2: 50 }) }]));
  new WallView(store, backend).reconcile();
  const s = backend.shapes.get("wl1")!;
  expect(s.layer).toBe("walls");
  expect(s.points).toEqual([0, 0, 100, 50]);
  expect(s.closed).toBe(false);
  expect(s.stroke).not.toBeNull();
});

test("a deleted wall removes its segment", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const view = new WallView(store, backend);
  const doc = wallDoc("wl1", { x1: 0, y1: 0, x2: 10, y2: 0 });
  store.applyCommand(cmd(1, [{ op: "create", doc }]));
  view.reconcile();
  store.applyCommand(cmd(2, [{ op: "delete", doc }]));
  view.reconcile();
  expect(backend.shapes.has("wl1")).toBe(false);
});

test("a viewedLevel function scopes reconcile() to that level's band", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const scene = buildSceneDoc(
    "w1",
    { levels: [{ id: "l1", name: "Ground", bottom: 0, top: 10, background: null }, { id: "l2", name: "Upper", bottom: 10, top: 20, background: null }] },
    "s1",
  );
  const ground = wallDoc("wl-ground", { x1: 0, y1: 0, x2: 10, y2: 0 }, { bottom: 0, top: 9 });
  const upper = wallDoc("wl-upper", { x1: 0, y1: 0, x2: 10, y2: 0 }, { bottom: 10, top: 20 });
  store.applyCommand(cmd(1, [{ op: "create", doc: scene }, { op: "create", doc: ground }, { op: "create", doc: upper }]));
  const view = new WallView(store, backend, () => "s1", () => "l1");
  view.reconcile();
  expect(backend.shapes.has("wl-ground")).toBe(true);
  expect(backend.shapes.has("wl-upper")).toBe(false);
});
