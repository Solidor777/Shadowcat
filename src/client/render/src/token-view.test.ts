import { test, expect } from "vitest";
import { DocumentStore, AssetResolver } from "@shadowcat/core";
import { MockBackend, TokenView } from "./index";
import type { WireDocument, WireOperation } from "@shadowcat/core";

function tokenDoc(id: string, x: number, y: number, asset: string): WireDocument {
  return {
    id, scope: { kind: "world", world_id: "w1" }, doc_type: "token", schema_version: 1,
    source: null, owner: null,
    permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
    embedded: {}, parent_id: null, system: { x, y, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset } },
    created_at: 0, updated_at: 0,
  };
}
const cmd = (seq: number, ops: WireOperation[]) => ({ seq, world_id: "w1", author: "a", ts: 0, ops });

test("reconcile creates a token node at its center transform with the resolved url", () => {
  const store = new DocumentStore();
  const assets = new AssetResolver();
  const backend = new MockBackend();
  store.applyCommand(cmd(1, [{ op: "create", doc: tokenDoc("t1", 100, 50, "img1") }]));
  new TokenView(store, assets, backend).reconcile();
  expect(backend.tokens.get("t1")).toEqual({ x: 100, y: 50, w: 100, h: 100, rotation: 0, url: assets.url("img1") });
});

test("a moved token tweens via tick toward the new position", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const view = new TokenView(store, new AssetResolver(), backend);
  store.applyCommand(cmd(1, [{ op: "create", doc: tokenDoc("t1", 0, 0, "img1") }]));
  view.reconcile();
  store.applyCommand(cmd(2, [{ op: "update", doc_id: "t1", changes: [{ path: "/system/x", old: 0, new: 100 }] }]));
  view.reconcile(); // sets the new target; current still ~0 (existing token, not snapped)
  expect(backend.tokens.get("t1")!.x).toBeLessThan(100);
  view.tick(10_000); // settle
  expect(backend.tokens.get("t1")!.x).toBe(100);
});

test("a deleted token doc removes its node", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const view = new TokenView(store, new AssetResolver(), backend);
  store.applyCommand(cmd(1, [{ op: "create", doc: tokenDoc("t1", 0, 0, "img1") }]));
  view.reconcile();
  store.applyCommand(cmd(2, [{ op: "delete", doc: tokenDoc("t1", 0, 0, "img1") }]));
  view.reconcile();
  expect(backend.tokens.has("t1")).toBe(false);
});
