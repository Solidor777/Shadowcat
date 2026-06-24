import { describe, it, expect, test } from "vitest";
import { DocumentStore, type ReadableDocuments } from "./store";
import type { WireDocument } from "./wire";
import { buildActorDoc, buildSceneDoc, buildTokenDoc, buildTokenFromActor, buildConditionRegistryDoc, type ActorSystem, type TokenOverrides } from "./scene-docs";
import { resolveTokenActor, actorDisplayName, resolveConditions, conditionTarget, resolveTokenBox, footprintRadius } from "./actor";

const sys: ActorSystem = {
  name: "Goblin",
  displayName: "Unknown",
  visual: { kind: "image", asset: "a1" },
  size: { w: 1, h: 1 },
  shape: "square",
  faction: null,
  conditions: [],
  prototype: true,
};

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}

describe("resolveTokenActor", () => {
  it("resolves a linked token from the shared actor", () => {
    const actor = buildActorDoc("w1", sys, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
    const eff = resolveTokenActor(token, storeWith(actor));
    expect(eff?.name).toBe("Goblin");
    expect(eff?.visual.asset).toBe("a1");
  });

  it("applies the per-token override whitelist over the linked actor", () => {
    const actor = buildActorDoc("w1", sys, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
    (token.system as { overrides?: TokenOverrides }).overrides = { name: "Boss", visual: { kind: "image", asset: "a2" } };
    const eff = resolveTokenActor(token, storeWith(actor));
    expect(eff?.name).toBe("Boss");
    expect(eff?.visual.asset).toBe("a2");
  });

  it("resolves an instanced token from its embedded copy (store-independent)", () => {
    const actor = buildActorDoc("w1", sys, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "instance", { x: 0, y: 0 }, 100);
    const eff = resolveTokenActor(token, new DocumentStore()); // empty store
    expect(eff?.name).toBe("Goblin");
  });

  it("returns null for a linked token whose actor is missing, and for a raw token", () => {
    const actor = buildActorDoc("w1", sys, "act1");
    const linked = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
    expect(resolveTokenActor(linked, new DocumentStore())).toBeNull();
    const raw = buildTokenDoc("w1", "scene1", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "z" } });
    expect(resolveTokenActor(raw, new DocumentStore())).toBeNull();
  });
});

describe("resolveConditions", () => {
  it("resolves effective condition ids through the world registry, dropping unknown ids", () => {
    const actor = buildActorDoc("w1", { ...sys, conditions: ["dead", "ghost"] }, "act1");
    const registry = buildConditionRegistryDoc("w1", { dead: { name: "Dead", icon: "💀" } }, "creg1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
    expect(resolveConditions(token, storeWith(actor, registry))).toEqual([{ id: "dead", name: "Dead", icon: "💀" }]);
  });

  it("is fail-closed when /system/conditions is absent (redacted or hand-built doc)", () => {
    const actor = buildActorDoc("w1", sys, "act1");
    delete (actor.system as { conditions?: string[] }).conditions; // simulate a stripped field
    const registry = buildConditionRegistryDoc("w1", { dead: { name: "Dead", icon: "💀" } }, "creg1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
    expect(() => resolveConditions(token, storeWith(actor, registry))).not.toThrow();
    expect(resolveConditions(token, storeWith(actor, registry))).toEqual([]);
  });

  it("returns no conditions for a raw token or an empty registry", () => {
    const actor = buildActorDoc("w1", { ...sys, conditions: ["dead"] }, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
    expect(resolveConditions(token, storeWith(actor))).toEqual([]); // no registry → all dropped
    const raw = buildTokenDoc("w1", "scene1", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "z" } });
    expect(resolveConditions(raw, new DocumentStore())).toEqual([]);
  });
});

describe("conditionTarget", () => {
  it("targets the shared actor doc + /system/conditions for a linked token", () => {
    const actor = buildActorDoc("w1", { ...sys, conditions: ["dead"] }, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
    const tgt = conditionTarget(token, storeWith(actor))!;
    expect(tgt.doc.id).toBe("act1");
    expect(tgt.path).toBe("/system/conditions");
    expect(tgt.conditions).toEqual(["dead"]);
  });

  it("targets the token doc + embedded copy path for an instanced token", () => {
    const actor = buildActorDoc("w1", { ...sys, conditions: ["dead"] }, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "instance", { x: 0, y: 0 }, 100);
    const tgt = conditionTarget(token, new DocumentStore())!; // store-independent (embedded)
    expect(tgt.doc.id).toBe(token.id);
    expect(tgt.path).toBe("/embedded/actor/0/system/conditions");
    expect(tgt.conditions).toEqual(["dead"]);
  });

  it("returns null for a raw token and a dangling linked token", () => {
    const actor = buildActorDoc("w1", sys, "act1");
    const linked = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
    expect(conditionTarget(linked, new DocumentStore())).toBeNull(); // actor missing
    const raw = buildTokenDoc("w1", "scene1", { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "z" } });
    expect(conditionTarget(raw, new DocumentStore())).toBeNull();
  });
});

describe("actorDisplayName", () => {
  it("prefers the real name, then displayName, then a generic fallback", () => {
    expect(actorDisplayName({ name: "Goblin Skirmisher", displayName: "Goblin" })).toBe("Goblin Skirmisher");
    expect(actorDisplayName({ displayName: "Goblin" })).toBe("Goblin");
    expect(actorDisplayName({})).toBe("Unknown Creature");
    expect(actorDisplayName({}, "Mystery")).toBe("Mystery");
  });
});

// Minimal read-only store over a fixed doc set.
function fakeStore(docs: WireDocument[]): ReadableDocuments {
  return {
    get: (id) => docs.find((d) => d.id === id),
    query: (type) => docs.filter((d) => d.doc_type === type),
    subscribe: () => () => {},
    appliedSeq: 0,
  } as ReadableDocuments;
}

const actorSys = (over: Partial<import("./scene-docs").ActorSystem> = {}) => ({
  name: "Goblin", displayName: "Goblin", visual: { kind: "image" as const, asset: "a1" },
  size: { w: 1, h: 1 }, shape: "square" as const, faction: null, conditions: [], prototype: false, ...over,
});

test("resolveTokenBox derives multi-cell pixel size from actor.size × scene grid cell", () => {
  const scene = buildSceneDoc("w1", { grid: { kind: "square", size: 100 } }, "scene1");
  const actor = buildActorDoc("w1", actorSys({ size: { w: 2, h: 3 } }), "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 50, y: 60 }, 100, "tok1");
  const box = resolveTokenBox(token, fakeStore([scene, actor, token]));
  expect(box).toEqual({ x: 50, y: 60, w: 200, h: 300, shape: "square" });
});

test("resolveTokenBox reads shape from the actor and applies a per-token override", () => {
  const scene = buildSceneDoc("w1", { grid: { kind: "square", size: 100 } }, "scene1");
  const actor = buildActorDoc("w1", actorSys({ shape: "circle" }), "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
  expect(resolveTokenBox(token, fakeStore([scene, actor, token])).shape).toBe("circle");
  (token.system as { overrides: import("./scene-docs").TokenOverrides }).overrides = { shape: "square", size: { w: 4, h: 4 } };
  const box = resolveTokenBox(token, fakeStore([scene, actor, token]));
  expect(box.shape).toBe("square");
  expect(box.w).toBe(400);
  expect(box.h).toBe(400);
});

test("resolveTokenBox falls back to token.system w/h + square for a raw (actorless) token", () => {
  const token = buildTokenDoc("w1", "scene1", { x: 10, y: 20, w: 64, h: 64, rotation: 0, visual: { kind: "image", asset: "a1" } }, "tok1");
  expect(resolveTokenBox(token, fakeStore([token]))).toEqual({ x: 10, y: 20, w: 64, h: 64, shape: "square" });
});

test("resolveTokenBox defaults the grid cell to 100 when the parent scene is absent", () => {
  const actor = buildActorDoc("w1", actorSys({ size: { w: 1, h: 1 } }), "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
  expect(resolveTokenBox(token, fakeStore([actor, token])).w).toBe(100);
});

test("footprintRadius: circle = max(w,h)/2, square = half-diagonal", () => {
  expect(footprintRadius({ shape: "circle", size: { w: 2, h: 4 } })).toBe(2);
  expect(footprintRadius({ shape: "square", size: { w: 2, h: 2 } })).toBeCloseTo(Math.SQRT2, 5);
});
