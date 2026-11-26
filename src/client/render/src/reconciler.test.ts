import { test, expect } from "vitest";
import { DocumentStore, AssetResolver } from "@shadowcat/core";
import { MockBackend, SceneReconciler } from "./index";
import type { WireDocument } from "@shadowcat/core";

function sceneDoc(background: string | null): WireDocument {
  return {
    id: "scene-1",
    scope: { kind: "world", world_id: "w1" },
    doc_type: "scene",
    schema_version: 1,
    name: null,
    source: null,
    owner: null,
    permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} }, gm_role: null },
    embedded: {},
    parent_id: null,
    engine: {
      grid: { kind: "square", size: 100, distance: null },
      background,
      bounds: null, snapToGrid: null, vision: null, lighting: null,
    },
    system: {},
    created_at: 0,
    updated_at: 0,
  };
}

test("reconcile resolves the scene background UUID to a URL", () => {
  const store = new DocumentStore();
  store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: sceneDoc("asset-uuid") }] });
  const assets = new AssetResolver();
  const backend = new MockBackend();
  new SceneReconciler(store, assets, backend).reconcile();
  expect(backend.background).toEqual({ url: assets.url("asset-uuid") });
});

test("reconcile clears the background when the scene has none", () => {
  const store = new DocumentStore();
  store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: sceneDoc(null) }] });
  const backend = new MockBackend();
  new SceneReconciler(store, new AssetResolver(), backend).reconcile();
  expect(backend.background).toBeNull();
});

test("reconcile with no scene doc clears the background", () => {
  const backend = new MockBackend();
  new SceneReconciler(new DocumentStore(), new AssetResolver(), backend).reconcile();
  expect(backend.background).toBeNull();
});

test("reconcile with no host resolver draws the OLDEST scene's background, whatever the store order", () => {
  const store = new DocumentStore();
  // The store holds the newer scene first, so a `query("scene")[0]` fallback would pick it.
  const newer = { ...sceneDoc("newer-bg"), id: "a-newer", created_at: 200 };
  const older = { ...sceneDoc("older-bg"), id: "z-older", created_at: 100 };
  store.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: newer }, { op: "create", doc: older }] });
  const assets = new AssetResolver();
  const backend = new MockBackend();
  new SceneReconciler(store, assets, backend).reconcile();
  expect(backend.background).toEqual({ url: assets.url("older-bg") });
});
