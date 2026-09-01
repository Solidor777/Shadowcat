import { describe, it, expect, test } from "vitest";
import { DocumentStore, type ReadableDocuments } from "./store";
import type { WireDocument } from "./wire";
import { buildActorDoc, buildSceneDoc, buildTokenDoc, buildTokenFromActor, buildConditionRegistryDoc, type ActorEngine, type TokenEngine } from "./scene-docs";
import { resolveTokenActor, effectiveOwner, ownerFloorApplies, actorDisplayName, resolveConditions, conditionTarget, resolveTokenBox, resolveTokenVisual, selectedFaceNamesFor } from "./actor";
import { EMPTY_FOOTPRINTS, type FootprintLookup } from "./footprints";
import type { TokenVisual } from "./scene-docs";

const NAME = "Goblin";
const eng: ActorEngine = {
  displayName: "Unknown",
  visual: { kind: "image", asset: "a1" },
  size: { w: 1, h: 1 },
  shape: "square",
  faction: null,
  conditions: [],
  prototype: true,
  vision: null,
  aura: null,
  sound: null,
  vfx: null,
};

/** A raw (actorless) token's full engine body — every generated key is required
 * (though nullable), so every fixture below spells out the shape in full. */
function rawTokenEngine(over: Partial<TokenEngine> = {}): TokenEngine {
  return { x: 0, y: 0, w: 100, h: 100, rotation: 0, visual: null, actor_id: null, overrides: null, face: null, ...over };
}

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}

describe("resolveTokenActor", () => {
  it("resolves a linked token from the shared actor", () => {
    const actor = buildActorDoc("w1", NAME, eng, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 });
    const eff = resolveTokenActor(token, storeWith(actor));
    expect(eff?.name).toBe("Goblin");
    expect(eff?.visual?.kind === "image" && eff.visual.asset).toBe("a1");
  });

  it("applies the per-token override whitelist over the linked actor", () => {
    const actor = buildActorDoc("w1", NAME, eng, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 });
    (token.engine as TokenEngine).overrides = { name: "Boss", visual: { kind: "image", asset: "a2" }, size: null, shape: null, vision: null, aura: null, sound: null, vfx: null };
    const eff = resolveTokenActor(token, storeWith(actor));
    expect(eff?.name).toBe("Boss");
    expect(eff?.visual?.kind === "image" && eff.visual.asset).toBe("a2");
  });

  it("resolves an instanced token from its embedded copy (store-independent)", () => {
    const actor = buildActorDoc("w1", NAME, eng, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "instance", { x: 0, y: 0 }, { w: 100, h: 100 });
    const eff = resolveTokenActor(token, new DocumentStore()); // empty store
    expect(eff?.name).toBe("Goblin");
  });

  it("returns null for a linked token whose actor is missing, and for a raw token", () => {
    const actor = buildActorDoc("w1", NAME, eng, "act1");
    const linked = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 });
    expect(resolveTokenActor(linked, new DocumentStore())).toBeNull();
    const raw = buildTokenDoc("w1", "scene1", rawTokenEngine({ visual: { kind: "image", asset: "z" } }));
    expect(resolveTokenActor(raw, new DocumentStore())).toBeNull();
  });

  it("projects emissions with override-replaces-base precedence, like visionModes", () => {
    const base = {
      aura: { color: "#ffcc66", opacity: 0.4, radius: 2, enabled: true },
      sound: { asset: "a1", radius: 5, volume: 0.8, loop: true, enabled: true },
      vfx: { asset: "a2", anchor: "above" as const, loop: false, enabled: true },
    };
    const actor = buildActorDoc("w1", NAME, { ...eng, ...base }, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 });
    // Absent override: the actor base falls straight through.
    const inherited = resolveTokenActor(token, storeWith(actor));
    expect(inherited?.aura).toEqual(base.aura);
    expect(inherited?.sound).toEqual(base.sound);
    expect(inherited?.vfx).toEqual(base.vfx);
    // Present override: replaces the base wholesale (never merged).
    const over = { aura: { color: "#0000ff", opacity: 1, radius: 4, enabled: false }, sound: null, vfx: null };
    (token.engine as TokenEngine).overrides = { name: null, visual: null, size: null, shape: null, vision: null, ...over };
    const replaced = resolveTokenActor(token, storeWith(actor));
    expect(replaced?.aura).toEqual(over.aura);
    expect(replaced?.sound).toEqual(base.sound); // null override field = inherit, same as vision
    expect(replaced?.vfx).toEqual(base.vfx);
  });

  it("projects no emissions for a raw (actorless) token", () => {
    const raw = buildTokenDoc("w1", "scene1", rawTokenEngine({ visual: { kind: "image", asset: "z" } }));
    const eff = resolveTokenActor(raw, new DocumentStore());
    expect(eff).toBeNull(); // a raw token has no EffectiveActor at all — hence no emissions
  });
});

describe("resolveConditions", () => {
  it("resolves effective condition ids through the world registry, dropping unknown ids", () => {
    const actor = buildActorDoc("w1", NAME, { ...eng, conditions: ["dead", "ghost"] }, "act1");
    const registry = buildConditionRegistryDoc("w1", { dead: { name: "Dead", icon: "💀" } }, "creg1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 });
    expect(resolveConditions(token, storeWith(actor, registry))).toEqual([{ id: "dead", name: "Dead", icon: "💀" }]);
  });

  it("is fail-closed when the actor's engine conditions is absent (redacted or hand-built doc)", () => {
    const actor = buildActorDoc("w1", NAME, eng, "act1");
    delete (actor.engine as { conditions?: string[] }).conditions; // simulate a stripped field
    const registry = buildConditionRegistryDoc("w1", { dead: { name: "Dead", icon: "💀" } }, "creg1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 });
    expect(() => resolveConditions(token, storeWith(actor, registry))).not.toThrow();
    expect(resolveConditions(token, storeWith(actor, registry))).toEqual([]);
  });

  it("returns no conditions for a raw token or an empty registry", () => {
    const actor = buildActorDoc("w1", NAME, { ...eng, conditions: ["dead"] }, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 });
    expect(resolveConditions(token, storeWith(actor))).toEqual([]); // no registry → all dropped
    const raw = buildTokenDoc("w1", "scene1", rawTokenEngine({ visual: { kind: "image", asset: "z" } }));
    expect(resolveConditions(raw, new DocumentStore())).toEqual([]);
  });
});

describe("conditionTarget", () => {
  it("targets the shared actor doc + /engine/conditions for a linked token", () => {
    const actor = buildActorDoc("w1", NAME, { ...eng, conditions: ["dead"] }, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 });
    const tgt = conditionTarget(token, storeWith(actor))!;
    expect(tgt.doc.id).toBe("act1");
    expect(tgt.path).toBe("/engine/conditions");
    expect(tgt.conditions).toEqual(["dead"]);
  });

  it("targets the token doc + embedded copy path for an instanced token", () => {
    const actor = buildActorDoc("w1", NAME, { ...eng, conditions: ["dead"] }, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "instance", { x: 0, y: 0 }, { w: 100, h: 100 });
    const tgt = conditionTarget(token, new DocumentStore())!; // store-independent (embedded)
    expect(tgt.doc.id).toBe(token.id);
    expect(tgt.path).toBe("/embedded/actor/0/engine/conditions");
    expect(tgt.conditions).toEqual(["dead"]);
  });

  it("returns null for a raw token and a dangling linked token", () => {
    const actor = buildActorDoc("w1", NAME, eng, "act1");
    const linked = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 });
    expect(conditionTarget(linked, new DocumentStore())).toBeNull(); // actor missing
    const raw = buildTokenDoc("w1", "scene1", rawTokenEngine({ visual: { kind: "image", asset: "z" } }));
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

const actorEngine = (over: Partial<ActorEngine> = {}): ActorEngine => ({
  displayName: "Goblin", visual: { kind: "image" as const, asset: "a1" },
  size: { w: 1, h: 1 }, shape: "square" as const, faction: null, conditions: [], prototype: false, vision: null, aura: null, sound: null, vfx: null, ...over,
});

/** A lookup stating `extent` for `tokenId` and nothing else, standing in for a `"footprints"`
 * frame the server has broadcast. `parseFootprints` is exercised against a real payload by its
 * own tests; these cases are about what `resolveTokenBox` does with what it is told. */
function footprintsFor(tokenId: string, extent: { w: number; h: number } | null): FootprintLookup {
  return { token: (id) => (id === tokenId ? extent : null), unit: () => null };
}

test("resolveTokenBox takes its box from the server's resolved extent, not from actor.size and the grid", () => {
  // The store carries every input a size formula would need — a 2x3 actor and a 100-unit square
  // grid, which such a formula would turn into 200x300 — while the wire states 173.2x200. A box
  // computed on this side would report the store's numbers; this one reports the wire's.
  const scene = buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: null } }, "scene1");
  const actor = buildActorDoc("w1", "Goblin", actorEngine({ size: { w: 2, h: 3 } }), "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 50, y: 60 }, { w: 100, h: 100 }, "tok1");
  const box = resolveTokenBox(token, fakeStore([scene, actor, token]), footprintsFor("tok1", { w: 173.2, h: 200 }));
  expect(box).toEqual({ x: 50, y: 60, w: 173.2, h: 200, shape: "square" });
});

test("resolveTokenBox reads shape from the actor and applies a per-token override", () => {
  const scene = buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: null } }, "scene1");
  const actor = buildActorDoc("w1", "Goblin", actorEngine({ shape: "circle" }), "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  const fp = footprintsFor("tok1", { w: 400, h: 400 });
  expect(resolveTokenBox(token, fakeStore([scene, actor, token]), fp).shape).toBe("circle");
  (token.engine as TokenEngine).overrides = { shape: "square", size: { w: 4, h: 4 }, name: null, visual: null, vision: null, aura: null, sound: null, vfx: null };
  const box = resolveTokenBox(token, fakeStore([scene, actor, token]), fp);
  expect(box.shape).toBe("square");
  expect(box.w).toBe(400);
  expect(box.h).toBe(400);
});

test("resolveTokenBox falls back to token.engine w/h + square for a raw (actorless) token", () => {
  const token = buildTokenDoc("w1", "scene1", rawTokenEngine({ x: 10, y: 20, w: 64, h: 64, visual: { kind: "image", asset: "a1" } }), "tok1");
  expect(resolveTokenBox(token, fakeStore([token]), EMPTY_FOOTPRINTS)).toEqual({ x: 10, y: 20, w: 64, h: 64, shape: "square" });
});

test("resolveTokenBox falls back to the token's own authored extent while the server has stated none", () => {
  // An optimistic token the server has not yet resolved, and a token whose size the server
  // REFUSES, reach this function identically: the lookup answers null and the authored extent
  // the placement path stamped stands.
  const actor = buildActorDoc("w1", "Goblin", actorEngine({ size: { w: 2, h: 2 } }), "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
  const box = resolveTokenBox(token, fakeStore([actor, token]), EMPTY_FOOTPRINTS);
  expect(box.w).toBe(100);
  expect(box.h).toBe(100);
});

test("resolveTokenBox on hex draws the hex's own bounding box, because that is what the wire states", () => {
  // A 1-hex token's own hex spans √3·size wide and 2·size tall. Nothing on this side derives
  // those numbers; the server does, from the same definition its movement gate collides with.
  const scene = buildSceneDoc("w1", { grid: { kind: "hex", size: 100, distance: null } }, "scene1");
  const actor = buildActorDoc("w1", "Goblin", actorEngine({ size: { w: 1, h: 1 } }), "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100 * Math.sqrt(3), h: 200 }, "tok1");
  const box = resolveTokenBox(token, fakeStore([scene, actor, token]), footprintsFor("tok1", { w: 100 * Math.sqrt(3), h: 200 }));
  expect(box.w).toBeCloseTo(100 * Math.sqrt(3), 6);
  expect(box.h).toBe(200);
});

it("resolves actor vision modes onto the effective actor", () => {
  const withVision = { ...eng, vision: [{ mode: "darkvision", range: 12 }] };
  const actor = buildActorDoc("w1", NAME, withVision, "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 });
  const eff = resolveTokenActor(token, storeWith(actor));
  expect(eff?.visionModes).toEqual([{ mode: "darkvision", range: 12 }]);
});

it("defaults visionModes to [] when actor has none", () => {
  const actor = buildActorDoc("w1", NAME, eng, "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 });
  expect(resolveTokenActor(token, storeWith(actor))?.visionModes).toEqual([]);
});

it("per-token override replaces actor vision modes", () => {
  const withVision = { ...eng, vision: [{ mode: "darkvision", range: 12 }] };
  const actor = buildActorDoc("w1", NAME, withVision, "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 });
  (token.engine as TokenEngine).overrides = { vision: [{ mode: "darkvision", range: 6 }], name: null, visual: null, size: null, shape: null, aura: null, sound: null, vfx: null };
  expect(resolveTokenActor(token, storeWith(actor))?.visionModes).toEqual([{ mode: "darkvision", range: 6 }]);
});

describe("resolveTokenVisual", () => {
  function actorWith(visual: TokenVisual, extra: Partial<{ conditions: string[] }> = {}) {
    return buildActorDoc("w1", NAME, { ...eng, visual, conditions: extra.conditions ?? [] }, "act1");
  }

  function setFace(token: WireDocument, face: string): void {
    (token.engine as TokenEngine).face = face;
  }

  it("passes an image visual through unchanged", () => {
    const actor = actorWith({ kind: "image", asset: "a1" });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toEqual({ kind: "image", asset: "a1" });
  });

  it("passes an animated visual through unchanged", () => {
    const animated: TokenVisual = { kind: "animated", source: { type: "frames", frames: ["a1", "a2"] }, fps: 8, loop: true };
    const actor = actorWith(animated);
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toEqual(animated);
  });

  it("resolves faces to the manual token.engine.face over the default", () => {
    const actor = actorWith({
      kind: "faces",
      faces: { normal: { kind: "image", asset: "n1" }, bloodied: { kind: "image", asset: "b1" } },
      default: "normal",
      faceMap: null,
    });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    setFace(token, "bloodied");
    expect(resolveTokenVisual(token, storeWith(actor))).toEqual({ kind: "image", asset: "b1" });
  });

  it("resolves an animated face — proves faces are not restricted to images", () => {
    const bloodied: TokenVisual = { kind: "animated", source: { type: "frames", frames: ["b1"] }, fps: 4, loop: true };
    const actor = actorWith({
      kind: "faces",
      faces: { normal: { kind: "image", asset: "n1" }, bloodied },
      default: "normal",
      faceMap: null,
    });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    setFace(token, "bloodied");
    expect(resolveTokenVisual(token, storeWith(actor))).toEqual(bloodied);
  });

  it("falls back to a faceMap match when no manual face is set", () => {
    const actor = actorWith(
      {
        kind: "faces",
        faces: { normal: { kind: "image", asset: "n1" }, bleeding: { kind: "image", asset: "bl1" } },
        default: "normal",
        faceMap: { poisoned: "bleeding" },
      },
      { conditions: ["poisoned"] },
    );
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toEqual({ kind: "image", asset: "bl1" });
  });

  it("falls back to default when neither manual face nor faceMap matches", () => {
    const actor = actorWith({
      kind: "faces",
      faces: { normal: { kind: "image", asset: "n1" }, bloodied: { kind: "image", asset: "b1" } },
      default: "normal",
      faceMap: null,
    });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toEqual({ kind: "image", asset: "n1" });
  });

  it("fails closed to the first face key when default itself is invalid", () => {
    const actor = actorWith({ kind: "faces", faces: { onlyOne: { kind: "image", asset: "o1" } }, default: "missing", faceMap: null });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toEqual({ kind: "image", asset: "o1" });
  });

  it("fails closed to null when the faces map is empty", () => {
    const actor = actorWith({ kind: "faces", faces: {}, default: "x", faceMap: null });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toBeNull();
  });

  it("fails closed on a malformed AnimatedSource (non-positive rows/cols)", () => {
    const actor = actorWith({ kind: "animated", source: { type: "sheet", asset: "s1", rows: 0, cols: 4, count: null }, fps: 8, loop: true });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toBeNull();
  });

  it("fails closed on a malformed AnimatedSource (empty frame list)", () => {
    const actor = actorWith({ kind: "animated", source: { type: "frames", frames: [] }, fps: 8, loop: true });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toBeNull();
  });

  it("accepts a generated visual with image art", () => {
    const generated: TokenVisual = { kind: "generated", art: { kind: "image", asset: "p1" }, crop: "circle", border: { color: "#ff8800", width: 0.06 }, background: { color: "#102030" } };
    const actor = actorWith(generated);
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toEqual(generated);
  });

  it("accepts a generated visual with animated art", () => {
    const generated: TokenVisual = { kind: "generated", art: { kind: "animated", source: { type: "frames", frames: ["f1"] }, fps: 4, loop: true }, crop: "square", border: null, background: null };
    const actor = actorWith(generated);
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toEqual(generated);
  });

  it("accepts a generated face selected out of a faces visual", () => {
    const framed = { kind: "generated", art: { kind: "image", asset: "p1" }, crop: "circle", border: null, background: null } as const;
    const actor = actorWith({ kind: "faces", faces: { normal: { kind: "image", asset: "n1" }, framed }, default: "framed", faceMap: null });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toEqual(framed);
  });

  it("fails closed on a nested generated art", () => {
    const nested: TokenVisual = { kind: "generated", art: { kind: "generated", art: { kind: "image", asset: "p1" }, crop: "circle", border: null, background: null }, crop: "circle", border: null, background: null };
    const actor = actorWith(nested);
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toBeNull();
  });

  it("fails closed on a generated visual with missing art (garbled wire data)", () => {
    const noArt = { kind: "generated", crop: "circle", border: null, background: null } as unknown as TokenVisual;
    const actor = actorWith(noArt);
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toBeNull();
  });

  it("fails closed on a generated visual whose art has a malformed AnimatedSource", () => {
    const badArt: TokenVisual = { kind: "generated", art: { kind: "animated", source: { type: "frames", frames: [] }, fps: 8, loop: true }, crop: "circle", border: null, background: null };
    const actor = actorWith(badArt);
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toBeNull();
  });

  it("fails closed on a generated visual with an unknown crop (garbled wire data)", () => {
    const badCrop = { kind: "generated", art: { kind: "image", asset: "p1" }, crop: "hex", border: null, background: null } as unknown as TokenVisual;
    const actor = actorWith(badCrop);
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toBeNull();
  });

  it("fails closed on a generated visual with a non-positive border width", () => {
    const badBorder: TokenVisual = { kind: "generated", art: { kind: "image", asset: "p1" }, crop: "circle", border: { color: "#ff8800", width: 0 }, background: null };
    const actor = actorWith(badBorder);
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toBeNull();
  });

  it("fails closed on a malformed nested faces value (defense in depth against garbled wire data)", () => {
    const nested = { kind: "faces", faces: {}, default: "x" } as unknown as { kind: "image"; asset: string };
    const actor = actorWith({ kind: "faces", faces: { bad: nested }, default: "bad", faceMap: null });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toBeNull();
  });

  it("resolveTokenActor and resolveTokenVisual agree when a token has both a faces-union visual override AND an active face-swap", () => {
    const actor = actorWith({ kind: "image", asset: "base-asset" });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    (token.engine as TokenEngine).overrides = {
      visual: {
        kind: "faces",
        faces: { smile: { kind: "image", asset: "smile-asset" }, frown: { kind: "image", asset: "frown-asset" } },
        default: "smile",
        faceMap: null,
      },
      name: null,
      size: null,
      shape: null,
      vision: null,
      aura: null,
      sound: null,
      vfx: null,
    };
    (token.engine as TokenEngine).face = "frown"; // active manual face-swap

    const store = storeWith(actor);
    const eff = resolveTokenActor(token, store);
    expect(eff?.visual).toEqual({
      kind: "faces",
      faces: { smile: { kind: "image", asset: "smile-asset" }, frown: { kind: "image", asset: "frown-asset" } },
      default: "smile",
      faceMap: null,
    });

    const renderVisual = resolveTokenVisual(token, store);
    expect(renderVisual).toEqual({ kind: "image", asset: "frown-asset" }); // the active face-swap wins over the union's own default

    // The face-swap palette's own face-name list must read the SAME projected override, not a
    // second independent resolution.
    const selected = selectedFaceNamesFor(token, store);
    expect(selected).toContain("frown");
  });
});

describe("effectiveOwner", () => {
  // Obviously-synthetic ids: no real user data.
  const P1 = "usr_test_a";
  const P2 = "usr_test_b";

  const ownedActor = (owner: string | null): WireDocument => {
    const a = buildActorDoc("w1", NAME, eng, "act1");
    a.owner = owner;
    return a;
  };

  it("inherits the LINKED actor's owner when the token carries no override", () => {
    const actor = ownedActor(P1);
    const token = buildTokenFromActor("w1", "s1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    expect(effectiveOwner(token, storeWith(actor))).toBe(P1);
    expect(ownerFloorApplies(token, P1, storeWith(actor))).toBe(true);
    // Non-vacuity: same call, different user.
    expect(ownerFloorApplies(token, P2, storeWith(actor))).toBe(false);
  });

  it("prefers the per-token override over the linked actor's owner", () => {
    const actor = ownedActor(P1);
    const token = buildTokenFromActor("w1", "s1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    token.owner = P2;
    expect(effectiveOwner(token, storeWith(actor))).toBe(P2);
  });

  it("fails closed on every degenerate link", () => {
    const actor = ownedActor(P1);
    // Dangling: the linked actor is not in the store.
    const dangling = buildTokenFromActor("w1", "s1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    expect(effectiveOwner(dangling, new DocumentStore())).toBeNull();
    // Linked to an actor nobody owns.
    const unowned = ownedActor(null);
    const linkedUnowned = buildTokenFromActor("w1", "s1", unowned, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok2");
    expect(effectiveOwner(linkedUnowned, storeWith(unowned))).toBeNull();
    // Raw token: no link, no override.
    const raw = buildTokenDoc("w1", "s1", rawTokenEngine(), "tok3");
    expect(effectiveOwner(raw, new DocumentStore())).toBeNull();
    // Control: the same shape with a resolvable owned actor DOES resolve, so the
    // rejections above are the guards, not a constant null.
    expect(effectiveOwner(dangling, storeWith(actor))).toBe(P1);
  });

  it("does NOT inherit from an INSTANCED token's frozen embedded copy", () => {
    // An instanced copy is a placement-time snapshot; inheriting from it would be
    // the stamped semantics this rule exists to avoid. Its own `owner` override is
    // the only source, so an un-overridden instanced token has no owner.
    const actor = ownedActor(P1);
    const instanced = buildTokenFromActor("w1", "s1", actor, "instance", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    expect(instanced.embedded?.actor?.[0]).toBeTruthy();
    expect(effectiveOwner(instanced, storeWith(actor))).toBeNull();
    instanced.owner = P2;
    expect(effectiveOwner(instanced, storeWith(actor))).toBe(P2);
  });

  it("the owner capability floor is token-scoped", () => {
    const actor = ownedActor(P1);
    // `owner` still resolves on a non-token (it is the document's own field)...
    expect(effectiveOwner(actor, storeWith(actor))).toBe(P1);
    // ...but confers no capability floor there, mirroring the server.
    expect(ownerFloorApplies(actor, P1, storeWith(actor))).toBe(false);
  });

  it("rejects a resolved actor whose scope differs from the token's (fails closed)", () => {
    const actor = ownedActor(P1);
    const token = buildTokenFromActor("w1", "s1", actor, "link", { x: 0, y: 0 }, { w: 100, h: 100 }, "tok1");
    // Same id as `actor`, but a DIFFERENT world scope — simulates a resolver bug or a
    // future relaxation of the "world-stream-only" invariant this check is defense-in-depth
    // against, not something reachable via today's normal WS flow.
    const crossScopeActor: WireDocument = { ...actor, scope: { kind: "world", world_id: "w2" } };
    expect(effectiveOwner(token, storeWith(crossScopeActor))).toBeNull();

    // Control: the same shape with a matching scope DOES resolve, so the rejection
    // above is the guard, not a constant null.
    const sameScopeActor: WireDocument = { ...actor, scope: { kind: "world", world_id: "w1" } };
    expect(effectiveOwner(token, storeWith(sameScopeActor))).toBe(P1);
  });
});
