import { test, expect } from "vitest";
import { buildSceneDoc, buildTokenDoc, buildActorDoc, type TokenSystem, type ActorSystem } from "./scene-docs";

const actorSys: ActorSystem = {
  name: "Goblin",
  displayName: "Goblin",
  visual: { kind: "image", asset: "a1" },
  size: { w: 1, h: 1 },
  shape: "square",
  faction: null,
  conditions: [],
  prototype: true,
};

test("buildSceneDoc makes a top-level world scene with a default square grid", () => {
  const doc = buildSceneDoc("w1");
  expect(doc.doc_type).toBe("scene");
  expect(doc.parent_id).toBeNull();
  expect(doc.scope).toEqual({ kind: "world", world_id: "w1" });
  expect(doc.system).toEqual({ grid: { kind: "square", size: 100 }, background: null });
  expect(typeof doc.id).toBe("string");
  expect(doc.id.length).toBeGreaterThan(0);
  expect(typeof doc.created_at).toBe("number");
});

test("buildSceneDoc honors a partial system override and an explicit id", () => {
  const doc = buildSceneDoc("w1", { grid: { kind: "hex", size: 50 } }, "scene-fixed");
  expect(doc.id).toBe("scene-fixed");
  expect(doc.system).toEqual({ grid: { kind: "hex", size: 50 }, background: null });
});

test("buildTokenDoc parents to the scene and preserves the token system", () => {
  const sys: TokenSystem = { x: 140, y: 160, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "img-1" } };
  const doc = buildTokenDoc("w1", "scene-1", sys);
  expect(doc.doc_type).toBe("token");
  expect(doc.parent_id).toBe("scene-1");
  expect(doc.scope).toEqual({ kind: "world", world_id: "w1" });
  expect(doc.system).toEqual(sys);
  expect(doc.permissions.default).toBe("observer");
});

test("buildActorDoc makes a top-level, parentless actor document", () => {
  const doc = buildActorDoc("w1", actorSys, "act1");
  expect(doc.doc_type).toBe("actor");
  expect(doc.parent_id).toBeNull();
  expect(doc.scope).toEqual({ kind: "world", world_id: "w1" });
  expect(doc.system).toEqual(actorSys);
  expect(doc.id).toBe("act1");
});
