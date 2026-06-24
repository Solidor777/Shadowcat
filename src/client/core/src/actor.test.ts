import { describe, it, expect } from "vitest";
import { DocumentStore } from "./store";
import type { WireDocument } from "./wire";
import { buildActorDoc, buildTokenDoc, buildTokenFromActor, buildConditionRegistryDoc, type ActorSystem, type TokenOverrides } from "./scene-docs";
import { resolveTokenActor, actorDisplayName, resolveConditions, conditionTarget } from "./actor";

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
