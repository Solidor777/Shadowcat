// Client-owned scene-entity doc builders + pure resolvers (re-rooted onto the three-band
// document shape — envelope(+name) + engine? + system). The per-doc-type ENGINE bodies are
// no longer client-owned interpretations of an opaque `system` blob: they are ts-rs-generated
// from the server's `data/engine` structs (`@shadowcat/types`), server-validated at ingress
// (`validate_engine`). Only genuinely game-system-owned shapes (`item`) and client-only
// resolution helpers stay defined here.
export type { WireDocument } from "./wire";
import type { WireDocument } from "./wire";
import type { ReadableDocuments } from "./store";
import type {
  MovementRestriction,
  MovementModel,
  LightMode,
  DiagonalRule,
  EasingMode,
  EnvironmentLight,
  GridDistance,
  SceneDimensions,
  SceneVisionOverrides,
  SceneLightingOverrides,
  Grid,
  SceneEngine,
  WorldSceneDefaults,
  WorldSettingsEngine,
  TokenEngine,
  TokenOverrides,
  TokenVisual,
  RenderVisual,
  AnimatedSource,
  VisionAssignment,
  ActorEngine,
  LightEngine,
  Falloff,
  RegionShape,
  RegionEngine,
  Faction,
  FactionStance,
  FactionRegistryEngine,
  Condition,
  ConditionRegistryEngine,
  GradationBand,
  LightGradationEngine,
  VisionMode,
  VisionModesEngine,
} from "@shadowcat/types";

// --- Re-exported generated engine types (ts-rs output, `@shadowcat/types`) ---
// These mirror `src/server/src/data/engine/*.rs` byte-for-byte; NEVER hand-edit the
// generated source. Nested/shared shapes keep their generated name (identical to the
// prior client name); per-doc-type body shapes are re-exported under a NEW `*Engine` name
// (never aliased back onto the old `*System` name — an alias would let stale `*System`-named
// reads/writes keep compiling against the wrong band undetected).
export type {
  MovementRestriction,
  MovementModel,
  LightMode,
  DiagonalRule,
  EasingMode,
  EnvironmentLight,
  GridDistance,
  SceneDimensions,
  SceneVisionOverrides,
  SceneLightingOverrides,
  Grid,
  SceneEngine,
  WorldSceneDefaults,
  WorldSettingsEngine,
  TokenEngine,
  TokenOverrides,
  TokenVisual,
  RenderVisual,
  AnimatedSource,
  VisionAssignment,
  ActorEngine,
  LightEngine,
  Falloff,
  RegionShape,
  RegionEngine,
  Faction,
  FactionStance,
  FactionRegistryEngine,
  Condition,
  ConditionRegistryEngine,
  GradationBand,
  LightGradationEngine,
  VisionMode,
  VisionModesEngine,
};

/** A face's own visual. Deliberately never itself `{kind:"faces"}` — no nesting — so an
 * animated face falls out of the same `RenderVisual` boundary with no separate mechanism.
 * Client-only alias (ts-rs does not export this — it is structurally `RenderVisual`). */
export type FaceVisual = RenderVisual;

/** Client-only convenience literal narrowing `RegionShape.kind`/`RegionEngine.behavior`
 * callers may cast to; the generated fields are plain `string` (asserted by the server's
 * unit battery, not enforced by a Rust enum). */
export type RegionShapeKind = "rect" | "circle" | "polygon";
export type RegionBehavior = "terrain" | "impassable" | "arrest";

// Recursive freeze helper — makes default constants immutable so shared refs
// returned by resolver functions cannot be mutated by consumers in dev.
function deepFreeze<T>(obj: T): T {
  Object.freeze(obj);
  for (const v of Object.values(obj as object)) {
    if (v !== null && typeof v === "object" && !Object.isFrozen(v)) deepFreeze(v);
  }
  return obj;
}

/** Fail-safe finite default scene size (grid units) when a scene has no authored `bounds`, so
 * navmesh construction never faces an unbounded plane. MUST match DEFAULT_SCENE_BOUNDS_UNITS in
 * the server `scene/mod.rs`. Deliberately a fixed constant — NOT a content AABB (content-derived
 * bounds were rejected: edge-drag re-mesh churn, ill-defined for open scenes). */
export const DEFAULT_SCENE_BOUNDS: SceneDimensions = deepFreeze({ width: 100, height: 100 });

/** Built-in defaults — used when no world-settings doc exists or a field is absent.
 * Deep-frozen so shared refs in resolveSceneSettings output are immutable in dev;
 * enumerable values are unchanged. MUST equal the server's `impl Default for
 * WorldSettingsEngine` (`data/engine/scene.rs`) — the client stays the authoritative
 * source; a server-side unit test cross-checks parity. */
export const DEFAULT_WORLD_SETTINGS: WorldSettingsEngine = deepFreeze({
  scene: {
    losRestriction: true,
    fog: true,
    lightingEnabled: true,
    lightMode: "environmentLight",
    environment: { color: "#0a0e1a", intensity: 0.0 },
    observerVision: false,
    movementRestriction: "visible",
    movementModel: "grid-stepped",
    partialCellLeniency: true,
  },
  pathfinding: { diagonalRule: "chebyshev" },
  animation: { speedCellsPerSec: 6, easing: "easeInOut" },
  activeScene: null,
});

// --- Resolved settings (M10e-1) ---

/** The fully resolved per-scene settings after merging built-ins → world defaults → scene overrides. */
export interface ResolvedSceneSettings {
  losRestriction: boolean;
  fog: boolean;
  observerVision: boolean;
  movementRestriction: MovementRestriction;
  movementModel: MovementModel;
  /** Effective snap-to-grid axis (M10f-3 §4.1): an explicit scene value overrides in either
   * direction (including `false`); absent falls back to a derived default keyed off the
   * RESOLVED `movementModel` (false for continuous, true otherwise). */
  snapToGrid: boolean;
  lightingEnabled: boolean;
  lightMode: LightMode;
  environment: EnvironmentLight;
  partialCellLeniency: boolean;
  diagonalRule: DiagonalRule;
  animation: { speedCellsPerSec: number; easing: EasingMode };
  gridDistance: GridDistance;
  bounds: SceneDimensions;
}

/** Visible-to-all defaults; the server normalizes permissions per the creator's role. */
function defaultPermissions(): WireDocument["permissions"] {
  return {
    default: "observer",
    users: {},
    property_overrides: {},
    capabilities: { by_role: {}, by_user: {} },
    // `null` preserves the GM's unconditional access — matches the server-side
    // default for every document type that predates this field.
    gm_role: null,
  };
}

/** Package-internal document envelope builder (shared by scene-docs and chat-docs).
 * `engine` is the typed, server-validated engine body — `undefined` for a non-engine-defined
 * `doc_type` (the key is then omitted on the wire, matching the server's
 * `Option<serde_json::Value>` default-`None`); `system` is always present (`{}` for a
 * doc_type whose real data lives entirely in `engine`). `name` is the universal envelope
 * display name, independent of the engine/system split. */
export function envelope(
  worldId: string,
  docType: string,
  parentId: string | null,
  system: unknown,
  id?: string,
  engine?: unknown,
  name: string | null = null,
): WireDocument {
  const now = Date.now();
  return {
    id: id ?? crypto.randomUUID(),
    scope: { kind: "world", world_id: worldId },
    doc_type: docType,
    schema_version: 1,
    name,
    engine,
    source: null,
    owner: null,
    permissions: defaultPermissions(),
    embedded: {},
    parent_id: parentId,
    system,
    created_at: now,
    updated_at: now,
  };
}

/** A top-level scene document with a default square/100 grid and no background.
 * Optional `vision`/`lighting` overrides, `grid.distance`, and `snapToGrid` are included
 * only when provided; absent keys fall back to world-settings defaults at resolution time.
 * `doc_type: "scene"` is engine-defined — the body lands in `engine`, `system` stays `{}`. */
export function buildSceneDoc(worldId: string, engine: Partial<SceneEngine> = {}, id?: string): WireDocument {
  const full: SceneEngine = {
    grid: engine.grid ?? { kind: "square", size: 100, distance: null },
    background: engine.background ?? null,
    bounds: engine.bounds ?? null,
    snapToGrid: engine.snapToGrid ?? null,
    vision: engine.vision ?? null,
    lighting: engine.lighting ?? null,
  };
  return envelope(worldId, "scene", null, {}, id, full, null);
}

/** A top-level (world-scoped, parentless) world-settings config document.
 * Seeds the FULL default object so that a world-settings doc is always complete;
 * single-field edits patch it in place via set_pointer.
 * Default param is a fresh deep clone — the returned doc's `.engine` must not alias
 * DEFAULT_WORLD_SETTINGS (value-independence-at-construction invariant). */
export function buildWorldSettingsDoc(
  worldId: string,
  engine: WorldSettingsEngine = structuredClone(DEFAULT_WORLD_SETTINGS),
  id?: string,
): WireDocument {
  return envelope(worldId, "world-settings", null, {}, id, engine, null);
}

/** Fail-closed bounds resolve: a present-but-malformed bounds (non-finite or ≤ 0 on either
 * axis) falls back to the finite default rather than yielding a degenerate navmesh rectangle. */
function resolveBounds(b: SceneDimensions | null | undefined): SceneDimensions {
  const w = b?.width, h = b?.height;
  if (typeof w === "number" && Number.isFinite(w) && w > 0 &&
      typeof h === "number" && Number.isFinite(h) && h > 0) {
    return { width: w, height: h };
  }
  return DEFAULT_SCENE_BOUNDS;
}

/** Resolve the effective settings for a scene by merging:
 *   built-in defaults → world-settings doc → scene-level overrides.
 * INVARIANT: fail-closed — absent OR structurally incomplete docs fall back to
 * DEFAULT_WORLD_SETTINGS; never throws. A partial wire payload (e.g. a set_pointer
 * that removed a top-level key) is non-null but structurally incomplete, so the `??`
 * guard alone is insufficient; we require all three top-level keys to be present.
 * Default gridDistance: 5 ft/cell (standard D&D 5e scale). */
export function resolveSceneSettings(scene: WireDocument | undefined, store: ReadableDocuments): ResolvedSceneSettings {
  const ws = store.query("world-settings")[0]?.engine as WorldSettingsEngine | undefined;
  // Structural guard: a partial doc (missing scene/pathfinding/animation) falls back to
  // built-in defaults rather than throwing at d.scene.* access below.
  const d = (ws?.scene && ws?.pathfinding && ws?.animation) ? ws : DEFAULT_WORLD_SETTINGS;
  const eng = scene?.engine as SceneEngine | undefined;
  // `vision`/`lighting` are required-but-nullable keys on the generated `SceneEngine`
  // (absent and explicit `null` are wire-equivalent) — read through optional chaining
  // rather than defaulting to `{}`, which no longer structurally satisfies the type.
  const v = eng?.vision;
  const l = eng?.lighting;
  const movementModel = v?.movementModel ?? d.scene.movementModel;
  return {
    losRestriction: v?.losRestriction ?? d.scene.losRestriction,
    fog: v?.fog ?? d.scene.fog,
    observerVision: v?.observerVision ?? d.scene.observerVision,
    movementRestriction: v?.movementRestriction ?? d.scene.movementRestriction,
    movementModel,
    // Derived default keyed off the RESOLVED movementModel (M10f-3 §4.1) — `??` only falls
    // through on null/undefined, never on `false`, so an explicit stored boolean (including
    // false) always overrides the derived default in either direction.
    snapToGrid: eng?.snapToGrid ?? (movementModel === "continuous" ? false : true),
    lightingEnabled: l?.enabled ?? d.scene.lightingEnabled,
    lightMode: l?.mode ?? d.scene.lightMode,
    environment: l?.environment ?? d.scene.environment,
    partialCellLeniency: d.scene.partialCellLeniency,
    diagonalRule: d.pathfinding.diagonalRule,
    animation: d.animation,
    gridDistance: eng?.grid?.distance ?? { perCell: 5, unit: "ft" },
    bounds: resolveBounds(eng?.bounds),
  };
}

/** The single client-side answer to "which scene does THIS client render/subscribe to"
 * (M12d). Resolution order: a resolvable `gmViewedScene` (GM local roam) → a resolvable
 * `world-settings.activeScene` (players follow) → the first scene (legacy). `null` ONLY when
 * no scene exists. Fail-closed by construction: an id that no longer names a scene is ignored
 * (never renders nothing while scenes exist, never leaks a nonexistent scene's channel).
 * Players never pass `gmViewedScene`, so they always follow `activeScene`. */
export function resolveViewedScene(
  store: ReadableDocuments,
  opts: { gmViewedScene?: string | null } = {},
): string | null {
  const scenes = store.query("scene");
  if (scenes.length === 0) return null;
  const exists = (id: string | null | undefined): id is string => !!id && scenes.some((s) => s.id === id);
  if (exists(opts.gmViewedScene)) return opts.gmViewedScene;
  const ws = store.query("world-settings")[0]?.engine as WorldSettingsEngine | undefined;
  if (exists(ws?.activeScene)) return ws!.activeScene;
  return scenes[0].id;
}

/** A top-level (world-scoped, parentless) actor document. `name` is the actor's real,
 * privacy-gateable identity (envelope field); `engine` carries every other engine-owned
 * field (`displayName`, visual, size, shape, faction, conditions, prototype, vision)
 * per `ActorEngine`. */
export function buildActorDoc(worldId: string, name: string | null, engine: ActorEngine, id?: string): WireDocument {
  return envelope(worldId, "actor", null, {}, id, engine, name);
}

/** Client-only `item` doc_type (M12c): NOT engine-defined (`data::engine::is_engine_doc_type`
 * excludes "item") — the server stays fully structural for items, mirroring
 * `movementModel`/`bounds`/`visual`. An item lives standalone (top-level, parentless) or
 * embedded in an actor's inventory. Display name lives in the envelope; every other field
 * is opaque, edited via the tree editor. */
export const ITEM_DOC_TYPE = "item";

export interface ItemSystem {
  [key: string]: unknown;
}

export function buildItemDoc(worldId: string, name: string | null, system: ItemSystem = {}, id?: string): WireDocument {
  return envelope(worldId, ITEM_DOC_TYPE, null, system, id, undefined, name);
}

/** Build a token from an actor. `link` references the shared actor; `instance` embeds an
 * independent copy with `source` provenance (the deferred merge engine consumes it). Size/
 * shape resolve from the actor (M10d); `w`/`h` seed the rendered cell size now.
 * `doc_type: "token"` is engine-defined — the transform/visual/link body lands in `engine`. */
export function buildTokenFromActor(
  worldId: string,
  sceneId: string,
  actor: WireDocument,
  mode: "link" | "instance",
  pos: { x: number; y: number },
  cellSize: number,
  id?: string,
): WireDocument {
  const base: TokenEngine = {
    x: pos.x, y: pos.y, w: cellSize, h: cellSize, rotation: 0,
    visual: null, actor_id: null, overrides: null, face: null,
  };
  if (mode === "link") {
    return envelope(worldId, "token", sceneId, {}, id, { ...base, actor_id: actor.id }, null);
  }
  // Deep-clone so the embedded copy is independent by value at construction (not just after
  // the wire round-trip) — no aliasing of the source actor's name/engine/system/permissions.
  const copy: WireDocument = { ...structuredClone(actor), id: crypto.randomUUID(), source: { id: actor.id, pack: null, version: 1 } };
  const doc = envelope(worldId, "token", sceneId, {}, id, base, null);
  doc.embedded = { actor: [copy] };
  return doc;
}

/** Set/clear the name-privacy override on an actor doc's permissions: hiding declares
 * `/name` (the envelope field) as the `owner_or_gm` tier (the server redacts it to `null`
 * from non-owner players on egress, and retroactively retracts an already-delivered value
 * when the override is added); clearing removes the declaration. Mutates in place +
 * returns `doc`. */
export function setNameHidden(doc: WireDocument, hidden: boolean): WireDocument {
  const overrides = { ...doc.permissions.property_overrides };
  if (hidden) overrides["/name"] = "owner_or_gm";
  else delete overrides["/name"];
  doc.permissions = { ...doc.permissions, property_overrides: overrides };
  return doc;
}

/** A token document parented to `sceneId`, carrying the given transform + visual. */
export function buildTokenDoc(worldId: string, sceneId: string, engine: TokenEngine, id?: string): WireDocument {
  return envelope(worldId, "token", sceneId, {}, id, engine, null);
}

/** A top-level (world-scoped, parentless) faction-registry document.
 * `doc_type: "faction-registry"` is engine-defined. */
export function buildFactionRegistryDoc(worldId: string, factions: Record<string, Faction>, id?: string): WireDocument {
  return envelope(worldId, "faction-registry", null, {}, id, { factions } satisfies FactionRegistryEngine, null);
}

/** A top-level (world-scoped, parentless) condition-registry document.
 * `doc_type: "condition-registry"` is engine-defined. */
export function buildConditionRegistryDoc(worldId: string, conditions: Record<string, Condition>, id?: string): WireDocument {
  return envelope(worldId, "condition-registry", null, {}, id, { conditions } satisfies ConditionRegistryEngine, null);
}

/** A generic scene-entity document (drawing/template/wall/…) parented to `sceneId`; every
 * doc_type this builder is used for (`wall`, `region`, `drawing`, `template`) is
 * engine-defined, so the caller's shape lands in `engine` — `system` stays `{}`. */
export function buildSceneEntityDoc(worldId: string, sceneId: string, docType: string, engine: unknown, id?: string): WireDocument {
  return envelope(worldId, docType, sceneId, {}, id, engine, null);
}

// --- Light-gradation registry (M10e-1) ---

/** Built-in three-band gradation (bright → dim → dark).
 * Stored unsorted; `resolveGradation` returns a sorted copy.
 * Deep-frozen so shared refs returned by resolveGradation cannot be mutated by consumers. */
export const DEFAULT_GRADATION: LightGradationEngine = deepFreeze({
  bands: [
    { name: "bright", minIllumination: 0.67 },
    { name: "dim", minIllumination: 0.34 },
    { name: "dark", minIllumination: 0.0 },
  ],
});

/** A top-level (world-scoped, parentless) light-gradation config document.
 * Default param is a fresh deep clone — the returned doc's `.engine` must not alias
 * DEFAULT_GRADATION (value-independence-at-construction invariant). */
export function buildLightGradationDoc(worldId: string, engine: LightGradationEngine = structuredClone(DEFAULT_GRADATION), id?: string): WireDocument {
  return envelope(worldId, "light-gradation", null, {}, id, engine, null);
}

/** Returns bands sorted brightest-first (descending `minIllumination`) so a consumer
 * can walk the array and pick the first band whose floor a cell's illumination meets.
 * Fail-closed: absent or malformed doc falls back to DEFAULT_GRADATION; never throws. */
export function resolveGradation(store: ReadableDocuments): GradationBand[] {
  const eng = store.query("light-gradation")[0]?.engine as LightGradationEngine | undefined;
  const bands = eng?.bands ?? DEFAULT_GRADATION.bands;
  return [...bands].sort((a, b) => b.minIllumination - a.minIllumination);
}

// --- Vision-modes registry (M10e-1) ---

/** Built-in two-mode seed: normal sight + darkvision.
 * Deep-frozen so shared refs returned by resolveVisionModes cannot be mutated by consumers. */
export const SEED_VISION_MODES: Record<string, VisionMode> = deepFreeze({
  normal: { id: "normal", name: "Normal", illuminationFloor: "dim", defaultRange: 0, renderHint: null },
  darkvision: { id: "darkvision", name: "Darkvision", illuminationFloor: "dark", defaultRange: 12, renderHint: "desaturate" },
});

/** A top-level (world-scoped, parentless) vision-modes config document.
 * Default param is a fresh deep clone — the returned doc's `.engine.modes` must not alias
 * SEED_VISION_MODES (value-independence-at-construction invariant). */
export function buildVisionModesDoc(worldId: string, engine: VisionModesEngine = { modes: structuredClone(SEED_VISION_MODES) }, id?: string): WireDocument {
  return envelope(worldId, "vision-modes", null, {}, id, engine, null);
}

/** Returns the effective vision-mode map.
 * Fail-closed: absent or malformed doc falls back to SEED_VISION_MODES; never throws. */
export function resolveVisionModes(store: ReadableDocuments): Record<string, VisionMode> {
  const eng = store.query("vision-modes")[0]?.engine as VisionModesEngine | undefined;
  return eng?.modes ?? SEED_VISION_MODES;
}

// --- Light source doc type (M10e-1) ---

/** A light-source document parented to `sceneId`. The caller supplies the full `engine`
 * body (no default constant — no aliasing concern). `doc_type: "light"` is engine-defined. */
export function buildLightDoc(worldId: string, sceneId: string, engine: LightEngine, id?: string): WireDocument {
  return envelope(worldId, "light", sceneId, {}, id, engine, null);
}

// --- Region doc type (M10g) ---

/** A region document parented to `sceneId`. Visible to all by default; use
 * `setRegionVisibility` to make it a secret trap. `doc_type: "region"` is engine-defined. */
export function buildRegionDoc(worldId: string, sceneId: string, engine: RegionEngine, id?: string): WireDocument {
  return envelope(worldId, "region", sceneId, {}, id, engine, null);
}

/** Set/clear the secrecy override on a region doc's permissions. Hiding sets BOTH:
 * (1) `permissions.default = "none"` — denies whole-document `READ` to any non-owner/non-GM
 *     recipient, so the server's `filter_command` drops the region's `Create` op ENTIRELY for
 *     them (never delivered, not even nulled) — this is what actually keeps a hidden region's
 *     existence, id, and shape out of a player's `documents` store;
 * (2) `property_overrides["/engine"] = "gm_only"` — defense-in-depth: nulls the `/engine` body
 *     (the region's shape/behavior/cost lives in `engine`, not `/system`) if a future
 *     capability grant ever widens `default`/`users` without revisiting secrecy.
 * Un-hiding restores `default = "observer"` (the original `envelope()` default) and clears the
 * override. Mutates in place + returns `doc`. */
export function setRegionVisibility(doc: WireDocument, hidden: boolean): WireDocument {
  const overrides = { ...doc.permissions.property_overrides };
  if (hidden) overrides["/engine"] = "gm_only";
  else delete overrides["/engine"];
  doc.permissions = {
    ...doc.permissions,
    default: hidden ? "none" : "observer",
    property_overrides: overrides,
  };
  return doc;
}
