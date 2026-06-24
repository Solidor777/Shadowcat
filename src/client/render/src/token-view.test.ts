import { test, expect } from "vitest";
import { DocumentStore, AssetResolver, buildActorDoc, buildTokenFromActor, buildFactionRegistryDoc } from "@shadowcat/core";
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

test("a dragging token snaps to its new position; a non-dragging one tweens", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const view = new TokenView(store, new AssetResolver(), backend);
  store.applyCommand(cmd(1, [{ op: "create", doc: tokenDoc("t1", 0, 0, "img1") }]));
  view.reconcile();
  // Mark dragging: the local dragger must follow the pointer with no tween lag.
  view.setDragging("t1");
  store.applyCommand(cmd(2, [{ op: "update", doc_id: "t1", changes: [{ path: "/system/x", old: 0, new: 100 }] }]));
  view.reconcile();
  expect(backend.tokens.get("t1")!.x).toBe(100); // snapped immediately
  // Clear dragging: a subsequent move tweens (current lags behind target).
  view.setDragging(null);
  store.applyCommand(cmd(3, [{ op: "update", doc_id: "t1", changes: [{ path: "/system/x", old: 100, new: 200 }] }]));
  view.reconcile();
  expect(backend.tokens.get("t1")!.x).toBeLessThan(200);
});

test("reconcile creates a token node at its center transform with the resolved url", () => {
  const store = new DocumentStore();
  const assets = new AssetResolver();
  const backend = new MockBackend();
  store.applyCommand(cmd(1, [{ op: "create", doc: tokenDoc("t1", 100, 50, "img1") }]));
  new TokenView(store, assets, backend).reconcile();
  expect(backend.tokens.get("t1")).toEqual({ x: 100, y: 50, w: 100, h: 100, rotation: 0, url: assets.url("img1"), borderColor: null });
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

test("renders a linked token using the actor's visual", () => {
  const store = new DocumentStore();
  const assets = new AssetResolver();
  const backend = new MockBackend();
  const actor = buildActorDoc(
    "w1",
    { name: "G", displayName: "G", visual: { kind: "image", asset: "actorimg" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false },
    "act1",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 10, y: 20 }, 100, "tok1");
  store.applyCommand(cmd(1, [{ op: "create", doc: actor }, { op: "create", doc: token }]));
  new TokenView(store, assets, backend).reconcile();
  expect(backend.tokens.get("tok1")!.url).toBe(assets.url("actorimg"));
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

test("resolves the faction border color from the registry", () => {
  const store = new DocumentStore();
  const assets = new AssetResolver();
  const backend = new MockBackend();
  const registry = buildFactionRegistryDoc("w1", { f1: { name: "F1", color: "#ff0000", stance: "hostile" } }, "reg1");
  const actor = buildActorDoc(
    "w1",
    { name: "G", displayName: "G", visual: { kind: "image", asset: "actorimg" }, size: { w: 1, h: 1 }, shape: "square", faction: "f1", conditions: [], prototype: false },
    "act1",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
  store.applyCommand(cmd(1, [{ op: "create", doc: registry }, { op: "create", doc: actor }, { op: "create", doc: token }]));
  new TokenView(store, assets, backend).reconcile();
  expect(backend.tokens.get("tok1")!.borderColor).toBe(0xff0000);
});

test("a token with no faction has a null border", () => {
  const store = new DocumentStore();
  const backend = new MockBackend();
  const actor = buildActorDoc(
    "w1",
    { name: "G", displayName: "G", visual: { kind: "image", asset: "actorimg" }, size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [], prototype: false },
    "act2",
  );
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok2");
  store.applyCommand(cmd(1, [{ op: "create", doc: actor }, { op: "create", doc: token }]));
  new TokenView(store, new AssetResolver(), backend).reconcile();
  expect(backend.tokens.get("tok2")!.borderColor).toBeNull();
});
