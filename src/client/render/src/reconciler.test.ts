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
    source: null,
    owner: null,
    permissions: { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } },
    embedded: {},
    parent_id: null,
    system: background === null ? {} : { background },
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
