// Client-owned scene-entity `system` shapes + pure document builders. The server
// stays structural-only (#6); these shapes are the client's interpretation of the
// opaque `system` body. Shared by WorldSession (scene auto-create) and the
// scene-tools module (token create) so the wire shape has one source of truth.
export type { WireDocument } from "./wire";
import type { WireDocument } from "./wire";
import type { ReadableDocuments } from "./store";

// --- Vision / lighting / movement types (M10e-1) ---

export type MovementRestriction = "visible" | "revealed" | "unrestricted";
export type LightMode = "globalIllumination" | "environmentLight";
export type DiagonalRule = "chebyshev" | "alternating" | "euclidean" | "manhattan";
export type EasingMode = "easeInOut" | "linear";
export interface EnvironmentLight { color: string; intensity: number; }
/** Distance-per-cell scale for a scene grid. `unit` is a display label (e.g. "ft", "m"). */
export interface GridDistance { perCell: number; unit: string; }

/** Per-scene overrides for vision behaviour; absent fields fall back to world defaults. */
export interface SceneVisionOverrides {
  losRestriction?: boolean;
  fog?: boolean;
  observerVision?: boolean;
  movementRestriction?: MovementRestriction;
}
/** Per-scene overrides for lighting; absent fields fall back to world defaults. */
export interface SceneLightingOverrides {
  enabled?: boolean;
  mode?: LightMode;
  environment?: EnvironmentLight;
}

/** A scene's engine-owned config (M8d §15, extended M10e-1). Dimensions deferred (canvas pans freely). */
export interface SceneSystem {
  grid: { kind: "square" | "hex"; size: number; distance?: GridDistance };
  background: string | null;
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
  partialCellLeniency: boolean;
}
/** The `system` body of a "world-settings" config document. */
export interface WorldSettingsSystem {
  scene: WorldSceneDefaults;
  pathfinding: { diagonalRule: DiagonalRule };
  animation: { speedCellsPerSec: number; easing: EasingMode };
}

// Recursive freeze helper — makes DEFAULT_WORLD_SETTINGS immutable so shared refs
// returned by resolveSceneSettings cannot be mutated by consumers in dev.
function deepFreeze<T>(obj: T): T {
  Object.freeze(obj);
  for (const v of Object.values(obj as object)) {
    if (v !== null && typeof v === "object" && !Object.isFrozen(v)) deepFreeze(v);
  }
  return obj;
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
  lightingEnabled: boolean;
  lightMode: LightMode;
  environment: EnvironmentLight;
  partialCellLeniency: boolean;
  diagonalRule: DiagonalRule;
  animation: { speedCellsPerSec: number; easing: EasingMode };
  gridDistance: GridDistance;
}

/** A token's transform + visual (M8d §4). `(x,y)` is the token CENTER. `visual` is the
 * forward-looking seam — only `kind:"image"` ships in M8d. */
export interface TokenSystem {
  x: number;
  y: number;
  w: number;
  h: number;
  rotation: number;
  /** Set on raw (actorless) tokens; actor-backed tokens resolve their visual via the actor. */
  visual?: { kind: "image"; asset: string };
  /** Linked token: the shared actor's id (null/absent ⇒ instanced, see `embedded.actor`). */
  actor_id?: string | null;
  /** Linked-only per-token override whitelist (see {@link TokenOverrides}). */
  overrides?: TokenOverrides;
}

/** An actor's appearance + defaults (M10a). Stats/sheet are M12; this is only what backs a
 * token. The server is structural-only — this `system` shape is the client's interpretation. */
export interface ActorVisual {
  kind: "image";
  asset: string;
}

/** A per-actor or per-token vision assignment: which mode (by id) + effective range in grid cells.
 * References a VisionMode in the world's vision-modes registry by id. */
export interface VisionAssignment { mode: string; range: number; }

export interface ActorSystem {
  name: string;
  displayName: string;
  visual: ActorVisual;
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
  visual?: ActorVisual;
  size?: { w: number; h: number };
  shape?: "square" | "circle";
  /** Per-token vision override: replaces the actor's vision[] entirely when present. */
  vision?: VisionAssignment[];
}

/** Visible-to-all defaults; the server normalizes permissions per the creator's role. */
function defaultPermissions(): WireDocument["permissions"] {
  return { default: "observer", users: {}, property_overrides: {}, capabilities: { by_role: {}, by_user: {} } };
}

function envelope(worldId: string, docType: string, parentId: string | null, system: unknown, id?: string): WireDocument {
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
    ...(system.vision ? { vision: system.vision } : {}),
    ...(system.lighting ? { lighting: system.lighting } : {}),
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
export function resolveSceneSettings(scene: WireDocument | undefined, store: ReadableDocuments): ResolvedSceneSettings {
  const ws = store.query("world-settings")[0]?.system as WorldSettingsSystem | undefined;
  // Structural guard: a partial doc (missing scene/pathfinding/animation) falls back to
  // built-in defaults rather than throwing at d.scene.* access below.
  const d = (ws?.scene && ws?.pathfinding && ws?.animation) ? ws : DEFAULT_WORLD_SETTINGS;
  const sys = scene?.system as SceneSystem | undefined;
  const v = sys?.vision ?? {};
  const l = sys?.lighting ?? {};
  return {
    losRestriction: v.losRestriction ?? d.scene.losRestriction,
    fog: v.fog ?? d.scene.fog,
    observerVision: v.observerVision ?? d.scene.observerVision,
    movementRestriction: v.movementRestriction ?? d.scene.movementRestriction,
    lightingEnabled: l.enabled ?? d.scene.lightingEnabled,
    lightMode: l.mode ?? d.scene.lightMode,
    environment: l.environment ?? d.scene.environment,
    partialCellLeniency: d.scene.partialCellLeniency,
    diagonalRule: d.pathfinding.diagonalRule,
    animation: d.animation,
    gridDistance: sys?.grid?.distance ?? { perCell: 5, unit: "ft" },
  };
}

/** A top-level (world-scoped, parentless) actor document. */
export function buildActorDoc(worldId: string, system: ActorSystem, id?: string): WireDocument {
  return envelope(worldId, "actor", null, system, id);
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
