import { test, expect, describe, it } from "vitest";
import { buildSceneDoc, buildTokenDoc, buildActorDoc, buildTokenFromActor, setNameHidden, buildFactionRegistryDoc, buildConditionRegistryDoc, buildItemDoc, ITEM_DOC_TYPE, deterministicId, type TokenEngine, type ActorEngine, type Faction, type Condition, type SceneEngine, type SceneDimensions, type TokenVisual, type FaceVisual, type AnimatedSource } from "./scene-docs";
import {
  buildWorldSettingsDoc, resolveSceneSettings, resolveViewedScene, DEFAULT_WORLD_SETTINGS, DEFAULT_SCENE_BOUNDS,
  type WireDocument, type WorldSettingsEngine,
} from "./scene-docs";
import { buildLightGradationDoc, resolveGradation, DEFAULT_GRADATION, buildVisionModesDoc, resolveVisionModes, SEED_VISION_MODES, buildLightDoc } from "./scene-docs";
import { buildRegionDoc, setRegionVisibility, type RegionEngine } from "./scene-docs";
import {
  buildCombatDoc, buildCombatantDoc, buildResourceRegistryDoc, buildEffectDoc, buildCombatHistoryDoc, seedResourceRegistryIfAbsent,
  COMBAT_DOC_TYPE, COMBATANT_DOC_TYPE, RESOURCE_REGISTRY_DOC_TYPE, EFFECT_DOC_TYPE, COMBAT_HISTORY_DOC_TYPE,
  type CombatEngine, type CombatantEngine, type ResourceRegistryEngine, type Resource, type EffectEngine,
} from "./scene-docs";
import {
  buildSystemDefaultsDoc, systemDefaultsUpsertOps, resolveSettingProvenance, SYSTEM_DEFAULTS_DOC_TYPE,
  type SystemDefaultsEngine,
} from "./scene-docs";
import { DocumentStore } from "./store";
import type { WireOperation } from "./wire";
import { resolveTokenActor, resolveTokenBox } from "./actor";
import { EMPTY_FOOTPRINTS, type FootprintLookup } from "./footprints";

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
    combat: null,
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

it("DEFAULT_WORLD_SETTINGS carries an unset combat chain and buildSceneDoc mirrors it", () => {
  expect(DEFAULT_WORLD_SETTINGS.combat).toBeNull();
  const scene = buildSceneDoc("world-1");
  expect((scene.engine as SceneEngine).combat).toBeNull();
  const ship = buildSceneDoc("world-1", { combat: { movementResource: "ship", enforcement: "hard" } });
  expect((ship.engine as SceneEngine).combat).toEqual({ movementResource: "ship", enforcement: "hard" });
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
  const t = buildTokenFromActor("w1", "scene1", actor, "link", { x: 50, y: 50 }, { w: 100, h: 100 });
  expect(t.doc_type).toBe("token");
  expect(t.parent_id).toBe("scene1");
  expect((t.engine as TokenEngine).actor_id).toBe("act1");
  expect((t.engine as TokenEngine).overrides).toBeNull();
  expect(t.embedded.actor).toBeUndefined();
});

test("buildTokenFromActor instance mode embeds an independent copy with provenance", () => {
  const actor = buildActorDoc("w1", "Goblin", actorEngine, "act1");
  const t = buildTokenFromActor("w1", "scene1", actor, "instance", { x: 0, y: 0 }, { w: 100, h: 100 });
  expect((t.engine as TokenEngine).actor_id).toBeNull();
  expect(t.embedded.actor).toHaveLength(1);
  const copy = t.embedded.actor[0];
  expect(copy.id).not.toBe(actor.id);
  expect(copy.source).toEqual({ id: "act1", pack: null, version: 1 });
  expect(copy.name).toBe("Goblin");
  expect(copy.engine).toEqual(actorEngine);
  expect(copy.engine).not.toBe(actor.engine); // independent by value, not aliased
});

test("buildTokenFromActor stamps the scene's unit footprint, which stands until the server states this token's own", () => {
  const actor = buildActorDoc("w1", "Goblin", actorEngine, "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, { w: 50, h: 50 }, "tok1");
  expect((token.engine as TokenEngine).w).toBe(50);
  expect((token.engine as TokenEngine).h).toBe(50);

  // The server's own resolved extent, once stated, wins over the stamped unit footprint.
  const store = storeWith(actor, token);
  const resolved: FootprintLookup = { token: () => ({ w: 100, h: 100 }), unit: () => null };
  expect(resolveTokenBox(token, store, resolved, resolveTokenActor(token, store)).w).toBe(100);

  // Until then — and permanently for a token no actor sizes — the stamped extent is the box.
  expect(resolveTokenBox(token, store, EMPTY_FOOTPRINTS, resolveTokenActor(token, store)).w).toBe(50);
});

test("buildTokenFromActor stamps a zero extent when the scene's unit footprint is not yet known", () => {
  // No `"footprints"` frame has arrived, so there is no authoritative unit extent to stamp and
  // none is invented here. The token draws at nothing until the server states its own.
  const actor = buildActorDoc("w1", "Goblin", actorEngine, "act1");
  const token = buildTokenFromActor("w1", "scene1", actor, "link", { x: 0, y: 0 }, null, "tok1");
  expect((token.engine as TokenEngine).w).toBe(0);
  expect((token.engine as TokenEngine).h).toBe(0);
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

describe("deterministicId", () => {
  const UUID_SHAPE = /^[0-9a-f]{8}-[0-9a-f]{4}-5[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

  test("produces a full 36-character UUIDv5-shaped string, not a truncated one", () => {
    const id = deterministicId("world-1", "faction-registry");
    expect(id).toHaveLength(36);
    expect(id).toMatch(UUID_SHAPE);
  });

  test("is stable across calls with the same inputs", () => {
    expect(deterministicId("world-1", "faction-registry")).toBe(deterministicId("world-1", "faction-registry"));
  });

  test("differs across namespaces and names", () => {
    expect(deterministicId("world-1", "faction-registry")).not.toBe(deterministicId("world-2", "faction-registry"));
    expect(deterministicId("world-1", "faction-registry")).not.toBe(deterministicId("world-1", "condition-registry"));
  });
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

describe("combat document builders", () => {
  const combatEngine: CombatEngine = {
    scene_id: "scene-1", active: false, round: 0, turn: null, turn_control: "owner_may_end", order: [],
    movement: { resource: null, interpretation: "per_cell", enforcement: "none" },
    effect_cleanup: true, rewind_restore: true, forward_restore: false,
    effect_lifecycle: { onCombatEnd: null, onTurnEnd: null, onAdvance: null },
  };

  it("buildCombatDoc is a parentless engine doc", () => {
    const d = buildCombatDoc("world-1", combatEngine);
    expect(d.doc_type).toBe(COMBAT_DOC_TYPE);
    expect(d.parent_id).toBeNull();
    expect(d.engine).toEqual(combatEngine);
    expect(d.system).toEqual({});
  });

  it("buildCombatantDoc parents to the combat and encodes hidden as an unreadable default role", () => {
    const eng: CombatantEngine = {
      kind: { type: "actor", token_id: "tok-1", actor_id: null }, initiative: null, tiebreak: 0, resources: {},
    };
    const visible = buildCombatantDoc("world-1", "combat-1", eng, { owner: "user-1" });
    expect(visible.doc_type).toBe(COMBATANT_DOC_TYPE);
    expect(visible.parent_id).toBe("combat-1");
    expect(visible.owner).toBe("user-1");
    expect(visible.permissions.default).toBe("observer");
    expect(visible.permissions.users).toEqual({ "user-1": "owner" });
    // Stored resource numbers default to owner-or-GM disclosure; a GM widens
    // deliberately by overwriting the entry after building.
    expect(visible.permissions.property_overrides["/engine/resources"]).toBe("owner_or_gm");
    const hidden = buildCombatantDoc("world-1", "combat-1", eng, { owner: "user-1", hidden: true });
    expect(hidden.permissions.property_overrides["/engine/resources"]).toBe("owner_or_gm");
    expect(hidden.permissions.default).toBe("none");
    // While hidden the owner is not listed either: hidden means unreadable to every non-GM.
    expect(hidden.permissions.users).toEqual({});
    expect(hidden.owner).toBe("user-1");
  });

  it("buildResourceRegistryDoc and the seed helper are idempotent and deterministic", () => {
    const seed: Record<string, Resource> = {
      movement: { name: "Movement", order: 0, binding: { kind: "tracked", max: "speed",
        recover: { turn_start: "speed", turn_end: 0, round_start: 0, round_end: 0 } } },
    };
    const id = deterministicId("world-1", RESOURCE_REGISTRY_DOC_TYPE);
    const doc = buildResourceRegistryDoc("world-1", seed, id);
    expect(doc.doc_type).toBe(RESOURCE_REGISTRY_DOC_TYPE);
    expect(doc.id).toBe(id);
    expect((doc.engine as ResourceRegistryEngine).resources.movement.binding.kind).toBe("tracked");

    const store = new DocumentStore();
    const dispatched: WireOperation[][] = [];
    seedResourceRegistryIfAbsent(store, "world-1", seed, (ops) => dispatched.push(ops));
    expect(dispatched).toHaveLength(1);
    expect(dispatched[0][0]).toMatchObject({ op: "create", doc: { id, doc_type: RESOURCE_REGISTRY_DOC_TYPE } });
    store.applyCommand({ seq: 1, world_id: "world-1", author: "u", ts: 0, ops: dispatched[0] });
    seedResourceRegistryIfAbsent(store, "world-1", seed, (ops) => dispatched.push(ops));
    expect(dispatched).toHaveLength(1);
  });

  it("buildEffectDoc carries the engine band and the system body", () => {
    const eng: EffectEngine = { active: true, transfer: true, duration: null };
    const d = buildEffectDoc("world-1", eng, { mechanics: { modifiers: {} } }, undefined, "Bless");
    expect(d.doc_type).toBe(EFFECT_DOC_TYPE);
    expect(d.engine).toEqual(eng);
    expect(d.name).toBe("Bless");
    expect(d.system).toEqual({ mechanics: { modifiers: {} } });
  });

  it("buildCombatHistoryDoc parents to the combat and is GM-only", () => {
    const d = buildCombatHistoryDoc("world-1", "combat-1");
    expect(d.doc_type).toBe(COMBAT_HISTORY_DOC_TYPE);
    expect(d.parent_id).toBe("combat-1");
    expect(d.permissions.default).toBe("none");
    expect(d.engine).toEqual({ records: [], cursor: 0 });
  });
});

describe("system defaults", () => {
  it("upsert creates the singleton at the deterministic id when absent", () => {
    const ops = systemDefaultsUpsertOps(storeWith(), "w1", { scene: { fog: false } });
    expect(ops).toHaveLength(1);
    expect(ops[0]).toMatchObject({
      op: "create",
      doc: { id: deterministicId("w1", SYSTEM_DEFAULTS_DOC_TYPE), doc_type: SYSTEM_DEFAULTS_DOC_TYPE, engine: { scene: { fog: false } } },
    });
  });

  it("upsert writes one field change per differing section with the stored value as pre-image", () => {
    const existing = buildSystemDefaultsDoc(
      "w1",
      { scene: { fog: false, losRestriction: true }, pathfinding: { diagonalRule: "euclidean" } },
      deterministicId("w1", SYSTEM_DEFAULTS_DOC_TYPE),
    );
    const ops = systemDefaultsUpsertOps(storeWith(existing), "w1", { scene: { fog: true, losRestriction: true } });
    expect(ops).toEqual([{
      op: "update",
      doc_id: existing.id,
      changes: [
        { path: "/engine/scene", old: { fog: false, losRestriction: true }, new: { fog: true, losRestriction: true } },
        { path: "/engine/pathfinding", old: { diagonalRule: "euclidean" }, new: null },
      ],
    }]);
  });

  it("upsert is a no-op when the stored doc equals the declaration", () => {
    const existing = buildSystemDefaultsDoc("w1", { combat: { enforcement: "hard" } }, deterministicId("w1", SYSTEM_DEFAULTS_DOC_TYPE));
    expect(systemDefaultsUpsertOps(storeWith(existing), "w1", { combat: { enforcement: "hard" } })).toEqual([]);
  });

  it("upsert is a no-op when a stored section round-tripped through key-order normalization (server BTreeMap) still matches the declaration", () => {
    // The author's declared order is non-alphabetical; the stored doc simulates what the
    // server's serde_json (no `preserve_order`) actually returns: alphabetically-keyed.
    const declared: SystemDefaultsEngine = { combat: { enforcement: "hard", movementResource: "gold" } };
    const existing = buildSystemDefaultsDoc(
      "w1",
      { combat: { movementResource: "gold", enforcement: "hard" } },
      deterministicId("w1", SYSTEM_DEFAULTS_DOC_TYPE),
    );
    expect(systemDefaultsUpsertOps(storeWith(existing), "w1", declared)).toEqual([]);
  });

  it("resolveSceneSettings folds engine < system < world < scene per field", () => {
    const sd = buildSystemDefaultsDoc("w1", { scene: { fog: false, observerVision: true }, animation: { speedCellsPerSec: 3 } });
    const ws = buildWorldSettingsDoc("w1", { ...structuredClone(DEFAULT_WORLD_SETTINGS), scene: { ...DEFAULT_WORLD_SETTINGS.scene, fog: true } }, "ws1");
    const scene = buildSceneDoc("w1", { vision: { losRestriction: null, fog: null, observerVision: false, movementRestriction: null, movementModel: null } }, "s1");
    const r = resolveSceneSettings(scene, storeWith(sd, ws, scene));
    expect(r.fog).toBe(true);
    expect(r.observerVision).toBe(false);
    expect(r.animation.speedCellsPerSec).toBe(6);
  });

  it("resolveSettingProvenance names the layer that supplied the value", () => {
    const sd: SystemDefaultsEngine = { scene: { fog: false }, pathfinding: { diagonalRule: "manhattan" } };
    const sdDoc = buildSystemDefaultsDoc("w1", sd);
    // World-settings values below are deliberately chosen to DIFFER from the system/engine
    // default beneath them, so the "world" source is a genuine override, not a coincidental match.
    const ws = buildWorldSettingsDoc(
      "w1",
      {
        ...structuredClone(DEFAULT_WORLD_SETTINGS),
        scene: { ...DEFAULT_WORLD_SETTINGS.scene, losRestriction: false },
        pathfinding: { diagonalRule: "chebyshev" },
      },
      "ws1",
    );
    const scene = buildSceneDoc("w1", { vision: { losRestriction: null, fog: true, observerVision: null, movementRestriction: null, movementModel: null } }, "s1");
    const storeWithWorld = storeWith(sdDoc, ws, scene);
    expect(resolveSettingProvenance(storeWithWorld, scene, "scene.fog")).toEqual({
      value: true, source: "scene", systemOrEngine: { value: false, source: "system" },
    });
    expect(resolveSettingProvenance(storeWithWorld, undefined, "scene.losRestriction")).toEqual({
      value: false, source: "world",
      systemOrEngine: { value: DEFAULT_WORLD_SETTINGS.scene.losRestriction, source: "engine" },
    });
    expect(resolveSettingProvenance(storeWithWorld, undefined, "pathfinding.diagonalRule")).toEqual({
      value: "chebyshev", source: "world", systemOrEngine: { value: "manhattan", source: "system" },
    });
    // No world doc: the system layer is what supplies scene.fog.
    const storeNoWorld = storeWith(sdDoc, scene);
    expect(resolveSettingProvenance(storeNoWorld, undefined, "scene.fog")).toEqual({
      value: false, source: "system", systemOrEngine: { value: false, source: "system" },
    });
  });

  it("resolveSettingProvenance reports the system/engine source, not \"world\", when a stored world value merely COINCIDES with the layer beneath it", () => {
    // A world-settings doc is required-field-complete on the wire — every WorldSceneDefaults
    // leaf is always present once the doc exists, even when nobody has genuinely overridden it.
    // resolvePick's presence-only check would report "world" here unconditionally; the
    // equality collapse must instead report the deeper layer that actually matches.
    const sdDoc = buildSystemDefaultsDoc("w1", { scene: { fog: false } });
    // Explicitly stores fog: false — coincidentally the same value the system doc supplies.
    const wsMatchesSystem = buildWorldSettingsDoc(
      "w1",
      { ...structuredClone(DEFAULT_WORLD_SETTINGS), scene: { ...DEFAULT_WORLD_SETTINGS.scene, fog: false } },
      "ws1",
    );
    const scene = buildSceneDoc("w1", {}, "s1");
    const store = storeWith(sdDoc, wsMatchesSystem, scene);
    expect(resolveSettingProvenance(store, undefined, "scene.fog")).toEqual({
      value: false, source: "system", systemOrEngine: { value: false, source: "system" },
    });
    // No system doc: world's stored fog (the default's own true) coincides with the built-in
    // engine default instead.
    const wsMatchesEngine = buildWorldSettingsDoc("w1", undefined, "ws2");
    const storeNoSystem = storeWith(wsMatchesEngine, scene);
    expect(resolveSettingProvenance(storeNoSystem, undefined, "scene.fog")).toEqual({
      value: DEFAULT_WORLD_SETTINGS.scene.fog, source: "engine",
      systemOrEngine: { value: DEFAULT_WORLD_SETTINGS.scene.fog, source: "engine" },
    });
  });

  it("resolveSettingProvenance treats a scene's explicit combat.movementResource null as a terminal clear, not a fall-through", () => {
    const sdDoc = buildSystemDefaultsDoc("w1", { combat: { movementResource: "gold" } });
    const scene = buildSceneDoc("w1", { combat: { movementResource: null } }, "s1");
    const store = storeWith(sdDoc, scene);
    // The scene's explicit clear wins over the system's supplied "gold" — mirrors
    // resolve_combat_rules's outer-Option-first pick, not a nullish coalesce.
    expect(resolveSettingProvenance(store, scene, "combat.movementResource")).toEqual({
      value: null, source: "scene", systemOrEngine: { value: "gold", source: "system" },
    });
  });

  it("resolveSettingProvenance falls through to the system layer when the scene has no combat.movementResource key at all", () => {
    const sdDoc = buildSystemDefaultsDoc("w1", { combat: { movementResource: "gold" } });
    const scene = buildSceneDoc("w1", {}, "s1");
    const store = storeWith(sdDoc, scene);
    expect(resolveSettingProvenance(store, scene, "combat.movementResource")).toEqual({
      value: "gold", source: "system", systemOrEngine: { value: "gold", source: "system" },
    });
  });
});
