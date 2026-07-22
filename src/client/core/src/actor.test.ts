import { describe, it, expect, test } from "vitest";
import { DocumentStore, type ReadableDocuments } from "./store";
import type { WireDocument } from "./wire";
import { buildActorDoc, buildSceneDoc, buildTokenDoc, buildTokenFromActor, buildConditionRegistryDoc, type ActorEngine, type TokenEngine } from "./scene-docs";
import { resolveTokenActor, actorDisplayName, resolveConditions, conditionTarget, resolveTokenBox, footprintRadius, resolveTokenVisual, selectedFaceNamesFor } from "./actor";
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
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
    const eff = resolveTokenActor(token, storeWith(actor));
    expect(eff?.name).toBe("Goblin");
    expect(eff?.visual?.kind === "image" && eff.visual.asset).toBe("a1");
  });

  it("applies the per-token override whitelist over the linked actor", () => {
    const actor = buildActorDoc("w1", NAME, eng, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
    (token.engine as TokenEngine).overrides = { name: "Boss", visual: { kind: "image", asset: "a2" }, size: null, shape: null, vision: null };
    const eff = resolveTokenActor(token, storeWith(actor));
    expect(eff?.name).toBe("Boss");
    expect(eff?.visual?.kind === "image" && eff.visual.asset).toBe("a2");
  });

  it("resolves an instanced token from its embedded copy (store-independent)", () => {
    const actor = buildActorDoc("w1", NAME, eng, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "instance", { x: 0, y: 0 }, 100);
    const eff = resolveTokenActor(token, new DocumentStore()); // empty store
    expect(eff?.name).toBe("Goblin");
  });

  it("returns null for a linked token whose actor is missing, and for a raw token", () => {
    const actor = buildActorDoc("w1", NAME, eng, "act1");
    const linked = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
    expect(resolveTokenActor(linked, new DocumentStore())).toBeNull();
    const raw = buildTokenDoc("w1", "scene1", rawTokenEngine({ visual: { kind: "image", asset: "z" } }));
    expect(resolveTokenActor(raw, new DocumentStore())).toBeNull();
  });
});

describe("resolveConditions", () => {
  it("resolves effective condition ids through the world registry, dropping unknown ids", () => {
    const actor = buildActorDoc("w1", NAME, { ...eng, conditions: ["dead", "ghost"] }, "act1");
    const registry = buildConditionRegistryDoc("w1", { dead: { name: "Dead", icon: "💀" } }, "creg1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
    expect(resolveConditions(token, storeWith(actor, registry))).toEqual([{ id: "dead", name: "Dead", icon: "💀" }]);
  });

  it("is fail-closed when the actor's engine conditions is absent (redacted or hand-built doc)", () => {
    const actor = buildActorDoc("w1", NAME, eng, "act1");
    delete (actor.engine as { conditions?: string[] }).conditions; // simulate a stripped field
    const registry = buildConditionRegistryDoc("w1", { dead: { name: "Dead", icon: "💀" } }, "creg1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
    expect(() => resolveConditions(token, storeWith(actor, registry))).not.toThrow();
    expect(resolveConditions(token, storeWith(actor, registry))).toEqual([]);
  });

  it("returns no conditions for a raw token or an empty registry", () => {
    const actor = buildActorDoc("w1", NAME, { ...eng, conditions: ["dead"] }, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
    expect(resolveConditions(token, storeWith(actor))).toEqual([]); // no registry → all dropped
    const raw = buildTokenDoc("w1", "scene1", rawTokenEngine({ visual: { kind: "image", asset: "z" } }));
    expect(resolveConditions(raw, new DocumentStore())).toEqual([]);
  });
});

describe("conditionTarget", () => {
  it("targets the shared actor doc + /engine/conditions for a linked token", () => {
    const actor = buildActorDoc("w1", NAME, { ...eng, conditions: ["dead"] }, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
    const tgt = conditionTarget(token, storeWith(actor))!;
    expect(tgt.doc.id).toBe("act1");
    expect(tgt.path).toBe("/engine/conditions");
    expect(tgt.conditions).toEqual(["dead"]);
  });

  it("targets the token doc + embedded copy path for an instanced token", () => {
    const actor = buildActorDoc("w1", NAME, { ...eng, conditions: ["dead"] }, "act1");
    const token = buildTokenFromActor("w1", "scene1", actor, "instance", { x: 0, y: 0 }, 100);
    const tgt = conditionTarget(token, new DocumentStore())!; // store-independent (embedded)
    expect(tgt.doc.id).toBe(token.id);
    expect(tgt.path).toBe("/embedded/actor/0/engine/conditions");
    expect(tgt.conditions).toEqual(["dead"]);
  });

  it("returns null for a raw token and a dangling linked token", () => {
    const actor = buildActorDoc("w1", NAME, eng, "act1");
    const linked = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
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
  size: { w: 1, h: 1 }, shape: "square" as const, faction: null, conditions: [], prototype: false, vision: null, ...over,
});

test("resolveTokenBox derives multi-cell pixel size from actor.size × scene grid cell", () => {
  const scene = buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: null } }, "scene1");
  const actor = buildActorDoc("w1", "Goblin", actorEngine({ size: { w: 2, h: 3 } }), "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 50, y: 60 }, 100, "tok1");
  const box = resolveTokenBox(token, fakeStore([scene, actor, token]));
  expect(box).toEqual({ x: 50, y: 60, w: 200, h: 300, shape: "square" });
});

test("resolveTokenBox reads shape from the actor and applies a per-token override", () => {
  const scene = buildSceneDoc("w1", { grid: { kind: "square", size: 100, distance: null } }, "scene1");
  const actor = buildActorDoc("w1", "Goblin", actorEngine({ shape: "circle" }), "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
  expect(resolveTokenBox(token, fakeStore([scene, actor, token])).shape).toBe("circle");
  (token.engine as TokenEngine).overrides = { shape: "square", size: { w: 4, h: 4 }, name: null, visual: null, vision: null };
  const box = resolveTokenBox(token, fakeStore([scene, actor, token]));
  expect(box.shape).toBe("square");
  expect(box.w).toBe(400);
  expect(box.h).toBe(400);
});

test("resolveTokenBox falls back to token.engine w/h + square for a raw (actorless) token", () => {
  const token = buildTokenDoc("w1", "scene1", rawTokenEngine({ x: 10, y: 20, w: 64, h: 64, visual: { kind: "image", asset: "a1" } }), "tok1");
  expect(resolveTokenBox(token, fakeStore([token]))).toEqual({ x: 10, y: 20, w: 64, h: 64, shape: "square" });
});

test("resolveTokenBox defaults the grid cell to 100 when the parent scene is absent", () => {
  const actor = buildActorDoc("w1", "Goblin", actorEngine({ size: { w: 1, h: 1 } }), "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
  expect(resolveTokenBox(token, fakeStore([actor, token])).w).toBe(100);
});

test("footprintRadius: circle = max(w,h)/2, square = half-diagonal", () => {
  expect(footprintRadius({ shape: "circle", size: { w: 2, h: 4 } })).toBe(2);
  expect(footprintRadius({ shape: "square", size: { w: 2, h: 2 } })).toBeCloseTo(Math.SQRT2, 5);
});

it("resolves actor vision modes onto the effective actor", () => {
  const withVision = { ...eng, vision: [{ mode: "darkvision", range: 12 }] };
  const actor = buildActorDoc("w1", NAME, withVision, "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
  const eff = resolveTokenActor(token, storeWith(actor));
  expect(eff?.visionModes).toEqual([{ mode: "darkvision", range: 12 }]);
});

it("defaults visionModes to [] when actor has none", () => {
  const actor = buildActorDoc("w1", NAME, eng, "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
  expect(resolveTokenActor(token, storeWith(actor))?.visionModes).toEqual([]);
});

it("per-token override replaces actor vision modes", () => {
  const withVision = { ...eng, vision: [{ mode: "darkvision", range: 12 }] };
  const actor = buildActorDoc("w1", NAME, withVision, "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100);
  (token.engine as TokenEngine).overrides = { vision: [{ mode: "darkvision", range: 6 }], name: null, visual: null, size: null, shape: null };
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
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toEqual({ kind: "image", asset: "a1" });
  });

  it("passes an animated visual through unchanged", () => {
    const animated: TokenVisual = { kind: "animated", source: { type: "frames", frames: ["a1", "a2"] }, fps: 8, loop: true };
    const actor = actorWith(animated);
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toEqual(animated);
  });

  it("resolves faces to the manual token.engine.face over the default", () => {
    const actor = actorWith({
      kind: "faces",
      faces: { normal: { kind: "image", asset: "n1" }, bloodied: { kind: "image", asset: "b1" } },
      default: "normal",
      faceMap: null,
    });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
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
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
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
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toEqual({ kind: "image", asset: "bl1" });
  });

  it("falls back to default when neither manual face nor faceMap matches", () => {
    const actor = actorWith({
      kind: "faces",
      faces: { normal: { kind: "image", asset: "n1" }, bloodied: { kind: "image", asset: "b1" } },
      default: "normal",
      faceMap: null,
    });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toEqual({ kind: "image", asset: "n1" });
  });

  it("fails closed to the first face key when default itself is invalid", () => {
    const actor = actorWith({ kind: "faces", faces: { onlyOne: { kind: "image", asset: "o1" } }, default: "missing", faceMap: null });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toEqual({ kind: "image", asset: "o1" });
  });

  it("fails closed to null when the faces map is empty", () => {
    const actor = actorWith({ kind: "faces", faces: {}, default: "x", faceMap: null });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toBeNull();
  });

  it("fails closed on a malformed AnimatedSource (non-positive rows/cols)", () => {
    const actor = actorWith({ kind: "animated", source: { type: "sheet", asset: "s1", rows: 0, cols: 4, count: null }, fps: 8, loop: true });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toBeNull();
  });

  it("fails closed on a malformed AnimatedSource (empty frame list)", () => {
    const actor = actorWith({ kind: "animated", source: { type: "frames", frames: [] }, fps: 8, loop: true });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toBeNull();
  });

  it("fails closed on a malformed nested faces value (defense in depth against garbled wire data)", () => {
    const nested = { kind: "faces", faces: {}, default: "x" } as unknown as { kind: "image"; asset: string };
    const actor = actorWith({ kind: "faces", faces: { bad: nested }, default: "bad", faceMap: null });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
    expect(resolveTokenVisual(token, storeWith(actor))).toBeNull();
  });

  it("resolveTokenActor and resolveTokenVisual agree when a token has both a faces-union visual override AND an active face-swap", () => {
    const actor = actorWith({ kind: "image", asset: "base-asset" });
    const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 100, "tok1");
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
