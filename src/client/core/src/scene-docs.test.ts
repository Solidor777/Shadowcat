import { test, expect, describe, it } from "vitest";
import { buildSceneDoc, buildTokenDoc, buildActorDoc, buildTokenFromActor, setNameHidden, buildFactionRegistryDoc, buildConditionRegistryDoc, type TokenSystem, type ActorSystem, type Faction, type Condition } from "./scene-docs";
import {
  buildWorldSettingsDoc, resolveSceneSettings, DEFAULT_WORLD_SETTINGS,
  type WireDocument,
} from "./scene-docs";
import { buildLightGradationDoc, resolveGradation, DEFAULT_GRADATION, buildVisionModesDoc, resolveVisionModes, SEED_VISION_MODES } from "./scene-docs";
import { DocumentStore } from "./store";

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
      vision: { movementRestriction: "revealed", losRestriction: false },
      lighting: { enabled: false },
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
    expect((ws.system as { scene: unknown }).scene).toBeTruthy();
  });

  it("fail-closed: partial world-settings wire doc (missing scene/pathfinding/animation) falls back to built-in defaults and does not throw", () => {
    // Simulates a future partial wire payload where a set_pointer removed `scene`,
    // leaving a non-null but structurally incomplete world-settings system object.
    const scene = buildSceneDoc("w1", {}, "scene-partial");
    const partialWs: WireDocument = {
      ...buildWorldSettingsDoc("w1", DEFAULT_WORLD_SETTINGS, "ws-partial"),
      system: {} as unknown, // missing scene, pathfinding, animation
    };
    const r = resolveSceneSettings(scene, storeWith(scene, partialWs));
    // Must not throw and must return built-in defaults, not access undefined fields.
    expect(r.movementRestriction).toBe("visible");
    expect(r.diagonalRule).toBe("chebyshev");
    expect(r.losRestriction).toBe(true);
    expect(r.fog).toBe(true);
  });
});

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

test("setNameHidden sets and clears the OwnerOrGm override on /system/name", () => {
  const d = buildActorDoc("w1", actorSys, "act1");
  setNameHidden(d, true);
  expect(d.permissions.property_overrides["/system/name"]).toBe("owner_or_gm");
  setNameHidden(d, false);
  expect(d.permissions.property_overrides["/system/name"]).toBeUndefined();
});

test("buildFactionRegistryDoc builds a world-scoped, parentless registry with an id-keyed map", () => {
  const factions: Record<string, Faction> = { hostile: { name: "Hostile", color: "#f85149", stance: "hostile" } };
  const d = buildFactionRegistryDoc("w1", factions, "reg1");
  expect(d.doc_type).toBe("faction-registry");
  expect(d.parent_id).toBeNull();
  expect(d.scope).toEqual({ kind: "world", world_id: "w1" });
  expect((d.system as { factions: unknown }).factions).toEqual(factions);
});

test("buildConditionRegistryDoc builds a world-scoped, parentless registry with an id-keyed map", () => {
  const conditions: Record<string, Condition> = { dead: { name: "Dead", icon: "💀" } };
  const d = buildConditionRegistryDoc("w1", conditions, "creg1");
  expect(d.doc_type).toBe("condition-registry");
  expect(d.parent_id).toBeNull();
  expect(d.scope).toEqual({ kind: "world", world_id: "w1" });
  expect((d.system as { conditions: unknown }).conditions).toEqual(conditions);
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
});
