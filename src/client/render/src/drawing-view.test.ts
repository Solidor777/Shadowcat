import { test, expect } from "vitest";
import { DocumentStore, type WireDocument, type WireOperation } from "@shadowcat/core";
import { MockBackend, DrawingView } from "./index";

function drawingDoc(id: string, kind: string, points: number[]): WireDocument {
  return {
    id, scope: { kind: "world", world_id: "w1" }, doc_type: "drawing", schema_version: 1,
    source: null, owner: null,
    permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
    embedded: {}, parent_id: "s1",
    system: { shape: { kind, points }, stroke: { color: "#ff0000", width: 2 }, fill: null },
    created_at: 0, updated_at: 0,
  };
}
const cmd = (seq: number, ops: WireOperation[]) => ({ seq, world_id: "w1", author: "a", ts: 0, ops });

test("a freehand drawing reconciles to an open polyline with parsed stroke", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  store.applyCommand(cmd(1, [{ op: "create", doc: drawingDoc("d1", "freehand", [0, 0, 5, 5, 10, 0]) }]));
  new DrawingView(store, backend).reconcile();
  const s = backend.shapes.get("d1")!;
  expect(s.layer).toBe("drawings");
  expect(s.points).toEqual([0, 0, 5, 5, 10, 0]);
  expect(s.closed).toBe(false);
  expect(s.stroke).toEqual({ color: 0xff0000, width: 2 });
});

test("a rect drawing tessellates its bbox corners and closes the path", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  store.applyCommand(cmd(1, [{ op: "create", doc: drawingDoc("d1", "rect", [0, 0, 10, 20]) }]));
  new DrawingView(store, backend).reconcile();
  const s = backend.shapes.get("d1")!;
  expect(s.points).toEqual([0, 0, 10, 0, 10, 20, 0, 20]);
  expect(s.closed).toBe(true);
});

test("a deleted drawing removes its shape node", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const view = new DrawingView(store, backend);
  store.applyCommand(cmd(1, [{ op: "create", doc: drawingDoc("d1", "freehand", [0, 0, 1, 1]) }]));
  view.reconcile();
  store.applyCommand(cmd(2, [{ op: "delete", doc: drawingDoc("d1", "freehand", [0, 0, 1, 1]) }]));
  view.reconcile();
  expect(backend.shapes.has("d1")).toBe(false);
});
