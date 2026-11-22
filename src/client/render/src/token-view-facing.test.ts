import { describe, it, expect } from "vitest";
import { DocumentStore, AssetResolver, buildSceneDoc, buildTokenDoc, buildWorldSettingsDoc, type TokenEngine, type WireDocument } from "@shadowcat/core";
import { MockBackend, TokenView } from "./index";

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create" as const, doc })) });
  return s;
}

/** A scene with the facing-vision switch set per `facingVision` (`null` = inherit). */
function scene(facingVision: boolean | null): WireDocument {
  return buildSceneDoc("w1", { vision: { losRestriction: null, fog: null, observerVision: null, movementRestriction: null, movementModel: null, fogPooling: null, facingVision } }, "scene1");
}

function token(facing: { arc: number; near: number } | null): WireDocument {
  const engine: TokenEngine = {
    x: 0, y: 0, w: 100, h: 100, rotation: 90, visual: null, actor_id: null, overrides: null, face: null,
    elevation: null, lock_rotation: false, identity_hidden: false, ...(facing ? { facing_vision: facing } : {}),
  };
  return buildTokenDoc("w1", "scene1", engine, "tok1");
}

function specFor(docs: WireDocument[], visible: boolean) {
  const backend = new MockBackend();
  const view = new TokenView(storeWith(...docs), new AssetResolver(), backend, () => "scene1", () => null, undefined, undefined, undefined, undefined, undefined, undefined, () => visible);
  view.setWorldUnitsPerCell(100);
  view.reconcile();
  return backend.tokens.get("tok1")!;
}

describe("TokenView facing wedge", () => {
  it("draws the wedge in scene units when the switch is on and the viewer may see it", () => {
    const spec = specFor([scene(true), token({ arc: 90, near: 2 })], true);
    expect(spec.facing).toEqual({ arc: 90, near: 200, radius: 300 });
  });

  it("draws nothing for a viewer the host does not entitle to the facing", () => {
    expect(specFor([scene(true), token({ arc: 90, near: 2 })], false).facing).toBeUndefined();
  });

  it("draws nothing while the scene's resolved switch is off, and a scene override beats a world default", () => {
    expect(specFor([scene(false), token({ arc: 90, near: 2 })], true).facing).toBeUndefined();
    const world = buildWorldSettingsDoc("w1", { scene: { facingVision: true } } as never, "ws1");
    expect(specFor([world, scene(null), token({ arc: 90, near: 2 })], true).facing).toBeDefined();
    expect(specFor([world, scene(false), token({ arc: 90, near: 2 })], true).facing).toBeUndefined();
  });

  it("draws nothing for a token with no facing vision or a full-circle arc", () => {
    expect(specFor([scene(true), token(null)], true).facing).toBeUndefined();
    expect(specFor([scene(true), token({ arc: 360, near: 0 })], true).facing).toBeUndefined();
  });
});
