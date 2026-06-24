import { describe, it, expect } from "vitest";
import { DocumentStore } from "./store";
import type { WireDocument } from "./wire";
import { buildActorDoc, buildTokenDoc, buildTokenFromActor, type ActorSystem, type TokenOverrides } from "./scene-docs";
import { resolveTokenActor } from "./actor";

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
