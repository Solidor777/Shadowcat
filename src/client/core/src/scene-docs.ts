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
  DrawingEngine,
  DrawingShape,
  TemplateEngine,
  TemplateShape,
  Stroke,
  Fill,
  WallEngine,
  Seg,
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
  DrawingEngine,
  DrawingShape,
  TemplateEngine,
  TemplateShape,
  Stroke,
  Fill,
  WallEngine,
  Seg,
};

/** A face's own visual. Deliberately never itself `{kind:"faces"}` — no nesting — so an
 * animated face falls out of the same `RenderVisual` boundary with no separate mechanism.
 * Client-only alias (ts-rs does not export this — it is structurally `RenderVisual`). */
export type FaceVisual = RenderVisual;

/** Client-only convenience literal narrowing `RegionShape.kind`/`RegionEngine.behavior`
 * callers may cast to; the generated fields are plain `string` (asserted by the server's
 * unit battery, not enforced by a Rust enum). */
export type RegionShapeKind = "rect" | "circle" | "polygon";
/** Server-enforced movement meaning composed from
 * `RegionBehavior`/`RegionEffect`: `"impassable"` blocks the router and the move gate
 * (`RegionField::is_impassable`); `"arrest"` truncates a route at that
 * cell (`RegionField::is_arrest`) without blocking it; `"terrain"` only
 * weights pathfinding cost and never blocks or truncates a route. */
export type RegionBehavior = "terrain" | "impassable" | "arrest";

/** Recursively `Object.freeze`s `obj` and every nested plain object reachable from it, so a
 * shared default constant (e.g. `DEFAULT_SCENE_BOUNDS`) cannot be mutated in place by a
 * consumer holding a reference to it. Internal helper — not exported from the package.
 * @param obj The value to freeze in place.
 * @returns `obj`, now recursively frozen (same reference, not a copy).
 * @example
 * ```
 * const frozen = deepFreeze({ nested: { a: 1 } });
 * Object.isFrozen(frozen.nested); // true
 * ```
 */
function deepFreeze<T>(obj: T): T {
  Object.freeze(obj);
  for (const v of Object.values(obj as object)) {
    if (v !== null && typeof v === "object" && !Object.isFrozen(v)) deepFreeze(v);
  }
  return obj;
}

/** Fail-safe finite default scene size (grid units) when a scene has no authored `bounds`, so
 * navmesh construction never faces an unbounded plane. MUST match the server's
 * `scene::DEFAULT_SCENE_BOUNDS_UNITS`. Deliberately a fixed constant — NOT a content AABB (content-derived
 * bounds were rejected: edge-drag re-mesh churn, ill-defined for open scenes). */
export const DEFAULT_SCENE_BOUNDS: SceneDimensions = deepFreeze({ width: 100, height: 100 });

/** Built-in defaults — used when no world-settings doc exists or a field is absent.
 * Deep-frozen so shared refs in resolveSceneSettings output are immutable in dev;
 * enumerable values are unchanged. MUST equal the server's
 * `data::engine::scene::WorldSettingsEngine`'s `Default` impl — the client stays the authoritative
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

// --- Resolved settings ---

/** The fully resolved per-scene settings after merging built-ins → world defaults → scene overrides. */
export interface ResolvedSceneSettings {
  /** Whether line-of-sight gates visibility on this scene; scene `vision` override, else the
   * world default. */
  losRestriction: boolean;
  /** Whether fog-of-war hides unexplored areas on this scene; scene `vision` override, else the
   * world default. */
  fog: boolean;
  /** Whether a player also sees through their observed tokens' own vision (observer mode);
   * scene `vision` override, else the world default. */
  observerVision: boolean;
  /** How movement is gated by visibility on this scene; scene `vision` override, else the world
   * default. */
  movementRestriction: MovementRestriction;
  /** The movement/routing engine this scene uses (grid vs continuous); scene `vision` override,
   * else the world default. */
  movementModel: MovementModel;
  /** Effective snap-to-grid axis: an explicit scene value overrides in either
   * direction (including `false`); absent falls back to a derived default keyed off the
   * RESOLVED `movementModel` (false for continuous, true otherwise). */
  snapToGrid: boolean;
  /** Whether dynamic lighting is active on this scene; scene `lighting` override, else the world
   * default. */
  lightingEnabled: boolean;
  /** Which lighting mode this scene uses; scene `lighting` override, else the world default. */
  lightMode: LightMode;
  /** The ambient/environment light this scene uses; scene `lighting` override, else the world
   * default. */
  environment: EnvironmentLight;
  /** Whether the grid pathfinder tolerates partial-cell overlap; world-settings only — no
   * per-scene override exists. */
  partialCellLeniency: boolean;
  /** The grid pathfinder's diagonal-movement rule; world-settings only — no per-scene override
   * exists. */
  diagonalRule: DiagonalRule;
  /** Token movement animation timing; world-settings only — no per-scene override exists. */
  animation: { /** Animated movement speed, grid cells per second. */ speedCellsPerSec: number; /** The easing curve applied to the animation. */ easing: EasingMode };
  /** The scene's grid-to-real-world distance mapping; scene `grid.distance`, else a built-in
   * `{ perCell: 5, unit: "ft" }` default (5e scale). */
  gridDistance: GridDistance;
  /** The scene's playable-area dimensions in grid units; scene `bounds`, else
   * `DEFAULT_SCENE_BOUNDS`. */
  bounds: SceneDimensions;
}

/** Visible-to-all defaults; the server normalizes permissions per the creator's role.
 * Internal helper — not exported from the package.
 * @returns A fresh `WireDocument["permissions"]` object (observer-default, no per-user or
 * per-property overrides, unconditional GM access via `gm_role: null`).
 * @example
 * ```
 * const perms = defaultPermissions();
 * perms.default; // "observer"
 * ```
 */
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
 * display name, independent of the engine/system split.
 * @param worldId The owning world's id (`scope.world_id`).
 * @param docType The document's `doc_type` (e.g. `"scene"`, `"token"`, `"region"`).
 * @param parentId The parent document id, or `null` for a top-level document.
 * @param system The opaque `system` body; always present on the wire (`{}` when a
 * doc_type's real data lives entirely in `engine`).
 * @param id An explicit id, or `undefined` to generate one via `crypto.randomUUID()`.
 * @param engine The typed, server-validated engine body, or `undefined` for a
 * non-engine-defined doc_type (omitted from the wire object entirely).
 * @param name The envelope display name, or `null` (default) when the doc_type has none.
 * @returns A fully-formed `WireDocument` with fresh `created_at`/`updated_at` timestamps,
 * visible-to-all-observers default permissions, no owner, and no embedded documents.
 * @example
 * ```ts
 * import { envelope } from "@shadowcat/core";
 *
 * const doc = envelope("world-1", "region", "scene-1", {}, undefined, {
 *   shape: { kind: "rect", points: [0, 0, 1, 1] },
 *   behavior: "terrain",
 *   cost: 1,
 *   enabled: true,
 * });
 * doc.doc_type; // "region"
 * ```
 */
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

/** 32-bit FNV-1a mix, seeded, over `str`. Building block for `deterministicId`.
 * Internal helper — not exported from the package.
 * @param str The input string to mix.
 * @param seed The 32-bit seed to mix in (a different seed over the same `str` yields an
 * independent-looking 32-bit output — used to derive 128 bits of id material from four seeds).
 * @returns The mixed 32-bit hash, unsigned (`>>> 0`).
 * @example
 * ```
 * const h = fnv1a32("world-1:faction-registry", 0x811c9dc5);
 * typeof h; // "number"
 * ```
 */
function fnv1a32(str: string, seed: number): number {
  let h = seed >>> 0;
  for (let i = 0; i < str.length; i++) {
    h ^= str.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/** Deterministic UUID-v5-SHAPED id derived from `namespace` + `name`: same inputs always
 * produce the same id, so independent callers (e.g. two GMs seeding a world-scoped singleton
 * doc at once) converge on one id without a lookup. Built from four independently-seeded
 * FNV-1a 32-bit mixes (128 bits total) rather than true SHA-1 UUIDv5, because Web Crypto's
 * `subtle.digest` is async and every doc builder in this file — `envelope` included — is
 * synchronous; the version(5)/variant nibbles are set purely for id-shape parity with
 * `crypto.randomUUID()`, not RFC 4122 conformance. Collision risk is not a security boundary:
 * the server's singleton create-gate rejects a duplicate Create by `doc_type`, not by id, so a
 * (practically impossible) id collision would only ever surface as an ordinary id-uniqueness
 * conflict. Reference id scheme for singleton config-doc seeders (`faction-registry`,
 * `condition-registry`, `world-settings`, …).
 * @param namespace The id-space discriminator (e.g. a world id).
 * @param name The item name within that namespace (e.g. `"faction-registry"`).
 * @returns A UUID-v5-shaped (36-char, hyphenated) deterministic string id.
 * @example
 * ```ts
 * import { deterministicId } from "@shadowcat/core";
 *
 * const idA = deterministicId("world-1", "faction-registry");
 * const idB = deterministicId("world-1", "faction-registry");
 * idA === idB; // true — same inputs always produce the same id
 * ```
 */
export function deterministicId(namespace: string, name: string): string {
  const input = `${namespace}:${name}`;
  const hex = [0x811c9dc5, 0x9e3779b9, 0x85ebca6b, 0xc2b2ae35]
    .map((seed) => fnv1a32(input, seed).toString(16).padStart(8, "0"))
    .join("");
  const versioned = hex.slice(0, 12) + "5" + hex.slice(13, 16);
  const variantNibble = ((parseInt(hex[16], 16) & 0x3) | 0x8).toString(16);
  const stamped = versioned.slice(0, 16) + variantNibble + versioned.slice(17);
  return `${stamped.slice(0, 8)}-${stamped.slice(8, 12)}-${stamped.slice(12, 16)}-${stamped.slice(16, 20)}-${stamped.slice(20, 32)}`;
}

/** A top-level scene document with a default square/100 grid and no background.
 * Optional `vision`/`lighting` overrides, `grid.distance`, and `snapToGrid` are included
 * only when provided; absent keys fall back to world-settings defaults at resolution time.
 * `doc_type: "scene"` is engine-defined — the body lands in `engine`, `system` stays `{}`.
 * @param worldId The owning world's id.
 * @param engine A partial `SceneEngine`; any omitted field falls back to the default square
 * grid (`{ kind: "square", size: 100, distance: null }`) or `null`.
 * @param id An explicit id, or `undefined` to generate one.
 * @returns A `WireDocument` with `doc_type: "scene"`.
 * @example
 * ```ts
 * import { buildSceneDoc } from "@shadowcat/core";
 *
 * const scene = buildSceneDoc("world-1", { grid: { kind: "square", size: 50, distance: null } });
 * scene.doc_type; // "scene"
 * ```
 */
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
 * DEFAULT_WORLD_SETTINGS (value-independence-at-construction invariant).
 * @param worldId The owning world's id.
 * @param engine The full `WorldSettingsEngine`; defaults to a deep clone of
 * `DEFAULT_WORLD_SETTINGS` when omitted.
 * @param id An explicit id, or `undefined` to generate one.
 * @returns A `WireDocument` with `doc_type: "world-settings"`.
 * @example
 * ```ts
 * import { buildWorldSettingsDoc } from "@shadowcat/core";
 *
 * const settings = buildWorldSettingsDoc("world-1");
 * settings.doc_type; // "world-settings"
 * ```
 */
export function buildWorldSettingsDoc(
  worldId: string,
  engine: WorldSettingsEngine = structuredClone(DEFAULT_WORLD_SETTINGS),
  id?: string,
): WireDocument {
  return envelope(worldId, "world-settings", null, {}, id, engine, null);
}

/** Fail-closed bounds resolve: a present-but-malformed bounds (non-finite or ≤ 0 on either
 * axis) falls back to the finite default rather than yielding a degenerate navmesh rectangle.
 * Internal helper — not exported from the package.
 * @param b A candidate `SceneDimensions`, possibly absent or malformed.
 * @returns `b` unchanged when both axes are finite and `> 0`; otherwise `DEFAULT_SCENE_BOUNDS`.
 * @example
 * ```
 * resolveBounds({ width: 0, height: 100 }); // DEFAULT_SCENE_BOUNDS (width is not > 0)
 * ```
 */
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
 * Default gridDistance: 5 ft/cell (standard D&D 5e scale).
 * @param scene The scene document to resolve settings for (may be `undefined`).
 * @param store The document store to query the world-settings singleton from.
 * @returns The fully resolved `ResolvedSceneSettings` for `scene`.
 * @example
 * ```ts
 * import { resolveSceneSettings, type ReadableDocuments } from "@shadowcat/core";
 *
 * declare const scene: Parameters<typeof resolveSceneSettings>[0];
 * declare const store: ReadableDocuments;
 * const settings = resolveSceneSettings(scene, store);
 * settings.gridDistance; // { perCell: 5, unit: "ft" } when unset
 * ```
 */
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
    // Derived default keyed off the RESOLVED movementModel — `??` only falls
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

/** The single client-side answer to "which scene does THIS client render/subscribe to".
 * Resolution order: a resolvable `gmViewedScene` (GM local roam) → a resolvable
 * `world-settings.activeScene` (players follow) → the first scene (legacy). `null` ONLY when
 * no scene exists. Fail-closed by construction: an id that no longer names a scene is ignored
 * (never renders nothing while scenes exist, never leaks a nonexistent scene's channel).
 * Players never pass `gmViewedScene`, so they always follow `activeScene`.
 * @param store The document store to query scenes and world-settings from.
 * @param opts Resolution options.
 * @param opts.gmViewedScene The GM's local-roam scene id override, or `undefined`/`null`
 * for a player (who always follows `activeScene`).
 * @returns The scene id this client should render/subscribe to, or `null` if no scene exists.
 * @example
 * ```ts
 * import { resolveViewedScene, type ReadableDocuments } from "@shadowcat/core";
 *
 * declare const store: ReadableDocuments;
 * const sceneId = resolveViewedScene(store, { gmViewedScene: null });
 * ```
 */
export function resolveViewedScene(
  store: ReadableDocuments,
  opts: { /** The GM's local-roam scene id override, or `undefined`/`null` for a player. */ gmViewedScene?: string | null } = {},
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
 * per `ActorEngine`.
 * @param worldId The owning world's id.
 * @param name The actor's real name (envelope field; use `setNameHidden` to privacy-gate it).
 * @param engine The full `ActorEngine` body.
 * @param id An explicit id, or `undefined` to generate one.
 * @returns A `WireDocument` with `doc_type: "actor"`.
 * @example
 * ```ts
 * import { buildActorDoc, type ActorEngine } from "@shadowcat/core";
 *
 * const engine: ActorEngine = {
 *   displayName: "Goblin", visual: { kind: "image", asset: "goblin.png" },
 *   size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [],
 *   prototype: false, vision: null,
 * };
 * const actor = buildActorDoc("world-1", "Goblin", engine);
 * actor.doc_type; // "actor"
 * ```
 */
export function buildActorDoc(worldId: string, name: string | null, engine: ActorEngine, id?: string): WireDocument {
  return envelope(worldId, "actor", null, {}, id, engine, name);
}

/** Client-only `item` doc_type: NOT engine-defined (`data::engine::is_engine_doc_type`
 * excludes "item") — the server stays fully structural for an item's body, same as every
 * other doc_type's `system` blob: `validate_system_size` caps it (`MAX_SYSTEM_BYTES`) and
 * `validate_system_schema_tree` enforces any module-registered tier-2 schema for `"item"`
 * (both run unconditionally on every doc_type). `validate_engine_tree` also runs on an item,
 * but only to REJECT one carrying an `engine` body (`normalize_engine_opt`'s
 * `(false, Some(_))` arm: "not engine-defined; `engine` must be absent") — an item never
 * receives the semantic/typed validation the 17 engine-defined doc types' `engine` bodies
 * get, because it has no typed struct and no `engine` body to deserialize.
 * An item lives standalone (top-level, parentless) or embedded in an actor's inventory.
 * Display name lives in the envelope; every other field is opaque, edited via the tree
 * editor. */
export const ITEM_DOC_TYPE = "item";

/** An item's opaque body — the game-system-owned `system` band, editable via the tree editor.
 * Any key may legitimately appear; the client's tree editor writes whatever the user enters.
 * The server enforces an overall size cap unconditionally
 * (`data::validation::validate_system_size`).
 * A game-system module may ADDITIONALLY register a
 * tier-2 JSON Schema for `doc_type: "item"`; when one is registered,
 * `data::validation::validate_system_schema_tree`
 * validates the shape of the subtree it names — but a
 * subtree it names and this document omits is not a violation (the schema governs shape only when
 * the field is present; it never compels a field to exist). With no schema registered for
 * `"item"`, nothing validates any individual field's shape or semantics — only the size cap
 * applies. */
export interface ItemSystem {
  /** An arbitrary field written by the tree editor; a module-registered schema may validate its
   * shape if present, but never requires it — see the interface doc above. */
  [key: string]: unknown;
}

/** A client-only `item` document (top-level, parentless, or later embedded in an actor's
 * inventory). NOT engine-defined — the server stays fully structural for `system`.
 * @param worldId The owning world's id.
 * @param name The item's display name (envelope field).
 * @param system The opaque item body, edited via the tree editor; defaults to `{}`.
 * @param id An explicit id, or `undefined` to generate one.
 * @returns A `WireDocument` with `doc_type: "item"` (no `engine` body).
 * @example
 * ```ts
 * import { buildItemDoc } from "@shadowcat/core";
 *
 * const item = buildItemDoc("world-1", "Longsword", { damage: "1d8" });
 * item.doc_type; // "item"
 * ```
 */
export function buildItemDoc(worldId: string, name: string | null, system: ItemSystem = {}, id?: string): WireDocument {
  return envelope(worldId, ITEM_DOC_TYPE, null, system, id, undefined, name);
}

/** Build a token from an actor. `link` references the shared actor; `instance` embeds an
 * independent copy with `source` provenance (the deferred merge engine consumes it). Size/
 * shape resolve from the actor; `w`/`h` seed the rendered cell size now.
 * `doc_type: "token"` is engine-defined — the transform/visual/link body lands in `engine`.
 * @param worldId The owning world's id.
 * @param sceneId The scene document this token is parented to.
 * @param actor The source actor document to link or instance.
 * @param mode `"link"` shares the actor (`engine.actor_id`); `"instance"` embeds a deep-cloned,
 * independent copy with `source` provenance.
 * @param pos The token's initial scene-unit position.
 * @param pos.x The initial x coordinate, scene units.
 * @param pos.y The initial y coordinate, scene units.
 * @param cellSize The dangling-link fallback size (`w`/`h`); the actor-backed render path
 * resolves size through `EffectiveActor.size × grid-cell` instead (see `resolveTokenBox`).
 * @param id An explicit id, or `undefined` to generate one.
 * @returns A `WireDocument` with `doc_type: "token"`, parented to `sceneId`.
 * @example
 * ```ts
 * import { buildTokenFromActor, type WireDocument } from "@shadowcat/core";
 *
 * declare const actor: WireDocument;
 * const token = buildTokenFromActor("world-1", "scene-1", actor, "link", { x: 0, y: 0 }, 100);
 * token.doc_type; // "token"
 * ```
 */
export function buildTokenFromActor(
  worldId: string,
  sceneId: string,
  actor: WireDocument,
  mode: "link" | "instance",
  pos: { /** Initial x coordinate, scene units. */ x: number; /** Initial y coordinate, scene units. */ y: number },
  cellSize: number,
  id?: string,
): WireDocument {
  // `w`/`h` are seeded solely as the dangling-link fallback: `resolveTokenBox`
  // uses this ONLY when the linked/instanced actor is missing (`resolveTokenBox`'s
  // missing-actor branch, `eng?.w ?? 0`). The actor-backed render path never reads these — size resolves
  // through EffectiveActor.size x grid-cell instead. This is an explicit, documented
  // fallback rather than a lazy derivation from the token's last-known actor size, which
  // would introduce a second size-derivation path.
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
 * returns `doc`.
 * @param doc The document to mutate (typically an `actor` or `token`).
 * @param hidden `true` to hide the name from non-owner/non-GM recipients; `false` to reveal it.
 * @returns `doc`, mutated in place.
 * @example
 * ```ts
 * import { setNameHidden, buildActorDoc, type ActorEngine } from "@shadowcat/core";
 *
 * const engine: ActorEngine = {
 *   displayName: "Goblin", visual: { kind: "image", asset: "goblin.png" },
 *   size: { w: 1, h: 1 }, shape: "square", faction: null, conditions: [],
 *   prototype: false, vision: null,
 * };
 * const actor = buildActorDoc("world-1", "Goblin", engine);
 * setNameHidden(actor, true);
 * actor.permissions.property_overrides["/name"]; // "owner_or_gm"
 * ```
 */
export function setNameHidden(doc: WireDocument, hidden: boolean): WireDocument {
  const overrides = { ...doc.permissions.property_overrides };
  if (hidden) overrides["/name"] = "owner_or_gm";
  else delete overrides["/name"];
  doc.permissions = { ...doc.permissions, property_overrides: overrides };
  return doc;
}

/** A token document parented to `sceneId`, carrying the given transform + visual.
 * @param worldId The owning world's id.
 * @param sceneId The scene document this token is parented to.
 * @param engine The full `TokenEngine` body (transform, visual, link, overrides, face).
 * @param id An explicit id, or `undefined` to generate one.
 * @returns A `WireDocument` with `doc_type: "token"`, parented to `sceneId`.
 * @example
 * ```ts
 * import { buildTokenDoc, type TokenEngine } from "@shadowcat/core";
 *
 * const engine: TokenEngine = {
 *   x: 50, y: 50, w: 100, h: 100, rotation: 0,
 *   visual: null, actor_id: null, overrides: null, face: null,
 * };
 * const token = buildTokenDoc("world-1", "scene-1", engine);
 * token.doc_type; // "token"
 * ```
 */
export function buildTokenDoc(worldId: string, sceneId: string, engine: TokenEngine, id?: string): WireDocument {
  return envelope(worldId, "token", sceneId, {}, id, engine, null);
}

/** A top-level (world-scoped, parentless) faction-registry document.
 * `doc_type: "faction-registry"` is engine-defined.
 * @param worldId The owning world's id.
 * @param factions The id-keyed faction map.
 * @param id An explicit id, or `undefined` to generate one; use `deterministicId` for a
 * singleton seed so racing GMs converge on one id.
 * @returns A `WireDocument` with `doc_type: "faction-registry"`.
 * @example
 * ```ts
 * import { buildFactionRegistryDoc } from "@shadowcat/core";
 *
 * const doc = buildFactionRegistryDoc("world-1", {
 *   goblins: { name: "Goblins", color: "#00ff00", stance: "hostile" },
 * });
 * doc.doc_type; // "faction-registry"
 * ```
 */
export function buildFactionRegistryDoc(worldId: string, factions: Record<string, Faction>, id?: string): WireDocument {
  return envelope(worldId, "faction-registry", null, {}, id, { factions } satisfies FactionRegistryEngine, null);
}

/** A top-level (world-scoped, parentless) condition-registry document.
 * `doc_type: "condition-registry"` is engine-defined.
 * @param worldId The owning world's id.
 * @param conditions The id-keyed condition map.
 * @param id An explicit id, or `undefined` to generate one; use `deterministicId` for a
 * singleton seed so racing GMs converge on one id.
 * @returns A `WireDocument` with `doc_type: "condition-registry"`.
 * @example
 * ```ts
 * import { buildConditionRegistryDoc } from "@shadowcat/core";
 *
 * const doc = buildConditionRegistryDoc("world-1", {
 *   prone: { name: "Prone", icon: "🔻" },
 * });
 * doc.doc_type; // "condition-registry"
 * ```
 */
export function buildConditionRegistryDoc(worldId: string, conditions: Record<string, Condition>, id?: string): WireDocument {
  return envelope(worldId, "condition-registry", null, {}, id, { conditions } satisfies ConditionRegistryEngine, null);
}

/** A generic scene-entity document (wall/drawing/template/…) parented to `sceneId`; every
 * doc_type this builder is used for (`wall`, `drawing`, `template` — see the call sites in
 * `makeWallTool`, `makeDrawTool`, and `makeTemplateTool`) is engine-defined, so the caller's shape
 * lands in `engine` — `system` stays `{}`. `region` documents use the dedicated
 * `buildRegionDoc` instead (it calls `envelope` directly), not this generic builder.
 * @param worldId The owning world's id.
 * @param sceneId The scene document this entity is parented to.
 * @param docType The engine-defined doc_type (e.g. `"wall"`, `"drawing"`, `"template"`).
 * @param engine The doc_type's engine body (typed by the caller per `docType`).
 * @param id An explicit id, or `undefined` to generate one.
 * @returns A `WireDocument` with the given `doc_type`, parented to `sceneId`.
 * @example
 * ```ts
 * import { buildSceneEntityDoc, type WallEngine } from "@shadowcat/core";
 *
 * const engine: WallEngine = {
 *   seg: { x1: 0, y1: 0, x2: 10, y2: 0 },
 *   blocksSight: true, blocksLight: true, blocksMove: true,
 * };
 * const wall = buildSceneEntityDoc("world-1", "scene-1", "wall", engine);
 * wall.doc_type; // "wall"
 * ```
 */
export function buildSceneEntityDoc(worldId: string, sceneId: string, docType: string, engine: unknown, id?: string): WireDocument {
  return envelope(worldId, docType, sceneId, {}, id, engine, null);
}

// --- Light-gradation registry ---

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
 * DEFAULT_GRADATION (value-independence-at-construction invariant).
 * @param worldId The owning world's id.
 * @param engine The full `LightGradationEngine`; defaults to a deep clone of
 * `DEFAULT_GRADATION` when omitted.
 * @param id An explicit id, or `undefined` to generate one.
 * @returns A `WireDocument` with `doc_type: "light-gradation"`.
 * @example
 * ```ts
 * import { buildLightGradationDoc } from "@shadowcat/core";
 *
 * const doc = buildLightGradationDoc("world-1");
 * doc.doc_type; // "light-gradation"
 * ```
 */
export function buildLightGradationDoc(worldId: string, engine: LightGradationEngine = structuredClone(DEFAULT_GRADATION), id?: string): WireDocument {
  return envelope(worldId, "light-gradation", null, {}, id, engine, null);
}

/** Returns bands sorted brightest-first (descending `minIllumination`) so a consumer
 * can walk the array and pick the first band whose floor a cell's illumination meets.
 * Fail-closed: absent or malformed doc falls back to DEFAULT_GRADATION; never throws.
 * @param store The document store to query the light-gradation singleton from.
 * @returns The effective gradation bands, sorted brightest-first.
 * @example
 * ```ts
 * import { resolveGradation, type ReadableDocuments } from "@shadowcat/core";
 *
 * declare const store: ReadableDocuments;
 * const bands = resolveGradation(store);
 * bands[0].name; // "bright" (or the highest-minIllumination band on record)
 * ```
 */
export function resolveGradation(store: ReadableDocuments): GradationBand[] {
  const eng = store.query("light-gradation")[0]?.engine as LightGradationEngine | undefined;
  const bands = eng?.bands ?? DEFAULT_GRADATION.bands;
  return [...bands].sort((a, b) => b.minIllumination - a.minIllumination);
}

// --- Vision-modes registry ---

/** Built-in two-mode seed: normal sight + darkvision.
 * Deep-frozen so shared refs returned by resolveVisionModes cannot be mutated by consumers. */
export const SEED_VISION_MODES: Record<string, VisionMode> = deepFreeze({
  normal: { id: "normal", name: "Normal", illuminationFloor: "dim", defaultRange: 0, renderHint: null },
  darkvision: { id: "darkvision", name: "Darkvision", illuminationFloor: "dark", defaultRange: 12, renderHint: "desaturate" },
});

/** A top-level (world-scoped, parentless) vision-modes config document.
 * Default param is a fresh deep clone — the returned doc's `.engine.modes` must not alias
 * SEED_VISION_MODES (value-independence-at-construction invariant).
 * @param worldId The owning world's id.
 * @param engine The full `VisionModesEngine`; defaults to a deep clone of the `modes` map
 * built from `SEED_VISION_MODES` when omitted.
 * @param id An explicit id, or `undefined` to generate one.
 * @returns A `WireDocument` with `doc_type: "vision-modes"`.
 * @example
 * ```ts
 * import { buildVisionModesDoc } from "@shadowcat/core";
 *
 * const doc = buildVisionModesDoc("world-1");
 * doc.doc_type; // "vision-modes"
 * ```
 */
export function buildVisionModesDoc(worldId: string, engine: VisionModesEngine = { modes: structuredClone(SEED_VISION_MODES) }, id?: string): WireDocument {
  return envelope(worldId, "vision-modes", null, {}, id, engine, null);
}

/** Returns the effective vision-mode map.
 * Fail-closed: absent or malformed doc falls back to SEED_VISION_MODES; never throws.
 * @param store The document store to query the vision-modes singleton from.
 * @returns The effective id-keyed `VisionMode` map.
 * @example
 * ```ts
 * import { resolveVisionModes, type ReadableDocuments } from "@shadowcat/core";
 *
 * declare const store: ReadableDocuments;
 * const modes = resolveVisionModes(store);
 * modes.darkvision?.defaultRange; // 12 when unconfigured
 * ```
 */
export function resolveVisionModes(store: ReadableDocuments): Record<string, VisionMode> {
  const eng = store.query("vision-modes")[0]?.engine as VisionModesEngine | undefined;
  return eng?.modes ?? SEED_VISION_MODES;
}

// --- Light source doc type ---

/** A light-source document parented to `sceneId`. The caller supplies the full `engine`
 * body (no default constant — no aliasing concern). `doc_type: "light"` is engine-defined.
 * @param worldId The owning world's id.
 * @param sceneId The scene document this light is parented to.
 * @param engine The full `LightEngine` body (position, color, intensity, radii, falloff).
 * @param id An explicit id, or `undefined` to generate one.
 * @returns A `WireDocument` with `doc_type: "light"`, parented to `sceneId`.
 * @example
 * ```ts
 * import { buildLightDoc, type LightEngine } from "@shadowcat/core";
 *
 * const engine: LightEngine = {
 *   x: 0, y: 0, color: "#ffcc66", intensity: 1,
 *   brightRadius: 4, dimRadius: 8, falloff: null, enabled: true,
 * };
 * const light = buildLightDoc("world-1", "scene-1", engine);
 * light.doc_type; // "light"
 * ```
 */
export function buildLightDoc(worldId: string, sceneId: string, engine: LightEngine, id?: string): WireDocument {
  return envelope(worldId, "light", sceneId, {}, id, engine, null);
}

// --- Region doc type ---

/** A region document parented to `sceneId`. Visible to all by default; use
 * `setRegionVisibility` to make it a secret trap. `doc_type: "region"` is engine-defined.
 * @param worldId The owning world's id.
 * @param sceneId The scene document this region is parented to.
 * @param engine The full `RegionEngine` body (shape, behavior, cost, enabled).
 * @param id An explicit id, or `undefined` to generate one.
 * @returns A `WireDocument` with `doc_type: "region"`, parented to `sceneId`.
 * @example
 * ```ts
 * import { buildRegionDoc, type RegionEngine } from "@shadowcat/core";
 *
 * const engine: RegionEngine = {
 *   shape: { kind: "rect", points: [0, 0, 5, 5] },
 *   behavior: "impassable", cost: 1, enabled: true,
 * };
 * const region = buildRegionDoc("world-1", "scene-1", engine);
 * region.doc_type; // "region"
 * ```
 */
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
 * override. Mutates in place + returns `doc`.
 * @param doc The region document to mutate.
 * @param hidden `true` to hide the region from non-owner/non-GM recipients; `false` to reveal it.
 * @returns `doc`, mutated in place.
 * @example
 * ```ts
 * import { setRegionVisibility, buildRegionDoc, type RegionEngine } from "@shadowcat/core";
 *
 * const engine: RegionEngine = {
 *   shape: { kind: "rect", points: [0, 0, 5, 5] },
 *   behavior: "impassable", cost: 1, enabled: true,
 * };
 * const region = buildRegionDoc("world-1", "scene-1", engine);
 * setRegionVisibility(region, true);
 * region.permissions.default; // "none"
 * ```
 */
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
