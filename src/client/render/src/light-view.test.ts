import { test, expect } from "vitest";
import { DocumentStore, buildLightDoc, type WireDocument, type WireOperation } from "@shadowcat/core";
import { MockBackend, LightView } from "./index";

function lightDoc(id: string, x: number, y: number, over: Record<string, unknown> = {}): WireDocument {
  const doc = buildLightDoc("w1", "s1", {
    x,
    y,
    elevation: null,
    emission: { color: "#ffcc66", intensity: 1, brightRadius: 2, dimRadius: 6, falloff: null, enabled: true, ...over },
  }, id);
  return doc;
}
const cmd = (seq: number, ops: WireOperation[]) => ({ seq, world_id: "w1", author: "a", ts: 0, ops });

test("a light reconciles to a filled marker in the walls layer", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  store.applyCommand(cmd(1, [{ op: "create", doc: lightDoc("l1", 50, 50) }]));
  new LightView(store, backend).reconcile();
  const s = backend.shapes.get("l1")!;
  expect(s.layer).toBe("walls");
  expect(s.closed).toBe(true);
  expect(s.fill).toMatchObject({ color: 0xffcc66, alpha: 0.9 });
  expect(s.points.length).toBeGreaterThanOrEqual(6); // a tessellated disc, not a point
});

test("a disabled light renders dimmed; a malformed position does not render", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  store.applyCommand(cmd(1, [{ op: "create", doc: lightDoc("l1", 50, 50, { enabled: false }) }]));
  store.applyCommand(cmd(2, [{ op: "create", doc: lightDoc("l2", Number.NaN, 50) }]));
  const view = new LightView(store, backend);
  view.reconcile();
  expect(backend.shapes.get("l1")?.fill?.alpha).toBe(0.25);
  expect(backend.shapes.has("l2")).toBe(false);
});

test("a deleted light removes its marker", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const view = new LightView(store, backend);
  const doc = lightDoc("l1", 50, 50);
  store.applyCommand(cmd(1, [{ op: "create", doc }]));
  view.reconcile();
  store.applyCommand(cmd(2, [{ op: "delete", doc }]));
  view.reconcile();
  expect(backend.shapes.has("l1")).toBe(false);
});
