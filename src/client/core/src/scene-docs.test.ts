import { test, expect, describe, it } from "vitest";
import { buildSceneDoc, buildTokenDoc, buildActorDoc, buildTokenFromActor, setNameHidden, buildFactionRegistryDoc, buildConditionRegistryDoc, buildItemDoc, ITEM_DOC_TYPE, type TokenEngine, type ActorEngine, type Faction, type Condition, type SceneEngine, type SceneDimensions, type TokenVisual, type FaceVisual, type AnimatedSource } from "./scene-docs";
import {
  buildWorldSettingsDoc, resolveSceneSettings, resolveViewedScene, DEFAULT_WORLD_SETTINGS, DEFAULT_SCENE_BOUNDS,
  type WireDocument, type WorldSettingsEngine,
} from "./scene-docs";
import { buildLightGradationDoc, resolveGradation, DEFAULT_GRADATION, buildVisionModesDoc, resolveVisionModes, SEED_VISION_MODES, buildLightDoc } from "./scene-docs";
import { buildRegionDoc, setRegionVisibility, type RegionEngine } from "./scene-docs";
import { DocumentStore } from "./store";
import { resolveTokenActor, resolveTokenBox } from "./actor";

function storeWith(...docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}

describe("resolveSceneSettings", () => {
  it("falls back to built-in defaults when no world-settings doc and no scene overrides", () => {
    const scene = buildSceneDoc("w1", {}, "scene1");
    const r = resolveSceneSettings(scene, storeWith(scene));
    expect(r.losRestriction).toBe(DEFAULT_WORLD_SETTINGS.scene.losRestriction);
    expect(r.movementRestriction).toBe("visible");
    expect(r.diagonalRule).toBe("chebyshev");
    expect(r.gridDistance).toEqual({ perCell: 5, unit: "ft" });
  });

  it("absent bounds resolves to DEFAULT_SCENE_BOUNDS", () => {
    const scene = buildSceneDoc("w1", {}, "scene1");
    const r = resolveSceneSettings(scene, storeWith(scene));
    expect(r.bounds).toEqual({ width: 100, height: 100 });
  });

  it("explicit bounds pass through", () => {
    const scene = buildSceneDoc("w1", { bounds: { width: 40, height: 25 } }, "scene1");
    const r = resolveSceneSettings(scene, storeWith(scene));
    expect(r.bounds).toEqual({ width: 40, height: 25 });
  });

  it("malformed bounds fail closed to the default", () => {
    const scene = buildSceneDoc("w1", {}, "scene1");
    // Non-positive on either axis is degenerate for a navmesh rectangle → default.
    (scene.engine as SceneEngine).bounds = { width: 0, height: -5 } as SceneDimensions;
    const r = resolveSceneSettings(scene, storeWith(scene));
    expect(r.bounds).toEqual(DEFAULT_SCENE_BOUNDS);
  });

  it("uses world-settings defaults over built-ins", () => {
    const scene = buildSceneDoc("w1", {}, "scene1");
    const ws = buildWorldSettingsDoc("w1", {
      ...DEFAULT_WORLD_SETTINGS,
      scene: { ...DEFAULT_WORLD_SETTINGS.scene, movementRestriction: "unrestricted" },
      pathfinding: { diagonalRule: "alternating" },
    }, "ws1");
    const r = resolveSceneSettings(scene, storeWith(scene, ws));
    expect(r.movementRestriction).toBe("unrestricted");
    expect(r.diagonalRule).toBe("alternating");
  });

  it("scene overrides win over world defaults", () => {
    const scene = buildSceneDoc("w1", {
      vision: { movementRestriction: "revealed", losRestriction: false, fog: null, observerVision: null, movementModel: null },
      lighting: { enabled: false, mode: null, environment: null },
      grid: { kind: "square", size: 100, distance: { perCell: 1.5, unit: "m" } },
    }, "scene1");
    const ws = buildWorldSettingsDoc("w1", DEFAULT_WORLD_SETTINGS, "ws1");
    const r = resolveSceneSettings(scene, storeWith(scene, ws));
    expect(r.movementRestriction).toBe("revealed");
    expect(r.losRestriction).toBe(false);
    expect(r.lightingEnabled).toBe(false);
    expect(r.gridDistance).toEqual({ perCell: 1.5, unit: "m" });
  });

  it("builds a world-settings doc with world scope and null parent", () => {
    const ws = buildWorldSettingsDoc("w1");
    expect(ws.doc_type).toBe("world-settings");
    expect(ws.parent_id).toBeNull();
    expect((ws.engine as { scene: unknown }).scene).toBeTruthy();
  });

  it("fail-closed: partial world-settings wire doc (missing scene/pathfinding/animation) falls back to built-in defaults and does not throw", () => {
    // Simulates a future partial wire payload where a set_pointer removed `scene`,
    // leaving a non-null but structurally incomplete world-settings engine object.
    const scene = buildSceneDoc("w1", {}, "scene-partial");
    const partialWs: WireDocument = {
      ...buildWorldSettingsDoc("w1", DEFAULT_WORLD_SETTINGS, "ws-partial"),
      engine: {} as unknown, // missing scene, pathfinding, animation
    };
    const r = resolveSceneSettings(scene, storeWith(scene, partialWs));
    // Must not throw and must return built-in defaults, not access undefined fields.
    expect(r.movementRestriction).toBe("visible");
    expect(r.diagonalRule).toBe("chebyshev");
    expect(r.losRestriction).toBe(true);
    expect(r.fog).toBe(true);
  });

  it("movementModel defaults to grid-stepped", () => {
    const scene = buildSceneDoc("w1", {}, "scene1");
    const r = resolveSceneSettings(scene, storeWith());
    expect(r.movementModel).toBe("grid-stepped");
  });

  it("movementModel: world override applies", () => {
    const ws = buildWorldSettingsDoc("w1", {
      ...DEFAULT_WORLD_SETTINGS,
      scene: { ...DEFAULT_WORLD_SETTINGS.scene, movementModel: "continuous" },
    }, "ws1");
    const scene = buildSceneDoc("w1", {}, "scene1");
    const r = resolveSceneSettings(scene, storeWith(ws));
    expect(r.movementModel).toBe("continuous");
  });

  it("movementModel: scene override beats world", () => {
    const ws = buildWorldSettingsDoc("w1", undefined, "ws1");
    const scene = buildSceneDoc("w1", {
      vision: { movementModel: "continuous", losRestriction: null, fog: null, observerVision: null, movementRestriction: null },
    }, "scene1");
    const r = resolveSceneSettings(scene, storeWith(ws));
    expect(r.movementModel).toBe("continuous");
  });

  it("movementModel: null scene override inherits world", () => {
    const ws = buildWorldSettingsDoc("w1", {
      ...DEFAULT_WORLD_SETTINGS,
      scene: { ...DEFAULT_WORLD_SETTINGS.scene, movementModel: "continuous" },
    }, "ws1");
    const scene = buildSceneDoc("w1", {
      vision: { movementModel: null, losRestriction: null, fog: null, observerVision: null, movementRestriction: null },
    }, "scene1");
    const r = resolveSceneSettings(scene, storeWith(ws));
    expect(r.movementModel).toBe("continuous");
  });

  it("snapToGrid defaults to true for a grid-stepped scene", () => {
    const scene = buildSceneDoc("w1", {}, "scene1");
    const r = resolveSceneSettings(scene, storeWith(scene));
    expect(r.movementModel).toBe("grid-stepped");
    expect(r.snapToGrid).toBe(true);
  });

  it("snapToGrid defaults to false for a continuous scene (derived default)", () => {
    const scene = buildSceneDoc("w1", { vision: { movementModel: "continuous", losRestriction: null, fog: null, observerVision: null, movementRestriction: null } }, "scene1");
    const r = resolveSceneSettings(scene, storeWith(scene));
    expect(r.movementModel).toBe("continuous");
    expect(r.snapToGrid).toBe(false);
  });

  it("snapToGrid: an explicit true overrides the continuous default", () => {
    const scene = buildSceneDoc(
      "w1",
      { vision: { movementModel: "continuous", losRestriction: null, fog: null, observerVision: null, movementRestriction: null }, snapToGrid: true },
      "scene1",
    );
    const r = resolveSceneSettings(scene, storeWith(scene));
    expect(r.snapToGrid).toBe(true);
  });

  it("snapToGrid: an explicit false overrides the grid-stepped default", () => {
    const scene = buildSceneDoc("w1", { snapToGrid: false }, "scene1");
    const r = resolveSceneSettings(scene, storeWith(scene));
    expect(r.movementModel).toBe("grid-stepped");
    expect(r.snapToGrid).toBe(false);
  });
});

const actorEngine: ActorEngine = {
  displayName: "Goblin",
  visual: { kind: "image", asset: "a1" },
  size: { w: 1, h: 1 },
  shape: "square",
  faction: null,
  conditions: [],
  prototype: true,
  vision: null,
};

test("buildSceneDoc makes a top-level world scene with a default square grid", () => {
  const doc = buildSceneDoc("w1");
  expect(doc.doc_type).toBe("scene");
  expect(doc.parent_id).toBeNull();
  expect(doc.scope).toEqual({ kind: "world", world_id: "w1" });
  expect(doc.system).toEqual({});
  expect(doc.engine).toEqual({
    grid: { kind: "square", size: 100, distance: null },
    background: null,
    bounds: null,
    snapToGrid: null,
    vision: null,
    lighting: null,
  });
  expect(typeof doc.id).toBe("string");
  expect(doc.id.length).toBeGreaterThan(0);
  expect(typeof doc.created_at).toBe("number");
});

test("buildSceneDoc honors a partial engine override and an explicit id", () => {
  const doc = buildSceneDoc("w1", { grid: { kind: "hex", size: 50, distance: null } }, "scene-fixed");
  expect(doc.id).toBe("scene-fixed");
  expect((doc.engine as SceneEngine).grid).toEqual({ kind: "hex", size: 50, distance: null });
  expect((doc.engine as SceneEngine).background).toBeNull();
});

test("buildSceneDoc persists an explicit snapToGrid:false (not omitted as falsy)", () => {
  const doc = buildSceneDoc("w1", { snapToGrid: false });
  expect((doc.engine as SceneEngine).snapToGrid).toBe(false);
});

test("buildTokenDoc parents to the scene and preserves the token engine body", () => {
  const eng: TokenEngine = { x: 140, y: 160, w: 100, h: 100, rotation: 0, visual: { kind: "image", asset: "img-1" }, actor_id: null, overrides: null, face: null };
  const doc = buildTokenDoc("w1", "scene-1", eng);
  expect(doc.doc_type).toBe("token");
  expect(doc.parent_id).toBe("scene-1");
  expect(doc.scope).toEqual({ kind: "world", world_id: "w1" });
  expect(doc.engine).toEqual(eng);
  expect(doc.system).toEqual({});
  expect(doc.permissions.default).toBe("observer");
});

test("buildActorDoc makes a top-level, parentless actor document with the name on the envelope", () => {
  const doc = buildActorDoc("w1", "Goblin", actorEngine, "act1");
  expect(doc.doc_type).toBe("actor");
  expect(doc.parent_id).toBeNull();
  expect(doc.scope).toEqual({ kind: "world", world_id: "w1" });
  expect(doc.name).toBe("Goblin");
  expect(doc.engine).toEqual(actorEngine);
  expect(doc.id).toBe("act1");
});

test("buildTokenFromActor link mode references the actor by id, no embedded copy", () => {
  const actor = buildActorDoc("w1", "Goblin", actorEngine, "act1");
  const t = buildTokenFromActor("w1", "scene1", actor, "link", { x: 50, y: 50 }, 100);
  expect(t.doc_type).toBe("token");
  expect(t.parent_id).toBe("scene1");
  expect((t.engine as TokenEngine).actor_id).toBe("act1");
  expect((t.engine as TokenEngine).overrides).toBeNull();
  expect(t.embedded.actor).toBeUndefined();
});

test("buildTokenFromActor instance mode embeds an independent copy with provenance", () => {
  const actor = buildActorDoc("w1", "Goblin", actorEngine, "act1");
  const t = buildTokenFromActor("w1", "scene1", actor, "instance", { x: 0, y: 0 }, 100);
  expect((t.engine as TokenEngine).actor_id).toBeNull();
  expect(t.embedded.actor).toHaveLength(1);
  const copy = t.embedded.actor[0];
  expect(copy.id).not.toBe(actor.id);
  expect(copy.source).toEqual({ id: "act1", pack: null, version: 1 });
  expect(copy.name).toBe("Goblin");
  expect(copy.engine).toEqual(actorEngine);
  expect(copy.engine).not.toBe(actor.engine); // independent by value, not aliased
});

test("buildTokenFromActor seeds w/h=cellSize solely as the dangling-link fallback (not consumed on the actor-backed render path)", () => {
  const actor = buildActorDoc("w1", "Goblin", actorEngine, "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, 50, "tok1");
  expect((token.engine as TokenEngine).w).toBe(50);
  expect((token.engine as TokenEngine).h).toBe(50);

  // Actor-backed render path: resolveTokenBox derives size from actor.size × cellSize,
  // NOT the seeded token.engine.w/h.
  const store = storeWith(actor, token);
  const box = resolveTokenBox(token, store, resolveTokenActor(token, store));
  expect(box.w).toBe(1 * 100); // actorEngine.size.w=1 × the store's default 100px cell, not the seeded 50

  // Dangling link (actor removed) — the seeded w/h becomes the real fallback.
  const withoutActor = new DocumentStore();
  withoutActor.applyCommand({ seq: 1, world_id: "w1", author: "a", ts: 0, ops: [{ op: "create", doc: token }] });
  const danglingBox = resolveTokenBox(token, withoutActor, resolveTokenActor(token, withoutActor));
  expect(danglingBox.w).toBe(50); // now the seeded engine.w is what's actually used
});

test("setNameHidden sets and clears the OwnerOrGm override on /name", () => {
  const d = buildActorDoc("w1", "Goblin", actorEngine, "act1");
  setNameHidden(d, true);
  expect(d.permissions.property_overrides["/name"]).toBe("owner_or_gm");
  setNameHidden(d, false);
  expect(d.permissions.property_overrides["/name"]).toBeUndefined();
});

test("buildFactionRegistryDoc builds a world-scoped, parentless registry with an id-keyed map", () => {
  const factions: Record<string, Faction> = { hostile: { name: "Hostile", color: "#f85149", stance: "hostile" } };
  const d = buildFactionRegistryDoc("w1", factions, "reg1");
  expect(d.doc_type).toBe("faction-registry");
  expect(d.parent_id).toBeNull();
  expect(d.scope).toEqual({ kind: "world", world_id: "w1" });
  expect((d.engine as { factions: unknown }).factions).toEqual(factions);
});

test("buildConditionRegistryDoc builds a world-scoped, parentless registry with an id-keyed map", () => {
  const conditions: Record<string, Condition> = { dead: { name: "Dead", icon: "💀" } };
  const d = buildConditionRegistryDoc("w1", conditions, "creg1");
  expect(d.doc_type).toBe("condition-registry");
  expect(d.parent_id).toBeNull();
  expect(d.scope).toEqual({ kind: "world", world_id: "w1" });
  expect((d.engine as { conditions: unknown }).conditions).toEqual(conditions);
  expect(d.id).toBe("creg1");
});

describe("light-gradation registry", () => {
  it("seeds bright/dim/dark sorted descending by minIllumination", () => {
    const g = resolveGradation(storeWith(buildLightGradationDoc("w1")));
    expect(g.map((b) => b.name)).toEqual(["bright", "dim", "dark"]);
    expect(g[0].minIllumination).toBeGreaterThan(g[1].minIllumination);
  });
  it("falls back to DEFAULT_GRADATION when no doc present", () => {
    expect(resolveGradation(storeWith())).toEqual([...DEFAULT_GRADATION.bands].sort((a, b) => b.minIllumination - a.minIllumination));
  });
  it("DEFAULT_GRADATION is frozen (immutable shared constant)", () => {
    expect(Object.isFrozen(DEFAULT_GRADATION)).toBe(true);
    expect(Object.isFrozen(DEFAULT_GRADATION.bands)).toBe(true);
  });
  it("buildLightGradationDoc engine is value-independent of DEFAULT_GRADATION", () => {
    const doc = buildLightGradationDoc("w1");
    // Must not alias the constant — a fresh clone so set_pointer edits do not mutate the seed.
    expect(doc.engine).not.toBe(DEFAULT_GRADATION);
    expect((doc.engine as { bands: unknown }).bands).not.toBe(DEFAULT_GRADATION.bands);
    // Values must be preserved.
    expect(doc.engine).toEqual(DEFAULT_GRADATION);
  });
});

describe("vision-modes registry", () => {
  it("seeds normal + darkvision with their floors", () => {
    const m = resolveVisionModes(storeWith(buildVisionModesDoc("w1")));
    expect(m.normal.illuminationFloor).toBe("dim");
    expect(m.darkvision.illuminationFloor).toBe("dark");
  });
  it("falls back to SEED_VISION_MODES when no doc present", () => {
    expect(resolveVisionModes(storeWith())).toEqual(SEED_VISION_MODES);
  });
  it("SEED_VISION_MODES is frozen (immutable shared constant)", () => {
    expect(Object.isFrozen(SEED_VISION_MODES)).toBe(true);
    expect(Object.isFrozen(SEED_VISION_MODES.normal)).toBe(true);
  });
  it("buildVisionModesDoc engine.modes is value-independent of SEED_VISION_MODES", () => {
    const doc = buildVisionModesDoc("w1");
    const modes = (doc.engine as { modes: Record<string, unknown> }).modes;
    // Must not alias the constant — a fresh clone so set_pointer edits do not mutate the seed.
    expect(modes).not.toBe(SEED_VISION_MODES);
    // Values must be preserved.
    expect(modes).toEqual(SEED_VISION_MODES);
  });
});

it("builds a light doc parented to its scene", () => {
  const l = buildLightDoc("w1", "scene1", { x: 10, y: 20, color: "#ffd9a0", intensity: 1, brightRadius: 4, dimRadius: 8, falloff: null, enabled: true });
  expect(l.doc_type).toBe("light");
  expect(l.parent_id).toBe("scene1");
  expect((l.engine as { brightRadius: number }).brightRadius).toBe(4);
  expect(l.system).toEqual({});
});

describe("buildRegionDoc", () => {
  it("builds a region doc parented to the scene with the given engine body", () => {
    const eng: RegionEngine = {
      shape: { kind: "rect", points: [0, 0, 100, 100] },
      behavior: "terrain",
      cost: 2,
      enabled: true,
    };
    const doc = buildRegionDoc("world1", "scene1", eng);
    expect(doc.doc_type).toBe("region");
    expect(doc.parent_id).toBe("scene1");
    expect(doc.engine).toEqual(eng);
    expect(doc.system).toEqual({});
    expect(doc.permissions.property_overrides).toEqual({});
  });

  it("setRegionVisibility(true) declares /engine as gm_only; false clears it", () => {
    const doc = buildRegionDoc("world1", "scene1", {
      shape: { kind: "circle", points: [50, 50, 25] },
      behavior: "arrest",
      cost: 1,
      enabled: true,
    });
    setRegionVisibility(doc, true);
    expect(doc.permissions.property_overrides["/engine"]).toBe("gm_only");
    setRegionVisibility(doc, false);
    expect(doc.permissions.property_overrides["/engine"]).toBeUndefined();
  });
});

describe("TokenVisual union", () => {
  it("admits a plain image visual", () => {
    const v: TokenVisual = { kind: "image", asset: "a1" };
    expect(v.kind).toBe("image");
  });

  it("admits an animated visual with a frame-list source", () => {
    const v: TokenVisual = { kind: "animated", source: { type: "frames", frames: ["a1", "a2"] }, fps: 8, loop: true };
    expect(v).toMatchObject({ kind: "animated", fps: 8, loop: true });
  });

  it("admits an animated visual with a grid-sheet source", () => {
    const source: AnimatedSource = { type: "sheet", asset: "sheet1", rows: 2, cols: 4, count: 7 };
    const v: TokenVisual = { kind: "animated", source, fps: 12, loop: false };
    expect(v.kind).toBe("animated");
  });

  it("admits a faces visual whose face values are themselves RenderVisuals (image or animated)", () => {
    const bloodied: FaceVisual = { kind: "animated", source: { type: "frames", frames: ["b1"] }, fps: 4, loop: true };
    const v: TokenVisual = {
      kind: "faces",
      faces: { normal: { kind: "image", asset: "n1" }, bloodied },
      default: "normal",
      faceMap: { bleeding: "bloodied" },
    };
    expect(Object.keys(v.faces)).toEqual(["normal", "bloodied"]);
  });
});

describe("buildItemDoc", () => {
  it("builds a top-level client-only item document with the name on the envelope", () => {
    const doc = buildItemDoc("w1", "Sword", { damage: "1d8" });
    expect(doc.doc_type).toBe(ITEM_DOC_TYPE);
    expect(doc.parent_id).toBeNull();
    expect(doc.name).toBe("Sword");
    expect((doc.system as { damage: string }).damage).toBe("1d8");
    // "item" is not engine-defined — no `engine` key is written on the wire.
    expect(doc.engine).toBeUndefined();
  });
});

function store(docs: WireDocument[]): DocumentStore {
  const s = new DocumentStore();
  s.applyCommand({ seq: 1, world_id: "w1", author: "u", ts: 0, ops: docs.map((doc) => ({ op: "create", doc })) });
  return s;
}
function ws(activeScene: string | null): WireDocument {
  return buildWorldSettingsDoc("w1", { ...structuredClone(DEFAULT_WORLD_SETTINGS), activeScene } as WorldSettingsEngine);
}

describe("resolveViewedScene", () => {
  it("returns null when no scene exists", () => {
    expect(resolveViewedScene(store([]))).toBeNull();
    expect(resolveViewedScene(store([ws(null)]))).toBeNull();
  });

  it("falls back to the first scene when activeScene is absent/null (legacy behavior)", () => {
    const s0 = buildSceneDoc("w1", {}, "s0");
    const s1 = buildSceneDoc("w1", {}, "s1");
    expect(resolveViewedScene(store([s0, s1]))).toBe("s0");
    expect(resolveViewedScene(store([s0, s1, ws(null)]))).toBe("s0");
  });

  it("follows a resolvable activeScene (players)", () => {
    const s0 = buildSceneDoc("w1", {}, "s0");
    const s1 = buildSceneDoc("w1", {}, "s1");
    expect(resolveViewedScene(store([s0, s1, ws("s1")]))).toBe("s1");
  });

  it("falls back to the first scene when activeScene dangles (deleted target)", () => {
    const s0 = buildSceneDoc("w1", {}, "s0");
    expect(resolveViewedScene(store([s0, ws("gone")]))).toBe("s0");
  });

  it("gmViewedScene overrides activeScene when it resolves", () => {
    const s0 = buildSceneDoc("w1", {}, "s0");
    const s1 = buildSceneDoc("w1", {}, "s1");
    const s = store([s0, s1, ws("s1")]);
    expect(resolveViewedScene(s, { gmViewedScene: "s0" })).toBe("s0");
  });

  it("ignores a dangling gmViewedScene and falls through to activeScene", () => {
    const s0 = buildSceneDoc("w1", {}, "s0");
    const s1 = buildSceneDoc("w1", {}, "s1");
    const s = store([s0, s1, ws("s1")]);
    expect(resolveViewedScene(s, { gmViewedScene: "gone" })).toBe("s1");
    expect(resolveViewedScene(s, { gmViewedScene: null })).toBe("s1");
  });
});
