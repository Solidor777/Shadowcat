// Client-owned scene-entity `system` shapes + pure document builders. The server
// stays structural-only (#6); these shapes are the client's interpretation of the
// opaque `system` body. Shared by WorldSession (scene auto-create) and the
// scene-tools module (token create) so the wire shape has one source of truth.
export type { WireDocument } from "./wire";
import type { WireDocument } from "./wire";
import type { ReadableDocuments } from "./store";

// --- Vision / lighting / movement types (M10e-1) ---

export type MovementRestriction = "visible" | "revealed" | "unrestricted";
/** Per-scene pathfinding engine choice (M10f-1). `grid-stepped` = the existing grid A* router;
 * `continuous` = the M10f polyanya navmesh router, executed end-to-end since M10f-3
 * (server-authoritative gated movement, same as grid-stepped). */
export type MovementModel = "grid-stepped" | "continuous";
export type LightMode = "globalIllumination" | "environmentLight";
export type DiagonalRule = "chebyshev" | "alternating" | "euclidean" | "manhattan";
export type EasingMode = "easeInOut" | "linear";
export interface EnvironmentLight { color: string; intensity: number; }
/** Distance-per-cell scale for a scene grid. `unit` is a display label (e.g. "ft", "m"). */
export interface GridDistance { perCell: number; unit: string; }

/** A scene's authored dimensions in GRID UNITS (width × height cells). The M10f navmesh
 * triangulates this bounded rectangle; grid A* never needs it. Absent ⇒ DEFAULT_SCENE_BOUNDS. */
export interface SceneDimensions { width: number; height: number; }

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

/** Per-scene overrides for vision behaviour; absent fields fall back to world defaults.
 * Null is a valid wire value: the UI writes null to clear an override; resolveSceneSettings
 * uses ?? so null and undefined both fall through to the world default. */
export interface SceneVisionOverrides {
  losRestriction?: boolean | null;
  fog?: boolean | null;
  observerVision?: boolean | null;
  movementRestriction?: MovementRestriction | null;
  movementModel?: MovementModel | null;
}
/** Per-scene overrides for lighting; absent fields fall back to world defaults.
 * Null is a valid wire value for the same reason as SceneVisionOverrides. */
export interface SceneLightingOverrides {
  enabled?: boolean | null;
  mode?: LightMode | null;
  environment?: EnvironmentLight | null;
}

/** A scene's engine-owned config (M8d §15, extended M10e-1). `bounds` (M10f-0) = the navmesh's
 * outer rectangle in grid units; absent ⇒ DEFAULT_SCENE_BOUNDS. */
export interface SceneSystem {
  grid: { kind: "square" | "hex"; size: number; distance?: GridDistance };
  background: string | null;
  bounds?: SceneDimensions;
  /** Scene-level snap-to-grid toggle (M10f-3 §4.1), independent of `movementModel`. Absent ⇒
   * derived default resolved in `resolveSceneSettings` (false for a continuous scene, true
   * otherwise) — reading this field alone is NOT the effective value. */
  snapToGrid?: boolean;
  vision?: SceneVisionOverrides;
  lighting?: SceneLightingOverrides;
}

// --- World-settings doc types (M10e-1) ---

/** The full set of world-level scene defaults that individual scenes may override. */
export interface WorldSceneDefaults {
  losRestriction: boolean;
  fog: boolean;
  lightingEnabled: boolean;
  lightMode: LightMode;
  environment: EnvironmentLight;
  observerVision: boolean;
  movementRestriction: MovementRestriction;
  movementModel: MovementModel;
  partialCellLeniency: boolean;
}
/** The `system` body of a "world-settings" config document. */
export interface WorldSettingsSystem {
  scene: WorldSceneDefaults;
  pathfinding: { diagonalRule: DiagonalRule };
  animation: { speedCellsPerSec: number; easing: EasingMode };
}

/** Built-in defaults — used when no world-settings doc exists or a field is absent.
 * Deep-frozen so shared refs in resolveSceneSettings output are immutable in dev;
 * enumerable values are unchanged. */
export const DEFAULT_WORLD_SETTINGS: WorldSettingsSystem = deepFreeze({
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

/** A token's transform + visual (M8d §4). `(x,y)` is the token CENTER. `visual` is the
 * forward-looking seam (M10h: image, animated, or multi-face union — see `TokenVisual`). */
export interface TokenSystem {
  x: number;
  y: number;
  w: number;
  h: number;
  rotation: number;
  /** Set on raw (actorless) tokens; actor-backed tokens resolve their visual via the actor. */
  visual?: TokenVisual;
  /** Linked token: the shared actor's id (null/absent ⇒ instanced, see `embedded.actor`). */
  actor_id?: string | null;
  /** Linked-only per-token override whitelist (see {@link TokenOverrides}). */
  overrides?: TokenOverrides;
  /** Active face name when the effective visual is a `faces` union member (M10h); token-local
   * always (not part of `overrides` — it selects INTO the actor's faces map, not an override
   * of actor-data). Ignored when the effective visual isn't `faces`. */
  face?: string;
}

/** The two kinds the render layer actually draws — the render/resolution boundary (M10h). */
export type RenderVisual =
  | { kind: "image"; asset: string }
  | { kind: "animated"; source: AnimatedSource; fps: number; loop: boolean };

/** An animated visual's frame source: an ordered list of individually-uploaded assets, or one
 * grid-sliced sheet asset. No packed-atlas-JSON format yet (M10h design spec §7). */
export type AnimatedSource =
  | { type: "frames"; frames: string[] }
  | { type: "sheet"; asset: string; rows: number; cols: number; count?: number };

/** A face's own visual. Deliberately never itself `{kind:"faces"}` — no nesting — so an animated
 * face falls out of the same RenderVisual boundary with no separate mechanism. */
export type FaceVisual = RenderVisual;

/** An actor's (or a linked token's override) declared visual: a plain RenderVisual, or a
 * multi-face union resolved per-token by `resolveTokenVisual` (M10h). Client-owned, opaque
 * `system`-body JSON — no ts-rs type, no server change (mirrors `movementModel`/`bounds`). */
export type TokenVisual =
  | RenderVisual
  | {
      kind: "faces";
      faces: Record<string, FaceVisual>;
      default: string;
      /** Optional conditionId -> face name map; the first match (in the token's effective
       * `conditions[]` order) wins over `default`, but never over a manual `token.system.face`. */
      faceMap?: Record<string, string>;
    };

/** A per-actor or per-token vision assignment: which mode (by id) + effective range in grid cells.
 * References a VisionMode in the world's vision-modes registry by id. */
export interface VisionAssignment { mode: string; range: number; }

export interface ActorSystem {
  name: string;
  displayName: string;
  visual: TokenVisual;
  size: { w: number; h: number };
  shape: "square" | "circle";
  faction: string | null;
  conditions: string[];
  /** Default place-mode: true ⇒ instance (independent copy) on drop; false ⇒ link (shared). */
  prototype: boolean;
  /** Vision modes granted to this actor; each references a VisionMode id + range in grid cells. */
  vision?: VisionAssignment[];
}

/** The per-token override whitelist for a linked token (M10a; shape added M10d). */
export interface TokenOverrides {
  name?: string;
  visual?: TokenVisual;
  size?: { w: number; h: number };
  shape?: "square" | "circle";
  /** Per-token vision override: replaces the actor's vision[] entirely when present. */
  vision?: VisionAssignment[];
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

/** Package-internal document envelope builder (shared by scene-docs and chat-docs). */
export function envelope(worldId: string, docType: string, parentId: string | null, system: unknown, id?: string): WireDocument {
  const now = Date.now();
  return {
    id: id ?? crypto.randomUUID(),
    scope: { kind: "world", world_id: worldId },
    doc_type: docType,
    schema_version: 1,
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
 * Optional `vision`/`lighting` overrides and `grid.distance` are included only when provided;
 * absent keys fall back to world-settings defaults at resolution time. */
export function buildSceneDoc(worldId: string, system: Partial<SceneSystem> = {}, id?: string): WireDocument {
  const full: SceneSystem = {
    grid: system.grid ?? { kind: "square", size: 100 },
    background: system.background ?? null,
    ...(system.bounds ? { bounds: system.bounds } : {}),
    ...(system.vision ? { vision: system.vision } : {}),
    ...(system.lighting ? { lighting: system.lighting } : {}),
    // Explicit undefined-check (not a truthy check) — false is a meaningful, persistable
    // value (M10f-3 §4.1); `system.snapToGrid ? ... : {}` would silently drop an explicit false.
    ...(system.snapToGrid !== undefined ? { snapToGrid: system.snapToGrid } : {}),
  };
  return envelope(worldId, "scene", null, full, id);
}

/** A top-level (world-scoped, parentless) world-settings config document.
 * Seeds the FULL default object so that a world-settings doc is always complete;
 * single-field edits patch it in place via set_pointer.
 * Default param is a fresh deep clone — the returned doc's `.system` must not alias
 * DEFAULT_WORLD_SETTINGS (value-independence-at-construction invariant). */
export function buildWorldSettingsDoc(worldId: string, system: WorldSettingsSystem = structuredClone(DEFAULT_WORLD_SETTINGS), id?: string): WireDocument {
  return envelope(worldId, "world-settings", null, system, id);
}

/** Resolve the effective settings for a scene by merging:
 *   built-in defaults → world-settings doc → scene-level overrides.
 * INVARIANT: fail-closed — absent OR structurally incomplete docs fall back to
 * DEFAULT_WORLD_SETTINGS; never throws. A partial wire payload (e.g. a set_pointer
 * that removed a top-level key) is non-null but structurally incomplete, so the `??`
 * guard alone is insufficient; we require all three top-level keys to be present.
 * Default gridDistance: 5 ft/cell (standard D&D 5e scale). */
/** Fail-closed bounds resolve: a present-but-malformed bounds (non-finite or ≤ 0 on either
 * axis) falls back to the finite default rather than yielding a degenerate navmesh rectangle. */
function resolveBounds(b: SceneDimensions | undefined): SceneDimensions {
  const w = b?.width, h = b?.height;
  if (typeof w === "number" && Number.isFinite(w) && w > 0 &&
      typeof h === "number" && Number.isFinite(h) && h > 0) {
    return { width: w, height: h };
  }
  return DEFAULT_SCENE_BOUNDS;
}

export function resolveSceneSettings(scene: WireDocument | undefined, store: ReadableDocuments): ResolvedSceneSettings {
  const ws = store.query("world-settings")[0]?.system as WorldSettingsSystem | undefined;
  // Structural guard: a partial doc (missing scene/pathfinding/animation) falls back to
  // built-in defaults rather than throwing at d.scene.* access below.
  const d = (ws?.scene && ws?.pathfinding && ws?.animation) ? ws : DEFAULT_WORLD_SETTINGS;
  const sys = scene?.system as SceneSystem | undefined;
  const v = sys?.vision ?? {};
  const l = sys?.lighting ?? {};
  const movementModel = v.movementModel ?? d.scene.movementModel;
  return {
    losRestriction: v.losRestriction ?? d.scene.losRestriction,
    fog: v.fog ?? d.scene.fog,
    observerVision: v.observerVision ?? d.scene.observerVision,
    movementRestriction: v.movementRestriction ?? d.scene.movementRestriction,
    movementModel,
    // Derived default keyed off the RESOLVED movementModel (M10f-3 §4.1) — `??` only falls
    // through on null/undefined, never on `false`, so an explicit stored boolean (including
    // false) always overrides the derived default in either direction.
    snapToGrid: sys?.snapToGrid ?? (movementModel === "continuous" ? false : true),
    lightingEnabled: l.enabled ?? d.scene.lightingEnabled,
    lightMode: l.mode ?? d.scene.lightMode,
    environment: l.environment ?? d.scene.environment,
    partialCellLeniency: d.scene.partialCellLeniency,
    diagonalRule: d.pathfinding.diagonalRule,
    animation: d.animation,
    gridDistance: sys?.grid?.distance ?? { perCell: 5, unit: "ft" },
    bounds: resolveBounds(sys?.bounds),
  };
}

/** A top-level (world-scoped, parentless) actor document. */
export function buildActorDoc(worldId: string, system: ActorSystem, id?: string): WireDocument {
  return envelope(worldId, "actor", null, system, id);
}

/** Client-only `item` doc_type (M12c): the server stays structural — no server change,
 * mirroring `movementModel`/`bounds`/`visual`. An item lives standalone (top-level,
 * parentless) or embedded in an actor's inventory. `system.name` is the only shape the
 * item sheet requires; every other field is opaque, edited via the tree editor. */
export const ITEM_DOC_TYPE = "item";

export interface ItemSystem {
  name: string;
  [key: string]: unknown;
}

export function buildItemDoc(worldId: string, system: ItemSystem, id?: string): WireDocument {
  return envelope(worldId, ITEM_DOC_TYPE, null, system, id);
}

/** Build a token from an actor. `link` references the shared actor; `instance` embeds an
 * independent copy with `source` provenance (the deferred merge engine consumes it). Size/
 * shape resolve from the actor (M10d); `w`/`h` seed the rendered cell size now. */
export function buildTokenFromActor(
  worldId: string,
  sceneId: string,
  actor: WireDocument,
  mode: "link" | "instance",
  pos: { x: number; y: number },
  cellSize: number,
  id?: string,
): WireDocument {
  const base: TokenSystem = { x: pos.x, y: pos.y, w: cellSize, h: cellSize, rotation: 0 };
  if (mode === "link") {
    return envelope(worldId, "token", sceneId, { ...base, actor_id: actor.id, overrides: {} }, id);
  }
  // Deep-clone so the embedded copy is independent by value at construction (not just after
  // the wire round-trip) — no aliasing of the source actor's system/permissions/embedded.
  const copy: WireDocument = { ...structuredClone(actor), id: crypto.randomUUID(), source: { id: actor.id, pack: null, version: 1 } };
  const doc = envelope(worldId, "token", sceneId, base, id);
  doc.embedded = { actor: [copy] };
  return doc;
}

/** Set/clear the name-privacy override on an actor doc's permissions: hiding declares
 * `/system/name` as the `owner_or_gm` tier (the server redacts it from non-owner players on
 * egress, and retroactively retracts an already-delivered value when the override is added);
 * clearing removes the declaration. Mutates in place + returns `doc`. */
export function setNameHidden(doc: WireDocument, hidden: boolean): WireDocument {
  const overrides = { ...doc.permissions.property_overrides };
  if (hidden) overrides["/system/name"] = "owner_or_gm";
  else delete overrides["/system/name"];
  doc.permissions = { ...doc.permissions, property_overrides: overrides };
  return doc;
}

/** A token document parented to `sceneId`, carrying the given transform + visual. */
export function buildTokenDoc(worldId: string, sceneId: string, system: TokenSystem, id?: string): WireDocument {
  return envelope(worldId, "token", sceneId, system, id);
}

/** A faction's display + stance. `color` is "#rrggbb" (the token border color); `stance` is
 * reserved for later combat/targeting/vision (present now to avoid a migration). */
export type FactionStance = "friendly" | "neutral" | "hostile";
export interface Faction {
  name: string;
  color: string;
  stance: FactionStance;
}

/** The world's faction registry: a singleton config document (doc_type "faction-registry").
 * `factions` is keyed by faction id — an actor's `faction` field references a key. A MAP, not
 * an array, so adding a faction is a single-key Update (`set_pointer` cannot grow arrays). */
export interface FactionRegistrySystem {
  factions: Record<string, Faction>;
}

/** A top-level (world-scoped, parentless) faction-registry document. */
export function buildFactionRegistryDoc(worldId: string, factions: Record<string, Faction>, id?: string): WireDocument {
  return envelope(worldId, "faction-registry", null, { factions } satisfies FactionRegistrySystem, id);
}

/** A status condition's display. `icon` is a short glyph (emoji) rendered as a token badge. */
export interface Condition {
  name: string;
  icon: string;
}

/** The world's condition registry: a singleton config document (doc_type "condition-registry").
 * `conditions` is keyed by condition id — an actor's `conditions` array holds keys. A MAP, not an
 * array, so adding a condition is a single-key Update (`set_pointer` cannot grow arrays). */
export interface ConditionRegistrySystem {
  conditions: Record<string, Condition>;
}

/** A top-level (world-scoped, parentless) condition-registry document. */
export function buildConditionRegistryDoc(worldId: string, conditions: Record<string, Condition>, id?: string): WireDocument {
  return envelope(worldId, "condition-registry", null, { conditions } satisfies ConditionRegistrySystem, id);
}

/** A generic scene-entity document (drawing/template/…) parented to `sceneId`; the
 * `system` shape is the caller's (client-owned, server structural-only). */
export function buildSceneEntityDoc(worldId: string, sceneId: string, docType: string, system: unknown, id?: string): WireDocument {
  return envelope(worldId, docType, sceneId, system, id);
}

// --- Light-gradation registry (M10e-1) ---

/** A named illumination band. `minIllumination` is the minimum light level [0,1]
 * a cell must reach to qualify; bands are sorted brightest-first at resolution time. */
export interface GradationBand { name: string; minIllumination: number; }

/** The `system` body of a "light-gradation" config document. */
export interface LightGradationSystem { bands: GradationBand[]; }

/** Built-in three-band gradation (bright → dim → dark).
 * Stored unsorted; `resolveGradation` returns a sorted copy.
 * Deep-frozen so shared refs returned by resolveGradation cannot be mutated by consumers. */
export const DEFAULT_GRADATION: LightGradationSystem = deepFreeze({
  bands: [
    { name: "bright", minIllumination: 0.67 },
    { name: "dim", minIllumination: 0.34 },
    { name: "dark", minIllumination: 0.0 },
  ],
});

/** A top-level (world-scoped, parentless) light-gradation config document.
 * Default param is a fresh deep clone — the returned doc's `.system` must not alias
 * DEFAULT_GRADATION (value-independence-at-construction invariant). */
export function buildLightGradationDoc(worldId: string, system: LightGradationSystem = structuredClone(DEFAULT_GRADATION), id?: string): WireDocument {
  return envelope(worldId, "light-gradation", null, system, id);
}

/** Returns bands sorted brightest-first (descending `minIllumination`) so a consumer
 * can walk the array and pick the first band whose floor a cell's illumination meets.
 * Fail-closed: absent or malformed doc falls back to DEFAULT_GRADATION; never throws. */
export function resolveGradation(store: ReadableDocuments): GradationBand[] {
  const sys = store.query("light-gradation")[0]?.system as LightGradationSystem | undefined;
  const bands = sys?.bands ?? DEFAULT_GRADATION.bands;
  return [...bands].sort((a, b) => b.minIllumination - a.minIllumination);
}

// --- Vision-modes registry (M10e-1) ---

/** A named vision mode that tokens/actors may possess.
 * `illuminationFloor`: the lowest gradation band name a token with this mode can see into.
 * `defaultRange`: effective sight distance in cells (0 = unlimited for normal vision).
 * `renderHint`: optional render-layer instruction (e.g. "desaturate" for darkvision). */
export interface VisionMode {
  id: string;
  name: string;
  illuminationFloor: string;
  defaultRange: number;
  renderHint?: string;
}

/** The `system` body of a "vision-modes" config document. */
export interface VisionModesSystem { modes: Record<string, VisionMode>; }

/** Built-in two-mode seed: normal sight + darkvision.
 * Deep-frozen so shared refs returned by resolveVisionModes cannot be mutated by consumers. */
export const SEED_VISION_MODES: Record<string, VisionMode> = deepFreeze({
  normal: { id: "normal", name: "Normal", illuminationFloor: "dim", defaultRange: 0 },
  darkvision: { id: "darkvision", name: "Darkvision", illuminationFloor: "dark", defaultRange: 12, renderHint: "desaturate" },
});

/** A top-level (world-scoped, parentless) vision-modes config document.
 * Default param is a fresh deep clone — the returned doc's `.system.modes` must not alias
 * SEED_VISION_MODES (value-independence-at-construction invariant). */
export function buildVisionModesDoc(worldId: string, system: VisionModesSystem = { modes: structuredClone(SEED_VISION_MODES) }, id?: string): WireDocument {
  return envelope(worldId, "vision-modes", null, system, id);
}

/** Returns the effective vision-mode map.
 * Fail-closed: absent or malformed doc falls back to SEED_VISION_MODES; never throws. */
export function resolveVisionModes(store: ReadableDocuments): Record<string, VisionMode> {
  const sys = store.query("vision-modes")[0]?.system as VisionModesSystem | undefined;
  return sys?.modes ?? SEED_VISION_MODES;
}

// --- Light source doc type (M10e-1) ---

/** A placed light source: position, photometric properties, and an optional falloff curve.
 * `brightRadius`/`dimRadius` are in grid cells. `falloff.curve` defaults to "linear" when
 * absent. The server treats this system body as opaque; the render layer reads it for
 * illumination computation. */
export interface LightSystem {
  x: number;
  y: number;
  color: string;
  intensity: number;
  brightRadius: number;
  dimRadius: number;
  falloff?: { curve: "linear" | "quadratic" | "none" };
  enabled: boolean;
}

/** A light-source document parented to `sceneId`. The caller supplies the full `system`
 * (no default constant — no aliasing concern). */
export function buildLightDoc(worldId: string, sceneId: string, system: LightSystem, id?: string): WireDocument {
  return envelope(worldId, "light", sceneId, system, id);
}

// --- Region doc type (M10g) ---

export type RegionShapeKind = "rect" | "circle" | "polygon";

/** A region's vector geometry (M8d-3a shape vocabulary). `points` layout by kind:
 *  rect: [x0,y0,x1,y1]; circle: [cx,cy,r]; polygon: [x0,y0,x1,y1,...] (>=3 vertices, even length). */
export interface RegionShape {
  kind: RegionShapeKind;
  points: number[];
}

export type RegionBehavior = "terrain" | "impassable" | "arrest";

/** A region document: a vector-shaped zone that weights, blocks, or arrests grid movement
 * (M10g). `cost` is a multiplier (>=1) meaningful only for `behavior:"terrain"`. `enabled`
 * lets a GM toggle a region off without deleting it (server drops disabled regions entirely). */
export interface RegionSystem {
  shape: RegionShape;
  behavior: RegionBehavior;
  cost: number;
  enabled: boolean;
}

/** A region document parented to `sceneId`. Visible to all by default; use
 * `setRegionVisibility` to make it a secret trap. */
export function buildRegionDoc(worldId: string, sceneId: string, system: RegionSystem, id?: string): WireDocument {
  return envelope(worldId, "region", sceneId, system, id);
}

/** Set/clear the secrecy override on a region doc's permissions. Hiding sets BOTH:
 * (1) `permissions.default = "none"` — denies whole-document `READ` to any non-owner/non-GM
 *     recipient, so the server's `filter_command` drops the region's `Create` op ENTIRELY for
 *     them (never delivered, not even nulled) — this is what actually keeps a hidden region's
 *     existence, id, and shape out of a player's `documents` store;
 * (2) `property_overrides["/system"] = "gm_only"` — defense-in-depth: nulls the `/system` body
 *     if a future capability grant ever widens `default`/`users` without revisiting secrecy.
 * Un-hiding restores `default = "observer"` (the original `envelope()` default) and clears the
 * override. Mutates in place + returns `doc`. */
export function setRegionVisibility(doc: WireDocument, hidden: boolean): WireDocument {
  const overrides = { ...doc.permissions.property_overrides };
  if (hidden) overrides["/system"] = "gm_only";
  else delete overrides["/system"];
  doc.permissions = {
    ...doc.permissions,
    default: hidden ? "none" : "observer",
    property_overrides: overrides,
  };
  return doc;
}
