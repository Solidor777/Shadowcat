import { test, expect } from "vitest";
import { DocumentStore, type WireDocument, type WireOperation } from "@shadowcat/core";
import { MockBackend, DrawingView } from "./index";

function drawingDoc(id: string, kind: string, points: number[]): WireDocument {
  return {
    id, scope: { kind: "world", world_id: "w1" }, doc_type: "drawing", schema_version: 1,
    name: null, source: null, owner: null,
    permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    embedded: {}, parent_id: "s1",
    engine: { shape: { kind, points }, stroke: { color: "#ff0000", width: 2 }, fill: null },
    system: {},
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

// The four *-view.ts siblings are near-identical in shape, and two of them (region, wall)
// guarded non-finite coordinates while drawing and template did not. JSON has no NaN/Infinity
// literal, but an oversized magnitude parses to Infinity — `JSON.parse('{"x":1e400}').x` is
// Infinity — which reaches Pixi as NaN geometry. This pins all four to the same behavior so the
// divergence cannot silently return.
test("a drawing with a non-finite coordinate does not render", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  store.applyCommand(cmd(1, [{ op: "create", doc: drawingDoc("d-inf", "freehand", [0, 0, Infinity, 5]) }]));
  new DrawingView(store, backend).reconcile();
  expect(backend.shapes.has("d-inf")).toBe(false);
});

test("a drawing whose tessellated output goes non-finite does not render", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  // Finite-looking bbox corners whose rect tessellation carries the non-finite through.
  store.applyCommand(cmd(1, [{ op: "create", doc: drawingDoc("d-rect", "rect", [0, 0, Number.NaN, 10]) }]));
  new DrawingView(store, backend).reconcile();
  expect(backend.shapes.has("d-rect")).toBe(false);
});
