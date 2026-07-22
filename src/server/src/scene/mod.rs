//! Per-world derived scene ECS. Hydrated from documents (#5); never persisted,
//! never authoritative. Holds one hecs entity per scene-entity document so
//! engine-owned systems (M9 vision, M10 pathfinding) can query spatial state.

pub mod explored;
pub(crate) mod grid_shape;
pub mod lighting;
pub(crate) mod move_exec;
pub(crate) mod move_stream;
pub mod movement;
pub(crate) mod navmesh;
pub(crate) mod pathfinding;
pub(crate) mod regions;
pub mod vision;

use std::collections::{BTreeMap, HashMap};

use uuid::Uuid;

use crate::data::command::{set_pointer, Operation};
use crate::data::document::Document;
// The typed, ingress-validated engine band, imported under a namespace alias: this module
// declares its own `LightMode`/`MovementRestriction`/`MovementModel` (the RESOLVED
// representation `ResolvedScene` exposes to callers elsewhere in `scene/`); the engine crate's
// identically-named enums are the wire representation read off a document's `engine` field.
// Keeping the two distinct avoids widening this file's already-declared public enum surface.
use crate::data::engine as eng;
use crate::data::membership::PermissionContext;
use crate::scene::lighting::Band;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightMode {
    GlobalIllumination,
    EnvironmentLight,
}

/// Per-scene movement gate mode. Mirrors `MovementRestriction` in `scene-docs.ts`.
/// `Visible` = move cells must be currently visible; `Revealed` = visible ∪ explored memory;
/// `Unrestricted` = walls only (the M9a gate alone).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovementRestriction {
    Visible,
    Revealed,
    Unrestricted,
}

/// Per-scene movement/pathfinding engine choice (M10f-1). Mirrors `MovementModel` in
/// `scene-docs.ts`. `GridStepped` = the existing grid A* router; `Continuous` = the polyanya
/// navmesh router.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovementModel {
    GridStepped,
    Continuous,
}

/// Fail-safe finite default scene size (grid units) when a scene has no authored `bounds`.
/// MUST match `DEFAULT_SCENE_BOUNDS` in the client `scene-docs.ts` (client/server parity).
pub const DEFAULT_SCENE_BOUNDS_UNITS: (f64, f64) = (100.0, 100.0);

/// The resolved per-scene lighting/vision/movement settings (subset of the client
/// `ResolvedSceneSettings`; pathfinding/animation fields are resolved in later checkpoints).
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedScene {
    pub los_restriction: bool,
    pub fog: bool,
    pub observer_vision: bool,
    pub lighting_enabled: bool,
    pub light_mode: LightMode,
    pub env_color: u32,
    pub env_intensity: f64,
    pub movement_restriction: MovementRestriction,
    /// Per-scene/world-default pathfinding engine choice (M10f-1). `GridStepped` dispatches to
    /// `pathfinding::find`; `Continuous` dispatches to `navmesh::navmesh_find`.
    pub movement_model: MovementModel,
    pub partial_cell_leniency: bool,
    /// Scene dimensions (width, height) in grid units. Always finite `> 0`
    /// (default `DEFAULT_SCENE_BOUNDS_UNITS`). The M10f navmesh's outer rectangle.
    pub bounds: (f64, f64),
}

/// A resolved vision mode (subset of the client `VisionMode`). `default_range` is in cells.
/// `render_hint` mirrors `SEED_VISION_MODES` in `scene-docs.ts` (e.g. `"desaturate"` for
/// darkvision); absent in seed → `None`, absent in an authored doc entry → `None`.
#[derive(Clone, Debug)]
pub struct VisionMode {
    pub illumination_floor: String,
    pub default_range: f64,
    pub render_hint: Option<String>,
}

/// Parse `#rrggbb` or CSS 3-digit `#rgb` → packed `0xRRGGBB`; fail-closed to `0x000000`
/// (untinted) on any malformed input. CSS shorthand: each nibble is doubled (`#abc` → `#aabbcc`).
fn parse_hex_color(s: &str) -> u32 {
    let h = s.trim_start_matches('#');
    // Shorthand only applies when the input had a leading '#' (bare 3-char strings without '#'
    // are not valid CSS color syntax and must fall through to fail-closed 0).
    let full = if h.len() == 3 && s.starts_with('#') {
        // CSS 3-digit shorthand: each nibble doubled (#abc → #aabbcc).
        h.chars().flat_map(|c| [c, c]).collect::<String>()
    } else {
        h.to_string()
    };
    if full.len() == 6 {
        u32::from_str_radix(&full, 16).unwrap_or(0)
    } else {
        0
    }
}

/// Deserialize a document's ingress-validated `engine` body into `T`; `None` when the document
/// carries no `engine` (non-engine doc type, or an engine doc type whose entity predates ingress
/// validation in a test fixture) or the stored value fails to parse. Mirrors the pre-M13-0
/// per-field `sys_f64`/pointer-walk contract (a `None` result, not a struct default) so every
/// caller keeps applying its own existing field-level fail-closed backstop unchanged.
fn engine_as<T: serde::de::DeserializeOwned>(doc: &Document) -> Option<T> {
    doc.engine
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// A single `engine_as` decode cached under the owning document's id, alongside the exact
/// `engine` `Value` it was decoded from. `source` is what makes the cache self-verifying:
/// `engine_as_cached` treats a cached entry as valid only when `source` still equals the
/// document's CURRENT `engine`, so it never depends on catching every possible mutation
/// site (`apply_op` is not the only one — `set_world_config`/`set_actors`, the room-hydration
/// setters, assign `Document`s directly too). `decoded` is type-erased (`Box<dyn Any + Send>`,
/// `Send` so the enclosing `Mutex` stays `Sync` — `SceneEcs` is shared behind a
/// `tokio::sync::RwLock` across connection tasks, matching `navmesh_cache`'s same constraint).
struct CachedEngine {
    source: serde_json::Value,
    decoded: Box<dyn std::any::Any + Send>,
}

fn conv_light_mode(v: eng::LightMode) -> LightMode {
    match v {
        eng::LightMode::GlobalIllumination => LightMode::GlobalIllumination,
        eng::LightMode::EnvironmentLight => LightMode::EnvironmentLight,
    }
}

fn conv_movement_restriction(v: eng::MovementRestriction) -> MovementRestriction {
    match v {
        eng::MovementRestriction::Visible => MovementRestriction::Visible,
        eng::MovementRestriction::Revealed => MovementRestriction::Revealed,
        eng::MovementRestriction::Unrestricted => MovementRestriction::Unrestricted,
    }
}

fn conv_movement_model(v: eng::MovementModel) -> MovementModel {
    match v {
        eng::MovementModel::GridStepped => MovementModel::GridStepped,
        eng::MovementModel::Continuous => MovementModel::Continuous,
    }
}

fn conv_diagonal_rule(v: eng::DiagonalRule) -> pathfinding::DiagonalRule {
    match v {
        eng::DiagonalRule::Chebyshev => pathfinding::DiagonalRule::Chebyshev,
        eng::DiagonalRule::Manhattan => pathfinding::DiagonalRule::Manhattan,
        eng::DiagonalRule::Euclidean => pathfinding::DiagonalRule::Euclidean,
        eng::DiagonalRule::Alternating => pathfinding::DiagonalRule::Alternating,
    }
}

/// A hydrated scene-entity document, one per hecs entity.
pub struct SceneEntity {
    pub doc: Document,
}

/// A document is scene runtime state if it is a scene or a child of one.
pub fn is_scene_entity(doc: &Document) -> bool {
    doc.doc_type == "scene" || doc.parent_id.is_some()
}

/// A resolved token move: `(scene id, committed start, post-image end)`.
pub type TokenMove = (Uuid, (f64, f64), (f64, f64));

/// One scene's visible cells for a player: `cells` are `(i, j, band_index, tint 0xRRGGBB, render_hint)`.
#[derive(Debug)]
pub struct LitScene {
    pub scene: Uuid,
    pub cell: f64,
    pub cells: Vec<(i32, i32, usize, u32, Option<String>)>,
}

/// Margin (scene units, ~one default grid cell) the vision bound box extends past the walls
/// so rays always terminate on the box rather than escaping to infinity.
const VISION_BOUND_MARGIN: f64 = 100.0;

/// Pre-collected per-move-constant inputs for the mover's vision trajectory.
/// Holds the full `blocksSight` wall set and the visibility polygons for every stationary
/// owned token (all owned tokens in the scene except the moving one). Computed once per move
/// via `SceneEcs::player_vision_inputs`; each sample then calls the cheaper `polygons_at`
/// (one moving-token raycast only, no repeated O(entities) ECS or wall scan).
pub(crate) struct VisionMoveInputs {
    /// Full `blocksSight` wall set (includes `gm_only` walls — full-wall-set invariant, M9b).
    walls: Vec<vision::Seg>,
    /// Vision polygons for every owned token in the scene EXCEPT the moving token, at their
    /// committed (stationary) positions. Constant across all samples of one move.
    static_polys: Vec<Vec<vision::P>>,
    /// The scene's own bounded extent (`ResolvedScene.bounds`) — so `polygons_at`'s per-sample
    /// bound stays scene-bounds-aware identically to `player_vision_polygons` (no fork).
    scene_bounds: (f64, f64),
    /// True when the user owns no token in this scene: `polygons_at` returns empty (fail-closed).
    empty: bool,
}

impl VisionMoveInputs {
    /// Per-sample: compute the moving token's visibility polygon at `viewpoint` and prepend it
    /// to the precomputed static polygons. Returns empty when `empty == true` (no owned token
    /// in this scene — fail-closed). Uses the same `sight_walls` set and raycast primitives as
    /// `player_vision_polygons` (full-wall-set invariant, M9b; no fork).
    pub(crate) fn polygons_at(&self, viewpoint: (f64, f64)) -> Vec<Vec<vision::P>> {
        if self.empty {
            return Vec::new();
        }
        let bound = vision::bound_for_scene(
            viewpoint,
            &self.walls,
            self.scene_bounds,
            VISION_BOUND_MARGIN,
        );
        let moving_poly = vision::visibility_polygon(viewpoint, &self.walls, bound);
        // Moving token's polygon first (index 0); static polygons follow.
        let mut out = Vec::with_capacity(1 + self.static_polys.len());
        out.push(moving_poly);
        out.extend_from_slice(&self.static_polys);
        out
    }
}

/// The per-world derived world. Writes are serialized by the caller
/// (`Room::publish` under `publish_guard`); reads (derived recompute) take a
/// shared borrow.
pub struct SceneEcs {
    world: hecs::World,
    index: HashMap<Uuid, hecs::Entity>,
    /// Per-world seq of the last command reflected in this ECS. Updated under
    /// the same `scene.write()` lock as the entities in `Room::publish`, so a
    /// reader holding the read lock sees a consistent `(entities, seq)` pair and
    /// the derived `computed_at_seq` watermark can never be below the state it
    /// describes (#2).
    committed_seq: i64,
    /// World config-docs (singletons) + actors, hydrated for the lighting-aware vision mask
    /// (M10e-2). Held outside the hecs `world` because they are NOT scene entities
    /// (`is_scene_entity` excludes them); they are maintained by `apply_op` and the room setters.
    world_settings: Option<Document>,
    gradation: Option<Document>,
    vision_modes: Option<Document>,
    /// Point-lookup table keyed by actor doc id. Used only for `actors.get(id)` joins; must
    /// not be iterated for ordered or wire output (HashMap iteration order is non-deterministic).
    actors: HashMap<Uuid, Document>,
    /// M10f-1 footprint-inflated navmesh cache, keyed by `(scene, quantized footprint-radius
    /// millicells)`. `std::sync::Mutex` (not `RefCell`) + `Arc` (not `Rc`): `SceneEcs` sits behind
    /// a `tokio::sync::RwLock` shared across connection tasks, so concurrent readers may call
    /// `pathfind`/`navmesh_for` simultaneously — the cache needs `Sync` interior mutability.
    /// Never held across an `.await` (lookup + build are synchronous). Quantized to the nearest
    /// 1/1000 cell (Buddy-check finding, 2026-07-02, Important: the design spec explicitly calls
    /// for "quantized footprintRadius" so the cache "stays bounded" given token sizes are a small
    /// discrete set — exact f64-bit keying was an unjustified departure from that, vulnerable to
    /// floating-point noise in a client-computed radius producing distinct bit-patterns for what
    /// is logically the same size).
    navmesh_cache: std::sync::Mutex<HashMap<(Uuid, i64), std::sync::Arc<navmesh::NavMesh>>>,
    /// Per-document decoded-`engine`-field cache (A2 perf item, `docs/TODO.md`), keyed on the
    /// owning document's own id. `engine_as` fully re-`serde_json::from_value`-decodes on every
    /// call; this cache lets the ~19 vision/lighting/pathfinding hot-path call sites in this file
    /// reuse a prior decode instead. `Mutex` (not `RefCell`), matching `navmesh_cache` above, for
    /// the same `Sync`-under-shared-`RwLock` reason. Never locked across an `.await` (every use
    /// here is synchronous). Correctness does NOT depend on catching every mutation site — see
    /// `CachedEngine`'s doc comment: a cached entry is only reused when its stored `source` Value
    /// still equals the document's current `engine`, self-verifying regardless of how the
    /// document was mutated (`apply_op`, `set_world_config`/`set_actors`, or a test-only direct
    /// field assignment). `apply_op` additionally removes the touched document's entry outright
    /// (a best-effort trim, not load-bearing for correctness) so a deleted document's stale entry
    /// doesn't linger indefinitely.
    engine_cache: std::sync::Mutex<HashMap<Uuid, CachedEngine>>,
    /// `visible_cells_cached`'s per-`(user, scene)` mask cache for the M10e-4 movement gate.
    /// Keyed `(user, scene)`, NOT `(user, scene, lenient)` — a `lenient` flip is just another
    /// fingerprint field, so it naturally invalidates the entry rather than needing a wider key
    /// (see `VisibilityInputsSnapshot`). Self-verifying like `engine_cache` above, generalized
    /// from a single document's `engine` `Value` to the FULL set of values `visible_cells`'s
    /// computation reads: a cached mask is reused only when a freshly rebuilt
    /// `VisibilityInputsSnapshot` compares equal to the one stored alongside it. Deliberately NOT
    /// a generation counter bumped at known mutation sites — `engine_cache` already proved that
    /// shape incomplete (`apply_op` is not the sole mutation chokepoint;
    /// `set_world_config`/`set_actors`/test helpers bypass it), and this cache's input surface is
    /// far larger (tokens, walls, lights, the scene doc, world-settings, gradation, vision-modes,
    /// linked actors) — enumerating every mutation site for all of that would repeat the same
    /// incompleteness risk at higher stakes, since this cache sits directly on the secrecy gate.
    /// `Mutex`, matching `navmesh_cache`/`engine_cache` (never locked across an `.await`;
    /// `SceneEcs` is shared behind a `tokio::sync::RwLock`).
    visible_cells_cache: std::sync::Mutex<HashMap<(Uuid, Uuid), VisibleCellsCacheEntry>>,
    /// Test-only instrumentation: counts `visible_cells_cached` snapshot-mismatch recomputes, so
    /// a test can assert reuse actually happened (a repeated call with an unchanged snapshot must
    /// NOT bump this), not merely that the returned mask was correct both times.
    #[cfg(test)]
    visible_cells_recompute_count: std::sync::atomic::AtomicU64,
}

impl SceneEcs {
    pub fn new() -> Self {
        Self {
            world: hecs::World::new(),
            index: HashMap::new(),
            committed_seq: 0,
            world_settings: None,
            gradation: None,
            vision_modes: None,
            actors: HashMap::new(),
            navmesh_cache: std::sync::Mutex::new(HashMap::new()),
            engine_cache: std::sync::Mutex::new(HashMap::new()),
            visible_cells_cache: std::sync::Mutex::new(HashMap::new()),
            #[cfg(test)]
            visible_cells_recompute_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Cached variant of the free function `engine_as`: decodes `doc.engine` into `T`, reusing a
    /// prior decode for the same `id` when its cached source `Value` still equals `doc.engine`
    /// (see `CachedEngine`'s doc comment — this equality check, not a mutation-site invalidation
    /// hook, is what makes the cache correct). Callers MUST pass the document's OWN id (`doc.id`,
    /// or the equivalent `self.index`/`self.actors` key it was looked up under) so a stale cache
    /// entry can never be read under a DIFFERENT document's id.
    fn engine_as_cached<T>(&self, id: Uuid, doc: &Document) -> Option<T>
    where
        T: serde::de::DeserializeOwned + Clone + Send + 'static,
    {
        let current = doc.engine.as_ref()?;
        {
            let cache = self.engine_cache.lock().unwrap();
            if let Some(entry) = cache.get(&id) {
                if &entry.source == current {
                    if let Some(t) = entry.decoded.downcast_ref::<T>() {
                        return Some(t.clone());
                    }
                }
            }
        }
        let decoded: T = serde_json::from_value(current.clone()).ok()?;
        self.engine_cache.lock().unwrap().insert(
            id,
            CachedEngine {
                source: current.clone(),
                decoded: Box::new(decoded.clone()),
            },
        );
        Some(decoded)
    }

    /// Hydrate from a document set (scene entities only; others are ignored),
    /// reflecting state as of `seq` (the world's current seq at hydration).
    pub fn from_documents(docs: Vec<Document>, seq: i64) -> Self {
        let mut ecs = Self::new();
        ecs.committed_seq = seq;
        for doc in docs {
            if is_scene_entity(&doc) {
                let id = doc.id;
                let e = ecs.world.spawn((SceneEntity { doc },));
                ecs.index.insert(id, e);
            }
        }
        ecs
    }

    /// Record the seq of the command just applied (called under the write lock).
    pub fn set_committed_seq(&mut self, seq: i64) {
        self.committed_seq = seq;
    }

    /// The seq the ECS currently reflects — emitted as `computed_at_seq`.
    pub fn committed_seq(&self) -> i64 {
        self.committed_seq
    }

    /// Seed the world config-docs (room-hydration path). Each is the singleton of its doc_type, or
    /// `None` when the world has not authored one (resolvers then fall back to built-in defaults).
    pub fn set_world_config(
        &mut self,
        world_settings: Option<Document>,
        gradation: Option<Document>,
        vision_modes: Option<Document>,
    ) {
        self.world_settings = world_settings;
        self.gradation = gradation;
        self.vision_modes = vision_modes;
    }

    /// Seed the actor table (room-hydration path). Keyed by actor doc id.
    /// Relies on actor docs being world-scoped (parentless) — see the debug_assert below.
    pub fn set_actors(&mut self, actors: Vec<Document>) {
        debug_assert!(
            actors.iter().all(|d| d.parent_id.is_none()),
            "INVARIANT: actor docs are world-scoped (parentless); a parented actor would also \
             hydrate as a scene entity via is_scene_entity and be double-represented"
        );
        self.actors = actors.into_iter().map(|d| (d.id, d)).collect();
    }

    pub fn actor(&self, id: &Uuid) -> Option<&Document> {
        self.actors.get(id)
    }
    pub fn world_settings_doc(&self) -> Option<&Document> {
        self.world_settings.as_ref()
    }
    pub fn vision_modes_doc(&self) -> Option<&Document> {
        self.vision_modes.as_ref()
    }
    pub fn gradation_doc(&self) -> Option<&Document> {
        self.gradation.as_ref()
    }

    /// Mirror a config/actor field Update into the side tables (Value round-trip, structural-only).
    /// Takes `&mut Option<Document>` (not `&mut self`) so the three call sites can borrow the
    /// three distinct singleton fields independently without conflicting on `self`.
    fn apply_config_update(
        slot: &mut Option<Document>,
        doc_id: Uuid,
        changes: &[crate::data::command::FieldChange],
    ) {
        if let Some(d) = slot {
            if d.id == doc_id {
                if let Ok(mut v) = serde_json::to_value(&*d) {
                    for ch in changes {
                        let _ = set_pointer(&mut v, &ch.path, ch.new.clone());
                    }
                    if let Ok(updated) = serde_json::from_value::<Document>(v) {
                        *d = updated;
                    }
                }
            }
        }
    }

    /// Reflect one already-committed authoritative op into the derived world.
    pub fn apply_op(&mut self, op: &Operation) {
        // Best-effort `engine_cache` trim (not load-bearing for correctness — see the
        // `engine_cache`/`CachedEngine` doc comments: a cached entry is only ever reused when its
        // stored source `Value` still matches the document's current `engine`, so a missed
        // invalidation site can only cost a redundant decode, never stale data). `apply_op` is
        // NOT the only place a `Document` in this ECS gets mutated — `set_world_config`/
        // `set_actors` (room hydration) and test-only helpers assign fields directly — so this
        // removal is deliberately narrow: it only ever drops the ONE id this op targets.
        let touched_id = match op {
            Operation::Create { doc } | Operation::Delete { doc } => doc.id,
            Operation::Update { doc_id, .. } => *doc_id,
        };
        self.engine_cache.lock().unwrap().remove(&touched_id);

        // Determine whether this op can affect any cached navmesh's geometry (a `wall` doc's
        // blocksMove/seg fields, or a `scene` doc's bounds) BEFORE applying it — an Update needs
        // the existing entity's doc_type (Update never changes doc_type, so a pre-mutation lookup
        // is safe and correct); Create/Delete carry their own doc_type directly.
        let touches_navmesh_geometry = match op {
            Operation::Create { doc } | Operation::Delete { doc } => {
                matches!(doc.doc_type.as_str(), "wall" | "scene")
            }
            Operation::Update { doc_id, .. } => self
                .index
                .get(doc_id)
                .and_then(|&e| self.world.get::<&SceneEntity>(e).ok())
                .map(|c| matches!(c.doc.doc_type.as_str(), "wall" | "scene"))
                .unwrap_or(false),
        };

        match op {
            Operation::Create { doc } if is_scene_entity(doc) => {
                if let Some(&e) = self.index.get(&doc.id) {
                    let _ = self.world.despawn(e);
                }
                let e = self.world.spawn((SceneEntity { doc: doc.clone() },));
                self.index.insert(doc.id, e);
            }
            Operation::Update { doc_id, changes } => {
                // An Update never changes scene-entity membership: `parent_id`
                // and `doc_type` are envelope fields, immutable via field-path
                // Update (`required_cap_for_path` maps them to no capability).
                // INVARIANT: if `parent_id` becomes mutable, this arm must
                // re-evaluate `is_scene_entity` and spawn/despawn accordingly.
                // TODO: re-evaluate is_scene_entity here once parent_id is mutable.
                if let Some(&e) = self.index.get(doc_id) {
                    if let Ok(mut comp) = self.world.get::<&mut SceneEntity>(e) {
                        // Mirror the same field-path changes apply_intent applied
                        // to SQLite, via Value round-trip (server stays
                        // structural-only; no semantic interpretation).
                        if let Ok(mut v) = serde_json::to_value(&comp.doc) {
                            for ch in changes {
                                let _ = set_pointer(&mut v, &ch.path, ch.new.clone());
                            }
                            if let Ok(updated) = serde_json::from_value::<Document>(v) {
                                comp.doc = updated;
                            }
                        }
                    }
                }
                // Config singletons + actors (not in the hecs index).
                Self::apply_config_update(&mut self.world_settings, *doc_id, changes);
                Self::apply_config_update(&mut self.gradation, *doc_id, changes);
                Self::apply_config_update(&mut self.vision_modes, *doc_id, changes);
                if let Some(a) = self.actors.get_mut(doc_id) {
                    if let Ok(mut v) = serde_json::to_value(&*a) {
                        for ch in changes {
                            let _ = set_pointer(&mut v, &ch.path, ch.new.clone());
                        }
                        if let Ok(updated) = serde_json::from_value::<Document>(v) {
                            *a = updated;
                        }
                    }
                }
            }
            Operation::Delete { doc } => {
                if let Some(e) = self.index.remove(&doc.id) {
                    let _ = self.world.despawn(e);
                }
                match doc.doc_type.as_str() {
                    "world-settings"
                        if self.world_settings.as_ref().map(|d| d.id) == Some(doc.id) =>
                    {
                        self.world_settings = None;
                    }
                    "light-gradation" if self.gradation.as_ref().map(|d| d.id) == Some(doc.id) => {
                        self.gradation = None;
                    }
                    "vision-modes" if self.vision_modes.as_ref().map(|d| d.id) == Some(doc.id) => {
                        self.vision_modes = None;
                    }
                    "actor" => {
                        self.actors.remove(&doc.id);
                    }
                    _ => {}
                }
            }
            Operation::Create { doc } => {
                match doc.doc_type.as_str() {
                    "world-settings" => self.world_settings = Some(doc.clone()),
                    "light-gradation" => self.gradation = Some(doc.clone()),
                    "vision-modes" => self.vision_modes = Some(doc.clone()),
                    "actor" => {
                        self.actors.insert(doc.id, doc.clone());
                    }
                    _ => {} // other non-scene document: ignored
                }
            }
        }

        if touches_navmesh_geometry {
            // Coarse but correct: clear every cached navmesh (all scenes), not just the one
            // touched. Over-invalidation only costs an extra rebuild on the next query, never
            // staleness — the safe failure direction, matching this codebase's established
            // fail-safe-direction convention (e.g. `supercover_cells`'s over-include-on-corner).
            self.navmesh_cache.lock().unwrap().clear();
        }
    }

    /// The validated world-settings engine body, or `None` when the doc is absent or its stored
    /// `engine` fails to deserialize into `WorldSettingsEngine`. Ingress validation
    /// (`data::engine::validate_engine`) already requires every persisted "world-settings" doc's
    /// `engine` to be a complete, `deny_unknown_fields`-checked `WorldSettingsEngine` — this is
    /// the direct successor of the pre-M13-0 `scene`+`pathfinding`+`animation`-all-present
    /// structural guard (mirrors the TS `ws?.scene && ws?.pathfinding && ws?.animation` check),
    /// now enforced at write time instead of read time. A doc that predates that guard (e.g. a
    /// test fixture built without going through the ingress gate) still falls back to built-in
    /// defaults exactly as before. Used by every resolver that reads world-settings so partial/
    /// malformed-doc handling stays consistent across all of them.
    fn validated_world_settings_engine(&self) -> Option<eng::WorldSettingsEngine> {
        let doc = self.world_settings.as_ref()?;
        self.engine_as_cached::<eng::WorldSettingsEngine>(doc.id, doc)
    }

    /// Resolve a scene's effective lighting/vision settings: built-in defaults < world-settings doc
    /// < per-scene override. Fail-closed and `null ⇒ inherit` (mirrors `resolveSceneSettings`).
    pub fn resolve_scene(&self, scene: Uuid) -> ResolvedScene {
        // World layer: `validated_world_settings_engine` already enforces the pre-M13-0
        // scene+pathfinding+animation-all-present structural guard at write time (ingress),
        // so a `None` here means the same "fall back to built-ins" case the old guard covered.
        let ws = self.validated_world_settings_engine();
        let ws_scene = ws.as_ref().map(|w| &w.scene);
        // Built-in defaults (mirror DEFAULT_WORLD_SETTINGS.scene / WorldSettingsEngine::default).
        let d_los = ws_scene.map(|s| s.los_restriction).unwrap_or(true);
        let d_fog = ws_scene.map(|s| s.fog).unwrap_or(true);
        let d_obs = ws_scene.map(|s| s.observer_vision).unwrap_or(false);
        let d_lit = ws_scene.map(|s| s.lighting_enabled).unwrap_or(true);
        let d_mode = ws_scene
            .map(|s| s.light_mode)
            .unwrap_or(eng::LightMode::EnvironmentLight);
        let d_env_color = ws_scene
            .map(|s| s.environment.color.clone())
            .unwrap_or_else(|| "#0a0e1a".to_string());
        let d_env_int = ws_scene.map(|s| s.environment.intensity).unwrap_or(0.0);
        let d_move = ws_scene
            .map(|s| s.movement_restriction)
            .unwrap_or(eng::MovementRestriction::Visible);
        let d_model = ws_scene
            .map(|s| s.movement_model)
            .unwrap_or(eng::MovementModel::GridStepped);
        let d_lenient = ws_scene.map(|s| s.partial_cell_leniency).unwrap_or(true);

        // Scene override layer (per-scene `vision`/`lighting`; absent/`null` ⇒ inherit — an
        // `Option<T>` field with `#[serde(default)]` deserializes a missing OR explicit-`null`
        // key to `None` identically, matching the pointer-on-null semantics of the pointer-walk
        // this replaces).
        let scene_eng: Option<eng::SceneEngine> = self
            .index
            .get(&scene)
            .and_then(|&e| self.world.get::<&SceneEntity>(e).ok())
            .and_then(|c| self.engine_as_cached::<eng::SceneEngine>(scene, &c.doc));
        let s = scene_eng.as_ref();
        let vision_ov = s.and_then(|s| s.vision.as_ref());
        let lighting_ov = s.and_then(|s| s.lighting.as_ref());
        let los = vision_ov.and_then(|v| v.los_restriction).unwrap_or(d_los);
        let fog = vision_ov.and_then(|v| v.fog).unwrap_or(d_fog);
        let obs = vision_ov.and_then(|v| v.observer_vision).unwrap_or(d_obs);
        let lit = lighting_ov.and_then(|l| l.enabled).unwrap_or(d_lit);
        let mode = lighting_ov.and_then(|l| l.mode).unwrap_or(d_mode);
        let env = lighting_ov.and_then(|l| l.environment.as_ref());
        let env_color = env.map(|e| e.color.clone()).unwrap_or(d_env_color);
        let env_int = env.map(|e| e.intensity).unwrap_or(d_env_int);
        let move_r = vision_ov
            .and_then(|v| v.movement_restriction)
            .unwrap_or(d_move);
        let mmodel = vision_ov.and_then(|v| v.movement_model).unwrap_or(d_model);

        // Scene bounds (M10f-0): per-scene, no world default — a fixed finite fallback. A
        // non-finite or non-positive axis is degenerate for a navmesh rectangle → fail closed.
        let bounds = s
            .and_then(|s| s.bounds.as_ref())
            .filter(|b| {
                b.width.is_finite() && b.width > 0.0 && b.height.is_finite() && b.height > 0.0
            })
            .map(|b| (b.width, b.height))
            .unwrap_or(DEFAULT_SCENE_BOUNDS_UNITS);

        ResolvedScene {
            los_restriction: los,
            fog,
            observer_vision: obs,
            lighting_enabled: lit,
            light_mode: conv_light_mode(mode),
            env_color: parse_hex_color(&env_color),
            env_intensity: env_int.clamp(0.0, 1.0),
            movement_restriction: conv_movement_restriction(move_r),
            movement_model: conv_movement_model(mmodel),
            partial_cell_leniency: d_lenient,
            bounds,
        }
    }

    /// Resolved gradation bands, brightest-first. Fail-closed to the built-in three-band default.
    pub fn resolved_bands(&self) -> Vec<Band> {
        let bands = self
            .gradation
            .as_ref()
            .and_then(|d| self.engine_as_cached::<eng::LightGradationEngine>(d.id, d))
            .map(|g| {
                g.bands
                    .into_iter()
                    .map(|b| Band {
                        name: b.name,
                        min_illumination: b.min_illumination,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        crate::scene::lighting::sorted_bands(bands)
    }

    /// The world's pathfinding diagonal-cost rule. World-scoped (no per-scene override; the scene doc
    /// overrides only vision/lighting/grid — parent §5.2). Reads `world-settings.pathfinding.diagonalRule`.
    /// Uses `validated_world_settings_engine` so a structurally incomplete/absent doc falls back to
    /// `Chebyshev`, consistent with `resolve_scene`'s handling of the same partial-doc case.
    pub(crate) fn resolved_diagonal_rule(&self) -> pathfinding::DiagonalRule {
        self.validated_world_settings_engine()
            .map(|w| conv_diagonal_rule(w.pathfinding.diagonal_rule))
            .unwrap_or(pathfinding::DiagonalRule::Chebyshev)
    }

    /// Resolved animation token speed in cells/second. World-scoped (no per-scene override;
    /// mirrors `resolved_diagonal_rule`'s structural guard). Reads
    /// `world-settings.animation.speedCellsPerSec`; falls back to 6 when the doc is absent or
    /// structurally incomplete. The floor of 0.001 prevents a zero/negative config from causing
    /// a division-by-zero in the duration formula.
    pub(crate) fn resolved_animation_speed(&self) -> f64 {
        self.validated_world_settings_engine()
            .map(|w| w.animation.speed_cells_per_sec)
            .unwrap_or(6.0)
            .max(0.001)
    }

    /// Resolved vision-mode registry. Returns a `BTreeMap` for deterministic key order (mirrors
    /// the plan's Global Constraint on determinism; `.get(id)` works identically for callers).
    /// Fail-closed to the built-in `normal`+`darkvision` seed ONLY when no doc/`modes` is present
    /// (mirrors TS `sys?.modes ?? SEED`). A GM-authored modes doc with all-malformed entries is
    /// returned as-is rather than silently re-granting built-in modes the GM may have removed.
    pub fn resolved_vision_modes(&self) -> BTreeMap<String, VisionMode> {
        let mut out = BTreeMap::new();
        // Seed only on the None (absent/malformed) branch — a present doc's modes being all
        // malformed must not silently replace a GM-authored registry with the built-in seed.
        let parsed = self
            .vision_modes
            .as_ref()
            .and_then(|d| self.engine_as_cached::<eng::VisionModesEngine>(d.id, d));
        match parsed {
            Some(vme) => {
                for (id, m) in vme.modes {
                    out.insert(
                        id,
                        VisionMode {
                            illumination_floor: m.illumination_floor,
                            default_range: m.default_range,
                            render_hint: m.render_hint,
                        },
                    );
                }
            }
            None => {
                // Mirrors `SEED_VISION_MODES` in scene-docs.ts: normal has no hint;
                // darkvision desaturates (faithful-darkvision render, M10e-3).
                out.insert(
                    "normal".into(),
                    VisionMode {
                        illumination_floor: "dim".into(),
                        default_range: 0.0,
                        render_hint: None,
                    },
                );
                out.insert(
                    "darkvision".into(),
                    VisionMode {
                        illumination_floor: "dark".into(),
                        default_range: 12.0,
                        render_hint: Some("desaturate".into()),
                    },
                );
            }
        }
        out
    }

    /// Count of hydrated scene entities (the M8a identity payload source).
    pub fn entity_count(&self) -> usize {
        self.index.len()
    }

    /// The token's current committed position `(x, y)` in scene coordinates.
    /// `None` if `token` is not a token entity or has no `(x, y)` in its `engine` band.
    /// Coupling: `move_exec::execute_move` calls this to verify `path[0]` against the
    /// authoritative ECS state; `Room::execute_move` calls it to read the committed start
    /// position before dispatching to the executor.
    pub(crate) fn token_position(&self, token: Uuid) -> Option<(f64, f64)> {
        let &e = self.index.get(&token)?;
        let tok = self.world.get::<&SceneEntity>(e).ok()?;
        if tok.doc.doc_type != "token" {
            return None;
        }
        let t = self.engine_as_cached::<eng::TokenEngine>(token, &tok.doc)?;
        Some((t.x, t.y))
    }

    /// `Room::publish`'s client-driven drag-move path still writes `/system` only pending Task
    /// 8/9 — such writes are structurally inert against this `/engine`-only gate; see room.rs's
    /// `system_field_write_bypasses_the_move_gate_and_does_not_desync_the_engine_band` test.
    ///
    /// Resolve a token move from an `Update`'s `changes`: `(scene, committed_start,
    /// post_image_end)`. The end is the committed `engine` band with **all** changes applied in
    /// array order (last-write-wins) — exactly what `apply_intent` commits — so a wholesale
    /// `/engine` write or duplicate `/engine/x` changes cannot evade the collision check by
    /// presenting a safe target while committing an unsafe one. A `/system/x` write on a token
    /// (game-system data) never reaches this gate — position lives exclusively in `/engine`.
    /// `None` if `token_id` is not a token with `(x,y)`. Reads the authoritative ECS state,
    /// never the client's `old`.
    pub fn token_move(
        &self,
        token_id: Uuid,
        changes: &[crate::data::command::FieldChange],
    ) -> Option<TokenMove> {
        let &e = self.index.get(&token_id)?;
        // Read scene + committed position in a scoped borrow so the reference is dropped
        // before the post-image serde round-trip (avoids holding two borrows).
        let (scene, cx, cy, doc_value) = {
            let tok = self.world.get::<&SceneEntity>(e).ok()?;
            if tok.doc.doc_type != "token" {
                return None;
            }
            let scene = tok.doc.parent_id?;
            let t: eng::TokenEngine = self.engine_as_cached(token_id, &tok.doc)?;
            let v = serde_json::to_value(&tok.doc).ok()?;
            (scene, t.x, t.y, v)
        };
        let mut v = doc_value;
        for ch in changes {
            let _ = set_pointer(&mut v, &ch.path, ch.new.clone());
        }
        let nx = v.pointer("/engine/x").and_then(|x| x.as_f64())?;
        let ny = v.pointer("/engine/y").and_then(|x| x.as_f64())?;
        Some((scene, (cx, cy), (nx, ny)))
    }

    /// Per-player visibility polygons (M9b), each tagged with the scene it belongs to: one
    /// star-shaped polygon per token the user owns, computed against that token's scene's
    /// `blocksSight` walls. The server raycasts the FULL wall set (so a `gm_only` wall the player
    /// never receives still occludes); the player only ever gets their own polygons (#4). The
    /// scene tag lets the client cut fog holes only for the scene it is rendering — a token in
    /// scene B must not punch a hole into scene A's fog (scene coordinates are scene-local).
    /// Empty when the player controls no tokens.
    pub fn player_vision_polygons(&self, user_id: Uuid) -> Vec<(Uuid, Vec<vision::P>)> {
        // Collect owned-token viewpoints first (drops the query borrow before the wall queries).
        let mut viewpoints: Vec<(Uuid, vision::P)> = Vec::new();
        for e in self.world.query::<&SceneEntity>().iter() {
            if e.doc.doc_type != "token" || e.doc.owner != Some(user_id) {
                continue;
            }
            if let (Some(t), Some(scene)) = (
                self.engine_as_cached::<eng::TokenEngine>(e.doc.id, &e.doc),
                e.doc.parent_id,
            ) {
                viewpoints.push((scene, (t.x, t.y)));
            }
        }
        let mut out = Vec::with_capacity(viewpoints.len());
        for (scene, vp) in viewpoints {
            let walls = self.sight_walls(scene);
            let scene_bounds = self.resolve_scene(scene).bounds;
            let bound = vision::bound_for_scene(vp, &walls, scene_bounds, VISION_BOUND_MARGIN);
            out.push((scene, vision::visibility_polygon(vp, &walls, bound)));
        }
        out
    }

    /// Pre-collect the per-move-constant vision inputs for the mover's fog-sweep trajectory:
    /// the full `blocksSight` wall set (computed once) and the visibility polygons for every
    /// owned token in `scene` EXCEPT `moving_token` (whose viewpoint varies per sample).
    /// Call once per move; then call `VisionMoveInputs::polygons_at` once per sample to obtain
    /// the moving token's polygon at that sample's viewpoint unioned with the static polygons.
    ///
    /// INVARIANT: same wall set and same raycast primitives as `player_vision_polygons`; no fork.
    pub(crate) fn player_vision_inputs(
        &self,
        user: Uuid,
        scene: Uuid,
        moving_token: Uuid,
    ) -> VisionMoveInputs {
        // Collect static-token viewpoints (non-moving owned tokens in `scene`). Drop the query
        // borrow before wall queries — mirrors player_vision_polygons collect-then-query order.
        let mut static_vps: Vec<vision::P> = Vec::new();
        let mut has_owned = false;
        for e in self.world.query::<&SceneEntity>().iter() {
            if e.doc.doc_type != "token"
                || e.doc.parent_id != Some(scene)
                || e.doc.owner != Some(user)
            {
                continue;
            }
            has_owned = true;
            if e.doc.id == moving_token {
                continue; // mover's viewpoint varies per sample; skip here
            }
            if let Some(t) = self.engine_as_cached::<eng::TokenEngine>(e.doc.id, &e.doc) {
                static_vps.push((t.x, t.y));
            }
        }
        let scene_bounds = self.resolve_scene(scene).bounds;
        if !has_owned {
            return VisionMoveInputs {
                walls: Vec::new(),
                static_polys: Vec::new(),
                scene_bounds,
                empty: true,
            };
        }
        // Full wall set: computed once for the entire move (same as player_vision_polygons).
        let walls = self.sight_walls(scene);
        // Static polygons: one per stationary owned token; constant across all samples.
        let static_polys = static_vps
            .iter()
            .map(|&vp| {
                let bound = vision::bound_for_scene(vp, &walls, scene_bounds, VISION_BOUND_MARGIN);
                vision::visibility_polygon(vp, &walls, bound)
            })
            .collect();
        VisionMoveInputs {
            walls,
            static_polys,
            scene_bounds,
            empty: false,
        }
    }

    /// Single-viewpoint convenience wrapper used by the `vision_at_*` tests. Production code
    /// calls `player_vision_inputs` once per move and then `VisionMoveInputs::polygons_at` per
    /// sample to avoid repeating the O(entities) ECS and wall scans each iteration.
    ///
    /// INVARIANT: same wall set and same raycast primitives as `player_vision_polygons`; no fork.
    #[cfg(test)]
    pub(crate) fn player_vision_polygons_at(
        &self,
        user: Uuid,
        scene: Uuid,
        moving_token: Uuid,
        viewpoint: (f64, f64),
    ) -> Vec<Vec<vision::P>> {
        let inputs = self.player_vision_inputs(user, scene, moving_token);
        inputs.polygons_at(viewpoint)
    }

    /// Each scene's grid cell size (`engine.grid.size`), defaulting to 100 — the unit the M9c
    /// explored-fog accumulation quantizes vision into. Read once per dispatch (cheap doc scan).
    pub fn scene_grid_sizes(&self) -> std::collections::HashMap<Uuid, f64> {
        let mut out = std::collections::HashMap::new();
        for e in self.world.query::<&SceneEntity>().iter() {
            if e.doc.doc_type != "scene" {
                continue;
            }
            let size = self
                .engine_as_cached::<eng::SceneEngine>(e.doc.id, &e.doc)
                .map(|s| s.grid.size)
                .filter(|s| *s > 0.0)
                .unwrap_or(100.0);
            out.insert(e.doc.id, size);
        }
        out
    }

    /// The `blocksSight` wall segments of `scene`.
    fn sight_walls(&self, scene: Uuid) -> Vec<vision::Seg> {
        let mut out = Vec::new();
        for w in self.world.query::<&SceneEntity>().iter() {
            if w.doc.doc_type != "wall" || w.doc.parent_id != Some(scene) {
                continue;
            }
            let Some(wall) = self.engine_as_cached::<eng::WallEngine>(w.doc.id, &w.doc) else {
                continue;
            };
            if wall.blocks_sight != Some(true) {
                continue;
            }
            out.push(vision::Seg {
                a: (wall.seg.x1, wall.seg.y1),
                b: (wall.seg.x2, wall.seg.y2),
            });
        }
        out
    }

    /// The `blocksLight` wall segments of `scene` (the light-occlusion geometry for lighting mask).
    pub(crate) fn light_walls(&self, scene: Uuid) -> Vec<vision::Seg> {
        let mut out = Vec::new();
        for w in self.world.query::<&SceneEntity>().iter() {
            if w.doc.doc_type != "wall" || w.doc.parent_id != Some(scene) {
                continue;
            }
            let Some(wall) = self.engine_as_cached::<eng::WallEngine>(w.doc.id, &w.doc) else {
                continue;
            };
            if wall.blocks_light != Some(true) {
                continue;
            }
            out.push(vision::Seg {
                a: (wall.seg.x1, wall.seg.y1),
                b: (wall.seg.x2, wall.seg.y2),
            });
        }
        out
    }

    /// The scene's `blocksMove` wall segments. Mirrors the wall filter in `blocks_move`
    /// (doc_type "wall", parent = scene, `engine.blocksMove == true`, endpoints at
    /// `engine.seg.{x1,y1,x2,y2}`). INVARIANT: same filter as `blocks_move` — any divergence
    /// would allow the pathfinder to route through walls the movement gate would then reject.
    pub(crate) fn move_walls(&self, scene: Uuid) -> Vec<vision::Seg> {
        let mut out = Vec::new();
        for w in self.world.query::<&SceneEntity>().iter() {
            if w.doc.doc_type != "wall" || w.doc.parent_id != Some(scene) {
                continue;
            }
            let Some(wall) = self.engine_as_cached::<eng::WallEngine>(w.doc.id, &w.doc) else {
                continue;
            };
            if wall.blocks_move != Some(true) {
                continue;
            }
            out.push(vision::Seg {
                a: (wall.seg.x1, wall.seg.y1),
                b: (wall.seg.x2, wall.seg.y2),
            });
        }
        out
    }

    /// Build-or-fetch the footprint-inflated navmesh for `(scene, footprint_radius_cells)`,
    /// memoized in `navmesh_cache` keyed on a quantized radius (nearest 1/1000 cell — see the
    /// field doc comment). Returns `None` when `navmesh::build_navmesh` fails closed (degenerate
    /// bounds/cell/footprint, or an over-cap obstacle count) — callers must treat this exactly
    /// like the grid router's `Unreachable` (no silent all-pass). A failed build is intentionally
    /// NOT cached: caching a failure under a mutable key would either mask a later successful
    /// build once the scene's geometry is fixed up (stale-failure, never re-attempted without an
    /// unrelated cache-clearing mutation), or require a separate "known-bad" sentinel distinct
    /// from "not yet built" — added complexity for no correctness gain, since a redundant re-run
    /// of `build_navmesh` on a still-degenerate scene hits the same fail-fast validation and is
    /// never unsafe, only wasted compute.
    ///
    /// Accepted tradeoff: the cache-miss path is
    /// lock→check→unlock→build→lock→insert, not atomic under the build. Two concurrent callers
    /// requesting the same new key can each build a redundant (but equally valid) `NavMesh` before
    /// one wins the insert — wasted compute, never a correctness issue (both builds are pure
    /// functions of the same inputs).
    pub(crate) fn navmesh_for(
        &self,
        scene: Uuid,
        footprint_radius_cells: f64,
    ) -> Option<std::sync::Arc<navmesh::NavMesh>> {
        // Validate BEFORE computing the cache key or touching the cache at all. `f64 as i64`
        // saturates NaN to 0 and rounds a tiny negative (e.g. -0.0001) to -0, which also casts to
        // 0 — colliding with the legitimate key for `footprint_radius_cells == 0.0`. Without this
        // upfront guard a degenerate radius would silently hit that cached entry and return a
        // valid-looking `Some` mesh instead of failing closed, bypassing `build_navmesh`'s own
        // range check entirely on any call after the 0.0 radius has already been cached. Mirrors
        // `build_navmesh`'s guard exactly so the two stay consistent.
        if !(0.0..=crate::scene::pathfinding::MAX_FOOTPRINT_CELLS).contains(&footprint_radius_cells)
        {
            return None;
        }
        // Quantize to the nearest 1/1000 cell so floating-point noise in a client-computed radius
        // (e.g. derived via division) collapses onto the same cache entry as the canonical value.
        let quantized = (footprint_radius_cells * 1000.0).round() as i64;
        let key = (scene, quantized);
        if let Some(cached) = self.navmesh_cache.lock().unwrap().get(&key) {
            return Some(cached.clone());
        }
        let bounds = self.resolve_scene(scene).bounds;
        let cell = self
            .scene_grid_sizes()
            .get(&scene)
            .copied()
            .unwrap_or(100.0);
        let walls = self.move_walls(scene);
        let built = navmesh::build_navmesh(bounds, cell, &walls, footprint_radius_cells)?;
        let arc = std::sync::Arc::new(built);
        self.navmesh_cache.lock().unwrap().insert(key, arc.clone());
        Some(arc)
    }

    /// Plan a route for `user`'s token in `scene` (M10e-6). Reuses the M10e-4 `visible_cells`
    /// mask so the preview agrees with the movement gate (spec §13). `is_gm`/`unrestricted` ⇒
    /// no mask; `visible` ⇒ `visible_cells`; `revealed` ⇒ `visible_cells ∪ explored`. `explored`
    /// is the caller's pre-fetched `ExploredSet` (only consulted under `revealed`; the handler
    /// fetches it off the lock). An empty non-GM mask ⇒ `find` returns Unreachable (fail-closed —
    /// the dark-scene freeze that mirrors the movement gate, by design).
    ///
    /// Coupling (spec §13): `visible_cells` is the ONE canonical mask shared between this method
    /// and the M10e-4 movement gate in `Room::publish`. Do NOT fork the per-cell decision here.
    // Eight args mirrors the flat ECS-assembly signature; the handler that calls this already
    // holds all inputs separately (user, scene, start, waypoints, footprint, is_gm, explored)
    // so a wrapper struct would only obscure the coupling to the movement gate.
    #[allow(clippy::too_many_arguments)]
    pub fn pathfind(
        &self,
        user: Uuid,
        scene: Uuid,
        start: (f64, f64),
        waypoints: &[(f64, f64)],
        footprint_radius: f64,
        is_gm: bool,
        explored: Option<&crate::scene::explored::ExploredSet>,
    ) -> Result<pathfinding::PathOutcome, pathfinding::PathFail> {
        let cell = self
            .scene_grid_sizes()
            .get(&scene)
            .copied()
            .unwrap_or(100.0);
        let rule = self.resolved_diagonal_rule();
        let walls = self.move_walls(scene);
        // Hoisted so `movement_model` is available to the dispatch below regardless of `is_gm`
        // (a GM can also route on a continuous scene) — the grid branch's OWN behavior is
        // unchanged, it just now reads `settings` from this shared binding instead of a local one.
        let settings = self.resolve_scene(scene);

        // Build the per-(user,scene) mask (None ⇒ unconstrained). Shared by both engines —
        // §13/§6.3: never fork the per-cell visibility decision.
        let mask: Option<std::collections::BTreeSet<pathfinding::Cell>> = if is_gm {
            None
        } else {
            match settings.movement_restriction {
                MovementRestriction::Unrestricted => None,
                MovementRestriction::Visible => {
                    Some(self.visible_cells(user, scene, settings.partial_cell_leniency))
                }
                MovementRestriction::Revealed => {
                    let mut m = self.visible_cells(user, scene, settings.partial_cell_leniency);
                    if let Some(ex) = explored {
                        m.extend(ex.iter());
                    }
                    Some(m)
                }
            }
        };

        match settings.movement_model {
            MovementModel::GridStepped => {
                // Per-requester region field (spec §4): GM (or `is_gm`) sees the authoritative
                // field; a non-GM requester's field silently omits any region they cannot see, so
                // a secret region never influences their route or budget (it "springs" only at
                // execution, `move_exec`, which always reads the authoritative field).
                let regions = self.region_field(scene, if is_gm { None } else { Some(user) });
                pathfinding::find(
                    start,
                    waypoints,
                    footprint_radius,
                    cell,
                    rule,
                    &walls,
                    mask.as_ref(),
                    Some(&regions),
                )
            }
            MovementModel::Continuous => {
                // M10f-4: the per-requester region field is the SINGLE weighting authority for the
                // continuous engine too (polyanya cannot weight — design spec §2). Terrain or
                // impassable present ⇒ route via the weighted grid A* forced to Euclidean
                // (continuous base metric), then LOS-smooth back to any-angle geometry. Otherwise
                // the unchanged pure polyanya route + an arrest post-filter. Arrest applies on both
                // paths. The per-requester field omits any region a non-GM cannot see (secret
                // regions spring only at `move_exec`).
                let regions = self.region_field(scene, if is_gm { None } else { Some(user) });
                if regions.has_terrain_or_impassable() {
                    let weighted = pathfinding::find(
                        start,
                        waypoints,
                        footprint_radius,
                        cell,
                        pathfinding::DiagonalRule::Euclidean,
                        &walls,
                        mask.as_ref(),
                        Some(&regions),
                    )?;
                    // `find` reports cost in CELLS; the continuous engine reports SCENE UNITS
                    // (parity with the polyanya path below). Convert before smoothing carries it
                    // through.
                    let weighted = pathfinding::PathOutcome {
                        cost: weighted.cost * cell,
                        ..weighted
                    };
                    Ok(navmesh::los_smooth(
                        weighted,
                        &walls,
                        mask.as_ref(),
                        &regions,
                        cell,
                        footprint_radius,
                    ))
                } else {
                    let nav = self
                        .navmesh_for(scene, footprint_radius)
                        .ok_or(pathfinding::PathFail::Unreachable)?;
                    let raw = navmesh::navmesh_find(&nav, start, waypoints)?;
                    // `raw.path.len() < 2` only when every waypoint leg collapsed to the start
                    // point (start == goal, mirroring `pathfinding::astar_leg`'s trivial-success
                    // case: a grid-stepped route to the cell you're already standing on succeeds
                    // with a single-cell, zero-cost route — see `astar_tests::
                    // start_equals_goal_is_a_single_cell_zero_cost`). `clip_to_visible_mask`'s own
                    // early return (`if outcome.path.len() < 2 { return outcome; }`) means a
                    // length-1 INPUT always passes through as a length-1 OUTPUT unchanged (nothing
                    // to truncate), so a length-1 `clipped` result can only originate from (a) this
                    // trivial case, or (b) a length-2+ raw route the mask/wall check truncated down
                    // to 1 point — a genuine rejection. Capture the flag before `raw` is consumed
                    // so both cases can be told apart afterward.
                    let raw_was_trivial = raw.path.len() < 2;
                    let clipped = navmesh::clip_to_visible_mask(
                        raw,
                        mask.as_ref(),
                        cell,
                        footprint_radius,
                        &walls,
                    );
                    if clipped.path.len() < 2 && !raw_was_trivial {
                        return Err(pathfinding::PathFail::Unreachable);
                    }
                    Ok(navmesh::truncate_at_arrest(clipped, &regions, cell))
                }
            }
        }
    }

    /// The composed region field for `scene`. `viewer: None` is the AUTHORITATIVE view (every
    /// enabled region, no filtering) — used by the GM and by `move_exec` (which springs secret
    /// regions on execution regardless of what the mover could see). `viewer: Some(user)` is the
    /// PER-REQUESTER view used by the grid A* router: a region is included only when `user` can
    /// see the visibility tier declared on its `/engine` (defaults to `All` when undeclared) —
    /// the SAME `resolve_access`/`property_overrides` mechanism that already gates every other
    /// document's egress (spec §3: "no new secrecy machinery"). A secret region's whole geometry
    /// lives in the `engine` band (M13-0), so the visibility-tier lookup targets the `/engine`
    /// property-override pointer, not `/system`. Callers MUST pass `None` for a GM requester (a
    /// GM always sees the authoritative field, mirroring `visible_cells`'s GM-skips-the-mask
    /// convention in `pathfind`).
    pub(crate) fn region_field(&self, scene: Uuid, viewer: Option<Uuid>) -> regions::RegionField {
        let cell = self
            .scene_grid_sizes()
            .get(&scene)
            .copied()
            .unwrap_or(100.0);
        let grid = crate::scene::grid_shape::SquareGrid {
            cell,
            rule: self.resolved_diagonal_rule(),
        };
        let mut builder = regions::RegionField::builder();
        for e in self.world.query::<&SceneEntity>().iter() {
            let doc = &e.doc;
            if doc.doc_type != "region" || doc.parent_id != Some(scene) {
                continue;
            }
            let Some(region_eng) = self.engine_as_cached::<eng::RegionEngine>(doc.id, doc) else {
                continue;
            };
            if !region_eng.enabled {
                continue;
            }
            if let Some(user) = viewer {
                let tier = doc
                    .permissions
                    .property_overrides
                    .get("/engine")
                    .copied()
                    .unwrap_or(crate::data::document::Visibility::All);
                let access = crate::data::permission::resolve_access(
                    user,
                    crate::data::document::WorldRole::Player,
                    doc,
                );
                if !access.can_see(tier) {
                    continue;
                }
            }
            let Some(shape) = regions::parse_region_shape(&region_eng.shape) else {
                continue;
            };
            let behavior = match region_eng.behavior.as_str() {
                "impassable" => regions::RegionBehavior::Impassable,
                "arrest" => regions::RegionBehavior::Arrest,
                _ => regions::RegionBehavior::Terrain,
            };
            let cost = region_eng.cost.max(1.0);
            builder.add(&shape, behavior, cost, cell, &grid);
        }
        builder.build()
    }

    /// The enabled `light` docs parented to `scene`, parsed into `lighting::Light`. Disabled lights
    /// are dropped here (they contribute nothing). `falloff` defaults to Linear; missing radii → 0.
    pub(crate) fn scene_lights(&self, scene: Uuid) -> Vec<crate::scene::lighting::Light> {
        use crate::scene::lighting::{Falloff, Light};
        let mut out = Vec::new();
        for e in self.world.query::<&SceneEntity>().iter() {
            if e.doc.doc_type != "light" || e.doc.parent_id != Some(scene) {
                continue;
            }
            let Some(le) = self.engine_as_cached::<eng::LightEngine>(e.doc.id, &e.doc) else {
                continue;
            };
            if !le.enabled {
                continue;
            }
            let falloff = match le.falloff.as_ref().map(|f| f.curve.as_str()) {
                Some("quadratic") => Falloff::Quadratic,
                Some("none") => Falloff::None,
                _ => Falloff::Linear,
            };
            out.push(Light {
                pos: (le.x, le.y),
                color: parse_hex_color(&le.color),
                intensity: le.intensity.clamp(0.0, 1.0),
                bright_radius: le.bright_radius,
                dim_radius: le.dim_radius,
                falloff,
                enabled: true, // INVARIANT: only enabled lights reach this push (disabled filtered above).
            });
        }
        // Deterministic order (entity-query order is unspecified): sort by id-stable position.
        // Uses total_cmp for a genuine total order — partial_cmp on f64 is a partial order
        // (NaN breaks trichotomy and makes sort_by non-deterministic under NaN inputs).
        out.sort_unstable_by(|a, b| {
            a.pos
                .0
                .total_cmp(&b.pos.0)
                .then(a.pos.1.total_cmp(&b.pos.1))
        });
        out
    }

    /// The token's effective vision modes as `(floor_min_illumination, range_cells, render_hint)`
    /// triples. `range_cells == 0.0` ⇒ unlimited. `render_hint` mirrors `VisionMode.render_hint`
    /// (e.g. `Some("desaturate")` for darkvision). Precedence (mirrors `resolveTokenActor` in
    /// actor.ts): a LINKED token (`actor_id` present) resolves the shared actor and applies
    /// `overrides.vision` as a wholesale replacement when present; a dangling link (actor absent)
    /// yields normal, ignoring overrides. An INSTANCED token (no `actor_id`) uses its
    /// `embedded.actor[0].engine.vision` without overrides. An unknown mode id is dropped
    /// (fail-closed: it contributes no vision floor). Always returns ≥1 triple (normal fallback
    /// with `render_hint: None`).
    pub fn token_vision_floors(&self, token: &Document) -> Vec<(f64, f64, Option<String>)> {
        let modes = self.resolved_vision_modes();
        let bands = self.resolved_bands();

        let token_eng = self.engine_as_cached::<eng::TokenEngine>(token.id, token);

        // Mirror actor.ts resolveTokenActor: a LINKED token (actor_id) resolves the shared actor and
        // applies the per-token override whitelist (overrides.vision REPLACES the actor's vision); a
        // dangling link (actor absent) yields normal, ignoring overrides. An INSTANCED token (no
        // actor_id) uses its embedded copy's vision; overrides do not apply to instanced tokens.
        let assignments: Option<Vec<eng::VisionAssignment>> =
            match token_eng.as_ref().and_then(|t| t.actor_id) {
                Some(id) => match self.actors.get(&id) {
                    Some(actor) => token_eng
                        .as_ref()
                        .and_then(|t| t.overrides.as_ref())
                        .and_then(|o| o.vision.clone())
                        .or_else(|| {
                            self.engine_as_cached::<eng::ActorEngine>(actor.id, actor)
                                .and_then(|a| a.vision)
                        }),
                    None => None, // dangling link → normal (overrides ignored, per resolveTokenActor)
                },
                // Uncached: an embedded actor's `id` doesn't match `token.id`, the key
                // `apply_op`'s invalidation removes on a token mutation — caching under the
                // embedded doc's own id would go stale on any `/embedded/actor/0/...` write.
                None => token
                    .embedded
                    .get("actor")
                    .and_then(|v| v.first())
                    .and_then(engine_as::<eng::ActorEngine>)
                    .and_then(|a| a.vision),
            };

        let mut out: Vec<(f64, f64, Option<String>)> = Vec::new();
        if let Some(arr) = assignments {
            for a in arr {
                let Some(vm) = modes.get(&a.mode) else {
                    continue;
                }; // unknown mode → drop (fail-closed)
                out.push((
                    crate::scene::lighting::floor_min(&bands, &vm.illumination_floor),
                    a.range,
                    vm.render_hint.clone(),
                ));
            }
        }
        if out.is_empty() {
            // Fallback: no vision assignments resolved → dim floor, unlimited range (mirrors
            // built-in "normal"; used even if a GM removed it from the registry).
            let normal_floor = modes
                .get("normal")
                .map(|m| m.illumination_floor.clone())
                .unwrap_or_else(|| "dim".into());
            out.push((
                crate::scene::lighting::floor_min(&bands, &normal_floor),
                0.0,
                None,
            ));
        }
        out
    }

    /// Scene-shared lighting/wall inputs for the visibility mask. Computed once per scene per
    /// dispatch and reused for every vision source via `lighting_inputs`. `all_bright`
    /// short-circuits light raycasts under lighting-off or globalIllumination (spec §3/§6).
    pub(crate) fn lighting_inputs(
        &self,
        scene: Uuid,
        settings: &ResolvedScene,
        cell: f64,
    ) -> LightingInputs {
        let all_bright = !settings.lighting_enabled
            || matches!(settings.light_mode, LightMode::GlobalIllumination);
        let lights = if all_bright {
            Vec::new()
        } else {
            self.scene_lights(scene)
        };
        let light_walls = if all_bright {
            Vec::new()
        } else {
            self.light_walls(scene)
        };
        Self::lighting_inputs_from(
            all_bright,
            lights,
            &light_walls,
            self.sight_walls(scene),
            settings.bounds,
            cell,
        )
    }

    /// Raycast step of `lighting_inputs`, split out so `visible_cells_cached` can gather the
    /// pre-raycast `lights`/`light_walls`/`sight_walls` (cheap: cached document decodes only, no
    /// geometry) to build its invalidation fingerprint WITHOUT paying for `lit_polys`' raycasts,
    /// then call this to do the raycast only on a fingerprint mismatch. `lighting_inputs` itself
    /// is unchanged behavior — it always gathers then immediately raycasts, same as before this
    /// split.
    fn lighting_inputs_from(
        all_bright: bool,
        lights: Vec<lighting::Light>,
        light_walls: &[vision::Seg],
        sight_walls: Vec<vision::Seg>,
        bounds: (f64, f64),
        cell: f64,
    ) -> LightingInputs {
        let lit_polys: Vec<Vec<vision::P>> = lights
            .iter()
            .map(|l| {
                let b = vision::bound_for(l.pos, light_walls, VISION_BOUND_MARGIN);
                vision::visibility_polygon(l.pos, light_walls, b)
            })
            .collect();
        // Boundary-projected environment occlusion (M10f/C1). Empty under all_bright (env is not
        // the mechanism there); occluded by the SAME blocksLight walls as the placed lights.
        let env_polys = if all_bright {
            Vec::new()
        } else {
            lighting::env_light_polys(bounds, cell, light_walls)
        };
        LightingInputs {
            all_bright,
            lights,
            lit_polys,
            env_polys,
            sight_walls,
        }
    }

    /// The per-player lighting-aware visibility mask: per scene, the cells the user can currently
    /// see = LOS-cells ∩ (illumination ≥ vision floor ∨ darkvision-in-range), each tagged with its
    /// illumination band + tint. Vision sources = owned tokens ∪ (observerVision ? Observer-tier
    /// tokens : ∅). Fail-closed: a source-less player gets empty cells. GM is handled by the caller
    /// (mode:"all"); this is the masked path only.
    pub fn player_lit_mask(&self, user: Uuid) -> Vec<LitScene> {
        // 0. Pre-resolve scene settings for every scene that has a token, so resolve_scene is
        //    called exactly once per scene rather than once per token (Fix 3: memoize). Collect
        //    scene ids in a first pass (drops the query borrow before the resolve calls).
        let mut all_scene_ids: Vec<Uuid> = Vec::new();
        for e in self.world.query::<&SceneEntity>().iter() {
            if e.doc.doc_type == "token" {
                if let Some(sid) = e.doc.parent_id {
                    all_scene_ids.push(sid);
                }
            }
        }
        all_scene_ids.sort();
        all_scene_ids.dedup();
        // Point-lookup only; never iterated into output so HashMap order doesn't affect determinism.
        let scene_settings: HashMap<Uuid, ResolvedScene> = all_scene_ids
            .iter()
            .map(|&sid| (sid, self.resolve_scene(sid)))
            .collect();

        // 1. Gather vision-source tokens per scene (owner ∪ observer-tier when observerVision on).
        //    Collect (scene, viewpoint, vision_floors) tuples; drop the query borrow before raycasts.
        struct Src {
            scene: Uuid,
            vp: vision::P,
            // (floor_min_value, range_cells, render_hint): render_hint drives per-cell
            // darkvision hint resolution in the cell-accumulation loop (admit_hint).
            floors: Vec<(f64, f64, Option<String>)>,
        }
        let mut sources: Vec<Src> = Vec::new();
        for e in self.world.query::<&SceneEntity>().iter() {
            if e.doc.doc_type != "token" {
                continue;
            }
            let Some(scene) = e.doc.parent_id else {
                continue;
            };
            let owns = e.doc.owner == Some(user);
            // Short-circuit: an owned token is a source regardless of observer_vision.
            let is_source = owns || {
                let observer_vision = scene_settings
                    .get(&scene)
                    .map(|s| s.observer_vision)
                    .unwrap_or(false);
                if observer_vision {
                    let role = e
                        .doc
                        .permissions
                        .users
                        .get(&user)
                        .copied()
                        .unwrap_or(e.doc.permissions.default);
                    role <= crate::data::document::DocRole::Observer
                } else {
                    false
                }
            };
            if !is_source {
                continue;
            }
            if let Some(t) = self.engine_as_cached::<eng::TokenEngine>(e.doc.id, &e.doc) {
                sources.push(Src {
                    scene,
                    vp: (t.x, t.y),
                    floors: self.token_vision_floors(&e.doc),
                });
            }
        }
        if sources.is_empty() {
            return Vec::new();
        }

        // 2. Per scene, accumulate visible cells across that scene's sources.
        let grid = self.scene_grid_sizes();
        let bands = self.resolved_bands();
        use std::collections::BTreeMap;
        // (i, j) -> (best_level, band_index, tint, hint_floor, hint). hint_floor seeds NEG_INFINITY so the
        // first admitting mode always sets it; brightness (level/band/tint) and hint reduce independently.
        type CellEntry = BTreeMap<(i32, i32), (f64, usize, u32, f64, Option<String>)>;
        // scene -> (cell_size, per-cell best)
        let mut per_scene: BTreeMap<Uuid, (f64, CellEntry)> = BTreeMap::new();

        // Distinct scenes among the sources.
        let mut scenes: Vec<Uuid> = sources.iter().map(|s| s.scene).collect();
        scenes.sort();
        scenes.dedup();

        for scene in scenes {
            // Use the memoized settings; fall back to resolve (unreachable in practice since
            // every source scene was resolved above, but keeps the code correct if the map misses).
            let settings = match scene_settings.get(&scene) {
                Some(s) => s,
                None => continue,
            };
            let cell = grid.get(&scene).copied().unwrap_or(100.0);
            if cell <= 0.0 {
                continue;
            }
            // Lighting inputs: under globalIllumination or lighting-off, every LOS cell is bright;
            // else compute per-cell from lights (occluded by blocksLight) + environment.
            let li = self.lighting_inputs(scene, settings, cell);

            let entry = per_scene
                .entry(scene)
                .or_insert_with(|| (cell, BTreeMap::new()));
            for src in sources.iter().filter(|s| s.scene == scene) {
                // LOS polygon for this source (or, LOS off, the whole bound box as a polygon).
                let poly = source_los_poly(
                    src.vp,
                    &li.sight_walls,
                    settings.los_restriction,
                    settings.bounds,
                );
                if poly.len() < 3 {
                    continue;
                }
                // Bbox → candidate cells (mirror explored's bounded scan).
                let (mut minx, mut miny, mut maxx, mut maxy) =
                    (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                for &(x, y) in &poly {
                    minx = minx.min(x);
                    miny = miny.min(y);
                    maxx = maxx.max(x);
                    maxy = maxy.max(y);
                }
                let i0 = (minx / cell).floor() as i32;
                let i1 = (maxx / cell).floor() as i32;
                let j0 = (miny / cell).floor() as i32;
                let j1 = (maxy / cell).floor() as i32;
                let w = i1 as i64 - i0 as i64 + 1;
                let h = j1 as i64 - j0 as i64 + 1;
                let span = w.saturating_mul(h);
                if span > crate::scene::explored::MAX_CELLS_PER_POLYGON {
                    tracing::warn!(span, "lit mask cell scan exceeds cap; skipping source");
                    continue;
                }
                for i in i0..=i1 {
                    for j in j0..=j1 {
                        let cx = (i as f64 + 0.5) * cell;
                        let cy = (j as f64 + 0.5) * cell;
                        if !crate::scene::vision::point_in_poly(&poly, (cx, cy)) {
                            continue;
                        }
                        // Spec §3/§6: lighting OFF ⇒ all-bright untinted; globalIllumination ⇒
                        // all-bright tinted by the environment. level=1.0 so every vision floor
                        // (incl. normal "dim") passes — every LOS cell is visible.
                        let cl = if li.all_bright {
                            crate::scene::lighting::CellLight {
                                level: 1.0,
                                tint: if settings.lighting_enabled {
                                    settings.env_color
                                } else {
                                    0
                                },
                            }
                        } else {
                            crate::scene::lighting::cell_illumination(
                                (cx, cy),
                                settings.env_intensity,
                                settings.env_color,
                                &li.lights,
                                &li.lit_polys,
                                &li.env_polys,
                                cell,
                            )
                        };
                        let dist_cells =
                            (((cx - src.vp.0).powi(2) + (cy - src.vp.1).powi(2)).sqrt()) / cell;
                        // Lowest applicable floor decides visibility; highest applicable floor decides the hint.
                        // `cell_visible` computes the same min-floor-over-in-range-modes decision
                        // and is reused verbatim by the movement gate (spec §13 anti-drift).
                        let mut admit_floor = f64::NEG_INFINITY; // max admitting floor → which mode's hint wins
                        let mut admit_hint: Option<String> = None;
                        for (fmin, range, hint) in &src.floors {
                            let in_range = *range == 0.0 || dist_cells <= *range;
                            if !in_range {
                                continue;
                            }
                            if cl.level >= *fmin {
                                // Highest admitting floor wins; on a tie, None (a normal-equivalent perception) wins.
                                let take = *fmin > admit_floor
                                    || (*fmin == admit_floor
                                        && admit_hint.is_some()
                                        && hint.is_none());
                                if take {
                                    admit_floor = *fmin;
                                    admit_hint = hint.clone();
                                }
                            }
                        }
                        if cell_visible(&src.floors, cl.level, dist_cells) {
                            let band = crate::scene::lighting::band_index(&bands, cl.level);
                            let slot = entry.1.entry((i, j)).or_insert((
                                cl.level,
                                band,
                                cl.tint,
                                admit_floor,
                                admit_hint.clone(),
                            ));
                            if cl.level > slot.0 {
                                slot.0 = cl.level;
                                slot.1 = band;
                                slot.2 = cl.tint; // brightest source wins band/tint
                            }
                            // Hint reduces across sources by the same highest-floor/None-wins rule.
                            if admit_floor > slot.3
                                || (admit_floor == slot.3
                                    && slot.4.is_some()
                                    && admit_hint.is_none())
                            {
                                slot.3 = admit_floor;
                                slot.4 = admit_hint;
                            }
                        }
                    }
                }
            }
        }

        per_scene
            .into_iter()
            .map(|(scene, (cell, cells))| LitScene {
                scene,
                cell,
                cells: cells
                    .into_iter()
                    .map(|((i, j), (_lvl, band, tint, _hf, hint))| (i, j, band, tint, hint))
                    .collect(),
            })
            .collect()
    }

    /// The set of cells visible to `user` in `scene` for the movement gate. Reuses the exact
    /// egress primitives (`lighting_inputs`, `source_los_poly`, `cell_visible`) so it agrees with
    /// the secrecy mask (spec §13). `lenient` selects the rasterization rule: strict samples the
    /// cell CENTER only (≡ `player_lit_mask`); lenient also samples the four corners, so a cell
    /// whose vision polygon merely overlaps it counts — a superset, never extending past polygon
    /// overlap. Empty ⇒ no in-scene vision source for this user (fail closed).
    pub fn visible_cells(
        &self,
        user: Uuid,
        scene: Uuid,
        lenient: bool,
    ) -> std::collections::BTreeSet<(i32, i32)> {
        use std::collections::BTreeSet;
        let mut out: BTreeSet<(i32, i32)> = BTreeSet::new();
        let settings = self.resolve_scene(scene);
        let cell = self
            .scene_grid_sizes()
            .get(&scene)
            .copied()
            .unwrap_or(100.0);
        if cell <= 0.0 {
            return out;
        }

        let sources = self.gather_vision_sources_in_scene(user, scene, &settings);
        if sources.is_empty() {
            return out;
        }

        // Scene-shared lighting inputs (once), then per-source per-cell test.
        let li = self.lighting_inputs(scene, &settings, cell);
        accumulate_visible_cells(&mut out, &sources, &settings, cell, &li, lenient);
        out
    }

    /// Cached variant of `visible_cells` for the M10e-4 movement gate (the ONLY intended caller —
    /// `visible_cells` itself and every other existing caller, incl. the pathfinder and the §13
    /// parity tests, are UNCHANGED and keep calling the uncached primitive). Reuses the mask from
    /// a prior call for the same `(user, scene)` only when a freshly rebuilt
    /// `VisibilityInputsSnapshot` — built from the SAME `gather_vision_sources_in_scene` call and
    /// the SAME raw `resolve_scene`/`scene_grid_sizes`/`scene_lights`/`light_walls`/`sight_walls`
    /// reads the uncached path uses — compares EQUAL to the snapshot stored alongside the cached
    /// mask. Any difference (token move, wall/light/vision-mode/world-settings/scene mutation, a
    /// token gaining or losing owner/observer-tier status in this scene, or `lenient` itself
    /// changing) is a snapshot mismatch and forces a full recompute — fails toward recompute,
    /// never toward serving a stale wider mask. The only work skipped on a cache HIT is the two
    /// genuinely expensive geometry passes: `lit_polys`' per-light raycasts (inside
    /// `lighting_inputs`) and `accumulate_visible_cells`'s per-source LOS raycast + nested
    /// per-cell scan — the snapshot itself still re-reads every input document on every call (via
    /// already-cheap, self-verifying `engine_as_cached` decodes), so a real change is always seen.
    pub fn visible_cells_cached(
        &self,
        user: Uuid,
        scene: Uuid,
        lenient: bool,
    ) -> std::collections::BTreeSet<(i32, i32)> {
        use std::collections::BTreeSet;
        let settings = self.resolve_scene(scene);
        let cell = self
            .scene_grid_sizes()
            .get(&scene)
            .copied()
            .unwrap_or(100.0);
        if cell <= 0.0 {
            return BTreeSet::new();
        }

        let mut sources = self.gather_vision_sources_in_scene(user, scene, &settings);
        if sources.is_empty() {
            return BTreeSet::new();
        }
        // Deterministic snapshot order: `sources`' emission order follows hecs entity iteration,
        // which is not a stable contract across unrelated entity churn. Sorting avoids a spurious
        // fingerprint mismatch (over-invalidation is merely a perf cost, never a safety one, but
        // a stable order is what makes the reuse test in Step 6 meaningful).
        sources.sort_by_key(|s| s.id);

        let all_bright = !settings.lighting_enabled
            || matches!(settings.light_mode, LightMode::GlobalIllumination);
        let lights = if all_bright {
            Vec::new()
        } else {
            self.scene_lights(scene)
        };
        let light_walls = if all_bright {
            Vec::new()
        } else {
            self.light_walls(scene)
        };
        let sight_walls = self.sight_walls(scene);

        let snapshot = VisibilityInputsSnapshot {
            lenient,
            settings: settings.clone(),
            cell,
            sources: sources
                .iter()
                .map(|s| (s.id, s.vp, s.floors.clone()))
                .collect(),
            lights: lights.clone(),
            light_walls: light_walls.clone(),
            sight_walls: sight_walls.clone(),
        };

        {
            let cache = self.visible_cells_cache.lock().unwrap();
            if let Some((cached_snapshot, cached_mask)) = cache.get(&(user, scene)) {
                if *cached_snapshot == snapshot {
                    return cached_mask.clone();
                }
            }
        }

        #[cfg(test)]
        self.visible_cells_recompute_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let li = Self::lighting_inputs_from(
            all_bright,
            lights,
            &light_walls,
            sight_walls,
            settings.bounds,
            cell,
        );
        let mut mask = BTreeSet::new();
        accumulate_visible_cells(&mut mask, &sources, &settings, cell, &li, lenient);

        let mut cache = self.visible_cells_cache.lock().unwrap();
        cache.insert((user, scene), (snapshot, mask.clone()));
        mask
    }

    /// Test-only: the number of times `visible_cells_cached` has fallen through to a full
    /// recompute (snapshot mismatch or first call), so a test can assert a repeated call with no
    /// input change was actually served from the cache rather than merely returning the same
    /// (recomputed) answer twice.
    #[cfg(test)]
    fn visible_cells_recompute_count(&self) -> u64 {
        self.visible_cells_recompute_count
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// This user's vision sources (owner ∪ observer-tier token when `observerVision`) in `scene`.
    /// Shared by `visible_cells` and `visible_cells_cached` so the cached path's invalidation
    /// fingerprint is built from the EXACT same source list the mask computation itself consumes
    /// — never a second, separately hand-kept "what counts as a source" implementation that could
    /// silently drift and omit an input the fingerprint should have caught.
    fn gather_vision_sources_in_scene(
        &self,
        user: Uuid,
        scene: Uuid,
        settings: &ResolvedScene,
    ) -> Vec<VisSrc> {
        let mut sources: Vec<VisSrc> = Vec::new();
        for e in self.world.query::<&SceneEntity>().iter() {
            if e.doc.doc_type != "token" || e.doc.parent_id != Some(scene) {
                continue;
            }
            let owns = e.doc.owner == Some(user);
            let is_source = owns
                || (settings.observer_vision && {
                    let role = e
                        .doc
                        .permissions
                        .users
                        .get(&user)
                        .copied()
                        .unwrap_or(e.doc.permissions.default);
                    role <= crate::data::document::DocRole::Observer
                });
            if !is_source {
                continue;
            }
            if let Some(t) = self.engine_as_cached::<eng::TokenEngine>(e.doc.id, &e.doc) {
                sources.push(VisSrc {
                    id: e.doc.id,
                    vp: (t.x, t.y),
                    floors: self.token_vision_floors(&e.doc),
                });
            }
        }
        sources
    }

    /// Engine-owned movement collision (M9a, the second ARCHITECTURE #6 geometric
    /// exception). True if the move segment `a0→a1` crosses any `blocksMove` wall in `scene`.
    /// A no-op move (`a0 == a1`) never blocks.
    pub fn blocks_move(&self, scene: Uuid, a0: (f64, f64), a1: (f64, f64)) -> bool {
        if a0 == a1 {
            return false;
        }
        for w in self.world.query::<&SceneEntity>().iter() {
            if w.doc.doc_type != "wall" || w.doc.parent_id != Some(scene) {
                continue;
            }
            let Some(wall) = self.engine_as_cached::<eng::WallEngine>(w.doc.id, &w.doc) else {
                continue;
            };
            if wall.blocks_move != Some(true) {
                continue;
            }
            if segments_cross(
                a0,
                a1,
                (wall.seg.x1, wall.seg.y1),
                (wall.seg.x2, wall.seg.y2),
            ) {
                return true;
            }
        }
        false
    }
}

/// Scene-shared lighting/wall inputs for the visibility mask. Computed once per scene per
/// dispatch and reused for every vision source. `all_bright` short-circuits light raycasts
/// under lighting-off or globalIllumination (spec §3/§6).
pub(crate) struct LightingInputs {
    pub(crate) all_bright: bool,
    pub(crate) lights: Vec<lighting::Light>,
    pub(crate) lit_polys: Vec<Vec<vision::P>>,
    /// Scene-boundary visibility polygons occluding the environment ambient (`env_light_polys`).
    /// Empty under `all_bright` (env is not the mechanism there — every LOS cell is forced bright).
    pub(crate) env_polys: Vec<Vec<vision::P>>,
    pub(crate) sight_walls: Vec<vision::Seg>,
}

/// Whether a single sample `point` (already known to lie inside the LOS polygon) qualifies a
/// cell as visible. Computes illumination (mirroring `player_lit_mask`'s all_bright arm exactly)
/// and delegates to `cell_visible`. This is the ONE canonical place the per-point illumination +
/// floor decision is made, shared by all three sampling arms of `visible_cells` (lenient-center,
/// lenient-corner, strict-center) to prevent the §13 anti-drift hazard: if the decision logic
/// were inlined separately in each arm, a future edit could silently fork the gate mask from the
/// egress mask.
///
/// INVARIANT (§13): the all_bright tint expression `if lighting_enabled {env_color} else {0}`
/// must stay identical to `player_lit_mask`'s copy. `cell_visible` reads only `level` today, but
/// tint is passed through so the two masks can never structurally diverge even if tint gains
/// semantics later.
fn point_qualifies(
    point: (f64, f64),
    src_vp: (f64, f64),
    floors: &[(f64, f64, Option<String>)],
    settings: &ResolvedScene,
    li: &LightingInputs,
    cell: f64,
) -> bool {
    let cl = if li.all_bright {
        crate::scene::lighting::CellLight {
            level: 1.0,
            tint: if settings.lighting_enabled {
                settings.env_color
            } else {
                0
            },
        }
    } else {
        crate::scene::lighting::cell_illumination(
            point,
            settings.env_intensity,
            settings.env_color,
            &li.lights,
            &li.lit_polys,
            &li.env_polys,
            cell,
        )
    };
    let dist_cells = (((point.0 - src_vp.0).powi(2) + (point.1 - src_vp.1).powi(2)).sqrt()) / cell;
    cell_visible(floors, cl.level, dist_cells)
}

/// One vision source gathered by `gather_vision_sources_in_scene`: an owned or observer-tier
/// token's viewpoint + resolved vision floors. `id` is carried only for `visible_cells_cached`'s
/// deterministic snapshot ordering — `visible_cells` itself never reads it.
struct VisSrc {
    id: Uuid,
    vp: vision::P,
    floors: Vec<(f64, f64, Option<String>)>,
}

/// Fingerprint of every input `visible_cells`'s computation reads for one `(user, scene,
/// lenient)` call, used by `visible_cells_cached` to decide whether a prior mask may be reused.
/// Built from the SAME calls the real computation makes (`gather_vision_sources_in_scene`,
/// `resolve_scene`, `scene_grid_sizes`, `scene_lights`, `light_walls`, `sight_walls`) — not a
/// separately-derived "things that might matter" list — so completeness reduces to "does this
/// struct hold every field `accumulate_visible_cells`/`gather_vision_sources_in_scene` read",
/// which is directly checkable by inspection, rather than "were all mutation call sites
/// enumerated", which `engine_cache`'s `CachedEngine` already proved is an open, unboundable
/// question for this codebase (`apply_op` is not the sole mutation chokepoint). Any change to
/// what these fields hold — a token moving/gaining-or-losing source status, a wall's
/// blocksSight/blocksLight/geometry changing, a light being added/moved/toggled, a vision-mode or
/// gradation band definition changing (both flow into `sources`' `floors` via
/// `token_vision_floors`), a linked actor's vision assignment changing (same path), the scene's
/// own grid size or vision/lighting overrides changing, or world-settings' `observerVision`/
/// `losRestriction`/lighting defaults changing — is captured because it necessarily changes the
/// value of one of these fields, making the snapshot compare unequal.
/// One `sources` entry in `VisibilityInputsSnapshot`: `(token id, viewpoint, vision floors)`.
type VisSrcSnapshot = (Uuid, vision::P, Vec<(f64, f64, Option<String>)>);

#[derive(Clone, PartialEq)]
struct VisibilityInputsSnapshot {
    lenient: bool,
    settings: ResolvedScene,
    cell: f64,
    sources: Vec<VisSrcSnapshot>,
    lights: Vec<lighting::Light>,
    light_walls: Vec<vision::Seg>,
    sight_walls: Vec<vision::Seg>,
}

/// `visible_cells_cache`'s per-entry value: the snapshot it was computed from, paired with the
/// mask itself.
type VisibleCellsCacheEntry = (
    VisibilityInputsSnapshot,
    std::collections::BTreeSet<(i32, i32)>,
);

/// The per-source LOS raycast + per-cell scan shared by `visible_cells` and
/// `visible_cells_cached` on a cache miss — extracted verbatim (no logic change) from
/// `visible_cells`'s prior inline loop so there is exactly one implementation of the expensive
/// half of the computation for both entry points to call.
fn accumulate_visible_cells(
    out: &mut std::collections::BTreeSet<(i32, i32)>,
    sources: &[VisSrc],
    settings: &ResolvedScene,
    cell: f64,
    li: &LightingInputs,
    lenient: bool,
) {
    for src in sources {
        let poly = source_los_poly(
            src.vp,
            &li.sight_walls,
            settings.los_restriction,
            settings.bounds,
        );
        if poly.len() < 3 {
            continue;
        }
        let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for &(x, y) in &poly {
            minx = minx.min(x);
            miny = miny.min(y);
            maxx = maxx.max(x);
            maxy = maxy.max(y);
        }
        // Lenient samples corners, so a cell just outside the center-bbox can still qualify:
        // expand the scan by one cell each side under leniency.
        let pad = if lenient { 1 } else { 0 };
        let i0 = (minx / cell).floor() as i32 - pad;
        let i1 = (maxx / cell).floor() as i32 + pad;
        let j0 = (miny / cell).floor() as i32 - pad;
        let j1 = (maxy / cell).floor() as i32 + pad;
        let w = i1 as i64 - i0 as i64 + 1;
        let h = j1 as i64 - j0 as i64 + 1;
        if w.saturating_mul(h) > crate::scene::explored::MAX_CELLS_PER_POLYGON {
            tracing::warn!("visible_cells scan exceeds cap; skipping source");
            continue;
        }
        for i in i0..=i1 {
            for j in j0..=j1 {
                if out.contains(&(i, j)) {
                    continue;
                }
                // Strict: center only. Lenient: center first (so §13 strict cells are always
                // included), then corners if center fails — a cell whose polygon merely clips
                // a corner still qualifies under leniency.
                let center = ((i as f64 + 0.5) * cell, (j as f64 + 0.5) * cell);
                let corners = [
                    (i as f64 * cell, j as f64 * cell),
                    ((i + 1) as f64 * cell, j as f64 * cell),
                    (i as f64 * cell, (j + 1) as f64 * cell),
                    ((i + 1) as f64 * cell, (j + 1) as f64 * cell),
                ];
                let mut found = false;
                if lenient {
                    // Check center first, then corners.
                    if vision::point_in_poly(&poly, center)
                        && point_qualifies(center, src.vp, &src.floors, settings, li, cell)
                    {
                        found = true;
                    }
                    if !found {
                        for &corner in &corners {
                            if vision::point_in_poly(&poly, corner)
                                && point_qualifies(corner, src.vp, &src.floors, settings, li, cell)
                            {
                                found = true;
                                break;
                            }
                        }
                    }
                } else {
                    // Strict: center only (mirrors player_lit_mask exactly).
                    if vision::point_in_poly(&poly, center)
                        && point_qualifies(center, src.vp, &src.floors, settings, li, cell)
                    {
                        found = true;
                    }
                }
                if found {
                    out.insert((i, j));
                }
            }
        }
    }
}

/// Per-cell visibility decision shared by `player_lit_mask` (egress/secrecy gate) and
/// `visible_cells` (movement gate). INVARIANT: identical for both so the move gate never
/// forbids a shipped-visible cell nor permits an unshipped one (spec §13). A cell is visible iff
/// some in-range vision mode's illumination floor is met. `floors`: `(floor_min, range_cells,
/// hint)`; `range == 0.0` ⇒ unbounded. Returns false when no mode is in range (fail closed).
fn cell_visible(floors: &[(f64, f64, Option<String>)], cl_level: f64, dist_cells: f64) -> bool {
    let mut min_floor = f64::INFINITY;
    for (fmin, range, _hint) in floors {
        if *range == 0.0 || dist_cells <= *range {
            min_floor = min_floor.min(*fmin);
        }
    }
    min_floor.is_finite() && cl_level >= min_floor
}

/// The LOS polygon for one vision source: the raycast visibility polygon when `los_restriction`
/// is on, else the whole bound box as a rectangle (whole-scene visible). Source: M9 raycast
/// (`vision::visibility_polygon`). `scene_bounds` (`ResolvedScene.bounds`) is unioned into the
/// wall-derived bound so a wall-less (or sparsely-walled) scene reveals its own full authored
/// extent instead of a degenerate `viewpoint±VISION_BOUND_MARGIN` box — the same
/// `vision::bound_for_scene` fix `player_vision_polygons`/`player_vision_inputs` already apply,
/// generalized to this shared source (feeds both `player_lit_mask` and `visible_cells`/
/// `visible_cells_cached`, never a forked bound computation).
fn source_los_poly(
    vp: vision::P,
    sight_walls: &[vision::Seg],
    los_restriction: bool,
    scene_bounds: (f64, f64),
) -> Vec<vision::P> {
    let b = vision::bound_for_scene(vp, sight_walls, scene_bounds, VISION_BOUND_MARGIN);
    if los_restriction {
        vision::visibility_polygon(vp, sight_walls, b)
    } else {
        vec![
            (b.minx, b.miny),
            (b.maxx, b.miny),
            (b.maxx, b.maxy),
            (b.minx, b.maxy),
        ]
    }
}

/// Signed area ×2 of triangle abc; >0 = ccw, <0 = cw, 0 = collinear.
fn orient(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> f64 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

/// Is `p` within the axis-aligned bounding box of segment `ab` (collinearity assumed)?
fn on_segment(a: (f64, f64), b: (f64, f64), p: (f64, f64)) -> bool {
    p.0 >= a.0.min(b.0) && p.0 <= a.0.max(b.0) && p.1 >= a.1.min(b.1) && p.1 <= a.1.max(b.1)
}

/// Do segments `p1p2` and `p3p4` intersect (proper crossing or a touching endpoint /
/// T-junction)? Source: standard orientation/cross-product segment-intersection test
/// (CLRS "Determining whether two segments intersect"). A move that merely touches a wall
/// counts as blocked (conservative — a token cannot end on or graze a wall).
pub(crate) fn segments_cross(
    p1: (f64, f64),
    p2: (f64, f64),
    p3: (f64, f64),
    p4: (f64, f64),
) -> bool {
    let d1 = orient(p3, p4, p1);
    let d2 = orient(p3, p4, p2);
    let d3 = orient(p1, p2, p3);
    let d4 = orient(p1, p2, p4);
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }
    (d1 == 0.0 && on_segment(p3, p4, p1))
        || (d2 == 0.0 && on_segment(p3, p4, p2))
        || (d3 == 0.0 && on_segment(p1, p2, p3))
        || (d4 == 0.0 && on_segment(p1, p2, p4))
}

impl Default for SceneEcs {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute a derived payload for `channel` from the scene ECS, for one
/// recipient. Returns `None` for unknown channels (→ SceneError). `ctx` is
/// accepted so M9 vision can derive per recipient; the identity payload is
/// non-sensitive and global.
pub fn compute_derived(
    channel: &str,
    ecs: &SceneEcs,
    ctx: &PermissionContext,
) -> Option<serde_json::Value> {
    match channel {
        // Debug seam proof (non-sensitive, global); absent in release.
        #[cfg(debug_assertions)]
        "identity" => Some(serde_json::json!({ "entity_count": ecs.entity_count() })),
        // Per-player vision (M9b): the GM sees all; a player gets ONLY their own visibility
        // polygons (#4 per-recipient). A token-less player gets empty polygons → full fog (the
        // client masks everything outside `polygons`, so empty = see nothing, never see-all).
        // Each polygon carries its `scene` so the client cuts fog holes only for the scene it
        // renders — a token in another scene must not punch a hole into the active scene's fog.
        "vision" => {
            if ctx.world_role == crate::data::document::WorldRole::Gm {
                Some(serde_json::json!({ "mode": "all" }))
            } else {
                let polygons: Vec<serde_json::Value> = ecs
                    .player_vision_polygons(ctx.user_id)
                    .into_iter()
                    .map(|(scene, poly)| {
                        let points: Vec<f64> = poly.into_iter().flat_map(|(x, y)| [x, y]).collect();
                        serde_json::json!({ "scene": scene, "points": points })
                    })
                    .collect();
                // M10e-2: the secrecy-safe lighting-aware mask — only currently-visible cells, each
                // tagged with its illumination band + tint. Carries the resolved gradation `bands`
                // so the client maps band indices → treatment. Additive: `polygons`/`explored` are
                // unchanged (the client consumes `lit` from M10e-3).
                // M10e-3: `renderHints` is a deterministic string table (first-seen order over the
                // BTreeMap-ordered mask); each cell emits 5 ints: [i,j,band,tint,hint_idx] where
                // hint_idx is the index into `renderHints`, or -1 for None.
                // TODO: thread the bands player_lit_mask already resolved to avoid this second resolve.
                let bands_json: Vec<serde_json::Value> = ecs
                    .resolved_bands()
                    .into_iter()
                    .map(|b| serde_json::json!({ "name": b.name, "min": b.min_illumination }))
                    .collect();
                // Build the hint table and 5-int cell packing in a plain loop to avoid a
                // mutable borrow of `hints` inside a closure/flat_map borrow conflict.
                let mask = ecs.player_lit_mask(ctx.user_id);
                let mut hints: Vec<String> = Vec::new();
                let mut lit: Vec<serde_json::Value> = Vec::new();
                for s in mask {
                    let mut flat: Vec<i64> = Vec::new();
                    for (i, j, band, tint, hint) in s.cells {
                        let hi: i64 = match hint {
                            None => -1,
                            Some(ref h) => match hints.iter().position(|x| x == h) {
                                Some(idx) => idx as i64,
                                None => {
                                    hints.push(h.clone());
                                    (hints.len() - 1) as i64
                                }
                            },
                        };
                        flat.extend_from_slice(&[i as i64, j as i64, band as i64, tint as i64, hi]);
                    }
                    lit.push(
                        serde_json::json!({ "scene": s.scene, "cell": s.cell, "cells": flat }),
                    );
                }
                Some(
                    serde_json::json!({ "mode": "masked", "polygons": polygons, "bands": bands_json, "renderHints": hints, "lit": lit }),
                )
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(id: u128, parent: Option<u128>, ty: &str) -> Document {
        let mut d = crate::data::document::tests::world_scoped_doc(
            Uuid::from_u128(9),
            Uuid::from_u128(id),
            ty,
        );
        d.parent_id = parent.map(Uuid::from_u128);
        d
    }

    #[test]
    fn hydrate_counts_scene_entities_only() {
        let ecs = SceneEcs::from_documents(
            vec![
                doc(10, None, "scene"),
                doc(11, Some(10), "token"),
                doc(99, None, "actor"), // not a scene entity → ignored
            ],
            0,
        );
        assert_eq!(ecs.entity_count(), 2);
        assert_eq!(ecs.committed_seq(), 0);
    }

    #[test]
    fn engine_as_cache_invalidates_on_engine_mutation() {
        let mut ecs = SceneEcs::from_documents(
            vec![
                doc(10, None, "scene"),
                entity_doc_eng(
                    12,
                    10,
                    "wall",
                    json!({ "seg": {"x1":0,"y1":0,"x2":10,"y2":10}, "blocksSight": true }),
                ),
            ],
            0,
        );
        let wall_id = Uuid::from_u128(12);
        let doc1 = {
            let e = ecs.index[&wall_id];
            ecs.world.get::<&SceneEntity>(e).unwrap().doc.clone()
        };
        let decoded1: eng::WallEngine = ecs.engine_as_cached(wall_id, &doc1).unwrap();
        assert_eq!(decoded1.blocks_sight, Some(true));

        // Mutate the engine field through the real apply_op chokepoint.
        ecs.apply_op(&Operation::Update {
            doc_id: wall_id,
            changes: vec![crate::data::command::FieldChange {
                remove: false,
                path: "/engine/blocksSight".into(),
                old: json!(true),
                new: json!(false),
            }],
        });

        let doc2 = {
            let e = ecs.index[&wall_id];
            ecs.world.get::<&SceneEntity>(e).unwrap().doc.clone()
        };
        let decoded2: eng::WallEngine = ecs.engine_as_cached(wall_id, &doc2).unwrap();
        assert_eq!(
            decoded2.blocks_sight,
            Some(false),
            "cache must invalidate on engine mutation, not serve stale decode"
        );
    }

    #[test]
    fn apply_op_create_update_delete() {
        let mut ecs = SceneEcs::new();
        ecs.apply_op(&Operation::Create {
            doc: doc(11, Some(10), "token"),
        });
        assert_eq!(ecs.entity_count(), 1);
        ecs.apply_op(&Operation::Update {
            doc_id: Uuid::from_u128(11),
            changes: vec![crate::data::command::FieldChange {
                remove: false,
                path: "/system/x".into(),
                old: json!(null),
                new: json!(5),
            }],
        });
        let e = ecs.index[&Uuid::from_u128(11)];
        let comp = ecs.world.get::<&SceneEntity>(e).unwrap();
        assert_eq!(comp.doc.system["x"], json!(5));
        drop(comp);
        ecs.apply_op(&Operation::Delete {
            doc: doc(11, Some(10), "token"),
        });
        assert_eq!(ecs.entity_count(), 0);
    }

    /// Builds a scene-entity fixture with `engine` set to `body` (`system` stays `{}`), used by
    /// every fixture whose doc_type this file's production code reads through `engine_as`/a
    /// typed `*Engine` struct — every derivation reader in this file, including `token_move`
    /// as of this task (movement position lives exclusively in `/engine`).
    fn entity_doc_eng(id: u128, parent: u128, ty: &str, body: serde_json::Value) -> Document {
        let mut d = doc(id, Some(parent), ty);
        d.engine = Some(body);
        d
    }

    /// World-scoped (parentless) counterpart of `entity_doc_eng`, for config-docs
    /// (`world-settings`/`vision-modes`/`light-gradation`) and `actor` docs.
    fn entity_doc_top_eng(id: u128, ty: &str, body: serde_json::Value) -> Document {
        let mut d = doc(id, None, ty);
        d.engine = Some(body);
        d
    }

    /// A minimal, structurally-complete `ActorEngine` body (`displayName`/`visual`/`size`/
    /// `shape`/`conditions`/`prototype` are all required, non-`Option` fields) with `vision` set
    /// to the caller's assignment array — this file's vision-floor tests only ever vary `vision`.
    fn actor_body(vision: serde_json::Value) -> serde_json::Value {
        json!({
            "displayName": "Fixture Actor",
            "visual": { "kind": "image", "asset": "a.png" },
            "size": { "w": 1.0, "h": 1.0 },
            "shape": "square",
            "conditions": [],
            "prototype": true,
            "vision": vision,
        })
    }

    #[test]
    fn segments_cross_truth_table() {
        assert!(segments_cross(
            (0.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
            (10.0, 0.0)
        )); // X crossing
        assert!(!segments_cross(
            (0.0, 0.0),
            (10.0, 0.0),
            (0.0, 5.0),
            (10.0, 5.0)
        )); // parallel
        assert!(!segments_cross(
            (0.0, 0.0),
            (5.0, 0.0),
            (10.0, 0.0),
            (15.0, 0.0)
        )); // collinear disjoint
        assert!(segments_cross(
            (0.0, 0.0),
            (5.0, 0.0),
            (5.0, 0.0),
            (5.0, 5.0)
        )); // touching endpoint (T)
        assert!(segments_cross(
            (0.0, 0.0),
            (5.0, 10.0),
            (0.0, 5.0),
            (10.0, 5.0)
        )); // crossing
        assert!(segments_cross(
            (2.0, 0.0),
            (8.0, 0.0),
            (0.0, 0.0),
            (10.0, 0.0)
        )); // collinear OVERLAP (sliding along a wall)
    }

    fn fc(path: &str, new: serde_json::Value) -> crate::data::command::FieldChange {
        crate::data::command::FieldChange {
            remove: false,
            path: path.into(),
            old: json!(0),
            new,
        }
    }

    #[test]
    fn blocks_move_geometry_scene_scoping_and_filters() {
        let scene = Uuid::from_u128(10);
        let other = Uuid::from_u128(20);
        let cross = json!({ "seg": {"x1":0,"y1":10,"x2":10,"y2":0}, "blocksMove": true });

        // Scene 10 has one crossing blocksMove wall.
        let ecs = SceneEcs::from_documents(
            vec![
                doc(10, None, "scene"),
                entity_doc_eng(12, 10, "wall", cross.clone()),
            ],
            0,
        );
        assert!(ecs.blocks_move(scene, (0.0, 0.0), (10.0, 10.0))); // crosses the wall
        assert!(!ecs.blocks_move(scene, (0.0, 0.0), (1.0, 1.0))); // misses (sum 2 < 10)
        assert!(!ecs.blocks_move(scene, (0.0, 0.0), (0.0, 0.0))); // a no-op move never blocks

        // Scene scoping: an identical crossing wall in scene 20 blocks a scene-20 move but NOT
        // a scene-10 move (the `parent_id == Some(scene)` filter).
        let ecs_scope = SceneEcs::from_documents(
            vec![
                doc(10, None, "scene"),
                doc(20, None, "scene"),
                entity_doc_eng(24, 20, "wall", cross.clone()),
            ],
            0,
        );
        assert!(ecs_scope.blocks_move(other, (0.0, 0.0), (10.0, 10.0))); // blocks in scene 20
        assert!(!ecs_scope.blocks_move(scene, (0.0, 0.0), (10.0, 10.0))); // not in scene 10

        // A scene whose only crossing wall is blocksMove:false must not block movement.
        let ecs2 = SceneEcs::from_documents(
            vec![
                doc(10, None, "scene"),
                entity_doc_eng(
                    13,
                    10,
                    "wall",
                    json!({ "seg": {"x1":0,"y1":10,"x2":10,"y2":0}, "blocksMove": false }),
                ),
            ],
            0,
        );
        assert!(!ecs2.blocks_move(scene, (0.0, 0.0), (10.0, 10.0)));
    }

    #[test]
    fn token_move_uses_post_image_resisting_forged_bypasses() {
        let ecs = SceneEcs::from_documents(
            vec![
                doc(10, None, "scene"),
                entity_doc_eng(
                    11,
                    10,
                    "token",
                    json!({ "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "rotation": 0.0 }),
                ),
            ],
            0,
        );
        let id = Uuid::from_u128(11);
        // A normal two-axis move.
        let (s, a0, a1) = ecs
            .token_move(
                id,
                &[fc("/engine/x", json!(10)), fc("/engine/y", json!(10))],
            )
            .unwrap();
        assert_eq!(s, Uuid::from_u128(10));
        assert_eq!(a0, (0.0, 0.0));
        assert_eq!(a1, (10.0, 10.0));
        // Bypass A: a wholesale `/engine` write — the post-image reads the new x/y.
        let whole = fc(
            "/engine",
            json!({ "x": 50, "y": 50, "w": 1.0, "h": 1.0, "rotation": 0.0 }),
        );
        assert_eq!(ecs.token_move(id, &[whole]).unwrap().2, (50.0, 50.0));
        // Bypass B: duplicate `/engine/x` — last write wins, mirroring apply_intent.
        let dup = ecs
            .token_move(id, &[fc("/engine/x", json!(5)), fc("/engine/x", json!(50))])
            .unwrap();
        assert_eq!(dup.2 .0, 50.0);
        // A non-position update is a no-op move (committed == post-image).
        let noop = ecs
            .token_move(id, &[fc("/engine/rotation", json!(1.5))])
            .unwrap();
        assert_eq!(noop.1, noop.2);
        // A `/system/x` write on a token never touches the gate — position lives exclusively
        // in `/engine`; this is game-system data the movement gate must not see.
        let system_decoy = ecs.token_move(id, &[fc("/system/x", json!(999))]).unwrap();
        assert_eq!(
            system_decoy.1, system_decoy.2,
            "/system writes are not position and must not move the gate's post-image"
        );
        // A non-token id resolves to nothing.
        assert!(ecs.token_move(Uuid::from_u128(99), &[]).is_none());
    }

    #[test]
    fn vision_channel_is_per_recipient() {
        use crate::data::document::WorldRole;
        let player = Uuid::from_u128(7);
        let mut token = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        token.owner = Some(player);
        let ecs = SceneEcs::from_documents(
            vec![
                doc(10, None, "scene"),
                token,
                entity_doc_eng(
                    12,
                    10,
                    "wall",
                    json!({ "seg": {"x1":10,"y1":-5,"x2":10,"y2":5}, "blocksSight": true }),
                ),
            ],
            0,
        );
        let gm = PermissionContext {
            user_id: Uuid::from_u128(1),
            world_role: WorldRole::Gm,
        };
        let pl = PermissionContext {
            user_id: player,
            world_role: WorldRole::Player,
        };
        let other = PermissionContext {
            user_id: Uuid::from_u128(9),
            world_role: WorldRole::Player,
        };

        // GM sees all (no fog).
        assert_eq!(compute_derived("vision", &ecs, &gm).unwrap()["mode"], "all");
        // The token owner gets one non-empty visibility polygon, tagged with its scene so the
        // client cuts holes only for the scene it renders (cross-scene leak guard).
        let pv = compute_derived("vision", &ecs, &pl).unwrap();
        assert_eq!(pv["mode"], "masked");
        assert_eq!(pv["polygons"].as_array().unwrap().len(), 1);
        assert_eq!(pv["polygons"][0]["scene"], json!(Uuid::from_u128(10)));
        assert!(!pv["polygons"][0]["points"].as_array().unwrap().is_empty());
        // A player who controls no token gets empty polygons → full fog (never see-all).
        let ov = compute_derived("vision", &ecs, &other).unwrap();
        assert_eq!(ov["mode"], "masked");
        assert!(ov["polygons"].as_array().unwrap().is_empty());
        // Unknown channel → None.
        assert!(compute_derived("nope", &ecs, &gm).is_none());
    }

    #[test]
    fn vision_payload_carries_lit_mask_for_players_not_gm() {
        use crate::data::document::WorldRole;
        use serde_json::json;
        let player = Uuid::from_u128(7);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(player);
        let light = entity_doc_eng(
            20,
            10,
            "light",
            json!({
            "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true }),
        );
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok, light], 0);

        let pl = PermissionContext {
            user_id: player,
            world_role: WorldRole::Player,
        };
        let pv = compute_derived("vision", &ecs, &pl).unwrap();
        assert_eq!(pv["mode"], "masked");
        let lit = pv["lit"]
            .as_array()
            .expect("lit present for masked payload");
        assert_eq!(lit.len(), 1);
        assert_eq!(lit[0]["scene"], json!(Uuid::from_u128(10)));
        let cells = lit[0]["cells"].as_array().unwrap();
        assert!(!cells.is_empty());
        assert_eq!(
            cells.len() % 5,
            0,
            "cells packed 5 ints/cell (i,j,band,tint,hint_idx)"
        );
        assert!(!pv["bands"].as_array().unwrap().is_empty()); // bands now top-level
        assert!(
            pv["renderHints"].is_array(),
            "renderHints table present at top level"
        );
        assert!(
            lit[0].get("bands").is_none(),
            "bands hoisted to top level, not per-entry"
        );

        // GM payload is unchanged — no lit key or bands key.
        let gm = PermissionContext {
            user_id: Uuid::from_u128(1),
            world_role: WorldRole::Gm,
        };
        let gv = compute_derived("vision", &ecs, &gm).unwrap();
        assert_eq!(gv["mode"], "all");
        assert!(gv.get("lit").is_none());
        assert!(gv.get("bands").is_none());
        assert!(gv.get("renderHints").is_none());
    }

    #[test]
    fn vision_payload_resolves_render_hint_index() {
        use crate::data::document::WorldRole;
        use serde_json::json;
        let player = Uuid::from_u128(7);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(player);
        tok.embedded.insert(
            "actor".into(),
            vec![{
                let mut a = doc(99, None, "actor");
                a.engine = Some(actor_body(json!([{ "mode": "darkvision", "range": 6 }])));
                a
            }],
        );
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok], 0);
        let pl = PermissionContext {
            user_id: player,
            world_role: WorldRole::Player,
        };
        let pv = compute_derived("vision", &ecs, &pl).unwrap();
        let hints = pv["renderHints"].as_array().unwrap();
        assert!(hints.iter().any(|h| h == "desaturate"));
        let cells = pv["lit"][0]["cells"].as_array().unwrap();
        let hint_idx = cells[4].as_i64().unwrap(); // 5th int of the first cell
        assert!(
            hint_idx >= 0,
            "first cell must have a resolved hint, not -1"
        );
        assert_eq!(pv["renderHints"][hint_idx as usize], json!("desaturate"));
    }

    #[test]
    fn resolvers_layer_world_then_scene_and_fail_closed() {
        use serde_json::json;
        let scene_id = Uuid::from_u128(10);
        let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);

        // No config docs → built-in defaults (lighting on, environmentLight, env intensity 0).
        let r0 = ecs.resolve_scene(scene_id);
        assert!(r0.lighting_enabled);
        assert!(matches!(r0.light_mode, LightMode::EnvironmentLight));
        assert_eq!(r0.env_intensity, 0.0);
        assert_eq!(ecs.resolved_bands()[0].name, "bright"); // default gradation
        assert_eq!(
            ecs.resolved_vision_modes()["darkvision"].illumination_floor,
            "dark"
        );

        // World default: lighting OFF, global illumination.
        let mut ws = doc(100, None, "world-settings");
        ws.engine = Some(ws_body(&[
            ("/scene/lightingEnabled", json!(false)),
            ("/scene/lightMode", json!("globalIllumination")),
            (
                "/scene/environment",
                json!({ "color": "#0a0e1a", "intensity": 0.25 }),
            ),
        ]));
        ecs.set_world_config(Some(ws), None, None);
        let r1 = ecs.resolve_scene(scene_id);
        assert!(!r1.lighting_enabled);
        assert!(matches!(r1.light_mode, LightMode::GlobalIllumination));
        assert_eq!(r1.env_color, 0x0A0E1A);
        assert!((r1.env_intensity - 0.25).abs() < 1e-9);

        // Per-scene override re-enables lighting (null/absent ⇒ inherit; a present value wins).
        let mut scene = doc(10, None, "scene");
        scene.engine = Some(
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                               "lighting": { "enabled": true } }),
        );
        ecs.apply_op(&Operation::Update {
            doc_id: scene_id,
            changes: vec![crate::data::command::FieldChange {
                remove: false,
                path: "/engine".into(),
                old: json!(null),
                new: scene.engine.clone().unwrap(),
            }],
        });
        assert!(ecs.resolve_scene(scene_id).lighting_enabled); // scene override beats world default
    }

    #[test]
    fn vision_modes_doc_is_respected_not_reseeded() {
        use serde_json::json;
        let mut ecs = SceneEcs::new();
        // A doc with ONLY a custom mode → returned as-is; normal/darkvision are NOT re-seeded.
        let mut vm = doc(101, None, "vision-modes");
        vm.engine = Some(json!({ "modes": { "blindsight": {
            "id": "blindsight", "name": "Blindsight",
            "illuminationFloor": "dark", "defaultRange": 4
        } } }));
        ecs.set_world_config(None, None, Some(vm));
        let modes = ecs.resolved_vision_modes();
        assert!(modes.contains_key("blindsight"));
        assert!(
            !modes.contains_key("normal"),
            "an authored modes doc must not be re-seeded"
        );
        // No doc at all → built-in seed.
        let empty = SceneEcs::new();
        assert!(empty.resolved_vision_modes().contains_key("darkvision"));
    }

    #[test]
    fn token_vision_floors_resolve_through_actor_join() {
        use serde_json::json;
        let mut ecs = SceneEcs::new();
        // An actor granting darkvision range 6.
        ecs.set_actors(vec![entity_doc_top_eng(
            200,
            "actor",
            actor_body(json!([{ "mode": "darkvision", "range": 6 }])),
        )]);

        // Linked token referencing the actor → darkvision floor (dark=0.0), range 6.
        let mut linked = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                    "actor_id": Uuid::from_u128(200).to_string() }),
        );
        let floors = ecs.token_vision_floors(&linked);
        assert_eq!(floors.len(), 1);
        assert_eq!(floors[0], (0.0, 6.0, Some("desaturate".to_string()))); // dark floor, 6-cell range, darkvision hint

        // A per-token override REPLACES the actor's vision entirely.
        linked.engine.as_mut().unwrap()["overrides"] =
            json!({ "vision": [{ "mode": "normal", "range": 0 }] });
        let f2 = ecs.token_vision_floors(&linked);
        assert_eq!(f2[0], (0.34, 0.0, None)); // dim floor, unlimited range, no hint (normal mode has render_hint: None)

        // An actorless token → normal only.
        let raw = entity_doc_eng(
            12,
            10,
            "token",
            json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        assert_eq!(ecs.token_vision_floors(&raw), vec![(0.34, 0.0, None)]);

        // An explicit EMPTY override REPLACES (no fall-through to the linked actor → normal).
        let mut linked_empty = entity_doc_eng(
            13,
            10,
            "token",
            json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                    "actor_id": Uuid::from_u128(200).to_string(),
                    "overrides": { "vision": [] } }),
        );
        assert_eq!(
            ecs.token_vision_floors(&linked_empty),
            vec![(0.34, 0.0, None)]
        );

        // A token with BOTH actor_id AND an embedded actor resolves the LINKED actor (matches the
        // client's actor_id-first resolveTokenActor), NOT the embedded copy.
        linked_empty.engine.as_mut().unwrap()["overrides"] = json!({}); // no vision override
        linked_empty.embedded.insert(
            "actor".into(),
            vec![entity_doc_top_eng(
                201,
                "actor",
                actor_body(json!([{ "mode": "normal", "range": 0 }])),
            )],
        );
        // actor 200 grants darkvision range 6 → linked wins → (0.0, 6.0), not the embedded normal.
        assert_eq!(
            ecs.token_vision_floors(&linked_empty),
            vec![(0.0, 6.0, Some("desaturate".to_string()))]
        );

        // A DANGLING link (actor_id with no matching actor) + an overrides.vision is normal — the
        // client ignores overrides when the linked actor is absent.
        let dangling = entity_doc_eng(
            14,
            10,
            "token",
            json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                    "actor_id": Uuid::from_u128(999).to_string(),
                    "overrides": { "vision": [{ "mode": "darkvision", "range": 9 }] } }),
        );
        assert_eq!(ecs.token_vision_floors(&dangling), vec![(0.34, 0.0, None)]);
    }

    #[test]
    fn light_and_blockslight_wall_accessors_filter_by_scene() {
        use serde_json::json;
        let scene = Uuid::from_u128(10);
        let ecs = SceneEcs::from_documents(
            vec![
                doc(10, None, "scene"),
                entity_doc_eng(
                    20,
                    10,
                    "light",
                    json!({
                        "x": 50.0, "y": 50.0, "color": "#ffeeaa", "intensity": 1.0,
                        "brightRadius": 2.0, "dimRadius": 6.0, "enabled": true
                    }),
                ),
                entity_doc_eng(
                    21,
                    10,
                    "light",
                    json!({ "x": 0.0, "y": 0.0, "color": "#fff",
                    "intensity": 1.0, "brightRadius": 1.0, "dimRadius": 2.0, "enabled": false }),
                ),
                entity_doc_eng(
                    22,
                    10,
                    "wall",
                    json!({ "seg": {"x1":0,"y1":0,"x2":10,"y2":0}, "blocksLight": true }),
                ),
                entity_doc_eng(
                    23,
                    10,
                    "wall",
                    json!({ "seg": {"x1":0,"y1":5,"x2":10,"y2":5}, "blocksLight": false }),
                ),
            ],
            0,
        );
        let lights = ecs.scene_lights(scene);
        assert_eq!(lights.len(), 1); // the disabled light is excluded
        assert_eq!(lights[0].color, 0xFFEEAA);
        assert_eq!(lights[0].bright_radius, 2.0);
        let walls = ecs.light_walls(scene);
        assert_eq!(walls.len(), 1); // only the blocksLight:true wall

        // Cross-scene isolation: a second scene (id 20) with its own enabled light and a
        // blocksLight:true wall must NOT appear in scene 10's results.
        let scene2 = Uuid::from_u128(20);
        let ecs2 = SceneEcs::from_documents(
            vec![
                doc(10, None, "scene"),
                entity_doc_eng(
                    20,
                    10,
                    "light",
                    json!({
                        "x": 50.0, "y": 50.0, "color": "#ffeeaa", "intensity": 1.0,
                        "brightRadius": 2.0, "dimRadius": 6.0, "enabled": true
                    }),
                ),
                entity_doc_eng(
                    22,
                    10,
                    "wall",
                    json!({ "seg": {"x1":0,"y1":0,"x2":10,"y2":0}, "blocksLight": true }),
                ),
                doc(30, None, "scene"), // scene id 20 (doc id 30 → Uuid 30; parent is None)
                entity_doc_eng(
                    31,
                    30,
                    "light",
                    json!({
                        "x": 10.0, "y": 10.0, "color": "#ffffff", "intensity": 0.8,
                        "brightRadius": 3.0, "dimRadius": 8.0, "enabled": true
                    }),
                ),
                entity_doc_eng(
                    32,
                    30,
                    "wall",
                    json!({ "seg": {"x1":5,"y1":0,"x2":15,"y2":0}, "blocksLight": true }),
                ),
            ],
            0,
        );
        // Scene 10 still yields exactly its own 1 light and 1 wall.
        assert_eq!(ecs2.scene_lights(scene).len(), 1);
        assert_eq!(ecs2.light_walls(scene).len(), 1);
        // The second scene (id 30 via Uuid) has its own light and wall.
        let scene3 = Uuid::from_u128(30);
        assert_eq!(ecs2.scene_lights(scene3).len(), 1);
        assert_eq!(ecs2.light_walls(scene3).len(), 1);
        // Cross-check: scene 10's light is NOT scene2's light and vice-versa.
        assert_ne!(
            ecs2.scene_lights(scene)[0].pos,
            ecs2.scene_lights(scene3)[0].pos
        );
        // The unused scene2 uuid (20) is not a scene doc → yields empty (no children parented to 20).
        assert_eq!(ecs2.scene_lights(scene2).len(), 0);
    }

    #[test]
    fn parse_hex_color_handles_6_and_3_digit() {
        assert_eq!(parse_hex_color("#0a0e1a"), 0x0A0E1A);
        assert_eq!(parse_hex_color("#fff"), 0xFFFFFF); // shorthand expands
        assert_eq!(parse_hex_color("#abc"), 0xAABBCC);
        assert_eq!(parse_hex_color("bad"), 0); // malformed → fail-closed black
        assert_eq!(parse_hex_color("#12345"), 0); // wrong length → 0
    }

    #[test]
    fn lit_mask_gates_los_by_illumination_and_darkvision() {
        use serde_json::json;
        let player = Uuid::from_u128(7);
        let scene = Uuid::from_u128(10);

        // A normal-vision token at origin in a walled-open scene. lightingEnabled defaults true,
        // environmentLight, env intensity 0 → with NO lights the scene is dark → normal vision sees
        // nothing (fail-closed): the lit mask is empty.
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(player);
        let dark = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok.clone()], 0);
        assert!(
            dark.player_lit_mask(player)
                .iter()
                .all(|s| s.cells.is_empty()),
            "dark scene + normal vision → empty lit mask"
        );

        // Add a bright light covering the token's cell → that cell becomes visible at the bright band.
        let light = entity_doc_eng(
            20,
            10,
            "light",
            json!({
            "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true }),
        );
        let lit = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok.clone(), light], 0);
        let mask = lit.player_lit_mask(player);
        let s = mask
            .iter()
            .find(|s| s.scene == scene)
            .expect("scene present");
        assert!(
            s.cells
                .iter()
                .any(|&(i, j, band, _, _)| i == 0 && j == 0 && band == 0),
            "the lit cell at (0,0) is visible at the bright band (cell_size 100)"
        );

        // all_bright: a scene with lighting disabled makes every LOS cell visible at the bright
        // band even for a normal-vision token with NO lights present (spec §3/§6).
        let mut bright_scene = doc(10, None, "scene");
        bright_scene.engine = Some(
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                                      "lighting": { "enabled": false } }),
        );
        let mut ntok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        ntok.owner = Some(player);
        let ab = SceneEcs::from_documents(vec![bright_scene, ntok], 0).player_lit_mask(player);
        let s = ab.iter().find(|s| s.scene == scene).expect("scene present");
        assert!(
            s.cells
                .iter()
                .any(|&(i, j, band, _, _)| i == 0 && j == 0 && band == 0),
            "lighting-disabled scene → LOS cell visible at the bright band"
        );

        // Darkvision token in the SAME dark scene (no light) sees within range despite darkness.
        // Uses an embedded actor (instanced token path) because overrides.vision only applies to
        // linked tokens with a resolved actor_id; an instanced token reads embedded.actor[0].engine.vision.
        let mut dv = entity_doc_eng(
            12,
            10,
            "token",
            json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        dv.embedded.insert(
            "actor".into(),
            vec![entity_doc_top_eng(
                900,
                "actor",
                actor_body(json!([{ "mode": "darkvision", "range": 6 }])),
            )],
        );
        dv.owner = Some(player);
        let dvmask =
            SceneEcs::from_documents(vec![doc(10, None, "scene"), dv], 0).player_lit_mask(player);
        assert!(
            dvmask.iter().any(|s| !s.cells.is_empty()),
            "darkvision sees in the dark within range"
        );
    }

    #[test]
    fn lit_mask_tags_darkvision_only_cells_with_hint() {
        use serde_json::json;
        let player = Uuid::from_u128(7);
        // Dark scene (no lights, environmentLight, lighting on) → only darkvision admits cells.
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(player);
        tok.embedded.insert(
            "actor".into(),
            vec![{
                let mut a = doc(99, None, "actor");
                a.engine = Some(actor_body(json!([{ "mode": "darkvision", "range": 6 }])));
                a
            }],
        );
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok], 0);
        let mask = ecs.player_lit_mask(player);
        assert_eq!(mask.len(), 1);
        assert!(
            !mask[0].cells.is_empty(),
            "darkvision must see at least one cell in range"
        );
        assert!(
            mask[0]
                .cells
                .iter()
                .all(|(_, _, _, _, h)| h.as_deref() == Some("desaturate")),
            "dark cells perceived only via darkvision carry the desaturate hint"
        );

        // Bright cell under a light, seen by normal vision → no hint (normal floor suppresses it).
        let player2 = Uuid::from_u128(8);
        let mut tok2 = entity_doc_eng(
            12,
            10,
            "token",
            json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok2.owner = Some(player2); // no embedded vision → normal fallback
        let light = entity_doc_eng(
            20,
            10,
            "light",
            json!({
            "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
            "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true }),
        );
        let lit = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok2, light], 0);
        let mask2 = lit.player_lit_mask(player2);
        assert!(
            mask2[0].cells.iter().any(|(_, _, _, _, h)| h.is_none()),
            "a normally-lit cell seen by normal vision carries no hint"
        );
    }

    #[test]
    fn committed_seq_tracks_last_applied_command() {
        // The watermark is the seq emitted as `computed_at_seq`; it advances only
        // via set_committed_seq, called under the same write lock as apply_op so a
        // reader never sees a watermark ahead of (or behind) the entities.
        let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 3);
        assert_eq!(ecs.committed_seq(), 3);
        ecs.set_committed_seq(7);
        assert_eq!(ecs.committed_seq(), 7);
    }

    /// Exercises the world-settings/vision-modes side tables and the `actor` table's
    /// `set_actors`/`apply_op` mirroring. Deliberately stays on `/system`, not the typed
    /// `engine` band: `world_settings_doc()`/`vision_modes_doc()` return the raw side-table
    /// `Document`, and this test's own `apply_op` Update targets `/system/scene/...` and asserts
    /// against `.system.pointer(...)` directly — a doc-round-trip mechanism check against the
    /// side-table storage itself, with no engine-band reader in the loop to exercise.
    #[test]
    fn config_and_actor_side_tables_track_ops() {
        use serde_json::json;
        let mut ecs = SceneEcs::new();
        // Seed via setters (the room-hydration path).
        let mut ws = doc(100, None, "world-settings");
        ws.system = json!({ "scene": { "lightingEnabled": false } });
        ecs.set_world_config(Some(ws), None, None);
        ecs.set_actors(vec![entity_doc_top_eng(
            200,
            "actor",
            json!({ "vision": [] }),
        )]);
        assert!(ecs.actor(&Uuid::from_u128(200)).is_some());

        // A live Create of a vision-modes doc lands in the side table.
        ecs.apply_op(&Operation::Create {
            doc: doc(101, None, "vision-modes"),
        });
        assert!(ecs.vision_modes_doc().is_some());

        // A second world-settings Create REPLACES the singleton (the current authoritative doc wins).
        ecs.apply_op(&Operation::Create {
            doc: doc(110, None, "world-settings"),
        });
        assert_eq!(ecs.world_settings_doc().unwrap().id, Uuid::from_u128(110));

        // A field Update to the current world-settings singleton (id 110) is mirrored.
        ecs.apply_op(&Operation::Update {
            doc_id: Uuid::from_u128(110),
            changes: vec![crate::data::command::FieldChange {
                remove: false,
                path: "/system/scene/lightingEnabled".into(),
                old: json!(null),
                new: json!(true),
            }],
        });
        assert_eq!(
            ecs.world_settings_doc()
                .unwrap()
                .system
                .pointer("/scene/lightingEnabled"),
            Some(&json!(true))
        );

        // A Delete of the actor removes it.
        ecs.apply_op(&Operation::Delete {
            doc: doc(200, None, "actor"),
        });
        assert!(ecs.actor(&Uuid::from_u128(200)).is_none());
    }

    #[test]
    fn vision_modes_carry_render_hint() {
        use serde_json::json;
        // Absent doc → built-in seed mirrors scene-docs.ts: darkvision desaturates, normal does not.
        let seeded = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
        let m = seeded.resolved_vision_modes();
        assert_eq!(m["normal"].render_hint, None);
        assert_eq!(m["darkvision"].render_hint.as_deref(), Some("desaturate"));

        // Present doc → renderHint parsed; absent field → None.
        let mut vm = entity_doc_eng(30, 10, "vision-modes", json!({}));
        vm.doc_type = "vision-modes".into();
        vm.parent_id = None;
        vm.engine = Some(json!({ "modes": {
            "truesight": { "id": "truesight", "name": "Truesight",
                           "illuminationFloor": "dark", "defaultRange": 8, "renderHint": "outline" },
            "plain":     { "id": "plain", "name": "Plain",
                           "illuminationFloor": "dim",  "defaultRange": 0 }
        }}));
        let mut ecs = SceneEcs::new();
        ecs.set_world_config(None, None, Some(vm));
        let m = ecs.resolved_vision_modes();
        assert_eq!(m["truesight"].render_hint.as_deref(), Some("outline"));
        assert_eq!(m["plain"].render_hint, None);
    }

    #[test]
    fn token_vision_floors_include_render_hint() {
        use serde_json::json;
        // Instanced token with embedded actor granting normal + darkvision.
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.embedded.insert(
            "actor".into(),
            vec![{
                let mut a = doc(99, None, "actor");
                a.engine = Some(actor_body(json!([
                    { "mode": "normal", "range": 0 },
                    { "mode": "darkvision", "range": 6 }
                ])));
                a
            }],
        );
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok.clone()], 0);
        let floors = ecs.token_vision_floors(&tok);
        // darkvision entry carries the desaturate hint; normal carries none.
        assert!(floors
            .iter()
            .any(|(_, _, h)| h.as_deref() == Some("desaturate")));
        assert!(floors.iter().any(|(_, _, h)| h.is_none()));
    }

    // --- Test helpers for movement-restriction resolution tests ---

    /// Set `world_settings` to a doc whose `engine` is `json_engine` (test-only).
    /// Mirrors how `room.rs` builds a world-settings config doc.
    #[cfg(test)]
    impl SceneEcs {
        pub(crate) fn set_world_settings_for_test(&mut self, json_engine: serde_json::Value) {
            let mut d = crate::data::document::tests::world_scoped_doc(
                Uuid::from_u128(9),
                Uuid::from_u128(100),
                "world-settings",
            );
            d.engine = Some(json_engine);
            self.world_settings = Some(d);
        }

        pub(crate) fn insert_scene_for_test(
            &mut self,
            scene_id: Uuid,
            json_engine: serde_json::Value,
        ) {
            let mut d = crate::data::document::tests::world_scoped_doc(
                Uuid::from_u128(9),
                scene_id,
                "scene",
            );
            d.engine = Some(json_engine);
            // Remove stale entity if re-inserting.
            if let Some(old_e) = self.index.remove(&scene_id) {
                let _ = self.world.despawn(old_e);
            }
            let e = self.world.spawn((SceneEntity { doc: d },));
            self.index.insert(scene_id, e);
        }
    }

    /// A COMPLETE `WorldSettingsEngine` body (all `WorldSceneDefaults` fields present, per the
    /// ingress-validated struct's `deny_unknown_fields` contract) with `patches` applied over the
    /// built-in default via JSON-pointer `set_pointer` — lets each test express only the field(s)
    /// it cares about instead of re-typing the full 9-field `scene` object every time.
    fn ws_body(patches: &[(&str, serde_json::Value)]) -> serde_json::Value {
        let mut v = serde_json::to_value(eng::WorldSettingsEngine::default()).unwrap();
        for (path, val) in patches {
            let _ = set_pointer(&mut v, path, val.clone());
        }
        v
    }

    #[test]
    fn diagonal_rule_defaults_to_chebyshev_without_world_settings() {
        let ecs = SceneEcs::new();
        assert_eq!(
            ecs.resolved_diagonal_rule(),
            crate::scene::pathfinding::DiagonalRule::Chebyshev
        );
    }

    #[test]
    fn diagonal_rule_falls_back_when_structural_keys_absent() {
        // A world-settings doc with `pathfinding.diagonalRule:"alternating"` but missing `scene`
        // or `animation` must resolve to `Chebyshev` — the structural guard (same as resolve_scene)
        // must reject a partial doc rather than partially resolving.
        use serde_json::json;
        let mut ecs = SceneEcs::new();

        // Missing `scene` key entirely.
        ecs.set_world_settings_for_test(json!({
            "pathfinding": { "diagonalRule": "alternating" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        }));
        assert_eq!(
            ecs.resolved_diagonal_rule(),
            crate::scene::pathfinding::DiagonalRule::Chebyshev,
            "missing scene key must fall back to Chebyshev"
        );

        // Missing `animation` key entirely.
        ecs.set_world_settings_for_test(json!({
            "scene": { "movementRestriction": "visible" },
            "pathfinding": { "diagonalRule": "alternating" }
        }));
        assert_eq!(
            ecs.resolved_diagonal_rule(),
            crate::scene::pathfinding::DiagonalRule::Chebyshev,
            "missing animation key must fall back to Chebyshev"
        );
    }

    #[test]
    fn diagonal_rule_reads_world_settings_and_unknown_falls_back() {
        use serde_json::json;
        let mut ecs = SceneEcs::new();
        ecs.set_world_settings_for_test(ws_body(&[(
            "/pathfinding/diagonalRule",
            json!("alternating"),
        )]));
        assert_eq!(
            ecs.resolved_diagonal_rule(),
            crate::scene::pathfinding::DiagonalRule::Alternating
        );

        ecs.set_world_settings_for_test(json!({
            "scene": {}, "pathfinding": { "diagonalRule": "bogus" }, "animation": {}
        }));
        assert_eq!(
            ecs.resolved_diagonal_rule(),
            crate::scene::pathfinding::DiagonalRule::Chebyshev,
            "unknown rule fails to chebyshev (mirrors client default)"
        );
    }

    #[test]
    fn resolve_scene_movement_restriction_defaults_to_visible_and_lenient() {
        // No world-settings doc, no scene override → built-in defaults.
        let ecs = SceneEcs::new();
        let r = ecs.resolve_scene(Uuid::from_u128(1));
        assert_eq!(r.movement_restriction, MovementRestriction::Visible);
        assert!(r.partial_cell_leniency);
    }

    #[test]
    fn resolve_scene_movement_restriction_world_override_and_leniency_off() {
        use serde_json::json;
        let mut ecs = SceneEcs::new();
        ecs.set_world_settings_for_test(ws_body(&[
            ("/scene/movementRestriction", json!("revealed")),
            ("/scene/partialCellLeniency", json!(false)),
        ]));
        let r = ecs.resolve_scene(Uuid::from_u128(1));
        assert_eq!(r.movement_restriction, MovementRestriction::Revealed);
        assert!(
            !r.partial_cell_leniency,
            "partialCellLeniency is world-only and was set false"
        );
    }

    #[test]
    fn resolve_scene_movement_restriction_scene_override_beats_world() {
        use serde_json::json;
        let mut ecs = SceneEcs::new();
        let scene_id = Uuid::from_u128(7);
        ecs.set_world_settings_for_test(ws_body(&[(
            "/scene/movementRestriction",
            json!("visible"),
        )]));
        // Scene overrides vision.movementRestriction to "unrestricted".
        ecs.insert_scene_for_test(
            scene_id,
            json!({
                "grid": { "kind": "square", "size": 100 },
                "background": null,
                "vision": { "movementRestriction": "unrestricted" }
            }),
        );
        let r = ecs.resolve_scene(scene_id);
        assert_eq!(r.movement_restriction, MovementRestriction::Unrestricted);
        // partialCellLeniency has NO scene override → still the world default (true here).
        assert!(r.partial_cell_leniency);
    }

    #[test]
    fn resolve_scene_movement_restriction_null_override_inherits_world() {
        use serde_json::json;
        let mut ecs = SceneEcs::new();
        let scene_id = Uuid::from_u128(8);
        ecs.set_world_settings_for_test(ws_body(&[(
            "/scene/movementRestriction",
            json!("revealed"),
        )]));
        // null clears the override → inherit world "revealed" (mirrors `?? d.scene.movementRestriction`).
        ecs.insert_scene_for_test(
            scene_id,
            json!({
                "grid": { "kind": "square", "size": 100 },
                "background": null,
                "vision": { "movementRestriction": null }
            }),
        );
        let r = ecs.resolve_scene(scene_id);
        assert_eq!(r.movement_restriction, MovementRestriction::Revealed);
    }

    #[test]
    fn resolve_scene_movement_model_defaults_to_grid_stepped() {
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
        let r = ecs.resolve_scene(Uuid::from_u128(10));
        assert_eq!(r.movement_model, MovementModel::GridStepped);
    }

    #[test]
    fn resolve_scene_movement_model_world_override_to_continuous() {
        let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
        ecs.set_world_settings_for_test(json!({
            "scene": {
                "losRestriction": true, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#0a0e1a", "intensity": 0.0 },
                "observerVision": false,
                "movementRestriction": "visible",
                "movementModel": "continuous",
                "partialCellLeniency": true
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        }));
        let r = ecs.resolve_scene(Uuid::from_u128(10));
        assert_eq!(r.movement_model, MovementModel::Continuous);
    }

    #[test]
    fn resolve_scene_movement_model_scene_override_beats_world() {
        let mut ecs = SceneEcs::from_documents(
            vec![entity_doc_top_eng(
                10,
                "scene",
                json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                        "vision": { "movementModel": "continuous" } }),
            )],
            0,
        );
        ecs.set_world_settings_for_test(json!({
            "scene": {
                "losRestriction": true, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#0a0e1a", "intensity": 0.0 },
                "observerVision": false,
                "movementRestriction": "visible",
                "movementModel": "grid-stepped",
                "partialCellLeniency": true
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        }));
        let r = ecs.resolve_scene(Uuid::from_u128(10));
        assert_eq!(r.movement_model, MovementModel::Continuous);
    }

    #[test]
    fn resolve_scene_movement_model_null_scene_override_inherits_world() {
        let mut ecs = SceneEcs::from_documents(
            vec![entity_doc_top_eng(
                10,
                "scene",
                json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                        "vision": { "movementModel": null } }),
            )],
            0,
        );
        ecs.set_world_settings_for_test(json!({
            "scene": {
                "losRestriction": true, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#0a0e1a", "intensity": 0.0 },
                "observerVision": false,
                "movementRestriction": "visible",
                "movementModel": "continuous",
                "partialCellLeniency": true
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        }));
        let r = ecs.resolve_scene(Uuid::from_u128(10));
        assert_eq!(r.movement_model, MovementModel::Continuous);
    }

    #[test]
    fn resolve_scene_bounds_defaults_when_absent() {
        let ecs = SceneEcs::new();
        let r = ecs.resolve_scene(Uuid::from_u128(1));
        assert_eq!(r.bounds, (100.0, 100.0));
    }

    #[test]
    fn resolve_scene_bounds_reads_authored_value() {
        use serde_json::json;
        let mut ecs = SceneEcs::new();
        let scene_id = Uuid::from_u128(2);
        ecs.insert_scene_for_test(
            scene_id,
            json!({
                "grid": { "kind": "square", "size": 100 },
                "bounds": { "width": 40.0, "height": 25.0 }
            }),
        );
        let r = ecs.resolve_scene(scene_id);
        assert_eq!(r.bounds, (40.0, 25.0));
    }

    #[test]
    fn resolve_scene_bounds_fail_closed_on_degenerate() {
        use serde_json::json;
        let mut ecs = SceneEcs::new();
        let scene_id = Uuid::from_u128(3);
        // Zero width + negative height are degenerate for a navmesh rectangle → default.
        ecs.insert_scene_for_test(
            scene_id,
            json!({
                "grid": { "kind": "square", "size": 100 },
                "bounds": { "width": 0.0, "height": -5.0 }
            }),
        );
        let r = ecs.resolve_scene(scene_id);
        assert_eq!(r.bounds, (100.0, 100.0));
    }

    #[test]
    fn lit_mask_suppresses_hint_when_normal_floor_wins_in_bright_cell() {
        use serde_json::json;
        // Combined-token suppression (buddy-check A1): an owned token whose embedded actor has
        // BOTH normal (floor=dim 0.34) AND darkvision (floor=dark 0.0).  Standing in a brightly-lit
        // cell (light placed at the token), normal's floor (0.34) is higher than darkvision's (0.0),
        // so normal is the highest-admitting mode → its hint (None) wins → lit cells carry no hint.
        let player = Uuid::from_u128(42);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(player);
        tok.embedded.insert(
            "actor".into(),
            vec![{
                let mut a = doc(99, None, "actor");
                a.engine = Some(actor_body(json!([
                    { "mode": "normal",     "range": 0 },
                    { "mode": "darkvision", "range": 6 }
                ])));
                a
            }],
        );
        // A bright light at the token location illuminates the cell at (0,0) above dim threshold.
        let light = entity_doc_eng(
            20,
            10,
            "light",
            json!({
                "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
                "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true
            }),
        );
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok, light], 0);
        let mask = ecs.player_lit_mask(player);
        let lit_cells: Vec<_> = mask.iter().flat_map(|s| s.cells.iter()).collect();
        assert!(
            !lit_cells.is_empty(),
            "token with normal+darkvision under bright light must see at least one cell"
        );
        // Every lit cell must carry None: normal's floor (0.34) > darkvision's floor (0.0),
        // so normal is the highest-admitting mode and its None hint suppresses desaturate.
        assert!(
            lit_cells.iter().all(|(_, _, _, _, h)| h.is_none()),
            "normal-floor wins in bright cell: desaturate hint must be suppressed (None)"
        );
    }

    #[test]
    fn cell_visible_predicate_honors_floor_and_range() {
        // floors: (floor_min_value, range_cells, render_hint). A normal mode (floor "dim" ~0.34),
        // range 0 = unbounded. Lit level 1.0 ≥ 0.34 → visible; 0.1 < 0.34 → not.
        let normal = vec![(0.34_f64, 0.0_f64, None)];
        assert!(cell_visible(&normal, 1.0, 5.0));
        assert!(!cell_visible(&normal, 0.1, 5.0));
        // Darkvision floor 0.0 within range 6 admits an unlit cell; beyond range it does not.
        let dark = vec![(0.0_f64, 6.0_f64, Some("desaturate".into()))];
        assert!(
            cell_visible(&dark, 0.0, 3.0),
            "unlit but within darkvision range"
        );
        assert!(
            !cell_visible(&dark, 0.0, 9.0),
            "beyond darkvision range, unlit → not visible"
        );
        // No in-range mode → not visible (fail closed).
        assert!(!cell_visible(&[], 1.0, 1.0));
    }

    /// Builds a SceneEcs with one scene (id 10), one player-owned token at (50, 50), and one
    /// enabled white light at (50, 50) with bright=3 / dim=6 cells. The token has normal vision
    /// (default), so cells within the lit radius are visible. Returns `(ecs, user, scene_id)`.
    fn scene_with_lit_player_token() -> (SceneEcs, Uuid, Uuid) {
        let user = Uuid::from_u128(7);
        let scene_id = Uuid::from_u128(10);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        let light = entity_doc_eng(
            20,
            10,
            "light",
            json!({
                "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
                "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true
            }),
        );
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok, light], 0);
        (ecs, user, scene_id)
    }

    /// Builds a SceneEcs whose light boundary crosses through cell (1,1), guaranteeing a
    /// lenient-only cell. Cell (1,1) has center at (150, 150), distance ≈ 141.4 from the light
    /// at (50, 50) — just beyond `dimRadius = 140` (1.4 cells × 100 units/cell), so
    /// `cell_illumination` returns 0 for the center and strict rejects it. Corner (100, 100) is
    /// at distance ≈ 70.7 < 140 → illuminated → lenient admits it. No sight walls + los_restriction
    /// defaults false → the bound-box polygon covers all cells, so the LOS test never rejects a
    /// corner. Returns `(ecs, user, scene_id)`.
    fn scene_with_boundary_crossing_light() -> (SceneEcs, Uuid, Uuid) {
        let user = Uuid::from_u128(7);
        let scene_id = Uuid::from_u128(10);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        // dimRadius = 1.4 cells (140 scene units): center of (1,1) at distance ≈141.4 > 140 (strict miss);
        // corner (100,100) at distance ≈70.7 < 140 (lenient hit).
        let light = entity_doc_eng(
            20,
            10,
            "light",
            json!({
                "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
                "brightRadius": 0.5, "dimRadius": 1.4, "enabled": true
            }),
        );
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok, light], 0);
        (ecs, user, scene_id)
    }

    #[test]
    fn visible_cells_strict_equals_player_lit_mask_cells() {
        // §13 parity: under strict (center-only) sampling, the movement gate mask must equal the
        // egress secrecy mask for the scene. Both paths use the same cell_visible predicate and
        // lighting_inputs, so any divergence is a sampling or illumination bug.
        let (ecs, user, scene) = scene_with_lit_player_token();
        let strict: std::collections::BTreeSet<(i32, i32)> = ecs.visible_cells(user, scene, false);
        let egress: std::collections::BTreeSet<(i32, i32)> = ecs
            .player_lit_mask(user)
            .into_iter()
            .filter(|s| s.scene == scene)
            .flat_map(|s| s.cells.into_iter().map(|(i, j, _b, _t, _h)| (i, j)))
            .collect();
        assert_eq!(
            strict, egress,
            "strict gate mask must equal the egress secrecy mask"
        );
        assert!(!strict.is_empty());
    }

    /// §13 parity helper: asserts `visible_cells(user, scene, false)` == the `(i,j)` set of
    /// `player_lit_mask(user)` filtered to `scene`, and that neither set is empty (non-vacuous).
    fn assert_strict_parity(ecs: &SceneEcs, user: Uuid, scene: Uuid) {
        let strict: std::collections::BTreeSet<(i32, i32)> = ecs.visible_cells(user, scene, false);
        let egress: std::collections::BTreeSet<(i32, i32)> = ecs
            .player_lit_mask(user)
            .into_iter()
            .filter(|s| s.scene == scene)
            .flat_map(|s| s.cells.into_iter().map(|(i, j, _b, _t, _h)| (i, j)))
            .collect();
        assert!(
            !strict.is_empty(),
            "parity check must be non-vacuous (strict set empty)"
        );
        assert!(
            !egress.is_empty(),
            "parity check must be non-vacuous (egress set empty)"
        );
        assert_eq!(
            strict, egress,
            "strict gate mask must equal the egress secrecy mask"
        );
    }

    /// An `environmentLight` scene at env intensity 1.0 with a player-owned normal-vision token
    /// at cell (0,0) and a 4-wall box sealing cell (3,3) (center (350,350)). The walls are
    /// `blocksSight` (so `bound_for` grows the scan to include the room) but LOS is OFF (so the
    /// LOS polygon is the plain bound rectangle — no LOS occlusion, isolating the env-occlusion
    /// effect). `blocks_light` toggles whether the box occludes the boundary-projected environment
    /// light: `true` seals the interior (env cannot reach), `false` is the no-occlusion baseline.
    fn env_lit_scene_with_room(blocks_light: bool) -> (SceneEcs, Uuid, Uuid) {
        let user = Uuid::from_u128(7);
        let scene_id = Uuid::from_u128(10);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        let wall = |id: u128, x1: f64, y1: f64, x2: f64, y2: f64| {
            entity_doc_eng(
                id,
                10,
                "wall",
                json!({ "seg": {"x1":x1,"y1":y1,"x2":x2,"y2":y2},
                        "blocksSight": true, "blocksMove": false, "blocksLight": blocks_light }),
            )
        };
        let scene = entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                    "bounds": { "width": 6.0, "height": 6.0 } }),
        );
        let mut ecs = SceneEcs::from_documents(
            vec![
                scene,
                tok,
                wall(31, 300.0, 300.0, 400.0, 300.0),
                wall(32, 400.0, 300.0, 400.0, 400.0),
                wall(33, 400.0, 400.0, 300.0, 400.0),
                wall(34, 300.0, 400.0, 300.0, 300.0),
            ],
            0,
        );
        ecs.set_world_settings_for_test(json!({
            "scene": {
                "losRestriction": false, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#ffffff", "intensity": 1.0 },
                "observerVision": false,
                "movementRestriction": "visible",
                "movementModel": "grid-stepped",
                "partialCellLeniency": false
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        }));
        (ecs, user, scene_id)
    }

    /// The `(i,j)` cell set of `player_lit_mask(user)` restricted to `scene`.
    fn mask_cells(
        ecs: &SceneEcs,
        user: Uuid,
        scene: Uuid,
    ) -> std::collections::BTreeSet<(i32, i32)> {
        ecs.player_lit_mask(user)
            .into_iter()
            .filter(|s| s.scene == scene)
            .flat_map(|s| s.cells.into_iter().map(|(i, j, _b, _t, _h)| (i, j)))
            .collect()
    }

    #[test]
    fn env_light_occlusion_narrows_the_mask_and_seals_the_interior() {
        // Option B: env light is a genuine (fail-closed) visibility input. A blocksLight-sealed
        // interior stops being visible to a normal-vision player; occlusion only REMOVES cells.
        let (ecs_after, user, scene) = env_lit_scene_with_room(true); // sealed
        let (ecs_before, _, _) = env_lit_scene_with_room(false); // no occlusion baseline
        let after = mask_cells(&ecs_after, user, scene);
        let before = mask_cells(&ecs_before, user, scene);
        let interior = (3, 3); // center (350,350), inside the sealed box
        assert!(
            before.contains(&interior),
            "baseline (blocksLight:false) still lights the interior"
        );
        assert!(
            !after.contains(&interior),
            "a blocksLight-sealed interior must drop out of a normal-vision player's mask"
        );
        assert!(
            after.is_subset(&before),
            "env occlusion is strictly narrowing: it only removes cells, never adds any"
        );
        assert!(
            after.contains(&(0, 0)),
            "the open exterior (the token's own cell) stays lit and visible"
        );
    }

    /// A wall-less `environmentLight`/`globalIllumination` scene (env 1.0) with a player-owned
    /// normal-vision token, used to prove open-scene equivalence: with nothing to occlude, the
    /// edge-projected env light reaches every LOS cell exactly as the flat all-bright fill does.
    fn open_env_lit_scene(mode: &str) -> (SceneEcs, Uuid, Uuid) {
        let user = Uuid::from_u128(7);
        let scene_id = Uuid::from_u128(10);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        let scene = entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                    "bounds": { "width": 6.0, "height": 6.0 } }),
        );
        let mut ecs = SceneEcs::from_documents(vec![scene, tok], 0);
        ecs.set_world_settings_for_test(json!({
            "scene": {
                "losRestriction": false, "fog": true,
                "lightingEnabled": true, "lightMode": mode,
                "environment": { "color": "#ffffff", "intensity": 1.0 },
                "observerVision": false,
                "movementRestriction": "visible",
                "movementModel": "grid-stepped",
                "partialCellLeniency": false
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        }));
        (ecs, user, scene_id)
    }

    #[test]
    fn env_light_open_scene_equals_global_illumination_no_holes() {
        // Open-scene equivalence: where there is nothing to occlude, edge-projected env at
        // intensity 1.0 must reach every LOS cell — identical to globalIllumination's all-bright
        // fill. A spurious occlusion hole (e.g. too-sparse boundary sampling) would drop a cell
        // here and break the equality.
        let (env_ecs, user, scene) = open_env_lit_scene("environmentLight");
        let (gi_ecs, _, _) = open_env_lit_scene("globalIllumination");
        let env_mask = mask_cells(&env_ecs, user, scene);
        let gi_mask = mask_cells(&gi_ecs, user, scene);
        assert!(!env_mask.is_empty(), "open env-lit scene is non-empty");
        assert_eq!(
            env_mask, gi_mask,
            "wall-less env=1.0 mask equals the flat all-bright mask (no occlusion holes)"
        );
    }

    #[test]
    fn strict_parity_holds_with_env_light_occlusion() {
        // §13 anti-drift with env occlusion active: the movement gate (visible_cells strict) must
        // still equal the egress secrecy mask (player_lit_mask cells) when a blocksLight-sealed
        // interior narrows both. Both consume the SAME env_polys via the same cell_illumination.
        let (ecs, user, scene) = env_lit_scene_with_room(true);
        assert_strict_parity(&ecs, user, scene);
    }

    #[test]
    fn visible_cells_strict_parity_global_illumination() {
        // §13 parity under globalIllumination: all LOS cells are all_bright. With no placed lights
        // the all_bright arm fires for both paths, so any divergence would be in the all_bright
        // branch — this guards it.
        use serde_json::json;
        let user = Uuid::from_u128(7);
        let scene_id = Uuid::from_u128(10);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        // lightMode = globalIllumination: lighting_enabled true, all cells bright, env tint applied.
        // No placed lights — confirms the all_bright short-circuit path in both player_lit_mask
        // and visible_cells fires identically.
        let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok], 0);
        ecs.set_world_settings_for_test(json!({
            "scene": {
                "losRestriction": false, "fog": true,
                "lightingEnabled": true, "lightMode": "globalIllumination",
                "environment": { "color": "#ffffff", "intensity": 1.0 },
                "observerVision": false,
                "movementRestriction": "visible",
                "movementModel": "grid-stepped",
                "partialCellLeniency": false
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        }));
        assert_strict_parity(&ecs, user, scene_id);
    }

    #[test]
    fn visible_cells_strict_parity_darkvision() {
        // §13 parity for a darkvision token in a dark scene (no placed lights, env intensity=0).
        // The darkvision floor (0.0) admits unlit-but-in-range cells; normal vision would see
        // nothing. Both paths must agree on exactly those cells.
        use serde_json::json;
        let user = Uuid::from_u128(7);
        let scene_id = Uuid::from_u128(10);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        // Embedded actor granting darkvision range 6 (mirrors lit_mask_gates_los test pattern).
        tok.embedded.insert(
            "actor".into(),
            vec![{
                let mut a = doc(99, None, "actor");
                a.engine = Some(actor_body(json!([{ "mode": "darkvision", "range": 6 }])));
                a
            }],
        );
        // Dark scene: lighting on, environmentLight, env intensity=0, no lights → only darkvision
        // cells are visible. losRestriction=false keeps the LOS polygon as the full bound box so
        // every in-range unlit cell is admitted — the test is purely about the floor/range gate.
        let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok], 0);
        ecs.set_world_settings_for_test(json!({
            "scene": {
                "losRestriction": false, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#000000", "intensity": 0.0 },
                "observerVision": false,
                "movementRestriction": "visible",
                "partialCellLeniency": false
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        }));
        assert_strict_parity(&ecs, user, scene_id);
    }

    #[test]
    fn visible_cells_strict_parity_los_restriction_with_occluding_wall() {
        // §13 parity with losRestriction=true and a blocksSight wall that occludes some cells.
        // Both paths use source_los_poly (the shared raycast), so any divergence would be in
        // per-cell sampling AFTER the LOS polygon is built — this guards the occluded-scene path.
        use serde_json::json;
        let user = Uuid::from_u128(7);
        let scene_id = Uuid::from_u128(10);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        // A blocksSight wall at x=200 (column 2) cuts off the right half of the scene from
        // the token at (50,50). Combined with a bright light at the token, cells to the left of
        // the wall are lit+visible; cells beyond the wall are occluded by LOS.
        let wall = entity_doc_eng(
            30,
            10,
            "wall",
            json!({ "seg": { "x1": 200, "y1": -200, "x2": 200, "y2": 400 }, "blocksSight": true }),
        );
        let light = entity_doc_eng(
            20,
            10,
            "light",
            json!({
                "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
                "brightRadius": 5.0, "dimRadius": 8.0, "enabled": true
            }),
        );
        let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok, wall, light], 0);
        ecs.set_world_settings_for_test(json!({
            "scene": {
                "losRestriction": true, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#000000", "intensity": 0.0 },
                "observerVision": false,
                "movementRestriction": "visible",
                "partialCellLeniency": false
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        }));
        assert_strict_parity(&ecs, user, scene_id);
    }

    #[test]
    fn visible_cells_lenient_is_a_superset_of_strict() {
        // Uses scene_with_boundary_crossing_light: the light boundary (dimRadius 1.4 cells =
        // 140 scene units) cuts through cell (1,1). Center (150,150) is ~141.4 units away →
        // outside dim radius → strict rejects it. Corner (100,100) is ~70.7 units away → inside
        // dim radius → lenient admits it. This guarantees at least one lenient-only cell, so the
        // corner-sampling path is live and proven, not vacuously skipped.
        let (ecs, user, scene) = scene_with_boundary_crossing_light();
        let strict = ecs.visible_cells(user, scene, false);
        let lenient = ecs.visible_cells(user, scene, true);
        // Subset invariant: every strict cell is also in lenient.
        assert!(
            strict.iter().all(|c| lenient.contains(c)),
            "lenient ⊇ strict"
        );
        // Strict superset: lenient must contain at least one cell strict does not.
        assert!(
            lenient.len() > strict.len(),
            "lenient must admit at least one corner-only cell not in strict"
        );
        // Non-empty difference set: the corner path is proven live.
        assert!(
            lenient.difference(&strict).next().is_some(),
            "difference(lenient, strict) must be non-empty"
        );
    }

    #[test]
    fn visible_cells_empty_when_user_has_no_source_in_scene() {
        let (ecs, _user, scene) = scene_with_lit_player_token();
        let stranger = Uuid::from_u128(999);
        assert!(
            ecs.visible_cells(stranger, scene, true).is_empty(),
            "no sources → empty (fail closed)"
        );
    }

    #[test]
    fn movement_gate_mask_cache_invalidates_on_wall_mutation() {
        // `visible_cells_cached` must never serve a cell an occluding wall has since removed from
        // view: a stale cache must fail toward recompute, never toward a wider mask.
        use serde_json::json;
        let user = Uuid::from_u128(7);
        let scene_id = Uuid::from_u128(10);
        // Grid size 20 (not the usual 100) so the target cell sits comfortably inside the
        // no-walls-yet `bound_for` margin box (±100 around the token) rather than on its edge.
        let scene = entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 20 }, "background": null }),
        );
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 10, "y": 10, "w": 20.0, "h": 20.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        let light = entity_doc_eng(
            20,
            10,
            "light",
            json!({
                "x": 10.0, "y": 10.0, "color": "#ffffff", "intensity": 1.0,
                "brightRadius": 5.0, "dimRadius": 8.0, "enabled": true
            }),
        );
        let mut ecs = SceneEcs::from_documents(vec![scene, tok, light], 0);
        ecs.set_world_settings_for_test(json!({
            "scene": {
                "losRestriction": true, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#000000", "intensity": 0.0 },
                "observerVision": false,
                "movementRestriction": "visible",
                "partialCellLeniency": false
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        }));

        // Cell (2,0), center (50,10): 40 scene units (2 cells) from the token at (10,10), well
        // within the 160-unit (8-cell) dim radius, and on the token's LOS with no wall present.
        let target_cell = (2, 0);
        let mask1 = ecs.visible_cells_cached(user, scene_id, false);
        assert!(
            mask1.contains(&target_cell),
            "cell visible before a blocksSight wall is added: {mask1:?}"
        );

        // A vertical blocksSight wall at x=30, between the token (x=10) and target_cell's center
        // (x=50) — an ordinary `apply_op` Create, the same path a real `Room::publish` Create op
        // takes.
        let wall = entity_doc_eng(
            30,
            10,
            "wall",
            json!({ "seg": { "x1": 30, "y1": -400, "x2": 30, "y2": 400 }, "blocksSight": true }),
        );
        ecs.apply_op(&Operation::Create { doc: wall });

        let mask2 = ecs.visible_cells_cached(user, scene_id, false);
        assert!(
            !mask2.contains(&target_cell),
            "cache must invalidate on wall mutation, never serve a stale wider mask: {mask2:?}"
        );
    }

    #[test]
    fn movement_gate_mask_cache_reused_across_repeated_moves_with_no_scene_change() {
        let (ecs, user, scene) = scene_with_lit_player_token();

        let mask1 = ecs.visible_cells_cached(user, scene, false);
        assert_eq!(
            ecs.visible_cells_recompute_count(),
            1,
            "first call is always a recompute (cold cache)"
        );

        let mask2 = ecs.visible_cells_cached(user, scene, false);
        assert_eq!(mask1, mask2);
        assert_eq!(
            ecs.visible_cells_recompute_count(),
            1,
            "a repeated call with no input change must be served from the cache, not recomputed"
        );

        // Sanity: `visible_cells` (the uncached primitive `visible_cells_cached` wraps) agrees.
        assert_eq!(mask1, ecs.visible_cells(user, scene, false));
    }

    /// Build an ECS with one `blocksMove` wall and one non-blocking wall in the same scene.
    /// The blocking wall runs from (100,0) to (100,200); the non-blocking wall runs elsewhere.
    fn scene_with_two_walls_one_blocking() -> (SceneEcs, Uuid) {
        let scene = Uuid::from_u128(10);
        let blocking =
            json!({ "seg": {"x1": 100, "y1": 0, "x2": 100, "y2": 200}, "blocksMove": true });
        let non_blocking =
            json!({ "seg": {"x1": 0, "y1": 0, "x2": 50, "y2": 50}, "blocksMove": false });
        let ecs = SceneEcs::from_documents(
            vec![
                doc(10, None, "scene"),
                entity_doc_eng(11, 10, "wall", blocking),
                entity_doc_eng(12, 10, "wall", non_blocking),
            ],
            0,
        );
        (ecs, scene)
    }

    #[test]
    fn region_field_authoritative_includes_secret_regions_visible_excludes_them() {
        use crate::data::document::Visibility;
        let scene_id = Uuid::from_u128(10);
        let owner = Uuid::from_u128(1);
        let player = Uuid::from_u128(2);

        let mut secret = crate::data::document::tests::world_scoped_doc(
            Uuid::from_u128(9),
            Uuid::from_u128(20),
            "region",
        );
        secret.parent_id = Some(scene_id);
        secret.owner = Some(owner);
        secret.engine = Some(serde_json::json!({
            "shape": { "kind": "rect", "points": [0.0, 0.0, 100.0, 100.0] },
            "behavior": "impassable",
            "cost": 1.0,
            "enabled": true,
        }));
        secret
            .permissions
            .property_overrides
            .insert("/engine".into(), Visibility::GmOnly);

        let mut visible = crate::data::document::tests::world_scoped_doc(
            Uuid::from_u128(9),
            Uuid::from_u128(21),
            "region",
        );
        visible.parent_id = Some(scene_id);
        visible.engine = Some(serde_json::json!({
            "shape": { "kind": "rect", "points": [200.0, 0.0, 300.0, 100.0] },
            "behavior": "terrain",
            "cost": 2.0,
            "enabled": true,
        }));

        let ecs = SceneEcs::from_documents(
            vec![
                crate::data::document::tests::world_scoped_doc(
                    Uuid::from_u128(9),
                    scene_id,
                    "scene",
                ),
                secret,
                visible,
            ],
            0,
        );

        let authoritative = ecs.region_field(scene_id, None);
        assert!(
            authoritative.is_impassable((0, 0)),
            "authoritative field includes the secret region"
        );
        assert_eq!(authoritative.terrain_multiplier((2, 0)), 2.0);

        let player_field = ecs.region_field(scene_id, Some(player));
        assert!(
            !player_field.is_impassable((0, 0)),
            "secret region absent from a non-owner player's field"
        );
        assert_eq!(
            player_field.terrain_multiplier((2, 0)),
            2.0,
            "visible region still present"
        );
    }

    #[test]
    fn region_field_ignores_disabled_regions() {
        let scene_id = Uuid::from_u128(10);
        let mut disabled = crate::data::document::tests::world_scoped_doc(
            Uuid::from_u128(9),
            Uuid::from_u128(20),
            "region",
        );
        disabled.parent_id = Some(scene_id);
        disabled.engine = Some(serde_json::json!({
            "shape": { "kind": "rect", "points": [0.0, 0.0, 100.0, 100.0] },
            "behavior": "impassable",
            "cost": 1.0,
            "enabled": false,
        }));
        let ecs = SceneEcs::from_documents(
            vec![
                crate::data::document::tests::world_scoped_doc(
                    Uuid::from_u128(9),
                    scene_id,
                    "scene",
                ),
                disabled,
            ],
            0,
        );
        assert!(!ecs.region_field(scene_id, None).is_impassable((0, 0)));
    }

    #[test]
    fn move_walls_returns_only_blocks_move_segments_for_the_scene() {
        // A scene with one blocksMove wall and one non-blocksMove wall yields exactly the blocking segment.
        let (ecs, scene) = scene_with_two_walls_one_blocking();
        let walls = ecs.move_walls(scene);
        assert_eq!(walls.len(), 1, "only the blocksMove wall is returned");
        let w = walls[0];
        assert_eq!((w.a, w.b), ((100.0, 0.0), (100.0, 200.0)));
    }

    #[test]
    fn navmesh_for_is_memoized_across_calls() {
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
        let scene = Uuid::from_u128(10);
        let a = ecs.navmesh_for(scene, 0.4).expect("navmesh builds");
        let b = ecs.navmesh_for(scene, 0.4).expect("navmesh builds");
        assert!(
            std::sync::Arc::ptr_eq(&a, &b),
            "same (scene, radius) must return the SAME cached Arc, not rebuild"
        );
    }

    #[test]
    fn navmesh_for_distinguishes_footprint_radii() {
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
        let scene = Uuid::from_u128(10);
        let a = ecs.navmesh_for(scene, 0.4).expect("navmesh builds");
        let b = ecs.navmesh_for(scene, 0.9).expect("navmesh builds");
        assert!(
            !std::sync::Arc::ptr_eq(&a, &b),
            "distinct footprint radii must get distinct cached meshes"
        );
    }

    #[test]
    fn navmesh_for_rejects_degenerate_radius_even_after_cache_primed_at_zero() {
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
        let scene = Uuid::from_u128(10);
        // Prime the cache at footprint_radius_cells == 0.0: quantized key (scene, 0).
        let primed = ecs.navmesh_for(scene, 0.0);
        assert!(
            primed.is_some(),
            "radius 0.0 must build and cache successfully"
        );

        // f64 as i64 saturates NaN to 0, colliding with the primed key above. Without an
        // upfront validation guard this would return the CACHED radius-0.0 mesh instead of
        // failing closed.
        assert!(
            ecs.navmesh_for(scene, f64::NAN).is_none(),
            "NaN footprint radius must fail closed, not reuse the cached radius-0.0 mesh"
        );

        // A small negative rounds to -0 under `(x * 1000.0).round() as i64`, which also casts
        // to the same colliding key.
        assert!(
            ecs.navmesh_for(scene, -0.0001).is_none(),
            "negative footprint radius must fail closed, not reuse the cached radius-0.0 mesh"
        );
    }

    #[test]
    fn wall_mutation_invalidates_the_navmesh_cache() {
        let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
        let scene = Uuid::from_u128(10);
        let a = ecs.navmesh_for(scene, 0.4).expect("navmesh builds");
        ecs.apply_op(&Operation::Create {
            doc: entity_doc_eng(
                20,
                10,
                "wall",
                json!({ "seg": { "x1": 10.0, "y1": 0.0, "x2": 10.0, "y2": 50.0 },
                        "blocksMove": true, "blocksSight": false, "blocksLight": false }),
            ),
        });
        let b = ecs.navmesh_for(scene, 0.4).expect("navmesh rebuilds");
        assert!(
            !std::sync::Arc::ptr_eq(&a, &b),
            "adding a blocksMove wall must invalidate the cached navmesh"
        );
    }

    #[test]
    fn bounds_mutation_invalidates_the_navmesh_cache() {
        let mut ecs = SceneEcs::from_documents(
            vec![entity_doc_top_eng(
                10,
                "scene",
                json!({ "grid": { "kind": "square", "size": 100 }, "background": null }),
            )],
            0,
        );
        let scene = Uuid::from_u128(10);
        let a = ecs.navmesh_for(scene, 0.4).expect("navmesh builds");
        ecs.apply_op(&Operation::Update {
            doc_id: scene,
            changes: vec![crate::data::command::FieldChange {
                remove: false,
                path: "/engine/bounds".into(),
                old: json!(null),
                new: json!({ "width": 40, "height": 40 }),
            }],
        });
        let b = ecs.navmesh_for(scene, 0.4).expect("navmesh rebuilds");
        assert!(
            !std::sync::Arc::ptr_eq(&a, &b),
            "changing scene bounds must invalidate the cached navmesh"
        );
    }

    /// Builds a SceneEcs with one scene (id 10), one player-owned token at (50, 50), and
    /// world-settings that set `movementRestriction = "revealed"` with no placed lights (env
    /// intensity = 0). The visible mask is therefore empty; only explored memory can admit cells.
    /// Returns `(ecs, user, scene_id)`.
    fn scene_revealed_player_token() -> (SceneEcs, Uuid, Uuid) {
        let user = Uuid::from_u128(7);
        let scene_id = Uuid::from_u128(10);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok], 0);
        // Dark scene + revealed restriction: visible cells = ∅, so only explored memory admits moves.
        ecs.set_world_settings_for_test(json!({
            "scene": {
                "losRestriction": false, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#000000", "intensity": 0.0 },
                "observerVision": false,
                "movementRestriction": "revealed",
                "movementModel": "grid-stepped",
                "partialCellLeniency": false
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        }));
        (ecs, user, scene_id)
    }

    #[test]
    fn pathfind_gm_unconstrained_routes_without_a_mask() {
        // GM (is_gm=true): no mask; an open scene routes start→goal at chebyshev cost.
        let (ecs, _user, scene) = scene_with_lit_player_token();
        let r = ecs.pathfind(
            Uuid::from_u128(1),
            scene,
            (50.0, 50.0),
            &[(250.0, 50.0)],
            0.1,
            true,
            None,
        );
        let outcome = r.expect("GM route");
        assert!((outcome.cost - 2.0).abs() < 1e-9);
        assert_eq!(outcome.path.last(), Some(&(250.0, 50.0)));
    }

    #[test]
    fn pathfind_dispatches_to_the_navmesh_router_for_a_continuous_scene() {
        let mut ecs = SceneEcs::from_documents(
            vec![entity_doc_top_eng(
                10,
                "scene",
                json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                        "vision": { "movementModel": "continuous" } }),
            )],
            0,
        );
        ecs.set_world_settings_for_test(json!({
            "scene": {
                "losRestriction": false, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#ffffff", "intensity": 1.0 },
                "observerVision": false,
                "movementRestriction": "unrestricted",
                "movementModel": "continuous",
                "partialCellLeniency": true
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        }));
        let outcome = ecs
            .pathfind(
                Uuid::from_u128(1),
                Uuid::from_u128(10),
                (50.0, 50.0),
                &[(950.0, 50.0)],
                0.1,
                true, // GM: unrestricted mask
                None,
            )
            .expect("continuous route over an open bounded scene");
        // Euclidean straight line ≈ 900, unlike a grid diagonal-rule cost — proves the navmesh
        // path was actually taken, not the grid router.
        assert!(
            (outcome.cost - 900.0).abs() < 2.0,
            "expected ~900 (Euclidean), got {}",
            outcome.cost
        );
    }

    #[test]
    fn pathfind_continuous_start_equals_goal_is_a_single_point_zero_cost() {
        // Mirrors `astar_tests::start_equals_goal_is_a_single_cell_zero_cost` (the grid-stepped
        // engine's trivial-success case) for the continuous engine: routing to the point you're
        // already standing on must succeed with a single-point, zero-cost route, not
        // `PathFail::Unreachable`.
        let mut ecs = SceneEcs::from_documents(
            vec![entity_doc_top_eng(
                10,
                "scene",
                json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                        "vision": { "movementModel": "continuous" } }),
            )],
            0,
        );
        ecs.set_world_settings_for_test(json!({
            "scene": {
                "losRestriction": false, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#ffffff", "intensity": 1.0 },
                "observerVision": false,
                "movementRestriction": "unrestricted",
                "movementModel": "continuous",
                "partialCellLeniency": true
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        }));
        let outcome = ecs
            .pathfind(
                Uuid::from_u128(1),
                Uuid::from_u128(10),
                (50.0, 50.0),
                &[(50.0, 50.0)],
                0.1,
                true, // GM: unrestricted mask
                None,
            )
            .expect("start == goal must succeed, not Unreachable");
        assert_eq!(outcome.path, vec![(50.0, 50.0)]);
        assert_eq!(outcome.cost, 0.0);
        assert!(!outcome.arrested);
    }

    /// Mirrors `scene_with_lit_player_token` (same token/light geometry) but the scene doc
    /// declares `vision.movementModel: "continuous"`, so the fixture drives the REAL non-GM
    /// `visible_cells` mask through the continuous dispatch branch instead of a hand-built
    /// `BTreeSet` test double.
    fn scene_with_lit_player_token_continuous() -> (SceneEcs, Uuid, Uuid) {
        let user = Uuid::from_u128(7);
        let scene_id = Uuid::from_u128(10);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        let light = entity_doc_eng(
            20,
            10,
            "light",
            json!({
                "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
                "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true
            }),
        );
        let scene = entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                    "vision": { "movementModel": "continuous" } }),
        );
        let ecs = SceneEcs::from_documents(vec![scene, tok, light], 0);
        (ecs, user, scene_id)
    }

    fn continuous_scene_docs() -> Vec<crate::data::document::Document> {
        vec![entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                    "vision": { "movementModel": "continuous" } }),
        )]
    }

    fn continuous_world_settings() -> serde_json::Value {
        json!({
            "scene": {
                "losRestriction": false, "fog": true,
                "lightingEnabled": true, "lightMode": "environmentLight",
                "environment": { "color": "#ffffff", "intensity": 1.0 },
                "observerVision": false,
                "movementRestriction": "unrestricted",
                "movementModel": "continuous",
                "partialCellLeniency": true
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn region_doc_top(
        id: u128,
        parent: u128,
        behavior: &str,
        cost: f64,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
    ) -> Document {
        entity_doc_eng(
            id,
            parent,
            "region",
            json!({ "shape": { "kind": "rect", "points": [x0, y0, x1, y1] },
                    "behavior": behavior, "cost": cost, "enabled": true }),
        )
    }

    #[test]
    fn pathfind_continuous_terrain_bends_the_route_and_costs_scene_units() {
        // Continuous scene, terrain mult 5 on cell (1,0) = Rect [100,0]-[200,100] between start and
        // goal. The weighted grid route (forced Euclidean) detours through row 1 (2 diagonal steps,
        // ~2*sqrt(2) cells => *cell = ~283 scene units) instead of straight through the mult-5 cell
        // (would be 1+5 = 6 cells => 600 scene units). Proves terrain BENDS the continuous route and
        // that cost is in scene units.
        let mut docs = continuous_scene_docs();
        docs.push(region_doc_top(
            12, 10, "terrain", 5.0, 100.0, 0.0, 200.0, 100.0,
        ));
        let mut ecs = SceneEcs::from_documents(docs, 0);
        ecs.set_world_settings_for_test(continuous_world_settings());
        let out = ecs
            .pathfind(
                Uuid::from_u128(1),
                Uuid::from_u128(10),
                (50.0, 50.0),
                &[(250.0, 50.0)],
                0.1,
                true,
                None,
            )
            .expect("weighted continuous route");
        assert!(
            out.cost < 400.0,
            "detour taken (scene units ~283), got {}",
            out.cost
        );
        assert!(
            out.cost > 150.0,
            "cost is scene units, not cells, got {}",
            out.cost
        );
        assert!(
            out.path.iter().any(|p| p.1 > 90.0),
            "route bends off the y=50 line to avoid the terrain: {:?}",
            out.path
        );
    }

    #[test]
    fn pathfind_continuous_no_region_is_a_straight_polyanya_route() {
        // Same scene WITHOUT a region: the pure polyanya path is taken — a straight 200px route.
        let mut ecs = SceneEcs::from_documents(continuous_scene_docs(), 0);
        ecs.set_world_settings_for_test(continuous_world_settings());
        let out = ecs
            .pathfind(
                Uuid::from_u128(1),
                Uuid::from_u128(10),
                (50.0, 50.0),
                &[(250.0, 50.0)],
                0.1,
                true,
                None,
            )
            .expect("polyanya route");
        assert!(
            (out.cost - 200.0).abs() < 3.0,
            "straight Euclidean ~200, got {}",
            out.cost
        );
    }

    #[test]
    fn pathfind_continuous_impassable_routes_around() {
        // Impassable wall-of-cells on column 1 (Rect [100,0]-[200,300]) blocks the straight line;
        // the weighted route must detour and still reach the goal.
        let mut docs = continuous_scene_docs();
        docs.push(region_doc_top(
            12,
            10,
            "impassable",
            1.0,
            100.0,
            0.0,
            200.0,
            300.0,
        ));
        let mut ecs = SceneEcs::from_documents(docs, 0);
        ecs.set_world_settings_for_test(continuous_world_settings());
        let out = ecs
            .pathfind(
                Uuid::from_u128(1),
                Uuid::from_u128(10),
                (50.0, 50.0),
                &[(250.0, 350.0)],
                0.1,
                true,
                None,
            )
            .expect("route around impassable");
        // No route point falls inside an impassable cell (column 1, y in [0,300)).
        assert!(
            !out.path
                .iter()
                .any(|p| p.0 >= 100.0 && p.0 < 200.0 && p.1 >= 0.0 && p.1 < 300.0),
            "route threads no impassable cell: {:?}",
            out.path
        );
    }

    #[test]
    fn pathfind_continuous_secret_terrain_absent_from_player_route_present_for_gm() {
        // gm_only terrain (mult 5) on cell (1,0). A player (non-GM) never sees it: their route is
        // the straight polyanya line (no bend, ~200 scene units). The GM's route bends (weighted).
        let mut docs = continuous_scene_docs();
        let mut secret = region_doc_top(12, 10, "terrain", 5.0, 100.0, 0.0, 200.0, 100.0);
        // Mark the region gm_only via the SAME `/engine` property-visibility override
        // `region_field`'s per-requester filter checks (`move_exec.rs` uses the identical
        // convention for its own gm_only region fixtures).
        secret
            .permissions
            .property_overrides
            .insert("/engine".into(), crate::data::document::Visibility::GmOnly);
        docs.push(secret);
        let mut ecs = SceneEcs::from_documents(docs, 0);
        ecs.set_world_settings_for_test(continuous_world_settings());
        let player = Uuid::from_u128(2);
        // Player (non-GM, unrestricted movement => no mask): secret terrain absent => straight route.
        let p = ecs
            .pathfind(
                player,
                Uuid::from_u128(10),
                (50.0, 50.0),
                &[(250.0, 50.0)],
                0.1,
                false,
                None,
            )
            .expect("player route");
        assert!(
            (p.cost - 200.0).abs() < 5.0,
            "secret terrain does not bend the player route, got {}",
            p.cost
        );
        // GM sees the authoritative field => bends.
        let g = ecs
            .pathfind(
                Uuid::from_u128(1),
                Uuid::from_u128(10),
                (50.0, 50.0),
                &[(250.0, 50.0)],
                0.1,
                true,
                None,
            )
            .expect("gm route");
        assert!(
            g.cost < 400.0 && g.cost > 150.0,
            "GM route is weighted, got {}",
            g.cost
        );
    }

    #[test]
    fn pathfind_continuous_nongm_route_clips_to_the_visible_mask() {
        // System-level §13 coverage: the two existing continuous `pathfind` tests
        // (`pathfind_dispatches_to_the_navmesh_router_for_a_continuous_scene`,
        // `pathfind_continuous_start_equals_goal_is_a_single_point_zero_cost`) both pass
        // `is_gm: true`, so `mask` is always `None` and `clip_to_visible_mask` runs as a pure
        // pass-through — nothing is ever actually clipped. This test drives a non-GM request
        // through the FULL chain (`pathfind` → dispatch → `navmesh_for` → `navmesh_find` →
        // `clip_to_visible_mask`) with the REAL per-(user,scene) `visible_cells` mask, proving a
        // future fork/null of the mask on the `Continuous` branch would fail this test.
        let (ecs, user, scene) = scene_with_lit_player_token_continuous();
        let lenient = ecs.resolve_scene(scene).partial_cell_leniency;
        let mask = ecs.visible_cells(user, scene, lenient);
        assert!(!mask.is_empty(), "the lit token has a non-empty mask");

        // Far goal well outside the light radius (dimRadius 6 cells = 600 scene units) but still
        // inside the scene's default 100x100-cell bounds, so navmesh construction over the
        // bounds rect itself never fails — only the visibility clip should stop the route short.
        let far_goal = (9500.0, 9500.0);
        let outcome = ecs
            .pathfind(user, scene, (50.0, 50.0), &[far_goal], 0.1, false, None)
            .expect(
                "clip truncates the route short of the unseen goal rather than failing outright",
            );
        let dist_to_goal =
            ((far_goal.0 - 50.0_f64).powi(2) + (far_goal.1 - 50.0_f64).powi(2)).sqrt();
        assert!(
            outcome.cost < dist_to_goal / 2.0,
            "route must truncate well short of the unseen far goal: cost {} vs distance {}",
            outcome.cost,
            dist_to_goal
        );
        let (lx, ly) = *outcome.path.last().expect("non-empty truncated path");
        let dist_from_start = ((lx - 50.0_f64).powi(2) + (ly - 50.0_f64).powi(2)).sqrt();
        assert!(
            dist_from_start < 700.0,
            "truncated endpoint must stay near the lit token, got ({lx}, {ly})"
        );
    }

    #[test]
    fn pathfind_continuous_weighted_nongm_route_clips_to_the_visible_mask() {
        // Whole-branch buddy-check Finding 3: the test above only drives the PURE-POLYANYA
        // sub-path (no terrain/impassable region present, so `has_terrain_or_impassable()` is
        // false). This test adds a terrain region so `pathfind`'s `Continuous` dispatch takes
        // the WEIGHTED sub-path (`pathfinding::find` forced Euclidean + `navmesh::los_smooth`)
        // for a non-GM requester under a real RESTRICTING `visible_cells` mask (default
        // fail-closed `MovementRestriction::Visible`, same fixture as the pure-polyanya test —
        // its default settings already yield a small, genuinely restricting mask).
        let user = Uuid::from_u128(7);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50, "y": 50, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        let light = entity_doc_eng(
            20,
            10,
            "light",
            json!({
                "x": 50.0, "y": 50.0, "color": "#ffffff", "intensity": 1.0,
                "brightRadius": 3.0, "dimRadius": 6.0, "enabled": true
            }),
        );
        let scene = entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                    "vision": { "movementModel": "continuous" } }),
        );
        // Terrain mult 5 FAR outside the token's (small, default-settings) visible mask — its
        // mere existence anywhere on the scene is what flips `has_terrain_or_impassable()` true
        // and routes `pathfind`'s `Continuous` dispatch to the WEIGHTED sub-path; it is
        // deliberately placed off the requester's route so this test isolates "does the weighted
        // sub-path correctly enforce the mask" from "does terrain bend the route" (already
        // covered by `pathfind_continuous_terrain_bends_the_route_and_costs_scene_units`).
        let terrain = region_doc_top(12, 10, "terrain", 5.0, 5000.0, 5000.0, 5100.0, 5100.0);
        let ecs = SceneEcs::from_documents(vec![scene, tok, light, terrain], 0);
        let scene_id = Uuid::from_u128(10);
        let cell = 100.0;

        let lenient = ecs.resolve_scene(scene_id).partial_cell_leniency;
        let mask = ecs.visible_cells(user, scene_id, lenient);
        assert!(!mask.is_empty(), "the lit token has a non-empty mask");
        assert!(
            ecs.region_field(scene_id, Some(user))
                .has_terrain_or_impassable(),
            "the terrain region flips the Continuous dispatch to the weighted sub-path"
        );

        // Near goal, still within the small visible mask: the weighted route must succeed and
        // stay entirely inside the mask (the grid A* mask check IS the enforcement mechanism for
        // this sub-path — Finding 1 — so a route can never even be found outside the mask).
        let near_goal = (150.0, 50.0);
        let near = ecs
            .pathfind(user, scene_id, (50.0, 50.0), &[near_goal], 0.1, false, None)
            .expect("weighted route to a visible goal succeeds");
        for &(px, py) in &near.path {
            let c = ((px / cell).floor() as i32, (py / cell).floor() as i32);
            assert!(
                mask.contains(&c),
                "weighted route point ({px},{py}) -> cell {c:?} lies outside the visible mask"
            );
        }

        // Far goal, well outside the visible mask: the weighted grid search cannot even discover
        // a route through the unseen cells surrounding it (the mask check is baked into the A*
        // search itself, not a post-hoc clip), so it fails closed (`Unreachable`) rather than
        // returning a route that threads unseen cells.
        let far_goal = (9500.0, 9500.0);
        let far = ecs.pathfind(user, scene_id, (50.0, 50.0), &[far_goal], 0.1, false, None);
        assert!(
            far.is_err(),
            "weighted route to an unseen goal fails closed rather than routing through fog: {far:?}"
        );
    }

    #[test]
    fn pathfind_continuous_secret_arrest_absent_from_player_preview_but_springs_at_execution() {
        // gm_only arrest region on cell (2,0) = Rect [200,0]-[300,100]. No terrain/impassable
        // region exists, so `has_terrain_or_impassable()` is false and `pathfind` takes the PURE
        // POLYANYA branch (`navmesh_find` -> `clip_to_visible_mask` -> `truncate_at_arrest`),
        // distinct from the weighted-grid branch. A player's per-requester region field omits the
        // secret region entirely, so their route preview is the full straight line with no
        // truncation; the GM's authoritative field truncates at the arrest cell. `move_exec`
        // always reads the authoritative field regardless of requester, so committing the
        // player's own (untruncated) preview still arrests at the same cell.
        let mut docs = continuous_scene_docs();
        let mut secret = region_doc_top(12, 10, "arrest", 1.0, 200.0, 0.0, 300.0, 100.0);
        secret
            .permissions
            .property_overrides
            .insert("/engine".into(), crate::data::document::Visibility::GmOnly);
        docs.push(secret);
        let player = Uuid::from_u128(2);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(player);
        docs.push(tok);
        let mut ecs = SceneEcs::from_documents(docs, 0);
        ecs.set_world_settings_for_test(continuous_world_settings());
        let scene = Uuid::from_u128(10);
        let token = Uuid::from_u128(11);

        // Player (non-GM): secret arrest is invisible to their per-requester field, so the
        // preview is the full, untruncated straight polyanya route.
        let p = ecs
            .pathfind(
                player,
                scene,
                (50.0, 50.0),
                &[(450.0, 50.0)],
                0.1,
                false,
                None,
            )
            .expect("player route");
        assert!(
            !p.arrested,
            "secret arrest region does not truncate the player's own route preview"
        );
        assert!(
            (p.cost - 400.0).abs() < 5.0,
            "player route reaches the full goal (~400 Euclidean), got {}",
            p.cost
        );

        // GM: authoritative field truncates the route at the arrest cell entry.
        let g = ecs
            .pathfind(
                Uuid::from_u128(1),
                scene,
                (50.0, 50.0),
                &[(450.0, 50.0)],
                0.1,
                true,
                None,
            )
            .expect("gm route");
        assert!(
            g.arrested,
            "GM sees the secret region and it truncates their route"
        );

        // `move_exec` always reads the AUTHORITATIVE field: committing the player's own
        // (untruncated) previewed route still springs the arrest at the same cell.
        let visible: std::collections::BTreeSet<(i32, i32)> = std::collections::BTreeSet::new();
        let exec_out = crate::scene::move_exec::execute_move(
            &ecs,
            scene,
            token,
            &p.path,
            MovementRestriction::Unrestricted,
            &visible,
            100.0,
        )
        .expect("move_exec handles the player's committed route");
        assert!(
            exec_out.truncated,
            "the authoritative field springs the secret arrest at execution"
        );
        assert!(
            exec_out.stop.0 < 400.0,
            "execution stops before the full player-preview route length, got {:?}",
            exec_out.stop
        );
    }

    #[test]
    fn pathfind_grid_stepped_scene_is_byte_for_byte_unchanged() {
        // Same fixture/assertions as the existing `pathfind_gm_unconstrained_routes_without_a_mask`
        // test, proving the default (grid-stepped) dispatch branch is untouched by this checkpoint.
        let (ecs, _user, scene) = scene_with_lit_player_token();
        let r = ecs.pathfind(
            Uuid::from_u128(1),
            scene,
            (50.0, 50.0),
            &[(250.0, 50.0)],
            0.1,
            true,
            None,
        );
        let outcome = r.expect("GM route");
        assert!(
            (outcome.cost - 2.0).abs() < 1e-9,
            "grid Chebyshev cost unchanged"
        );
    }

    #[test]
    fn pathfind_nongm_visible_is_bounded_by_the_mask() {
        // Non-GM under movementRestriction "visible": a goal outside the lit mask is Unreachable.
        let (ecs, user, scene) = scene_with_lit_player_token();
        let lenient = ecs.resolve_scene(scene).partial_cell_leniency;
        let mask = ecs.visible_cells(user, scene, lenient);
        assert!(!mask.is_empty(), "the lit token has a non-empty mask");
        // A far goal well outside the lit radius → Unreachable.
        let far = ecs.pathfind(
            user,
            scene,
            (50.0, 50.0),
            &[(5000.0, 5000.0)],
            0.1,
            false,
            None,
        );
        assert_eq!(far, Err(crate::scene::pathfinding::PathFail::Unreachable));
    }

    #[test]
    fn pathfind_revealed_unions_explored_memory() {
        // movementRestriction "revealed": an explored corridor covering start..goal makes an otherwise-unlit
        // goal routable.
        let (ecs, user, scene) = scene_revealed_player_token();
        let cell = 100.0;
        let mut explored = crate::scene::explored::ExploredSet::new();
        // Mark cells (0,0)..(3,0) as explored (a straight corridor).
        explored.mark_polygons(
            &[vec![0.0, 0.0, 4.0 * cell, 0.0, 4.0 * cell, cell, 0.0, cell]],
            cell,
        );
        let r = ecs.pathfind(
            user,
            scene,
            (50.0, 50.0),
            &[(350.0, 50.0)],
            0.1,
            false,
            Some(&explored),
        );
        assert!(
            r.is_ok(),
            "explored corridor makes the goal routable under revealed"
        );
    }

    // --- player_vision_polygons_at: mover vision trajectory ---

    /// Advancing past a `blocksSight` wall changes the visibility polygon: a point beyond
    /// the wall is invisible from the near viewpoint but visible from the far viewpoint.
    #[test]
    fn vision_at_grows_as_token_advances() {
        // Vertical blocksSight wall at x=100 (y ±200). Token committed at (50,50).
        // The wall spans the relevant y range of the bounding box so the test point
        // (150,50) is directly occluded from the near side.
        let scene = Uuid::from_u128(10);
        let user = Uuid::from_u128(7);
        let token_id = Uuid::from_u128(11);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        let wall = entity_doc_eng(
            12,
            10,
            "wall",
            json!({ "seg": {"x1": 100, "y1": -200, "x2": 100, "y2": 200},
                    "blocksSight": true }),
        );
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok, wall], 0);

        // Near viewpoint (50,50): wall at x=100 occludes (150,50) — ray crosses the wall.
        let polys_near = ecs.player_vision_polygons_at(user, scene, token_id, (50.0, 50.0));
        assert!(
            !polys_near.is_empty(),
            "must return a polygon for an owned token"
        );
        let in_near = vision::point_in_poly(&polys_near[0], (150.0, 50.0));

        // Far viewpoint (200,50): token is past the wall; (150,50) is between wall and viewpoint
        // on the same side, so it IS visible.
        let polys_far = ecs.player_vision_polygons_at(user, scene, token_id, (200.0, 50.0));
        assert!(
            !polys_far.is_empty(),
            "must return a polygon for an owned token"
        );
        let in_far = vision::point_in_poly(&polys_far[0], (150.0, 50.0));

        assert!(
            !in_near,
            "near viewpoint (50,50) must NOT see (150,50) past the wall at x=100"
        );
        assert!(
            in_far,
            "far viewpoint (200,50) must see (150,50) between wall and viewpoint"
        );
    }

    /// A `blocksSight` wall with `gm_only` permissions (DocRole::None default — players cannot
    /// read this wall doc) must produce the SAME occlusion as an identically-placed normal wall.
    /// Invariant: `sight_walls` uses the FULL ECS wall set regardless of doc permissions;
    /// the server never leaks the wall's existence, only uses it for raycast geometry.
    #[test]
    fn vision_at_uses_full_wall_set() {
        use crate::data::document::DocRole;
        let scene = Uuid::from_u128(10);
        let user = Uuid::from_u128(7);
        let token_id = Uuid::from_u128(11);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        let wall_sys =
            json!({ "seg": {"x1": 100, "y1": -200, "x2": 100, "y2": 200}, "blocksSight": true });

        // Normal wall (default permissions): occludes from (50,50).
        let normal_wall = entity_doc_eng(12, 10, "wall", wall_sys.clone());
        let ecs_normal =
            SceneEcs::from_documents(vec![doc(10, None, "scene"), tok.clone(), normal_wall], 0);
        let polys_normal =
            ecs_normal.player_vision_polygons_at(user, scene, token_id, (50.0, 50.0));
        assert!(!polys_normal.is_empty());

        // gm_only wall (DocRole::None): players cannot access this doc, but must occlude equally.
        let mut gm_wall = entity_doc_eng(12, 10, "wall", wall_sys);
        gm_wall.permissions.default = DocRole::None;
        let ecs_gm = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok, gm_wall], 0);
        let polys_gm = ecs_gm.player_vision_polygons_at(user, scene, token_id, (50.0, 50.0));
        assert!(!polys_gm.is_empty());

        // Both walls must produce identical polygons — sight_walls is permission-blind.
        assert_eq!(
            polys_normal[0], polys_gm[0],
            "gm_only wall must occlude identically to a normal wall with the same geometry"
        );

        // Cross-check: the occluded point (150,50) is NOT visible from (50,50) with either wall.
        assert!(
            !vision::point_in_poly(&polys_gm[0], (150.0, 50.0)),
            "gm_only wall must occlude (150,50): point must not be inside the polygon"
        );
    }

    // --- wall-less scene full intrascene vision (C2) ---

    /// A wall-less 40x40-unit scene must reveal its own full bounded extent, not a small
    /// `VISION_BOUND_MARGIN` box around the viewpoint.
    #[test]
    fn wall_less_scene_gives_full_intrascene_vision_not_a_degenerate_box() {
        let user = Uuid::from_u128(7);
        let scene_id = Uuid::from_u128(10);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 5.0, "y": 5.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        let scene = entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                    "bounds": { "width": 500.0, "height": 500.0 } }),
        );
        let ecs = SceneEcs::from_documents(vec![scene, tok], 0);

        let polys = ecs.player_vision_polygons(user);
        let (_, poly) = polys
            .iter()
            .find(|(sid, _)| *sid == scene_id)
            .expect("scene present");

        let far_corner = (490.0, 490.0);
        assert!(
            vision::point_in_poly(poly, far_corner),
            "a wall-less scene must reveal its own full bounded extent, not a small box around the viewpoint"
        );
    }

    /// The wall-less-scene vision fix must stay bounded to the scene's own extent — never
    /// unbounded, never leaking beyond `bounds`.
    #[test]
    fn wall_less_scene_vision_does_not_leak_beyond_its_own_bounds() {
        let user = Uuid::from_u128(7);
        let scene_id = Uuid::from_u128(10);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 5.0, "y": 5.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        let scene = entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                    "bounds": { "width": 500.0, "height": 500.0 } }),
        );
        let ecs = SceneEcs::from_documents(vec![scene, tok], 0);

        let polys = ecs.player_vision_polygons(user);
        let (_, poly) = polys.iter().find(|(sid, _)| *sid == scene_id).unwrap();

        let beyond_bounds = (1000.0, 1000.0);
        assert!(
            !vision::point_in_poly(poly, beyond_bounds),
            "vision must stay bounded to the scene's own extent, never unbounded"
        );
    }

    /// `player_vision_polygons` and `player_vision_inputs` (via its `polygons_at` per-sample
    /// path) must not fork: same wall set (empty), same scene-bounds-aware bound.
    #[test]
    fn player_vision_polygons_and_player_vision_inputs_agree_on_wall_less_bound() {
        let user = Uuid::from_u128(7);
        let scene_id = Uuid::from_u128(10);
        let token_id = Uuid::from_u128(11);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 5.0, "y": 5.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        let scene = entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                    "bounds": { "width": 500.0, "height": 500.0 } }),
        );
        let ecs = SceneEcs::from_documents(vec![scene, tok], 0);

        let poly_from_polygons = ecs
            .player_vision_polygons(user)
            .into_iter()
            .find(|(sid, _)| *sid == scene_id)
            .map(|(_, p)| p);
        let poly_from_inputs = ecs
            .player_vision_polygons_at(user, scene_id, token_id, (5.0, 5.0))
            .into_iter()
            .next();

        assert_eq!(
            poly_from_polygons, poly_from_inputs,
            "player_vision_polygons and player_vision_inputs must compute the identical bound for the same wall-less scene"
        );
    }

    /// Returns empty when the user owns no token in the scene, even when `moving_token`
    /// points to an existing token owned by another user.
    #[test]
    fn vision_at_empty_when_user_owns_no_token() {
        let scene = Uuid::from_u128(10);
        let user = Uuid::from_u128(7);
        let stranger = Uuid::from_u128(999);
        let token_id = Uuid::from_u128(11);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user); // owned by user, NOT stranger
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok], 0);

        // Stranger owns no token in this scene → empty (fail-closed).
        let polys = ecs.player_vision_polygons_at(stranger, scene, token_id, (50.0, 50.0));
        assert!(
            polys.is_empty(),
            "user with no owned token must get empty polygons (fail-closed)"
        );
    }

    // --- source_los_poly wall-less degenerate box (C2 follow-up: player_lit_mask/visible_cells) ---

    /// A wall-less 500x500-unit scene, all-bright lighting (isolates the bound-box defect from
    /// illumination), `losRestriction` off (so `source_los_poly` takes the plain-rectangle branch
    /// — the same branch the original C2 bug hit). Cell (4,4) — center (450,450) — lies within the
    /// scene's authored bounds but strictly outside a degenerate
    /// `viewpoint±VISION_BOUND_MARGIN(100)` box around the token at (50,50): `[-50,-50]..[150,150]`.
    fn wall_less_large_scene_all_bright() -> (SceneEcs, Uuid, Uuid) {
        let user = Uuid::from_u128(7);
        let scene_id = Uuid::from_u128(10);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        let scene = entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                    "bounds": { "width": 500.0, "height": 500.0 } }),
        );
        let mut ecs = SceneEcs::from_documents(vec![scene, tok], 0);
        ecs.set_world_settings_for_test(json!({
            "scene": {
                "losRestriction": false, "fog": true,
                "lightingEnabled": false, "lightMode": "environmentLight",
                "environment": { "color": "#ffffff", "intensity": 1.0 },
                "observerVision": false,
                "movementRestriction": "visible",
                "movementModel": "grid-stepped",
                "partialCellLeniency": false
            },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        }));
        (ecs, user, scene_id)
    }

    /// `player_lit_mask` (the egress/secrecy gate) must cover a wall-less scene's full authored
    /// bounds, not a degenerate box around the viewpoint — the same C2 defect class fixed in
    /// `player_vision_polygons`/`player_vision_inputs`, applied here to `source_los_poly`, the
    /// primitive `player_lit_mask` shares with `visible_cells`.
    #[test]
    fn player_lit_mask_wall_less_scene_covers_full_bounds_not_a_degenerate_box() {
        let (ecs, user, scene_id) = wall_less_large_scene_all_bright();
        let cells: std::collections::BTreeSet<(i32, i32)> = ecs
            .player_lit_mask(user)
            .into_iter()
            .filter(|s| s.scene == scene_id)
            .flat_map(|s| s.cells.into_iter().map(|(i, j, _b, _t, _h)| (i, j)))
            .collect();
        assert!(
            cells.contains(&(4, 4)),
            "a wall-less scene's lit mask must cover its full authored bounds, not a degenerate box around the viewpoint"
        );
    }

    /// `visible_cells` (the movement gate) must cover a wall-less scene's full authored bounds,
    /// not a degenerate box — same defect class as above, mirrored to the movement-gate consumer.
    #[test]
    fn visible_cells_wall_less_scene_covers_full_bounds_not_a_degenerate_box() {
        let (ecs, user, scene_id) = wall_less_large_scene_all_bright();
        let mask = ecs.visible_cells(user, scene_id, false);
        assert!(
            mask.contains(&(4, 4)),
            "a wall-less scene's movement-gate mask must cover its full authored bounds, not a degenerate box around the viewpoint"
        );
    }

    /// No-fork parity: `source_los_poly`'s bound (as exercised via `visible_cells`) must agree
    /// with `player_vision_polygons`'s bound (via `vision::bound_for_scene` directly) on the same
    /// wall-less scene — closing the "two/three vision paths diverge" defect class the original
    /// C2 fix's brief warned about, generalized to this third path.
    #[test]
    fn visible_cells_agrees_with_player_vision_polygons_bound_on_wall_less_scene() {
        let (ecs, user, scene_id) = wall_less_large_scene_all_bright();

        let polys = ecs.player_vision_polygons(user);
        let (_, poly) = polys
            .iter()
            .find(|(sid, _)| *sid == scene_id)
            .expect("scene present");
        let far_corner = (490.0, 490.0);
        assert!(
            vision::point_in_poly(poly, far_corner),
            "player_vision_polygons must reveal the scene's own full bounded extent"
        );

        let mask = ecs.visible_cells(user, scene_id, false);
        assert!(
            mask.contains(&(4, 4)),
            "visible_cells (via source_los_poly) must not diverge from player_vision_polygons' bound for the same wall-less scene"
        );
    }
}
