import { test, expect } from "vitest";
import { buildSceneDoc, buildTokenDoc, buildActorDoc, buildTokenFromActor, type TokenSystem, type ActorSystem } from "./scene-docs";

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

test("buildTokenFromActor link mode references the actor by id, no embedded copy", () => {
  const actor = buildActorDoc("w1", actorSys, "act1");
  const t = buildTokenFromActor("w1", "scene1", actor, "link", { x: 50, y: 50 }, 100);
  expect(t.doc_type).toBe("token");
  expect(t.parent_id).toBe("scene1");
  expect((t.system as { actor_id?: string }).actor_id).toBe("act1");
  expect((t.system as { overrides?: object }).overrides).toEqual({});
  expect(t.embedded.actor).toBeUndefined();
});

test("buildTokenFromActor instance mode embeds an independent copy with provenance", () => {
  const actor = buildActorDoc("w1", actorSys, "act1");
  const t = buildTokenFromActor("w1", "scene1", actor, "instance", { x: 0, y: 0 }, 100);
  expect((t.system as { actor_id?: string | null }).actor_id ?? null).toBeNull();
  expect(t.embedded.actor).toHaveLength(1);
  const copy = t.embedded.actor[0];
  expect(copy.id).not.toBe(actor.id);
  expect(copy.source).toEqual({ id: "act1", pack: null, version: 1 });
  expect(copy.system).toEqual(actorSys);
  expect(copy.system).not.toBe(actor.system); // independent by value, not aliased
});
