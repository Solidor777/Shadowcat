import { test, expect } from "vitest";
import { DocumentStore, type WireDocument, type WireOperation } from "@shadowcat/core";
import { MockBackend, TemplateView } from "./index";

function tmplDoc(id: string, shape: Record<string, unknown>): WireDocument {
  return {
    id, scope: { kind: "world", world_id: "w1" }, doc_type: "template", schema_version: 1,
    source: null, owner: null,
    permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
    embedded: {}, parent_id: "s1",
    system: { shape, color: "#3388ff" },
    created_at: 0, updated_at: 0,
  };
}
const cmd = (seq: number, ops: WireOperation[]) => ({ seq, world_id: "w1", author: "a", ts: 0, ops });
const dist = (x: number, y: number, cx = 0, cy = 0): number => Math.hypot(x - cx, y - cy);

test("a circle template renders a closed translucent disc", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  store.applyCommand(cmd(1, [{ op: "create", doc: tmplDoc("t1", { kind: "circle", x: 0, y: 0, size: 10, direction: 0 }) }]));
  new TemplateView(store, backend).reconcile();
  const s = backend.shapes.get("t1")!;
  expect(s.layer).toBe("templates");
  expect(s.closed).toBe(true);
  expect(s.fill).toEqual({ color: 0x3388ff, alpha: 0.25 });
  expect(s.stroke).toEqual({ color: 0x3388ff, width: 2 });
  for (let i = 0; i < s.points.length; i += 2) expect(dist(s.points[i], s.points[i + 1])).toBeCloseTo(10);
});

test("a line template is an open 2-point segment along its direction", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  store.applyCommand(cmd(1, [{ op: "create", doc: tmplDoc("t1", { kind: "line", x: 0, y: 0, size: 10, direction: 0 }) }]));
  new TemplateView(store, backend).reconcile();
  const s = backend.shapes.get("t1")!;
  expect(s.closed).toBe(false);
  expect(s.points[0]).toBeCloseTo(0);
  expect(s.points[2]).toBeCloseTo(10); // +x for direction 0
  expect(s.points[3]).toBeCloseTo(0);
});

test("a cone template is an apex + two base corners", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  store.applyCommand(cmd(1, [{ op: "create", doc: tmplDoc("t1", { kind: "cone", x: 0, y: 0, size: 10, direction: 0 }) }]));
  new TemplateView(store, backend).reconcile();
  const s = backend.shapes.get("t1")!;
  expect(s.points.slice(0, 2)).toEqual([0, 0]); // apex
  expect(s.points).toHaveLength(6);
  expect(s.closed).toBe(true);
});

test("a deleted template removes its node", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const view = new TemplateView(store, backend);
  const doc = tmplDoc("t1", { kind: "circle", x: 0, y: 0, size: 5, direction: 0 });
  store.applyCommand(cmd(1, [{ op: "create", doc }]));
  view.reconcile();
  store.applyCommand(cmd(2, [{ op: "delete", doc }]));
  view.reconcile();
  expect(backend.shapes.has("t1")).toBe(false);
});
