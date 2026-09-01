//! Per-world derived scene ECS. Hydrated from documents; never persisted,
//! never authoritative. Holds one hecs entity per scene-entity document so
//! engine-owned systems (vision, pathfinding) can query spatial state.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub(crate) mod elevation;
pub(crate) mod emitters;
pub mod explored;
pub mod footprint;
pub(crate) mod grid_shape;
pub mod lighting;
pub(crate) mod move_exec;
pub(crate) mod move_stream;
pub mod movement;
pub(crate) mod navmesh;
pub(crate) mod pathfinding;
pub(crate) mod regions;
pub mod vision;

#[cfg(test)]
mod grid_shape_parity_tests;

use std::collections::{BTreeMap, HashMap};

use uuid::Uuid;

use crate::data::command::{apply_field_change, FieldChange, Operation};
use crate::data::document::Document;
// The typed, ingress-validated engine band, imported under a namespace alias: this module
// declares its own `LightMode`/`MovementRestriction`/`MovementModel` (the RESOLVED
// representation `ResolvedScene` exposes to callers elsewhere in `scene/`); the engine crate's
// identically-named enums are the wire representation read off a document's `engine` field.
// Keeping the two distinct avoids widening the `scene` module's already-declared public enum
// surface.
use crate::data::engine as eng;
use crate::data::membership::PermissionContext;
use crate::scene::lighting::Band;

/// Resolved per-scene lighting mode. The client's wire twin is generated from
/// `eng::LightMode`, the identically-named wire enum this module imports under the `eng`
/// alias and keeps distinct from this resolved representation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightMode {
    /// Every LOS cell is fully bright; per-light raycasts are skipped
    /// (`LightingInputs::all_bright`).
    GlobalIllumination,
    /// Ambient environment level + per-light contributions, sampled per cell
    /// by `lighting::cell_illumination`.
    EnvironmentLight,
}

/// Per-scene movement gate mode. The client's wire twin is generated from `eng::MovementRestriction`.
/// Selects the VISION-MASK arm of the gate only (`move_exec::execute_move`'s
/// `check_mask`); the wall and region gates apply to every non-GM move
/// regardless of mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovementRestriction {
    /// Move cells must be currently visible to the mover's owner.
    Visible,
    /// Move cells must be visible OR in the owner's explored memory.
    Revealed,
    /// Vision mask skipped; walls and region gates still apply to non-GM
    /// movers (`check_walls`/`check_regions` are independent of the mode).
    Unrestricted,
}

/// Per-scene movement/pathfinding engine choice. The client's wire twin is generated
/// from `eng::MovementModel`. `GridStepped` = the existing grid A* router; `Continuous` = the
/// polyanya navmesh router.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovementModel {
    /// Grid A* router (`pathfinding::find`).
    GridStepped,
    /// Polyanya navmesh router (`navmesh::navmesh_find`).
    Continuous,
}

/// A scene's cell geometry family, resolved from its `engine.grid.kind`. The single decision
/// behind which `GridShape` implementation a scene uses, which coordinate system its persisted
/// fog is indexed in, and which cached masks a change to it must invalidate. Anything other than
/// the hex spelling resolves to `Square` — the hardened default an absent or malformed scene
/// document falls back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridKind {
    /// Axis-aligned square cells.
    Square,
    /// Pointy-top axial hexes.
    Hex,
}

/// The grid kind a decoded scene engine declares. Pure, so the two readers that each already hold
/// a decoded `SceneEngine` — `resolve_scene` and `SceneEcs::resolve_grid_kind` — read ONE
/// implementation of the comparison rather than repeating it, and neither pays a second decode to
/// reach it.
fn grid_kind_from(eng: Option<&eng::SceneEngine>) -> GridKind {
    if eng.map(|s| s.grid.kind.as_str()) == Some("hex") {
        GridKind::Hex
    } else {
        GridKind::Square
    }
}

/// Fail-safe finite default scene size (grid units) when a scene has no authored `bounds`.
/// MUST match the client's `DEFAULT_SCENE_BOUNDS` (client/server parity).
pub const DEFAULT_SCENE_BOUNDS_UNITS: (f64, f64) = (100.0, 100.0);

/// The resolved per-scene lighting/vision/movement settings (subset of the client
/// `ResolvedSceneSettings`; the pathfinding diagonal-cost rule and animation speed are
/// world-scoped, not per-scene, so they are resolved separately by
/// `SceneEcs::resolved_diagonal_rule`/`SceneEcs::resolved_animation_speed` rather than carried
/// as fields here).
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedScene {
    /// Walls with `blocksSight` restrict line of sight (LOS raycasting on).
    pub los_restriction: bool,
    /// Fog-of-war on: unseen state is withheld/clipped for players.
    pub fog: bool,
    /// Observer-tier tokens also contribute vision sources
    /// (`gather_vision_sources_in_scene`).
    pub observer_vision: bool,
    /// Master lighting toggle; off forces the all-bright arm with tint 0.
    pub lighting_enabled: bool,
    /// Resolved lighting mode (see `LightMode`).
    pub light_mode: LightMode,
    /// Environment ambient color, packed `0xRRGGBB` (`parse_hex_color`).
    pub env_color: u32,
    /// Environment ambient intensity level fed to `cell_illumination`.
    pub env_intensity: f64,
    /// Resolved movement gate mode (see `MovementRestriction`).
    pub movement_restriction: MovementRestriction,
    /// Per-scene/world-default pathfinding engine choice. `GridStepped` dispatches to
    /// `pathfinding::find`; `Continuous` dispatches to `navmesh::navmesh_find`.
    pub movement_model: MovementModel,
    /// Lenient cell sampling: a cell qualifies if its center or a sampled
    /// corner qualifies; strict samples the center only (`point_qualifies`
    /// is the shared per-point decision for all arms).
    pub partial_cell_leniency: bool,
    /// Scene dimensions (width, height) measured in grid units (cells), continuous — never world
    /// units, and not required to be integral. Always finite `> 0` (default
    /// `DEFAULT_SCENE_BOUNDS_UNITS`). The navmesh's outer rectangle, after
    /// `GridShape::world_extent` converts it.
    pub bounds: (f64, f64),
    /// The scene's cell geometry family. Decides the `GridShape` implementation, the coordinate
    /// system of persisted explored fog, and — because it is part of `ResolvedScene` — the
    /// visibility cache's own invalidation key.
    pub grid_kind: GridKind,
}

/// A resolved vision mode (subset of the client `VisionMode`). `default_range` is in cells.
/// `render_hint` mirrors the client's `SEED_VISION_MODES` (e.g. `"desaturate"` for
/// darkvision); absent in seed → `None`, absent in an authored doc entry → `None`.
/// `perceives`/`requires_los` carry the sense descriptor through: a `Creatures` mode
/// contributes nothing to the illumination-floor mask and instead feeds
/// `SceneEcs::player_perceived_tokens`.
#[derive(Clone, Debug)]
pub struct VisionMode {
    /// Minimum illumination band name the mode can see under.
    pub illumination_floor: String,
    /// Default vision range in cells (used when a token authors none).
    pub default_range: f64,
    /// Client render treatment (e.g. `"desaturate"`); `None` = plain.
    pub render_hint: Option<String>,
    /// What the mode perceives (terrain mask vs creature perception).
    pub perceives: eng::Perception,
    /// Whether sight walls bound the mode's reach (creature senses only; the
    /// terrain mask is always LOS-gated).
    pub requires_los: bool,
}

/// Wire (`eng::VisionMode`) → resolved bridge — the single conversion both the
/// authored-doc branch and the seed fallback of `resolved_vision_modes` pass through.
fn conv_vision_mode(m: eng::VisionMode) -> VisionMode {
    VisionMode {
        illumination_floor: m.illumination_floor,
        default_range: m.default_range,
        render_hint: m.render_hint,
        perceives: m.perceives,
        requires_los: m.requires_los,
    }
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
/// validation in a test fixture) or the stored value fails to parse. Returns `None`, not a struct
/// default, so every caller keeps applying its own existing field-level fail-closed backstop
/// unchanged.
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
    /// The exact `engine` `Value` `decoded` was parsed from (validity check).
    source: serde_json::Value,
    /// The type-erased successful decode.
    decoded: Box<dyn std::any::Any + Send>,
}

/// Wire (`eng::LightMode`) → resolved bridge: the `eng` alias is the wire representation,
/// this enum the resolved one.
fn conv_light_mode(v: eng::LightMode) -> LightMode {
    match v {
        eng::LightMode::GlobalIllumination => LightMode::GlobalIllumination,
        eng::LightMode::EnvironmentLight => LightMode::EnvironmentLight,
    }
}

/// Wire (`eng::MovementRestriction`) → resolved bridge.
fn conv_movement_restriction(v: eng::MovementRestriction) -> MovementRestriction {
    match v {
        eng::MovementRestriction::Visible => MovementRestriction::Visible,
        eng::MovementRestriction::Revealed => MovementRestriction::Revealed,
        eng::MovementRestriction::Unrestricted => MovementRestriction::Unrestricted,
    }
}

/// Wire (`eng::MovementModel`) → resolved bridge.
fn conv_movement_model(v: eng::MovementModel) -> MovementModel {
    match v {
        eng::MovementModel::GridStepped => MovementModel::GridStepped,
        eng::MovementModel::Continuous => MovementModel::Continuous,
    }
}

/// Wire (`eng::DiagonalRule`) → router-enum bridge.
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
    /// The authoritative document this entity mirrors (derived, ephemeral).
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
    /// Scene document id.
    pub scene: Uuid,
    /// Grid cell size in scene units.
    pub cell: f64,
    /// Visible cells as `(i, j, band_index, tint, render_hint)` tuples.
    pub cells: Vec<(i32, i32, usize, u32, Option<String>)>,
}

/// Margin (scene units, ~one default grid cell) the vision bound box extends past the walls
/// so rays always terminate on the box rather than escaping to infinity.
const VISION_BOUND_MARGIN: f64 = 100.0;

/// Pre-collected per-move-constant inputs for the mover's vision trajectory.
/// Holds the `blocksSight` wall set filtered at the mover's elevation and the visibility
/// polygons for every stationary
/// owned token (all owned tokens in the scene except the moving one). Computed once per move
/// via `SceneEcs::player_vision_inputs`; each sample then calls the cheaper `polygons_at`
/// (one moving-token raycast only, no repeated O(entities) ECS or wall scan).
pub(crate) struct VisionMoveInputs {
    /// `blocksSight` wall set at the mover's elevation (includes `gm_only` walls — the
    /// full-wall-set invariant, narrowed only by the elevation band test).
    walls: Vec<vision::Seg>,
    /// Vision polygons for every owned token in the scene EXCEPT the moving token, at their
    /// committed (stationary) positions. Constant across all samples of one move.
    static_polys: Vec<Vec<vision::P>>,
    /// The scene's own WORLD-unit envelope (`SceneEcs::scene_world_extent`) — so `polygons_at`'s
    /// per-sample bound stays scene-extent-aware identically to `player_vision_polygons` (no
    /// fork). Never the raw authored bounds, which are measured in grid units (cells),
    /// continuous — never world units, and not required to be integral.
    scene_extent: grid_shape::WorldExtent,
    /// True when the user owns no token in this scene: `polygons_at` returns empty (fail-closed).
    empty: bool,
}

impl VisionMoveInputs {
    /// Per-sample: compute the moving token's visibility polygon at `viewpoint` and prepend it
    /// to the precomputed static polygons. Returns empty when `empty == true` (no owned token
    /// in this scene — fail-closed). Uses the same raycast primitives and wall provenance
    /// (`sight_walls_for`) as `player_vision_polygons` — no fork.
    pub(crate) fn polygons_at(&self, viewpoint: (f64, f64)) -> Vec<Vec<vision::P>> {
        if self.empty {
            return Vec::new();
        }
        let bound = vision::bound_for_scene(
            viewpoint,
            &self.walls,
            self.scene_extent,
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

/// Who is asking `SceneEcs::pathfind` for a route, and what they are allowed to see. These three
/// values decide every per-requester filter the router applies: the visibility mask
/// (`SceneEcs::visible_cells`), the routing wall set (`SceneEcs::move_walls`) and the region field
/// (`SceneEcs::region_field`).
///
/// INVARIANT: this describes the requester ONLY. The route itself (`scene`, `start`, `waypoints`,
/// `footprint_radius`) stays in `pathfind`'s own parameters, and the wire frame that ultimately
/// supplies those values has its own type — `ws::conn`'s `PathfindRequest`, which is
/// client-controlled and unauthorized. The two are deliberately not one type: `PathfindRequest`
/// crosses into this layer only after the presence gate and the named-token ownership check have
/// run, and `footprint_radius` is REPLACED with the token-derived value on the way through.
pub struct RouteRequester<'a> {
    /// The requesting user. Selects the per-requester wall/region view via
    /// `move_walls(scene, Some(user))` / `region_field(scene, Some(user))`, and the visibility
    /// mask via `visible_cells(user, ..)`.
    pub user: Uuid,
    /// Whether the requester is a GM. Skips the mask entirely and selects the AUTHORITATIVE
    /// (`None`-viewer) wall set and region field — callers must never pass a GM's id as the
    /// viewer, per `move_walls`/`region_field`'s two-value contract.
    pub is_gm: bool,
    /// The requester's fog memory for this scene, pre-fetched by the caller off the scene read
    /// lock. Consulted ONLY under `MovementRestriction::Revealed`, where it is unioned into the
    /// mask; `None` degrades `Revealed` to visible-only, which is the fail-closed direction.
    pub explored: Option<&'a crate::scene::explored::ExploredSet>,
}

/// The per-world derived world. Writes are serialized by the caller
/// (`Room::publish` under `publish_guard`); reads (derived recompute) take a
/// shared borrow.
pub struct SceneEcs {
    /// The hecs world holding one `SceneEntity` per hydrated scene doc.
    world: hecs::World,
    /// Document id → hecs entity handle (single lookup index).
    index: HashMap<Uuid, hecs::Entity>,
    /// Per-world seq of the last command reflected in this ECS. Updated under
    /// the same `scene.write()` lock as the entities in `Room::publish`, so a
    /// reader holding the read lock sees a consistent `(entities, seq)` pair and
    /// the derived `computed_at_seq` watermark can never be below the state it
    /// describes.
    committed_seq: i64,
    /// World config-docs (singletons) + actors, hydrated for the lighting-aware vision mask.
    /// Held outside the hecs `world` because they are NOT scene entities
    /// (`is_scene_entity` excludes them); they are maintained by `apply_op` and the room setters.
    world_settings: Option<Document>,
    /// The `system-defaults` singleton, hydrated for the settings chain (engine literal <
    /// system-defaults < world-settings < scene); not a scene entity.
    system_defaults: Option<Document>,
    /// The `resource-registry` singleton, or `None` (the world defines no
    /// turn resources; the movement-budget gate then resolves no binding).
    resource_registry: Option<Document>,
    /// The `light-gradation` singleton config-doc, or `None` (built-in bands).
    gradation: Option<Document>,
    /// The `vision-modes` singleton config-doc, or `None` (seed modes).
    vision_modes: Option<Document>,
    /// Point-lookup table keyed by actor doc id. Used only for `actors.get(id)` joins; must
    /// not be iterated for ordered or wire output (HashMap iteration order is non-deterministic).
    actors: HashMap<Uuid, Document>,
    /// World-level `combat` documents, keyed by doc id. NOT scene entities
    /// (`is_scene_entity` excludes them — a combat is never parented, per
    /// `data::validation`'s containment rule), so held here alongside `actors` rather than in the
    /// hecs `world`. Maintained by `apply_op`; hydrated via `set_combats`. At most one entry per
    /// scene has `active: true` (enforced at write time, not here), so
    /// `active_combat_for_scene`'s "first match" is well-defined regardless of `HashMap`
    /// iteration order.
    combats: HashMap<Uuid, Document>,
    /// Footprint-inflated navmesh cache, keyed by `(scene, quantized footprint-radius
    /// millicells, wall-set key)`. `std::sync::Mutex` (not `RefCell`) + `Arc` (not `Rc`):
    /// `SceneEcs` sits behind a `tokio::sync::RwLock` shared across connection tasks, so
    /// concurrent readers may call `pathfind`/`navmesh_for` simultaneously — the cache needs
    /// `Sync` interior mutability. Never held across an `.await` (lookup + build are
    /// synchronous). Radius quantized to the nearest 1/1000 cell so the cache stays bounded,
    /// since token sizes are a small discrete set — exact f64-bit keying would be vulnerable to
    /// floating-point noise in a client-computed radius producing distinct bit-patterns for what
    /// is logically the same size. The wall-set component (`wall_set_key`) is an exact sorted key
    /// over the included
    /// segments, not a hash: `build_navmesh` inflates walls into obstacles, so a mesh is valid
    /// only for the wall set it was built from — two requesters share an entry exactly when they
    /// see the same walls, and a hash collision here would leak one requester's mesh (and its
    /// wall geometry) to another with a differing view.
    navmesh_cache: std::sync::Mutex<HashMap<NavmeshCacheKey, std::sync::Arc<navmesh::NavMesh>>>,
    /// Per-document decoded-`engine`-field cache, keyed on the
    /// owning document's own id. `engine_as` fully re-`serde_json::from_value`-decodes on every
    /// call; this cache lets the ~19 vision/lighting/pathfinding hot-path call sites in the
    /// `scene` module reuse a prior decode instead. `Mutex` (not `RefCell`), matching `navmesh_cache`, for
    /// the same `Sync`-under-shared-`RwLock` reason. Never locked across an `.await` (every use
    /// here is synchronous). Correctness does NOT depend on catching every mutation site — see
    /// `CachedEngine`'s doc comment: a cached entry is only reused when its stored `source` Value
    /// still equals the document's current `engine`, self-verifying regardless of how the
    /// document was mutated (`apply_op`, `set_world_config`/`set_actors`, or a test-only direct
    /// field assignment). `apply_op` additionally removes the touched document's entry outright
    /// (a best-effort trim, not load-bearing for correctness) so a deleted document's stale entry
    /// doesn't linger indefinitely.
    engine_cache: std::sync::Mutex<HashMap<Uuid, CachedEngine>>,
    /// `visible_cells_cached`'s per-`(user, scene)` mask cache for the movement gate.
    /// Keyed `(user, scene)`, NOT `(user, scene, lenient)` — a `lenient` flip is just another
    /// fingerprint field, so it naturally invalidates the entry rather than needing a wider key
    /// (see `VisibilityInputsSnapshot`). Self-verifying like `engine_cache`, generalized
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

/// Whether the changes being mirrored have cleared the authoritative write path.
/// A failed pointer op means something categorically different on each side, so the
/// two are never reported at the same level.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MirrorInput {
    /// Already committed and broadcast (`apply_op`). BOTH authoritative loops —
    /// `apply_intent` AND `apply_command` (the trusted replay/undo substrate, which
    /// does NOT run `validate_field_change`) — apply the same change through
    /// `apply_field_change` with `?`, so any pointer-op error aborts the transaction
    /// before commit. The guarantee is that `?`, not the ingress gate: attributing it
    /// to `validate_field_change` would cover only one of the two paths and would stop
    /// being true if that gate moved. A failure HERE is therefore a
    /// should-never-happen invariant breach — the store applied a change the mirror
    /// could not. Logged at `error`: the derived world has silently diverged from the
    /// store and needs re-hydration.
    Committed,
    /// Raw client-PROPOSED changes, not yet authorized or even path-validated
    /// (`token_move`, reached from `Room::publish` strictly before `apply_intent`).
    /// A malformed path here is ROUTINE untrusted input, not a defect: any
    /// authenticated client can send `/engine/x/y/z` and it fails closed (the gate
    /// derives no target, the write is rejected downstream). Logged at `debug` —
    /// `error` would let a client flood the channel that exists to surface real
    /// divergence, degrading exactly the signal it carries.
    Proposed,
}

/// Mirror one `FieldChange` into a serialized document value through
/// `command::apply_field_change`, THE store-equal mutation rule (stated once, there;
/// see its INVARIANT for why re-deriving the remove/set branch is the defect).
///
/// A pointer-op error is logged and skipped rather than propagated: this runs on
/// derived state, where the caller cannot reject. `origin` decides the level and the
/// meaning — see `MirrorInput`. Either way the mirror must not silently fall behind
/// the store; `reapply_changes`' round-trip branch reports the other way it can.
fn mirror_field_change(v: &mut serde_json::Value, ch: &FieldChange, origin: MirrorInput) {
    let Err(e) = apply_field_change(v, ch) else {
        return;
    };
    match origin {
        MirrorInput::Committed => tracing::error!(
            path = %ch.path, error = %e,
            "derived ECS: committed field change could not be applied; mirror has diverged from the store"
        ),
        MirrorInput::Proposed => tracing::debug!(
            path = %ch.path, error = %e,
            "derived ECS: proposed field change is malformed; gate derives no target"
        ),
    }
}

/// Re-apply `changes` onto `doc` in place through `mirror_field_change`, via a
/// `Value` round-trip (the server stays structural-only here; no semantic
/// interpretation).
///
/// A post-image that fails to deserialize leaves `doc` UNTOUCHED and is logged at
/// `error`: the op is already committed and broadcast, so the derived ECS cannot
/// reject it, and a stale entity silently dropping every other change in the batch
/// is a defect to observe rather than to swallow (recovery is re-hydration, not a
/// panic — this runs on the broadcast path, and event replay may legitimately carry
/// an older document shape). Unreachable through `apply_intent`, which fails the
/// identical round-trip inside its transaction and never commits.
fn reapply_changes(doc: &mut Document, changes: &[FieldChange]) {
    let mut v = match serde_json::to_value(&*doc) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(doc = %doc.id, error = %e, "derived ECS: document failed to serialize; entity left stale");
            return;
        }
    };
    // Only ever called from `apply_op`/`apply_config_update`, i.e. already-committed ops.
    for ch in changes {
        mirror_field_change(&mut v, ch, MirrorInput::Committed);
    }
    match serde_json::from_value::<Document>(v) {
        Ok(updated) => *doc = updated,
        Err(e) => tracing::error!(
            doc = %doc.id, error = %e,
            "derived ECS: post-image failed to round-trip; entity left stale (re-hydration required)"
        ),
    }
}

/// The document a token's drawn geometry is authored in, as `SceneEcs::token_geometry_source`
/// resolves it.
enum GeometrySource<'a> {
    /// The shared actor document a LINKED token's `actor_id` resolves to.
    Linked(&'a Document),
    /// An INSTANCED token's own embedded actor copy, which reaches a recipient under the token's
    /// access rather than one of its own.
    Embedded(&'a Document),
}

/// The single authority for the `/engine`-tier visibility decision, expressed against an ALREADY
/// RESOLVED `Access`: the tier declared at `permissions.property_overrides["/engine"]` (default
/// `All` when absent) tested through `Access::can_see` — the exact pair `filter_properties` runs
/// per override pointer on whole-document egress, so this channel hides the band on precisely the
/// recipients whose document stream nulls it. Every path needing that decision reaches it here
/// rather than keeping a private copy of the lookup or the predicate (anti-fork). Do not re-inline
/// this at a new call site.
fn engine_tier_visible_to(doc: &Document, access: &crate::data::permission::Access) -> bool {
    let tier = doc
        .permissions
        .property_overrides
        .get("/engine")
        .copied()
        .unwrap_or(crate::data::document::Visibility::All);
    access.can_see(tier)
}

/// Whether `access` receives `doc`'s `/engine` geometry: BOTH gates document egress applies, in
/// egress order — whole-document `cap::READ`, without which `filter_command` withholds the
/// document entirely, then the `/engine` property tier, without which `filter_properties` nulls
/// the band. A derived channel restating engine geometry must clear both, or it hands a recipient
/// the very band their document stream nulled. Stated once here so no caller composes its own.
fn engine_geometry_visible_to(doc: &Document, access: &crate::data::permission::Access) -> bool {
    access.has(crate::data::permission::cap::READ) && engine_tier_visible_to(doc, access)
}

/// The per-requester form of `engine_tier_visible_to`, for callers holding a user id rather than
/// an `Access`. `viewer: None` is the AUTHORITATIVE caller (a GM, or the execution path) and
/// always sees everything — `true` unconditionally. `viewer: Some(user)` resolves that user's
/// access via `permission::resolve_access` + `effective_owner(doc, None)` — the no-actor-join
/// form, exact for any doc type that never carries an actor link (wall, region) — and defers the
/// tier decision itself to `engine_tier_visible_to`. `move_walls` and `region_field` both call
/// this rather than keep a private copy.
fn engine_tier_visible(doc: &Document, viewer: Option<Uuid>) -> bool {
    let Some(user) = viewer else {
        return true;
    };
    let access = crate::data::permission::resolve_access(
        user,
        crate::data::document::WorldRole::Player,
        doc,
        crate::data::permission::effective_owner(doc, None),
    );
    engine_tier_visible_to(doc, &access)
}

/// Exact, order-independent key for a routing wall set — the third component of
/// `NavmeshCacheKey`. A mesh is only valid for the wall set it was inflated from, so two
/// requesters share a mesh exactly when they see the same walls. An EXACT sorted key rather than
/// a hash: a collision would serve one requester a mesh built from another's wall set — the leak
/// this key exists to close — and wall counts here are bounded by
/// `MAX_NAVMESH_OBSTACLE_SEGMENTS` so the cost is irrelevant. Sorted on the raw bit patterns so
/// `hecs`'s unstable iteration order cannot cause a miss.
fn wall_set_key(walls: &[vision::Seg]) -> Vec<(u64, u64, u64, u64)> {
    let mut k: Vec<(u64, u64, u64, u64)> = walls
        .iter()
        .map(|s| {
            (
                s.a.0.to_bits(),
                s.a.1.to_bits(),
                s.b.0.to_bits(),
                s.b.1.to_bits(),
            )
        })
        .collect();
    k.sort_unstable();
    k
}

/// `(scene, quantized footprint-radius millicells, wall-set key)` — see `navmesh_cache`'s field
/// doc comment for what each component means and why.
type NavmeshCacheKey = (Uuid, i64, Vec<(u64, u64, u64, u64)>);

/// The footprint radius used when no effective actor resolves. Not a fail-closed choice: it is
/// more permissive than a 1×1 square's 0.707, and it is the value the gate, the router and a
/// tokenless client route preview all stand on for a token nothing sizes.
pub(crate) const DEFAULT_FOOTPRINT_RADIUS_CELLS: f64 = 0.4;

impl SceneEcs {
    /// An empty derived world: no entities, `committed_seq` 0, cold caches.
    ///
    /// # Examples
    ///
    /// ```
    /// let ecs = shadowcat::scene::SceneEcs::new();
    /// assert_eq!(ecs.committed_seq(), 0);
    /// ```
    pub fn new() -> Self {
        Self {
            world: hecs::World::new(),
            index: HashMap::new(),
            committed_seq: 0,
            world_settings: None,
            system_defaults: None,
            resource_registry: None,
            gradation: None,
            vision_modes: None,
            actors: HashMap::new(),
            combats: HashMap::new(),
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
        system_defaults: Option<Document>,
        resource_registry: Option<Document>,
    ) {
        self.world_settings = world_settings;
        self.gradation = gradation;
        self.vision_modes = vision_modes;
        self.system_defaults = system_defaults;
        self.resource_registry = resource_registry;
    }

    /// Seed the actor table (room-hydration path). Keyed by actor doc id.
    /// Relies on actor docs being world-scoped (parentless), which this method
    /// `debug_assert!`s: a parented actor would also hydrate as a scene entity.
    pub fn set_actors(&mut self, actors: Vec<Document>) {
        debug_assert!(
            actors.iter().all(|d| d.parent_id.is_none()),
            "INVARIANT: actor docs are world-scoped (parentless); a parented actor would also \
             hydrate as a scene entity via is_scene_entity and be double-represented"
        );
        self.actors = actors.into_iter().map(|d| (d.id, d)).collect();
    }

    /// Seed the world's `combat` documents (room-hydration path). World-level, not scene
    /// entities (see the `combats` field doc comment). Kept live thereafter by `apply_op`.
    pub fn set_combats(&mut self, docs: Vec<Document>) {
        self.combats = docs.into_iter().map(|d| (d.id, d)).collect();
    }

    /// Point-lookup into the hydrated actor table (effective-owner joins).
    ///
    /// # Examples
    ///
    /// ```text
    /// let owner = ecs.actor(&actor_id).and_then(|d| d.owner); // in-memory, no pool read
    /// ```
    pub fn actor(&self, id: &Uuid) -> Option<&Document> {
        self.actors.get(id)
    }
    /// The `world-settings` singleton, or `None` (resolvers use defaults).
    pub fn world_settings_doc(&self) -> Option<&Document> {
        self.world_settings.as_ref()
    }
    /// The `system-defaults` singleton, or `None` (resolvers fall through to the engine literal).
    pub fn system_defaults_doc(&self) -> Option<&Document> {
        self.system_defaults.as_ref()
    }
    /// The `resource-registry` singleton's parsed engine, or `None` (absent,
    /// or a malformed body — fail closed to "no binding" rather than guessing).
    pub fn resource_registry_engine(&self) -> Option<eng::ResourceRegistryEngine> {
        let doc = self.resource_registry.as_ref()?;
        self.engine_as_cached::<eng::ResourceRegistryEngine>(doc.id, doc)
    }
    /// The `vision-modes` singleton, or `None` (seed modes apply).
    pub fn vision_modes_doc(&self) -> Option<&Document> {
        self.vision_modes.as_ref()
    }
    /// The `light-gradation` singleton, or `None` (built-in bands apply).
    pub fn gradation_doc(&self) -> Option<&Document> {
        self.gradation.as_ref()
    }

    /// Mirror a config/actor field Update into the side tables (see `reapply_changes`).
    /// Takes `&mut Option<Document>` (not `&mut self`) so the three call sites can borrow the
    /// three distinct singleton fields independently without conflicting on `self`.
    fn apply_config_update(
        slot: &mut Option<Document>,
        doc_id: Uuid,
        changes: &[crate::data::command::FieldChange],
    ) {
        if let Some(d) = slot {
            if d.id == doc_id {
                reapply_changes(d, changes);
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
                        // to SQLite — set AND remove (see `reapply_changes`).
                        reapply_changes(&mut comp.doc, changes);
                    }
                }
                // Config singletons + actors (not in the hecs index).
                Self::apply_config_update(&mut self.world_settings, *doc_id, changes);
                Self::apply_config_update(&mut self.system_defaults, *doc_id, changes);
                Self::apply_config_update(&mut self.gradation, *doc_id, changes);
                Self::apply_config_update(&mut self.vision_modes, *doc_id, changes);
                Self::apply_config_update(&mut self.resource_registry, *doc_id, changes);
                if let Some(a) = self.actors.get_mut(doc_id) {
                    // Same store-equal mutation rule: an actor's `/owner` is an authz
                    // input for every token linked to it, so a forked `remove` here
                    // re-owns tokens the store considers unowned.
                    reapply_changes(a, changes);
                }
                if let Some(d) = self.combats.get_mut(doc_id) {
                    reapply_changes(d, changes);
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
                    "system-defaults"
                        if self.system_defaults.as_ref().map(|d| d.id) == Some(doc.id) =>
                    {
                        self.system_defaults = None;
                    }
                    "light-gradation" if self.gradation.as_ref().map(|d| d.id) == Some(doc.id) => {
                        self.gradation = None;
                    }
                    "vision-modes" if self.vision_modes.as_ref().map(|d| d.id) == Some(doc.id) => {
                        self.vision_modes = None;
                    }
                    "resource-registry"
                        if self.resource_registry.as_ref().map(|d| d.id) == Some(doc.id) =>
                    {
                        self.resource_registry = None;
                    }
                    "actor" => {
                        self.actors.remove(&doc.id);
                    }
                    "combat" => {
                        self.combats.remove(&doc.id);
                    }
                    _ => {}
                }
            }
            Operation::Create { doc } => {
                match doc.doc_type.as_str() {
                    "world-settings" => self.world_settings = Some(doc.clone()),
                    "system-defaults" => self.system_defaults = Some(doc.clone()),
                    "light-gradation" => self.gradation = Some(doc.clone()),
                    "vision-modes" => self.vision_modes = Some(doc.clone()),
                    "resource-registry" => self.resource_registry = Some(doc.clone()),
                    "actor" => {
                        self.actors.insert(doc.id, doc.clone());
                    }
                    "combat" => {
                        self.combats.insert(doc.id, doc.clone());
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
    /// `engine` fails to deserialize into `WorldSettingsEngine`. Every leaf is optional
    /// (`Option`-lifted overlay, the same shape `validated_system_defaults_engine` decodes), so
    /// a partial doc still decodes and contributes only the leaves it declares; `None` means "no
    /// doc" or a malformed body, and every reader falls through to the system layer, then the
    /// engine literals. Used by every resolver that reads world-settings so partial/
    /// malformed-doc handling stays consistent across all of them.
    fn validated_world_settings_engine(&self) -> Option<eng::WorldSettingsEngine> {
        let doc = self.world_settings.as_ref()?;
        self.engine_as_cached::<eng::WorldSettingsEngine>(doc.id, doc)
    }

    /// The validated `system-defaults` engine body, or `None` when the doc is absent or its
    /// stored `engine` fails to deserialize into `SystemDefaultsEngine`. Every leaf of
    /// `SystemDefaultsEngine` is optional (`Option`-lifted overlay), so a partial doc still
    /// decodes; `None` here means "no doc at all", not "malformed". Mirrors
    /// `validated_world_settings_engine`'s caching/fail-closed shape.
    fn validated_system_defaults_engine(&self) -> Option<eng::SystemDefaultsEngine> {
        let doc = self.system_defaults.as_ref()?;
        self.engine_as_cached::<eng::SystemDefaultsEngine>(doc.id, doc)
    }

    /// Resolve a scene's effective lighting/vision settings: engine literal < system-defaults <
    /// world < scene. Fail-closed and `null ⇒ inherit` (mirrors `resolveSceneSettings`).
    pub fn resolve_scene(&self, scene: Uuid) -> ResolvedScene {
        // World and system layers share one overlay shape: each contributes
        // only the leaves it declares.
        let ws = self.validated_world_settings_engine();
        let ws_scene = ws.as_ref().and_then(|w| w.scene.as_ref());
        let sd = self.validated_system_defaults_engine();
        let sd_scene = sd.as_ref().and_then(|s| s.scene.as_ref());
        // Engine literal < system-defaults < world. The innermost fallback is
        // the ONE shared source `WorldSceneDefaults::default` (the client's
        // `DEFAULT_WORLD_SETTINGS` mirrors it) — never a per-field literal.
        let d = eng::WorldSceneDefaults::default();
        let d_los = ws_scene
            .and_then(|s| s.los_restriction)
            .or(sd_scene.and_then(|s| s.los_restriction))
            .unwrap_or(d.los_restriction);
        let d_fog = ws_scene
            .and_then(|s| s.fog)
            .or(sd_scene.and_then(|s| s.fog))
            .unwrap_or(d.fog);
        let d_obs = ws_scene
            .and_then(|s| s.observer_vision)
            .or(sd_scene.and_then(|s| s.observer_vision))
            .unwrap_or(d.observer_vision);
        let d_lit = ws_scene
            .and_then(|s| s.lighting_enabled)
            .or(sd_scene.and_then(|s| s.lighting_enabled))
            .unwrap_or(d.lighting_enabled);
        let d_mode = ws_scene
            .and_then(|s| s.light_mode)
            .or(sd_scene.and_then(|s| s.light_mode))
            .unwrap_or(d.light_mode);
        let d_env_color = ws_scene
            .and_then(|s| s.environment.as_ref().map(|e| e.color.clone()))
            .or_else(|| sd_scene.and_then(|s| s.environment.as_ref().map(|e| e.color.clone())))
            .unwrap_or_else(|| d.environment.color.clone());
        let d_env_int = ws_scene
            .and_then(|s| s.environment.as_ref().map(|e| e.intensity))
            .or_else(|| sd_scene.and_then(|s| s.environment.as_ref().map(|e| e.intensity)))
            .unwrap_or(d.environment.intensity);
        let d_move = ws_scene
            .and_then(|s| s.movement_restriction)
            .or(sd_scene.and_then(|s| s.movement_restriction))
            .unwrap_or(d.movement_restriction);
        let d_model = ws_scene
            .and_then(|s| s.movement_model)
            .or(sd_scene.and_then(|s| s.movement_model))
            .unwrap_or(d.movement_model);
        let d_lenient = ws_scene
            .and_then(|s| s.partial_cell_leniency)
            .or(sd_scene.and_then(|s| s.partial_cell_leniency))
            .unwrap_or(d.partial_cell_leniency);

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

        // Scene bounds: per-scene, no world default — a fixed finite fallback. A
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
            grid_kind: grid_kind_from(s),
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
    /// overrides only vision/lighting/grid). Chain: `world-settings.pathfinding.diagonalRule`
    /// (an authored overlay leaf) < `system-defaults.pathfinding.diagonalRule` < the
    /// `Pathfinding::default` engine literal — consistent with `resolve_scene`'s per-leaf fold.
    pub(crate) fn resolved_diagonal_rule(&self) -> pathfinding::DiagonalRule {
        self.validated_world_settings_engine()
            .and_then(|w| w.pathfinding.and_then(|p| p.diagonal_rule))
            .or_else(|| {
                self.validated_system_defaults_engine()
                    .and_then(|s| s.pathfinding.and_then(|p| p.diagonal_rule))
            })
            .map(conv_diagonal_rule)
            .unwrap_or_else(|| conv_diagonal_rule(eng::Pathfinding::default().diagonal_rule))
    }

    /// Resolves `scene`'s `GridShape` implementation from its own `engine.grid.kind`. `"hex"`
    /// selects `HexGrid { size: cell }`; anything else (including `"square"`, an unrecognized
    /// string, or an absent/malformed scene doc) fails closed to the already-hardened
    /// `SquareGrid { cell, rule: resolved_diagonal_rule() }` — mirrors `resolved_diagonal_rule`'s
    /// own structural-guard convention. `cell` is the caller's own already-resolved grid size
    /// (`scene_grid_sizes()`), not re-derived here, so this can never disagree with a caller's own
    /// cell-size resolution.
    pub(crate) fn resolve_grid_shape(
        &self,
        scene: Uuid,
        cell: f64,
    ) -> Box<dyn grid_shape::GridShape + Send + Sync> {
        self.resolve_grid_shape_with_rule(scene, cell, self.resolved_diagonal_rule())
    }

    /// The scene's authored bounds converted to a world-unit rectangle through its own
    /// `GridShape`, for a caller holding only a scene id — so the raw grid-unit value never
    /// reaches a comparison against world coordinates. Reads the grid-size map itself and defers
    /// to `world_extent_from`, which carries the vision paths' refusal policy over
    /// `scene_world_extent_at`, the single expression of the conversion; a caller that already
    /// holds that map (`player_vision_polygons`, whose loop spans several scenes) calls
    /// `world_extent_from` directly rather than paying for the scan per scene.
    pub(crate) fn scene_world_extent(&self, scene: Uuid) -> grid_shape::WorldExtent {
        self.world_extent_from(&self.scene_grid_sizes(), scene)
    }

    /// The vision paths' REFUSAL policy over `scene_world_extent_at`: the conversion against an
    /// ALREADY-READ `scene_grid_sizes` map, substituting `grid_shape::REFUSED_EXTENT` for a scene
    /// that map does not carry. `scene_world_extent` and `player_vision_polygons`' per-scene memo both
    /// reach the conversion through this, so the two cannot drift into disagreeing about either
    /// the extent or what an absent scene means.
    ///
    /// The zero-AREA envelope (both corners at the origin) when `grid_sizes` has no entry for the
    /// scene: it carries one for every live
    /// scene, so an absent entry means the scene is gone and no extent may be synthesised. Both
    /// corners at the origin cannot SHRINK `vision::bound_for_scene`'s union on any edge; they do
    /// still clamp its low edges to the origin, exactly as a square scene's own minimum does, so
    /// the substitute widens the bound rather than dropping out of it. `navmesh_for` shares the
    /// conversion but NOT this policy: it refuses with `None`, because a navmesh cannot be
    /// triangulated over a zero-area envelope.
    fn world_extent_from(
        &self,
        grid_sizes: &std::collections::HashMap<Uuid, f64>,
        scene: Uuid,
    ) -> grid_shape::WorldExtent {
        grid_sizes
            .get(&scene)
            .copied()
            .map_or(grid_shape::REFUSED_EXTENT, |cell| {
                self.scene_world_extent_at(scene, cell)
            })
    }

    /// The conversion itself, and its ONLY expression: `scene`'s authored bounds through its own
    /// resolved `GridShape`, at a grid size the caller has already resolved. Refuses nothing —
    /// the caller that looked `cell` up owns the policy for a scene that has none, and the two
    /// policies genuinely differ (`world_extent_from` substitutes `grid_shape::REFUSED_EXTENT`,
    /// the zero-area envelope every extent guard already refuses; `navmesh_for` returns `None`).
    ///
    /// A caller that ALREADY holds the scene's resolved settings — `lighting_inputs`,
    /// `player_lit_mask`, `visible_cells_cached` — calls `GridShape::world_extent` on the shape it
    /// holds instead, and must: routing through here would re-read `resolve_scene` per dispatch,
    /// and those re-read settings could disagree with the ones its own caller resolved and is
    /// gating on. The grid size is not part of that argument — this takes `cell` as a parameter
    /// and never reads `scene_grid_sizes`.
    ///
    /// `accumulate_visible_cells` is carved out for a structural reason rather than that one: it
    /// is a free function with no `&self`, so it cannot call this method at all, and converts from
    /// the shape and settings its caller passes it.
    fn scene_world_extent_at(&self, scene: Uuid, cell: f64) -> grid_shape::WorldExtent {
        self.resolve_grid_shape(scene, cell)
            .world_extent(self.resolve_scene(scene).bounds)
    }

    /// The scene's `GridKind`, for a caller that holds no decoded scene engine. Performs exactly
    /// the lookup `resolve_grid_shape_with_rule` already performed inline, and defers the
    /// comparison to `grid_kind_from`, so the shape path pays nothing new and the decision has
    /// one implementation.
    ///
    /// Deliberately NOT routed through `resolve_scene`: that resolver reads the world-settings
    /// document and resolves the full settings set, a cost the shape path runs in per-scene and
    /// per-move loops (`scene_grid_shapes`, `pathfind`, `execute_move`) and does not need.
    pub(crate) fn resolve_grid_kind(&self, scene: Uuid) -> GridKind {
        let eng = self
            .index
            .get(&scene)
            .and_then(|&e| self.world.get::<&SceneEntity>(e).ok())
            .and_then(|c| self.engine_as_cached::<eng::SceneEngine>(scene, &c.doc));
        grid_kind_from(eng.as_ref())
    }

    /// `resolve_grid_shape` with an explicit `SquareGrid` diagonal rule instead of the world-resolved
    /// one. The continuous (navmesh) engine's weighted grid sub-path passes `DiagonalRule::Euclidean`
    /// here so the grid it routes on uses the Euclidean base metric (its cost and its admissible
    /// heuristic both come from the shape), never the world's configured diagonal rule
    /// (continuous ignores the world diagonal rule; only cell topology + terrain multiplier come
    /// from the grid). `rule` is inert on a hex scene — `HexGrid` uses uniform 1-cost steps and the
    /// axial heuristic regardless. Reads `resolve_grid_kind` for the hex-vs-square decision, so a
    /// resolved shape's `GridShape::kind()` can never disagree with it.
    pub(crate) fn resolve_grid_shape_with_rule(
        &self,
        scene: Uuid,
        cell: f64,
        rule: pathfinding::DiagonalRule,
    ) -> Box<dyn grid_shape::GridShape + Send + Sync> {
        // `+ Send + Sync`: `enrich_vision_explored`'s post-lock explored write holds a
        // per-scene map of resolved shapes by shared reference across the spawned egress task's
        // `.await` boundary (a `&Map` is `Send` only when the values are `Sync`). The bound only
        // widens the returned value's capability; every synchronous caller (publish gate, executor)
        // is unaffected, and both concrete impls hold only `f64`/enum fields (trivially `Send + Sync`).
        match self.resolve_grid_kind(scene) {
            GridKind::Hex => Box::new(grid_shape::HexGrid { size: cell }),
            GridKind::Square => Box::new(grid_shape::SquareGrid { cell, rule }),
        }
    }

    /// The resolved `GridShape` for every scene entity, keyed by scene id — the grid-shape
    /// companion to `scene_grid_sizes`. Captured under the ECS read lock so the post-lock explored
    /// accumulation (`enrich_vision_explored`) can index each scene's fog through its own
    /// hex/square geometry without re-borrowing the ECS. Each shape resolves via
    /// `resolve_grid_shape(scene, size)` with the scene's own resolved cell size, so it matches the
    /// movement gate and vision mask exactly.
    pub(crate) fn scene_grid_shapes(
        &self,
    ) -> std::collections::HashMap<Uuid, Box<dyn grid_shape::GridShape + Send + Sync>> {
        let mut out = std::collections::HashMap::new();
        for (scene, size) in self.scene_grid_sizes() {
            out.insert(scene, self.resolve_grid_shape(scene, size));
        }
        out
    }

    /// Resolved animation token speed in cells/second. World-scoped (no per-scene override;
    /// mirrors `resolved_diagonal_rule`'s fold). Chain:
    /// `world-settings.animation.speedCellsPerSec` (an authored overlay leaf) <
    /// `system-defaults.animation.speedCellsPerSec` < the `AnimationSettings::default` engine
    /// literal. The floor of 0.001 prevents a zero/negative config from causing a
    /// division-by-zero in the duration formula.
    pub(crate) fn resolved_animation_speed(&self) -> f64 {
        self.validated_world_settings_engine()
            .and_then(|w| w.animation.and_then(|a| a.speed_cells_per_sec))
            .or_else(|| {
                self.validated_system_defaults_engine()
                    .and_then(|s| s.animation.and_then(|a| a.speed_cells_per_sec))
            })
            .unwrap_or(eng::AnimationSettings::default().speed_cells_per_sec)
            .max(0.001)
    }

    /// Resolved vision-mode registry. Returns a `BTreeMap` for deterministic key order
    /// (`.get(id)` works identically for callers).
    /// Fail-closed to the engine seed (`eng::VisionModesEngine::seed`) ONLY when no doc/`modes`
    /// is present (mirrors TS `sys?.modes ?? SEED`). A GM-authored modes doc with all-malformed entries is
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
                    out.insert(id, conv_vision_mode(m));
                }
            }
            None => {
                // The engine seed is the fallback (the client's `SEED_VISION_MODES` mirrors it):
                // read through the SAME `conv_vision_mode` as the authored-doc branch so the two
                // can never drift.
                for (id, m) in eng::VisionModesEngine::seed().modes {
                    out.insert(id, conv_vision_mode(m));
                }
            }
        }
        out
    }

    /// Count of hydrated scene entities. Feeds the debug-only `"identity"` channel's payload.
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

    /// `Room::publish`'s sole caller: the refusal predicate compares this call's pre- and
    /// post-image to reject any non-GM `Update` that changes a token's position (players move
    /// only via `MoveRequest` → `execute_move`). A `/system/x` write on a token is structurally
    /// inert against this `/engine`-only read; see the
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
        // Store-equal mutation rule (`command::apply_field_change`). These changes are
        // client-PROPOSED, not yet committed, so this is hardening rather than a
        // reachable divergence: `TokenEngine.x`/`y` are required `f64`, so a `remove`
        // of `/engine/x` fails `validate_engine_tree` on the post-image and never
        // commits. Deriving the projected position by the same rule the store uses
        // keeps that true by construction instead of by coincidence — the gate can
        // never judge a position on a value the store computes differently.
        //
        // `Proposed`: `Room::publish` reaches here with RAW client changes, strictly
        // before `apply_intent` runs `validate_field_change`, so a malformed path is
        // untrusted input a client can send at will — not an invariant breach, and not
        // an `error!` a client may emit on demand.
        for ch in changes {
            mirror_field_change(&mut v, ch, MirrorInput::Proposed);
        }
        let nx = v.pointer("/engine/x").and_then(|x| x.as_f64())?;
        let ny = v.pointer("/engine/y").and_then(|x| x.as_f64())?;
        Some((scene, (cx, cy), (nx, ny)))
    }

    /// Per-player visibility polygons, each tagged with the scene it belongs to: one
    /// star-shaped polygon per token the user owns, computed against that token's scene's
    /// `blocksSight` walls. The server raycasts the FULL wall set (so a `gm_only` wall the player
    /// never receives still occludes); the player only ever gets their own polygons. The
    /// scene tag lets the client cut fog holes only for the scene it is rendering — a token in
    /// scene B must not punch a hole into scene A's fog (scene coordinates are scene-local).
    /// Empty when the player controls no tokens.
    pub fn player_vision_polygons(&self, user_id: Uuid) -> Vec<(Uuid, Vec<vision::P>)> {
        // Collect owned-token viewpoints first (drops the query borrow before the wall queries).
        // Each carries its token's elevation: the sight-wall set is filtered per source through
        // `sight_walls_for` (a token above a wall's band sees over it).
        let mut viewpoints: Vec<(Uuid, vision::P, f64)> = Vec::new();
        for e in self.world.query::<&SceneEntity>().iter() {
            if e.doc.doc_type != "token" || self.token_effective_owner(&e.doc) != Some(user_id) {
                continue;
            }
            if let (Some(t), Some(scene)) = (
                self.engine_as_cached::<eng::TokenEngine>(e.doc.id, &e.doc),
                e.doc.parent_id,
            ) {
                viewpoints.push((
                    scene,
                    (t.x, t.y),
                    elevation::elevation_or_ground(t.elevation),
                ));
            }
        }
        // `scene_grid_sizes` is a full entity scan, so it is read ONCE here rather than per
        // viewpoint. The extent is then memoised PER SCENE ID, never hoisted to a single value:
        // this loop spans every scene the user owns a token in, so one extent applied across it
        // would measure one scene's vision bound against another scene's rectangle. Both the
        // conversion (`scene_world_extent_at`) and the absent-scene policy (`world_extent_from`)
        // stay shared with the streamed path in `player_vision_inputs`, which reaches the same two
        // bodies through `scene_world_extent` — by construction, not by convention.
        let grid_sizes = self.scene_grid_sizes();
        let mut extents: std::collections::HashMap<Uuid, grid_shape::WorldExtent> =
            std::collections::HashMap::new();
        let mut out = Vec::with_capacity(viewpoints.len());
        for (scene, vp, elev) in viewpoints {
            let walls = self.sight_walls_for(scene, elev);
            let scene_extent = *extents
                .entry(scene)
                .or_insert_with(|| self.world_extent_from(&grid_sizes, scene));
            let bound = vision::bound_for_scene(vp, &walls, scene_extent, VISION_BOUND_MARGIN);
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
        // Collect static-token viewpoints (non-moving owned tokens in `scene`), each with its
        // elevation, plus the mover's own elevation (its per-sample raycast is filtered by the
        // walls its height can see over/under). Drop the query borrow before wall queries —
        // mirrors player_vision_polygons collect-then-query order.
        let mut static_vps: Vec<(vision::P, f64)> = Vec::new();
        let mut mover_elevation = elevation::GROUND;
        let mut has_owned = false;
        for e in self.world.query::<&SceneEntity>().iter() {
            if e.doc.doc_type != "token"
                || e.doc.parent_id != Some(scene)
                || self.token_effective_owner(&e.doc) != Some(user)
            {
                continue;
            }
            has_owned = true;
            let Some(t) = self.engine_as_cached::<eng::TokenEngine>(e.doc.id, &e.doc) else {
                continue;
            };
            let elev = elevation::elevation_or_ground(t.elevation);
            if e.doc.id == moving_token {
                mover_elevation = elev;
                continue; // mover's viewpoint varies per sample; skip here
            }
            static_vps.push(((t.x, t.y), elev));
        }
        let scene_extent = self.scene_world_extent(scene);
        if !has_owned {
            return VisionMoveInputs {
                walls: Vec::new(),
                static_polys: Vec::new(),
                scene_extent,
                empty: true,
            };
        }
        // The mover's wall set, filtered at the mover's elevation: computed once for the
        // entire move (the mover's elevation is constant across its own samples).
        let walls = self.sight_walls_for(scene, mover_elevation);
        // Static polygons: one per stationary owned token, each filtered at that token's own
        // elevation; constant across all samples.
        let static_polys = static_vps
            .iter()
            .map(|&(vp, elev)| {
                let walls = self.sight_walls_for(scene, elev);
                let bound = vision::bound_for_scene(vp, &walls, scene_extent, VISION_BOUND_MARGIN);
                vision::visibility_polygon(vp, &walls, bound)
            })
            .collect();
        VisionMoveInputs {
            walls,
            static_polys,
            scene_extent,
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

    /// Each scene's grid cell size (`engine.grid.size`), defaulting to 100 — the unit the
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

    /// The scene's `blocksMove` wall segments. Mirrors the wall filter in `blocks_move`
    /// (doc_type "wall", parent = scene, `engine.blocksMove == true`, endpoints at
    /// `engine.seg.{x1,y1,x2,y2}`). INVARIANT: same filter as `blocks_move` — any divergence
    /// would allow the pathfinder to route through walls the movement gate would then reject.
    ///
    /// Two-value secrecy contract, identical to `region_field`'s and never a third mode:
    /// `viewer: None` is the AUTHORITATIVE set — used by `execute_move` and by a GM requester;
    /// `viewer: Some(user)` is the PER-REQUESTER set used by the routers, where a wall is included
    /// only when `user` can see the visibility tier declared on its `/engine`. A `gm_only` wall is
    /// therefore absent from a non-GM's route (its geometry cannot be inferred from route shape)
    /// but still blocks at execution, exactly as a secret region springs. Callers MUST pass `None`
    /// for a GM requester.
    ///
    /// Scope: this is the ROUTING wall set only. `sight_walls`/`light_walls` deliberately carry the
    /// full set including `gm_only` walls (full-wall-set invariant) — a wall you cannot see
    /// still blocks your sight, which under-reveals and is correct. Do not unify the two.
    pub(crate) fn move_walls(&self, scene: Uuid, viewer: Option<Uuid>) -> Vec<vision::Seg> {
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
            if !engine_tier_visible(&w.doc, viewer) {
                continue;
            }
            out.push(vision::Seg {
                a: (wall.seg.x1, wall.seg.y1),
                b: (wall.seg.x2, wall.seg.y2),
            });
        }
        out
    }

    /// Build-or-fetch the footprint-inflated navmesh for `(scene, footprint_radius_cells,
    /// walls)`, memoized in `navmesh_cache` keyed on a quantized radius (nearest 1/1000 cell —
    /// see the field doc comment) plus an exact wall-set key (`wall_set_key`). Returns `None`
    /// when `navmesh::build_navmesh` fails closed (a degenerate world extent — which is what a
    /// degenerate cell size becomes — a degenerate footprint distance, or an over-cap obstacle
    /// count) — callers must treat this exactly like the grid router's
    /// `Unreachable` (no silent all-pass). A failed build is intentionally NOT cached: caching a
    /// failure under a mutable key would either mask a later successful build once the scene's
    /// geometry is fixed up (stale-failure, never re-attempted without an unrelated
    /// cache-clearing mutation), or require a separate "known-bad" sentinel distinct from "not
    /// yet built" — added complexity for no correctness gain, since a redundant re-run of
    /// `build_navmesh` on a still-degenerate scene hits the same fail-fast validation and is
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
        walls: &[vision::Seg],
    ) -> Option<std::sync::Arc<navmesh::NavMesh>> {
        // Validate BEFORE computing the cache key or touching the cache at all. `f64 as i64`
        // saturates NaN to 0 and rounds a tiny negative (e.g. -0.0001) to -0, which also casts to
        // 0 — colliding with the legitimate key for `footprint_radius_cells == 0.0`. Without this
        // upfront guard a degenerate radius would silently hit that cached entry and return a
        // valid-looking `Some` mesh instead of failing closed. This is the SOLE site of the
        // radius-RANGE refusal: `build_navmesh` receives the already-converted world distance and
        // refuses only on that distance's own magnitude, so the range check cannot be re-derived
        // downstream and must not be dropped here.
        if !(0.0..=crate::scene::pathfinding::MAX_FOOTPRINT_CELLS).contains(&footprint_radius_cells)
        {
            return None;
        }
        // Quantize to the nearest 1/1000 cell so floating-point noise in a client-computed radius
        // (e.g. derived via division) collapses onto the same cache entry as the canonical value.
        let quantized = (footprint_radius_cells * 1000.0).round() as i64;
        let key = (scene, quantized, wall_set_key(walls));
        if let Some(cached) = self.navmesh_cache.lock().unwrap().get(&key) {
            return Some(cached.clone());
        }
        // An absent `scene_grid_sizes` entry means the scene has no live document — refuse
        // rather than synthesize a grid (`scene_grid_sizes`'s own doc comment is the source
        // of this invariant; every reader here keys off it). This `?` is this path's OWN refusal
        // policy: `scene_world_extent_at` is shared with the vision-bound paths, which substitute
        // the zero rectangle instead, and a navmesh has no use for a rectangle of zero area.
        let cell = self.scene_grid_sizes().get(&scene).copied()?;
        let extent = self.scene_world_extent_at(scene, cell);
        // The footprint radius is already stated in the grid's OWN cells by
        // `footprint::resolve_footprint_cells` — the authored block's conservative enclosure on
        // square, the circumscribing radius of the authored hex count on hex — so it converts
        // through the INDEXING scale, not the per-cell world distance; see
        // `GridShape::world_units_per_cell`'s own note on why scaling it is a rules change.
        let footprint_scene = footprint_radius_cells * cell;
        let built = navmesh::build_navmesh(extent, footprint_scene, walls)?;
        let arc = std::sync::Arc::new(built);
        self.navmesh_cache.lock().unwrap().insert(key, arc.clone());
        Some(arc)
    }

    /// Plan a route for `requester`'s token in `scene`. Reuses the `visible_cells`
    /// mask so the preview agrees with the movement gate. `requester.is_gm`/`unrestricted` ⇒
    /// no mask; `visible` ⇒ `visible_cells`; `revealed` ⇒ `visible_cells ∪ requester.explored`.
    /// An empty non-GM mask ⇒ `find` returns Unreachable (fail-closed —
    /// the dark-scene freeze that mirrors the movement gate, by design).
    /// `budget_cells` is the movement-budget preview clamp: `Some` cuts the
    /// route at the last step whose cumulative weighted cost fits, setting
    /// `PathOutcome.truncated` (both engines; the caller resolves it through
    /// the same gate the executor enforces).
    ///
    /// Coupling: `visible_cells` is the ONE canonical mask shared between this
    /// method, the movement gate (`move_exec::execute_move`, reached via
    /// `Room::execute_move`), and `Room::publish`'s token-placement gate. Do NOT fork the
    /// per-cell decision here.
    pub fn pathfind(
        &self,
        requester: RouteRequester<'_>,
        scene: Uuid,
        start: (f64, f64),
        waypoints: &[(f64, f64)],
        footprint_radius: f64,
        budget_cells: Option<f64>,
    ) -> Result<pathfinding::PathOutcome, pathfinding::PathFail> {
        let RouteRequester {
            user,
            is_gm,
            explored,
        } = requester;
        // Scene-existence admissibility, ahead of any routing work and for every requester
        // including a GM. Coupling: both movement gates (`Room::publish`, `Room::execute_move`)
        // refuse a scene with no document, so the router agrees with them on which scenes are
        // admissible at all — a router that silently substituted a 100-unit default would build
        // its mask, region field, and grid shape in a grid no scene declared, while the gate that
        // must later authorize the resulting route refuses the same input outright.
        // `scene_grid_sizes` carries an entry — already defaulted to 100 — for every live scene,
        // so an absent entry means the scene itself is gone.
        let Some(cell) = self.scene_grid_sizes().get(&scene).copied() else {
            return Err(pathfinding::PathFail::Invalid);
        };
        let grid_shape = self.resolve_grid_shape(scene, cell);
        // Per-requester routing wall set: a non-GM's route omits `gm_only` walls, so their
        // geometry cannot be inferred from route shape. The executor always reads the authoritative
        // set (`None`) and springs a secret wall at execution, exactly as a secret region springs.
        // Hoisted out of the engine dispatch so BOTH engines receive the SAME slice — never a
        // forked wall computation (the same discipline `mask` follows).
        let walls = self.move_walls(scene, if is_gm { None } else { Some(user) });
        // Hoisted so `movement_model` is available to the engine dispatch regardless of `is_gm`
        // (a GM can also route on a continuous scene); the mask build and the dispatch discriminant
        // read this one resolution.
        let settings = self.resolve_scene(scene);

        // Build the per-(user,scene) mask (None ⇒ unconstrained). Shared by both engines —
        // Never fork the per-cell visibility decision.
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
                // Per-requester region field: GM (or `is_gm`) sees the authoritative
                // field; a non-GM requester's field silently omits any region they cannot see, so
                // a secret region never influences their route or budget (it "springs" only at
                // execution, `move_exec`, which always reads the authoritative field).
                let Some(regions) = self.region_field(scene, if is_gm { None } else { Some(user) })
                else {
                    return Err(pathfinding::PathFail::Invalid);
                };
                pathfinding::find(
                    start,
                    waypoints,
                    pathfinding::PathInputs {
                        footprint_radius_cells: footprint_radius,
                        cell,
                        walls: &walls,
                        mask: mask.as_ref(),
                        regions: Some(&regions),
                        shape: &*grid_shape,
                        budget_cells,
                    },
                )
            }
            MovementModel::Continuous => {
                // The per-requester region field is the SINGLE weighting authority for the
                // continuous engine too (polyanya cannot weight). Terrain or
                // impassable present ⇒ route via the weighted grid A* forced to Euclidean
                // (continuous base metric), then LOS-smooth back to any-angle geometry. Otherwise
                // the unchanged pure polyanya route + an arrest post-filter. Arrest applies on both
                // paths. The per-requester field omits any region a non-GM cannot see (secret
                // regions spring only at `move_exec`).
                let Some(regions) = self.region_field(scene, if is_gm { None } else { Some(user) })
                else {
                    return Err(pathfinding::PathFail::Invalid);
                };
                if regions.has_terrain_or_impassable() {
                    // Euclidean base metric: the grid's step cost AND its admissible
                    // heuristic both come from this shape, so the weighted continuous route ignores
                    // the world's configured diagonal rule — only cell topology + terrain multiplier
                    // come from the grid. A hex scene's shape is rule-agnostic (uniform 1-cost).
                    let euclid_shape = self.resolve_grid_shape_with_rule(
                        scene,
                        cell,
                        pathfinding::DiagonalRule::Euclidean,
                    );
                    let weighted = pathfinding::find(
                        start,
                        waypoints,
                        pathfinding::PathInputs {
                            footprint_radius_cells: footprint_radius,
                            cell,
                            walls: &walls,
                            mask: mask.as_ref(),
                            regions: Some(&regions),
                            shape: &*euclid_shape,
                            // The budget cuts the PRE-smooth route: `los_smooth` only ever
                            // shortens a chord, so the smoothed result stays within budget —
                            // an occasional under-reach, never an over-show.
                            budget_cells,
                        },
                    )?;
                    // `find` already reports cost in CELLS — the wire contract `PathResult`'s
                    // own doc comment promises (`ws::protocol`) and the grid-stepped branch
                    // above already honors — but `los_smooth` recomputes its own exact per-span
                    // cost rather than carrying `weighted.cost` through (see `los_smooth`'s doc
                    // comment), also already in cells. The pure-polyanya sub-path below is the
                    // one that computes Euclidean scene-unit lengths and converts once, at its
                    // own boundary.
                    // `grid_shape`, not `euclid_shape`: the smoother's cell indexing must match the
                    // shape `mask` (`visible_cells`) and `regions` (`region_field`) were built with,
                    // both of which resolve through `resolve_grid_shape`. The two shapes are
                    // cell-identical by construction — `DiagonalRule` feeds only
                    // `neighbors_with_cost`/`heuristic` (step cost + search order), never
                    // `cell_of`/`footprint_cells`/`line_traversal` — so this is an identity
                    // statement, not a behavior change.
                    Ok(navmesh::los_smooth(
                        weighted,
                        &walls,
                        mask.as_ref(),
                        &regions,
                        cell,
                        footprint_radius,
                        &*grid_shape,
                    ))
                } else {
                    let nav = self
                        .navmesh_for(scene, footprint_radius, &walls)
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
                        &*grid_shape,
                    );
                    if clipped.path.len() < 2 && !raw_was_trivial {
                        return Err(pathfinding::PathFail::Unreachable);
                    }
                    let outcome =
                        navmesh::truncate_at_arrest(clipped, &regions, cell, &*grid_shape);
                    // Budget cut in scene units (the budget is authored in cells; `wu`
                    // converts below): valid on this walls-only path because the field has
                    // no terrain weights here, so Euclidean length IS the weighted cost —
                    // the same assumption `truncate_at_arrest`'s own recompute makes.
                    let wu_for_budget = grid_shape.world_units_per_cell();
                    let outcome = match budget_cells {
                        Some(b) if wu_for_budget.is_finite() && wu_for_budget > 0.0 => {
                            navmesh::truncate_at_budget(outcome, b, wu_for_budget)
                        }
                        _ => outcome,
                    };
                    // Convert once, at the boundary: `navmesh_find`/`clip_to_visible_mask`/
                    // `truncate_at_arrest` all compute Euclidean lengths in SCENE units, but
                    // `PathResult`'s wire contract (`ws::protocol`) promises cells, matching the
                    // grid-stepped/weighted branches above. `world_units_per_cell` — the
                    // authored-distance conversion, not `cell` (the indexing scale) — is the same
                    // symbol `lighting_inputs_from` converts an authored light radius through; a
                    // route length is the same class of authored distance. Guarded like that
                    // conversion's own divisor: a non-finite or non-positive per-cell distance
                    // refuses rather than dividing into an infinity the client would render as a
                    // label.
                    let wu = grid_shape.world_units_per_cell();
                    if !wu.is_finite() || wu <= 0.0 {
                        return Err(pathfinding::PathFail::Invalid);
                    }
                    Ok(pathfinding::PathOutcome {
                        cost: outcome.cost / wu,
                        ..outcome
                    })
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
    /// document's egress — no new secrecy machinery. A secret region's whole geometry
    /// lives in the `engine` band, so the visibility-tier lookup targets the `/engine`
    /// property-override pointer, not `/system`. Callers MUST pass `None` for a GM requester (a
    /// GM always sees the authoritative field, mirroring `visible_cells`'s GM-skips-the-mask
    /// convention in `pathfind`).
    ///
    /// Returns `None` when `scene` has no live document (an absent `scene_grid_sizes` entry) —
    /// refuse rather than synthesize a grid. Callers must refuse the whole operation on `None`,
    /// mirroring `pathfind`'s `PathFail::Invalid`.
    pub(crate) fn region_field(
        &self,
        scene: Uuid,
        viewer: Option<Uuid>,
    ) -> Option<regions::RegionField> {
        let cell = self.scene_grid_sizes().get(&scene).copied()?;
        let grid = self.resolve_grid_shape(scene, cell);
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
            if !engine_tier_visible(doc, viewer) {
                continue;
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
            builder.add(&shape, behavior, cost, cell, &*grid);
        }
        Some(builder.build())
    }

    /// The user this token effectively belongs to — the SAME rule the write-authz
    /// path enforces (`permission::effective_owner`): the token's own `owner`
    /// override, else its LINKED actor's owner, joined live through `self.actors`
    /// exactly as `token_vision_floors` joins vision. Nothing is stamped, so a
    /// re-assigned actor re-owns its linked tokens on the next resolution.
    ///
    /// Coupling: every "does this user control this token?" test in the vision /
    /// lit-mask family calls this. Reading `doc.owner` directly at any of those
    /// sites forks ownership — a player could then move a token that contributes
    /// no vision, or see through one they cannot move.
    pub fn token_effective_owner(&self, token: &Document) -> Option<Uuid> {
        crate::data::permission::effective_owner_via(token, &|id| self.actors.get(id))
    }

    /// Whether `user` effectively controls a token parented to `scene`. A pure document scan —
    /// no raycast, no illumination — so it is cheap enough for an unrate-limited request path.
    ///
    /// Presence, not vision: ownership resolves through `token_effective_owner` (per-token
    /// override, else the linked actor's owner), so it is the same rule the write-authz and
    /// vision paths enforce; observer-vision tokens are excluded, since seeing a token in a
    /// scene is not controlling one there. Equals the condition under which
    /// `player_vision_inputs` returns a non-empty polygon set for the same `(user, scene)` —
    /// both key on `parent_id` plus effective ownership, and neither requires the token to
    /// carry a position: `has_owned` is set on the ownership predicate alone, BEFORE the
    /// `engine_as_cached::<TokenEngine>` parse, which only decides whether that token also
    /// contributes a static viewpoint; `polygons_at` is empty iff `!has_owned`.
    ///
    /// That equality is against `player_vision_inputs`/`VisionMoveInputs::polygons_at` ONLY —
    /// NOT `gather_vision_sources_in_scene`, the visibility mask's source, which is a different
    /// set in both directions: it additionally requires a parseable `TokenEngine`, and it unions
    /// observer-tier tokens when `observerVision` is on. Conflating the two reads this comment
    /// as a false claim about the mask.
    ///
    /// Coupling: `handle_pathfind` gates a non-GM route request on this. Weakening it to a raw
    /// `doc.owner` read would fork ownership away from `token_effective_owner` and let an
    /// actor-inherited owner lose access to a scene they legitimately hold a token in.
    pub(crate) fn user_owns_token_in_scene(&self, user: Uuid, scene: Uuid) -> bool {
        for e in self.world.query::<&SceneEntity>().iter() {
            if e.doc.doc_type == "token"
                && e.doc.parent_id == Some(scene)
                && self.token_effective_owner(&e.doc) == Some(user)
            {
                return true;
            }
        }
        false
    }

    /// A token's `(parent scene, effective owner)`, or `None` when `token` is not a known token
    /// document. Ownership routes through the SAME `token_effective_owner` rule
    /// `user_owns_token_in_scene` uses — never a forked, looser test — so a caller authorizing a
    /// client-named token id (e.g. `Pathfind`'s `token`) gets the identical rule the presence gate
    /// and write-authz enforce.
    pub(crate) fn token_scene_and_effective_owner(
        &self,
        token: Uuid,
    ) -> Option<(Uuid, Option<Uuid>)> {
        let &e = self.index.get(&token)?;
        let ent = self.world.get::<&SceneEntity>(e).ok()?;
        if ent.doc.doc_type != "token" {
            return None;
        }
        let scene = ent.doc.parent_id?;
        Some((scene, self.token_effective_owner(&ent.doc)))
    }

    /// The token's effective vision modes as `(floor_min_illumination, range_cells, render_hint)`
    /// triples. `range_cells == 0.0` ⇒ unlimited. `render_hint` mirrors `VisionMode.render_hint`
    /// (e.g. `Some("desaturate")` for darkvision). Precedence (mirrors `resolveTokenActor`):
    /// a LINKED token (`actor_id` present) resolves the shared actor and applies
    /// `overrides.vision` as a wholesale replacement when present; a dangling link (actor absent)
    /// yields normal, ignoring overrides. An INSTANCED token (no `actor_id`) uses its
    /// `embedded.actor[0].engine.vision` without overrides. An unknown mode id is dropped
    /// (fail-closed: it contributes no vision floor). A `Perception::Creatures` mode is likewise
    /// absent here — creature senses perceive tokens, not terrain
    /// (`SceneEcs::player_perceived_tokens` is their consumer), so they must not widen the
    /// illumination-floor mask. Always returns ≥1 triple (normal fallback
    /// with `render_hint: None`).
    pub fn token_vision_floors(&self, token: &Document) -> Vec<(f64, f64, Option<String>)> {
        let modes = self.resolved_vision_modes();
        let bands = self.resolved_bands();

        let token_eng = self.engine_as_cached::<eng::TokenEngine>(token.id, token);

        // Mirror `resolveTokenActor`: a LINKED token (actor_id) resolves the shared actor and
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
                   // A creature sense contributes no terrain floor — it perceives tokens
                   // (`player_perceived_tokens`), never the illumination mask.
                if vm.perceives == eng::Perception::Creatures {
                    continue;
                }
                // An omitted assignment range inherits the mode's own authored default — both
                // are authored in the SAME unit (grid cells; see `VisionAssignment::range`'s and
                // `VisionMode::default_range`'s docs), so no additional per-cell conversion is
                // needed here: the value feeds straight into the same `dist_cells` comparison
                // `a.range` always fed.
                out.push((
                    crate::scene::lighting::floor_min(&bands, &vm.illumination_floor),
                    a.range.unwrap_or(vm.default_range),
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

    /// The document a token's `shape`/`size` are authored in, joined through the SAME actor
    /// precedence `token_vision_floors` implements: a LINKED token (`actor_id` present) resolves
    /// the shared actor, and a dangling link yields `None` (overrides ignored, mirroring
    /// `resolveTokenActor`); an INSTANCED token (no `actor_id`) resolves its own embedded copy.
    ///
    /// Stated once because two callers ask about the same join for different reasons —
    /// `token_shape_and_size` reads the values out of it, `token_footprint_visible` decides
    /// whether a recipient may receive them — and a second copy of the branch would let the
    /// document whose band is checked drift from the document the size comes from.
    fn token_geometry_source<'a>(&'a self, token_doc: &'a Document) -> Option<GeometrySource<'a>> {
        match self
            .engine_as_cached::<eng::TokenEngine>(token_doc.id, token_doc)
            .and_then(|t| t.actor_id)
        {
            Some(id) => self.actor(&id).map(GeometrySource::Linked),
            None => token_doc
                .embedded
                .get("actor")
                .and_then(|v| v.first())
                .map(GeometrySource::Embedded),
        }
    }

    /// A token's effective `(shape, size)`, read out of the document `token_geometry_source`
    /// resolves: a LINKED token applies `overrides.shape`/`overrides.size` (each independently,
    /// per-field) over the shared actor's own value; an INSTANCED token reads its embedded copy
    /// through the deliberately-uncached direct `engine_as` path — an embedded actor's own `id`
    /// differs from the token's, so caching under either key would go stale on an
    /// `/embedded/actor/0/...` write.
    fn token_shape_and_size(&self, token: Uuid) -> Option<(String, eng::Size)> {
        let &e = self.index.get(&token)?;
        let tok = self.world.get::<&SceneEntity>(e).ok()?;
        let doc = &tok.doc;

        match self.token_geometry_source(doc)? {
            GeometrySource::Linked(actor) => {
                let actor_eng = self.engine_as_cached::<eng::ActorEngine>(actor.id, actor)?;
                let token_eng = self.engine_as_cached::<eng::TokenEngine>(token, doc);
                let overrides = token_eng.as_ref().and_then(|t| t.overrides.as_ref());
                let shape = overrides
                    .and_then(|o| o.shape.clone())
                    .unwrap_or(actor_eng.shape);
                let size = overrides.and_then(|o| o.size).unwrap_or(actor_eng.size);
                Some((shape, size))
            }
            GeometrySource::Embedded(child) => {
                engine_as::<eng::ActorEngine>(child).map(|a| (a.shape, a.size))
            }
        }
    }

    /// A token's bounding-disc radius in GRID UNITS (cells), resolved against `scene`'s grid kind
    /// via `footprint::resolve_footprint_cells`. Effective-actor resolution mirrors `resolveTokenActor`
    /// via the SAME join `token_vision_floors` implements: a LINKED token resolves the shared actor
    /// and applies the per-token override whitelist; a dangling link ignores overrides; an
    /// INSTANCED token uses its embedded copy and overrides do not apply.
    ///
    /// `None` means REFUSE — `footprint::resolve_checked` declined, because the stored size is
    /// degenerate or the derived radius is outside `[0, MAX_FOOTPRINT_CELLS]`. Callers must fail
    /// closed, never substitute a default: clamping an oversized token to the bound would route
    /// and gate it as a smaller disc, letting it enter gaps its real footprint cannot (a geometric
    /// fail-open).
    pub(crate) fn resolve_token_footprint(&self, token: Uuid, scene: Uuid) -> Option<f64> {
        let Some((shape, size)) = self.token_shape_and_size(token) else {
            return Some(DEFAULT_FOOTPRINT_RADIUS_CELLS);
        };
        match footprint::resolve_checked(self.resolve_grid_kind(scene), &shape, size.w, size.h) {
            Ok(f) => Some(f.radius),
            Err(reason) => {
                tracing::warn!(
                    ?token,
                    w = size.w,
                    h = size.h,
                    ?reason,
                    "refusing a token footprint"
                );
                None
            }
        }
    }

    /// The scene's running combat, if any: the first `combats` entry (see that field's doc
    /// comment for why "first" is well-defined) whose decoded `CombatEngine` is `active` and
    /// bound to `scene`. `None` means no gate applies at all — the caller (`Room::execute_move`)
    /// must treat that as unlimited movement, not a refusal.
    pub fn active_combat_for_scene(&self, scene: Uuid) -> Option<(Uuid, eng::CombatEngine)> {
        self.combats.iter().find_map(|(id, doc)| {
            let ce = self.engine_as_cached::<eng::CombatEngine>(*id, doc)?;
            (ce.active && ce.scene_id == scene).then_some((*id, ce))
        })
    }

    /// The `combatant` document parented to `combat` that represents `token`: matches
    /// `CombatantKind::Actor.token_id == Some(token)` first, else `actor_id` against the
    /// token's own resolved actor id (`TokenEngine.actor_id` for a LINKED token, else the id of
    /// its embedded actor copy for an INSTANCED token — the same join `token_geometry_source`
    /// performs). Returns `(combatant_id, engine, access)`, where `access` is `ctx`'s resolved
    /// `Access` on that combatant document — the SAME `effective_owner_via` +
    /// `resolve_access_world` pair every other whole-document READ decision uses (`ctx_access`),
    /// never a hand-rolled readability predicate: a combatant's hidden state is whole-document
    /// unreadability, so `permissions.users` grants and overrides decide it exactly as they do
    /// at document egress. `None` means the token names no combatant in this combat — the caller
    /// must treat that as "moves freely", not a refusal (a token need not be in the fight to
    /// move on a scene where a fight is happening).
    pub fn combatant_for_token(
        &self,
        combat: Uuid,
        token: Uuid,
        ctx: &PermissionContext,
        world_defaults: &crate::data::document::WorldCapDefaults,
    ) -> Option<(Uuid, eng::CombatantEngine, crate::data::permission::Access)> {
        let resolved_actor = self
            .index
            .get(&token)
            .and_then(|&e| self.world.get::<&SceneEntity>(e).ok())
            .and_then(|c| {
                let token_eng = self.engine_as_cached::<eng::TokenEngine>(token, &c.doc);
                token_eng.and_then(|t| t.actor_id).or_else(|| {
                    c.doc
                        .embedded
                        .get("actor")
                        .and_then(|v| v.first())
                        .map(|a| a.id)
                })
            });

        // `token_id` matches take precedence over `actor_id` matches (a combatant explicitly
        // bound to this token wins over one merely sharing its resolved actor).
        let mut by_actor: Option<(Uuid, eng::CombatantEngine, crate::data::permission::Access)> =
            None;
        for e in self.world.query::<&SceneEntity>().iter() {
            if e.doc.doc_type != "combatant" || e.doc.parent_id != Some(combat) {
                continue;
            }
            let Some(ce) = self.engine_as_cached::<eng::CombatantEngine>(e.doc.id, &e.doc) else {
                continue;
            };
            let eng::CombatantKind::Actor { token_id, actor_id } = &ce.kind else {
                continue;
            };
            if *token_id == Some(token) {
                let access = self.ctx_access(ctx, world_defaults, &e.doc);
                return Some((e.doc.id, ce, access));
            }
            if by_actor.is_none() && resolved_actor.is_some() && *actor_id == resolved_actor {
                let access = self.ctx_access(ctx, world_defaults, &e.doc);
                by_actor = Some((e.doc.id, ce, access));
            }
        }
        by_actor
    }

    /// The combatant's formula-host DOCUMENT (cloned): resolved through
    /// `combat::eval::formula_host` over a two-entry map built from the ECS's
    /// own cached copies of the token (a scene entity) and its linked actor
    /// (`actors` table) — so the movement gate and the combat transitions
    /// share ONE host-precedence rule rather than two documented-to-agree
    /// copies.
    pub fn combatant_formula_host(&self, kind: &eng::CombatantKind) -> Option<Document> {
        let eng::CombatantKind::Actor { token_id, actor_id } = kind else {
            return None;
        };
        let mut hosts: HashMap<Uuid, Document> = HashMap::new();
        if let Some(tid) = token_id {
            if let Some(&e) = self.index.get(tid) {
                if let Ok(c) = self.world.get::<&SceneEntity>(e) {
                    hosts.insert(*tid, c.doc.clone());
                }
            }
        }
        if let Some(aid) = actor_id {
            if let Some(a) = self.actors.get(aid) {
                hosts.insert(*aid, a.clone());
            }
        }
        crate::combat::eval::formula_host(&hosts, kind).cloned()
    }

    /// The scene's real-world distance-per-cell scale (`SceneEngine.grid.distance.per_cell`), or
    /// `None` when the scene is absent or authors no distance scale — the caller (the movement-
    /// budget gate's `Interpretation::PerCell` conversion) must treat `None` as "cannot resolve
    /// the budget", never substitute a default: a fabricated scale would convert a resource
    /// budget into the wrong number of cells.
    pub(crate) fn scene_per_cell(&self, scene: Uuid) -> Option<f64> {
        let &e = self.index.get(&scene)?;
        let comp = self.world.get::<&SceneEntity>(e).ok()?;
        let scene_eng = self.engine_as_cached::<eng::SceneEngine>(scene, &comp.doc)?;
        scene_eng.grid.distance.map(|d| d.per_cell)
    }

    /// The access `ctx` holds on `doc`, resolved through the SAME `effective_owner_via` +
    /// `resolve_access_world` pair document egress uses (`filter_command`), with the grants
    /// projected from `doc`'s OWN `doc_type` so a caller cannot supply a mismatched set.
    ///
    /// Returns the `Access` rather than a verdict because egress asks TWO questions of it — whole
    /// document `cap::READ` and the per-property tier through `Access::can_see` — and resolving it
    /// twice is how the two answers drift apart.
    fn ctx_access(
        &self,
        ctx: &PermissionContext,
        world_defaults: &crate::data::document::WorldCapDefaults,
        doc: &Document,
    ) -> crate::data::permission::Access {
        let owner = crate::data::permission::effective_owner_via(doc, &|id: &Uuid| self.actor(id));
        crate::data::permission::resolve_access_world(
            ctx.user_id,
            ctx.world_role,
            doc,
            &world_defaults.grants_for(&doc.doc_type),
            owner,
        )
    }

    /// `engine_geometry_visible_to` against the access `ctx` holds on `doc`.
    ///
    /// `resolved_footprints` applies this one decision to EVERY document an entry discloses
    /// geometry from — the scene, the token, and the actor authoring that token's size — so no
    /// level is gated by a decision the others do not share.
    fn ctx_can_see_engine(
        &self,
        ctx: &PermissionContext,
        world_defaults: &crate::data::document::WorldCapDefaults,
        doc: &Document,
    ) -> bool {
        engine_geometry_visible_to(doc, &self.ctx_access(ctx, world_defaults, doc))
    }

    /// Whether `ctx` receives every band a token's footprint entry would disclose: the token
    /// document's own — `overrides.shape`/`overrides.size` live in it — and the document its
    /// `shape`/`size` are authored in, resolved through the same `token_geometry_source` join
    /// `token_shape_and_size` reads those values through.
    ///
    /// An embedded child's band is tested against the access resolved for the token it rides in,
    /// because that is how a child reaches a recipient at all: `filter_properties` recurses into
    /// `embedded` carrying the PARENT's access and applies each child's own overrides, and no
    /// whole-document `cap::READ` is ever resolved for a child.
    ///
    /// `false` for a token with no geometry source — a dangling `actor_id`, or an instanced token
    /// with no embedded actor. `token_shape_and_size` yields nothing to disclose for either, so
    /// the entry is absent regardless; refusing here states that rather than leaving it to a later
    /// step.
    fn token_footprint_visible(
        &self,
        ctx: &PermissionContext,
        world_defaults: &crate::data::document::WorldCapDefaults,
        token_doc: &Document,
    ) -> bool {
        let access = self.ctx_access(ctx, world_defaults, token_doc);
        if !engine_geometry_visible_to(token_doc, &access) {
            return false;
        }
        match self.token_geometry_source(token_doc) {
            Some(GeometrySource::Linked(actor)) => {
                self.ctx_can_see_engine(ctx, world_defaults, actor)
            }
            Some(GeometrySource::Embedded(child)) => engine_tier_visible_to(child, &access),
            None => false,
        }
    }

    /// The resolved drawn extents the `"footprints"` derived channel carries: for every scene with
    /// a resolvable grid that `ctx` may read, that scene's unit (1x1) extent plus one entry per
    /// token whose footprint this ECS resolves and `ctx` may read. Scene units throughout —
    /// `footprint::FootprintCells` is in grid units and is scaled here by the scene's own
    /// `grid.size` (the INDEXING scale, the circumradius on hex), the one conversion a footprint
    /// takes.
    ///
    /// Egress rule: an entry — a scene's as much as a token's — is included only when `ctx`
    /// receives the `/engine` geometry of every document that entry is computed from, both gates
    /// of it (`ctx_can_see_engine`, and `token_footprint_visible` for the token's actor join). The
    /// envelope IS the disclosure at both levels: a scene entry states that scene's id and its
    /// grid-derived unit geometry, so an entry with an empty `tokens` list is not a redaction of a
    /// scene the recipient may not see. A token parented to a withheld scene is withheld with it.
    ///
    /// A token with no entry has no server-resolved footprint: it carries no actor, its actor link
    /// dangles, or `ctx` does not receive the band sizing it. An entry with `extent: None` is a
    /// REFUSAL — the same `footprint::resolve_checked` refusal `resolve_token_footprint` returns
    /// `None` for.
    pub(crate) fn resolved_footprints(
        &self,
        ctx: &PermissionContext,
        world_defaults: &crate::data::document::WorldCapDefaults,
    ) -> footprint::FootprintsPayload {
        let mut by_scene: BTreeMap<Uuid, (f64, footprint::SceneFootprints)> = BTreeMap::new();
        // The cell size comes from `scene_grid_sizes` rather than a second `grid.size` read, so
        // this channel's scale can never disagree with the gates'; the entity scan alongside it
        // supplies the scene DOCUMENT that map does not carry, which the egress check needs.
        let grid_sizes = self.scene_grid_sizes();
        for e in self.world.query::<&SceneEntity>().iter() {
            let doc = &e.doc;
            if doc.doc_type != "scene" || !self.ctx_can_see_engine(ctx, world_defaults, doc) {
                continue;
            }
            let Some(cell) = grid_sizes.get(&doc.id).copied() else {
                continue;
            };
            let scene = doc.id;
            let kind = self.resolve_grid_kind(scene);
            let unit = footprint::resolve_footprint_cells(kind, "square", 1.0, 1.0);
            by_scene.insert(
                scene,
                (
                    cell,
                    footprint::SceneFootprints {
                        scene,
                        unit: footprint::FootprintExtent {
                            w: unit.box_w * cell,
                            h: unit.box_h * cell,
                        },
                        tokens: Vec::new(),
                    },
                ),
            );
        }
        // Sorted so the payload is a stable value: the egress loop's change detection compares
        // whole payloads, and `hecs` iteration order is not stable.
        let mut tokens: Vec<(Uuid, Uuid)> = Vec::new();
        for e in self.world.query::<&SceneEntity>().iter() {
            let doc = &e.doc;
            if doc.doc_type != "token" {
                continue;
            }
            let Some(scene) = doc.parent_id else {
                continue;
            };
            if !by_scene.contains_key(&scene) {
                continue;
            }
            if !self.token_footprint_visible(ctx, world_defaults, doc) {
                continue;
            }
            tokens.push((scene, doc.id));
        }
        tokens.sort_unstable();
        for (scene, token) in tokens {
            let Some((cell, entry)) = by_scene.get_mut(&scene) else {
                continue;
            };
            let Some((shape, size)) = self.token_shape_and_size(token) else {
                continue; // no actor to resolve a footprint from; the document's own w/h stand
            };
            let kind = self.resolve_grid_kind(scene);
            let extent = footprint::resolve_checked(kind, &shape, size.w, size.h)
                .ok()
                .map(|f| footprint::FootprintExtent {
                    w: f.box_w * *cell,
                    h: f.box_h * *cell,
                });
            entry
                .tokens
                .push(footprint::TokenFootprint { token, extent });
        }
        footprint::FootprintsPayload {
            scenes: by_scene.into_values().map(|(_, s)| s).collect(),
        }
    }

    /// Scene-shared lighting/wall inputs for the visibility mask. Computed once per scene per
    /// dispatch and reused for every vision source via `lighting_inputs`. `all_bright`
    /// short-circuits light raycasts under lighting-off or globalIllumination.
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
            self.light_wall_entries(scene)
        };
        let grid = self.resolve_grid_shape(scene, cell);
        Self::lighting_inputs_from(
            all_bright,
            lights,
            &light_walls,
            self.sight_wall_entries(scene),
            grid.world_extent(settings.bounds),
            cell,
            grid.world_units_per_cell(),
        )
    }

    /// Raycast step of `lighting_inputs`, split out so `visible_cells_cached` can gather the
    /// pre-raycast `lights`/`light_walls`/`sight_walls` (cheap: cached document decodes only, no
    /// geometry) to build its invalidation fingerprint WITHOUT paying for `lit_polys`' raycasts,
    /// then call this to do the raycast only on a fingerprint mismatch. `lighting_inputs` itself
    /// takes no such split: it always gathers then immediately raycasts in one call.
    ///
    /// `extent` is the scene's WORLD-unit envelope, produced by
    /// `GridShape::world_extent` from the scene's authored bounds — those are measured in grid
    /// units (cells), continuous, and must never reach `env_light_polys`, which measures against
    /// wall coordinates in world units.
    ///
    /// `world_units_per_cell` is `GridShape::world_units_per_cell` for the same scene, used ONLY to
    /// convert each light's authored (cell) radii into the world-unit reach `bound_for_reach` grows
    /// its occlusion polygon's bound to cover — it is NOT the indexing scale `cell`/`extent` carry,
    /// which differ from it on hex. Without this, a placed light's occlusion polygon is bounded by
    /// wall endpoints and `VISION_BOUND_MARGIN` alone, capping its reach independent of the radii the
    /// light was authored with.
    fn lighting_inputs_from(
        all_bright: bool,
        lights: Vec<lighting::Light>,
        light_walls: &[elevation::BandedWall],
        sight_walls: Vec<elevation::BandedWall>,
        extent: grid_shape::WorldExtent,
        cell: f64,
        world_units_per_cell: f64,
    ) -> LightingInputs {
        let wu = if world_units_per_cell.is_finite() && world_units_per_cell > 0.0 {
            world_units_per_cell
        } else {
            0.0
        };
        // Each light raycasts against the light walls whose elevation band covers the LIGHT's
        // own elevation (`wall_occludes`): a lamp above a wall's band shines over it.
        let lit_polys: Vec<Vec<vision::P>> = lights
            .iter()
            .map(|l| {
                let lw = elevation::walls_at_elevation(light_walls, l.elevation);
                let reach = [l.bright_radius, l.dim_radius]
                    .into_iter()
                    .filter(|r| r.is_finite() && *r > 0.0)
                    .fold(0.0_f64, f64::max)
                    * wu;
                let b = vision::bound_for_reach(l.pos, &lw, VISION_BOUND_MARGIN, reach);
                vision::visibility_polygon(l.pos, &lw, b)
            })
            .collect();
        // Boundary-projected environment occlusion. Empty under all_bright (env is not
        // the mechanism there). Environment ambient keeps the FULL light-wall set at every
        // elevation (it is sky-light; walls always shadow it, or daylight would flood
        // interiors) — the one place elevation does not filter occlusion.
        let env_polys = if all_bright {
            Vec::new()
        } else {
            let full: Vec<vision::Seg> = light_walls.iter().map(|(s, _)| *s).collect();
            lighting::env_light_polys(extent, cell, &full)
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
    ///
    /// `bands` is the caller-resolved gradation (`resolved_bands`), passed in so the sole
    /// production caller (`compute_derived`) resolves the gradation ONCE and the `vision`
    /// payload's `bands` array is the same resolution the mask's band indices were computed
    /// against — never a second read that could disagree.
    pub fn player_lit_mask(&self, user: Uuid, bands: &[Band]) -> Vec<LitScene> {
        // 0. Pre-resolve scene settings for every scene that has a token, so resolve_scene is
        //    called exactly once per scene rather than once per token. Collect
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
            // Source token's elevation: filters the sight-wall set (see-over/see-under).
            elevation: f64,
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
            let owns = self.token_effective_owner(&e.doc) == Some(user);
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
                    elevation: elevation::elevation_or_ground(t.elevation),
                    floors: self.token_vision_floors(&e.doc),
                });
            }
        }
        if sources.is_empty() {
            return Vec::new();
        }

        // 2. Per scene, accumulate visible cells across that scene's sources.
        let grid = self.scene_grid_sizes();
        use std::collections::BTreeMap;
        // (i, j) -> (best_level, band_index, tint, hint_floor, hint). hint_floor seeds NEG_INFINITY so the
        // first admitting mode always sets it; brightness (level/band/tint) and hint reduce independently.
        type CellEntry = BTreeMap<(i32, i32), (f64, usize, u32, f64, Option<String>)>;
        // scene -> (the scene's `cell` indexing scale, per-cell best)
        let mut per_scene: BTreeMap<Uuid, (f64, CellEntry)> = BTreeMap::new();

        // Distinct scenes among the sources.
        let mut scenes: Vec<Uuid> = sources.iter().map(|s| s.scene).collect();
        scenes.sort();
        scenes.dedup();

        for scene in scenes {
            // Use the memoized settings; fall back to resolve (unreachable in practice since
            // `scene_settings` was populated from every source scene, but keeps the code correct
            // if the map misses).
            let settings = match scene_settings.get(&scene) {
                Some(s) => s,
                None => continue,
            };
            // An absent entry means no scene document — skip rather than synthesize a grid.
            let Some(cell) = grid.get(&scene).copied() else {
                continue;
            };
            if cell <= 0.0 {
                continue;
            }
            let cell_grid = self.resolve_grid_shape(scene, cell);
            // One grid step's world distance, resolved once per scene: it is a property of the
            // shape, so every candidate cell of every source in this scene shares the value.
            let world_units_per_cell = cell_grid.world_units_per_cell();
            // Lighting inputs: under globalIllumination or lighting-off, every LOS cell is bright;
            // else compute per-cell from lights (occluded by blocksLight) + environment.
            let li = self.lighting_inputs(scene, settings, cell);

            let entry = per_scene
                .entry(scene)
                .or_insert_with(|| (cell, BTreeMap::new()));
            for src in sources.iter().filter(|s| s.scene == scene) {
                // LOS polygon for this source (or, LOS off, the whole bound box as a polygon),
                // raycast against the sight walls whose band covers the source's elevation.
                let src_walls = elevation::walls_at_elevation(&li.sight_walls, src.elevation);
                let poly = source_los_poly(
                    src.vp,
                    &src_walls,
                    settings.los_restriction,
                    cell_grid.world_extent(settings.bounds),
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
                // Strict mode: this scan must produce the exact box `accumulate_visible_cells`'s
                // own strict call scans for the same source (`cell_visible`'s doc states that
                // parity as an invariant) — `scan_box_for` is what makes the two calls agree.
                let bbox = ((minx, miny), (maxx, maxy));
                let (scan_min, scan_max) = crate::scene::explored::scan_box_for(
                    cell_grid.as_ref(),
                    src.vp,
                    bbox,
                    cell,
                    crate::scene::explored::MAX_CELLS_PER_POLYGON,
                    crate::scene::explored::ScanMode::Strict,
                );
                let candidates = match cell_grid.cells_in_bounds(
                    scan_min,
                    scan_max,
                    cell,
                    crate::scene::explored::MAX_CELLS_PER_POLYGON,
                ) {
                    Some(c) => c,
                    None => {
                        tracing::warn!("lit mask cell scan degenerate; skipping source");
                        continue;
                    }
                };
                for (i, j) in candidates {
                    let (cx, cy) = cell_grid.cell_center((i, j));
                    if !crate::scene::vision::point_in_poly(&poly, (cx, cy)) {
                        continue;
                    }
                    // Lighting OFF ⇒ all-bright untinted; globalIllumination ⇒
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
                            world_units_per_cell,
                        )
                    };
                    // Both a light's radii and a vision mode's range are authored in cells, so
                    // each measures against the shape's per-cell world distance, never its
                    // indexing scale — the two coincide on square and differ by √3 on hex.
                    let dist_cells = (((cx - src.vp.0).powi(2) + (cy - src.vp.1).powi(2)).sqrt())
                        / world_units_per_cell;
                    // Lowest applicable floor decides visibility; highest applicable floor decides the hint.
                    // `cell_visible` computes the same min-floor-over-in-range-modes decision
                    // and is reused verbatim by the movement gate (anti-drift).
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
                                || (*fmin == admit_floor && admit_hint.is_some() && hint.is_none());
                            if take {
                                admit_floor = *fmin;
                                admit_hint = hint.clone();
                            }
                        }
                    }
                    if cell_visible(&src.floors, cl.level, dist_cells) {
                        let band = crate::scene::lighting::band_index(bands, cl.level);
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
                            || (admit_floor == slot.3 && slot.4.is_some() && admit_hint.is_none())
                        {
                            slot.3 = admit_floor;
                            slot.4 = admit_hint;
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
    /// the secrecy mask. `lenient` selects the rasterization rule: strict samples the
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
        // An absent entry means no scene document — refuse rather than synthesize a grid.
        let Some(cell) = self.scene_grid_sizes().get(&scene).copied() else {
            return out;
        };
        if cell <= 0.0 {
            return out;
        }

        let sources = self.gather_vision_sources_in_scene(user, scene, &settings);
        if sources.is_empty() {
            return out;
        }

        // Scene-shared lighting inputs (once), then per-source per-cell test.
        let li = self.lighting_inputs(scene, &settings, cell);
        let grid = self.resolve_grid_shape(scene, cell);
        accumulate_visible_cells(&mut out, &sources, &settings, cell, &li, lenient, &*grid);
        out
    }

    /// Cached variant of `visible_cells` for the movement gate (the ONLY intended caller —
    /// `visible_cells` itself and every other existing caller, incl. the pathfinder and the
    /// parity tests, are UNCHANGED and keep calling the uncached primitive). Reuses the mask from
    /// a prior call for the same `(user, scene)` only when a freshly rebuilt
    /// `VisibilityInputsSnapshot` — built from the SAME `gather_vision_sources_in_scene` call and
    /// the SAME raw `resolve_scene`/`scene_grid_sizes`/`scene_lights` and banded wall-collector
    /// (`sight_wall_entries`/`light_wall_entries`)
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
        // An absent entry means no scene document — refuse rather than synthesize a grid.
        let Some(cell) = self.scene_grid_sizes().get(&scene).copied() else {
            return BTreeSet::new();
        };
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
            self.light_wall_entries(scene)
        };
        let sight_walls = self.sight_wall_entries(scene);

        let snapshot = VisibilityInputsSnapshot {
            lenient,
            settings: settings.clone(),
            cell,
            sources: sources
                .iter()
                .map(|s| (s.id, s.vp, s.elevation, s.floors.clone()))
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

        let grid = self.resolve_grid_shape(scene, cell);
        let li = Self::lighting_inputs_from(
            all_bright,
            lights,
            &light_walls,
            sight_walls,
            grid.world_extent(settings.bounds),
            cell,
            grid.world_units_per_cell(),
        );
        let mut mask = BTreeSet::new();
        accumulate_visible_cells(&mut mask, &sources, &settings, cell, &li, lenient, &*grid);

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
            let owns = self.token_effective_owner(&e.doc) == Some(user);
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
                    elevation: elevation::elevation_or_ground(t.elevation),
                    floors: self.token_vision_floors(&e.doc),
                });
            }
        }
        sources
    }

    /// Engine-owned movement collision. True if the move segment `a0→a1` crosses any `blocksMove`
    /// wall in `scene`. A no-op move (`a0 == a1`) never blocks.
    ///
    /// This is the REFERENCE implementation of wall-crossing semantics — one home for it, per the
    /// module's own INVARIANT on `move_walls`. `move_exec::execute_move`'s per-cell wall gate is
    /// the production traversal path and does not call this function directly (it composes
    /// `move_walls(scene, None)` with `segments_cross` inline instead); an anti-drift test pins
    /// the two to agreement, so a change to either wall filter that drifts them apart fails it.
    /// Test-only: it has no production caller, so it compiles only into test builds.
    #[cfg(test)]
    pub(crate) fn blocks_move(&self, scene: Uuid, a0: (f64, f64), a1: (f64, f64)) -> bool {
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
/// under lighting-off or globalIllumination.
pub(crate) struct LightingInputs {
    /// Skip per-light raycasts: lighting off or `GlobalIllumination`.
    pub(crate) all_bright: bool,
    /// Resolved scene lights (empty under `all_bright`).
    pub(crate) lights: Vec<lighting::Light>,
    /// Per-light visibility polygons, index-aligned with `lights` (built by mapping over it, so
    /// the lengths always agree). `visibility_polygon` unions the raycast bound's own edges into
    /// the occluder set, so a non-degenerate bound always yields a non-empty polygon; an EMPTY
    /// entry arises only from degenerate (non-finite) light positions, and `cell_illumination`
    /// reads an empty polygon as "no occluder computed" — never occludes. That fail-open is
    /// inert on this path: a position degenerate enough to empty the polygon also makes the
    /// per-cell distance non-finite, which `cell_illumination` zeroes per source.
    pub(crate) lit_polys: Vec<Vec<vision::P>>,
    /// Scene-boundary visibility polygons occluding the environment ambient (`env_light_polys`).
    /// Empty under `all_bright` (env is not the mechanism there — every LOS cell is forced bright).
    pub(crate) env_polys: Vec<Vec<vision::P>>,
    /// `blocksSight` wall segments with their elevation bands (the LOS raycast input —
    /// each vision source filters them at its own elevation through
    /// `elevation::walls_at_elevation` before raycasting).
    pub(crate) sight_walls: Vec<elevation::BandedWall>,
}

/// Whether a single sample `point` (already known to lie inside the LOS polygon) qualifies a
/// cell as visible. Computes illumination (mirroring `player_lit_mask`'s all_bright arm exactly)
/// and delegates to `cell_visible`. This is the ONE canonical place the per-point illumination +
/// floor decision is made, shared by all three sampling arms of `visible_cells` (lenient-center,
/// lenient-corner, strict-center) to prevent the gate-vs-egress drift hazard: if the decision logic
/// were inlined separately in each arm, a future edit could silently fork the gate mask from the
/// egress mask.
///
/// INVARIANT: the all_bright tint expression `if lighting_enabled {env_color} else {0}`
/// must stay identical to `player_lit_mask`'s copy. `cell_visible` reads only `level` today, but
/// tint is passed through so the two masks can never structurally diverge even if tint gains
/// semantics later.
///
/// `world_units_per_cell` is the shape-derived world distance of one grid step
/// (`GridShape::world_units_per_cell`), NOT the cell indexing scale. Both quantities it feeds — a
/// light's radii through `cell_illumination`, and the vision range this function's own
/// `dist_cells` is compared against — are authored in cells, so both convert through it; the two
/// scalars coincide on square and differ by √3 on hex.
fn point_qualifies(
    point: (f64, f64),
    src_vp: (f64, f64),
    floors: &[(f64, f64, Option<String>)],
    settings: &ResolvedScene,
    li: &LightingInputs,
    world_units_per_cell: f64,
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
            world_units_per_cell,
        )
    };
    let dist_cells = (((point.0 - src_vp.0).powi(2) + (point.1 - src_vp.1).powi(2)).sqrt())
        / world_units_per_cell;
    cell_visible(floors, cl.level, dist_cells)
}

/// One vision source gathered by `gather_vision_sources_in_scene`: an owned or observer-tier
/// token's viewpoint + resolved vision floors. `id` is carried only for `visible_cells_cached`'s
/// deterministic snapshot ordering — `visible_cells` itself never reads it.
struct VisSrc {
    /// Source token id (snapshot ordering only; see the struct doc).
    id: Uuid,
    /// Viewpoint in scene units.
    vp: vision::P,
    /// The source token's elevation (0 = grounded): filters the sight-wall set through
    /// `elevation::wall_occludes` and grounds tremorsense (`SceneEcs::player_perceived_tokens`).
    elevation: f64,
    /// Resolved vision floors: `(illumination floor, range cells, render hint)`.
    floors: Vec<(f64, f64, Option<String>)>,
}

/// One `sources` entry in `VisibilityInputsSnapshot`: `(token id, viewpoint, elevation, floors)`.
/// Elevation is part of the fingerprint: a token gaining/losing height changes which walls
/// occlude it, so the same walls at two elevations must never share a cached mask.
type VisSrcSnapshot = (Uuid, vision::P, f64, Vec<(f64, f64, Option<String>)>);

/// Fingerprint of every input `visible_cells`'s computation reads for one `(user, scene,
/// lenient)` call, used by `visible_cells_cached` to decide whether a prior mask may be reused.
/// Built from the SAME calls the real computation makes (`gather_vision_sources_in_scene`,
/// `resolve_scene`, `scene_grid_sizes`, `scene_lights`, and the banded wall collectors
/// `sight_wall_entries`/`light_wall_entries` — wall geometry, block flags AND elevation bands) —
/// not a
/// separately-derived "things that might matter" list — so completeness reduces to "does this
/// struct hold every field `accumulate_visible_cells`/`gather_vision_sources_in_scene` read",
/// which is directly checkable by inspection, rather than "were all mutation call sites
/// enumerated", which `engine_cache`'s `CachedEngine` already proved is an open, unboundable
/// question for this codebase (`apply_op` is not the sole mutation chokepoint). Any change to
/// what these fields hold — a token moving/changing elevation/gaining-or-losing source status,
/// a wall's blocksSight/blocksLight/geometry/elevation-band changing, a light being
/// added/moved/toggled (its `elevation` rides `lights`), a vision-mode or
/// gradation band definition changing (both flow into `sources`' `floors` via
/// `token_vision_floors`), a linked actor's vision assignment changing (same path), the scene's
/// own grid size or vision/lighting overrides changing, or world-settings' `observerVision`/
/// `losRestriction`/lighting defaults changing — is captured because it necessarily changes the
/// value of one of these fields, making the snapshot compare unequal.
#[derive(Clone, PartialEq)]
struct VisibilityInputsSnapshot {
    /// The sampling mode the mask was computed under.
    lenient: bool,
    /// The resolved scene settings the computation read.
    settings: ResolvedScene,
    /// Grid cell size in scene units.
    cell: f64,
    /// Every vision source's `(id, viewpoint, floors)` snapshot.
    sources: Vec<VisSrcSnapshot>,
    /// Resolved scene lights.
    lights: Vec<lighting::Light>,
    /// `blocksLight` wall segments with their elevation bands.
    light_walls: Vec<elevation::BandedWall>,
    /// `blocksSight` wall segments with their elevation bands.
    sight_walls: Vec<elevation::BandedWall>,
}

/// `visible_cells_cache`'s per-entry value: the snapshot it was computed from, paired with the
/// mask itself.
type VisibleCellsCacheEntry = (
    VisibilityInputsSnapshot,
    std::collections::BTreeSet<(i32, i32)>,
);

/// The per-source LOS raycast + per-cell scan shared by `visible_cells` and
/// `visible_cells_cached` on a cache miss — the sole implementation of the expensive half of the
/// computation, so both entry points share identical behavior.
fn accumulate_visible_cells(
    out: &mut std::collections::BTreeSet<(i32, i32)>,
    sources: &[VisSrc],
    settings: &ResolvedScene,
    cell: f64,
    li: &LightingInputs,
    lenient: bool,
    grid: &dyn grid_shape::GridShape,
) {
    // One grid step's world distance, resolved once: it is a property of the shape, so every
    // sample of every candidate cell of every source shares the value.
    let world_units_per_cell = grid.world_units_per_cell();
    for src in sources {
        let src_walls = elevation::walls_at_elevation(&li.sight_walls, src.elevation);
        let poly = source_los_poly(
            src.vp,
            &src_walls,
            settings.los_restriction,
            grid.world_extent(settings.bounds),
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
        // Lenient samples corners, so a cell just outside the center-bbox can still qualify: this
        // invocation's mode (whichever `lenient` selects) decides how much this call's OWN box is
        // padded; `scan_box_for` derives both that pad and the (always fully-padded) clamp
        // decision from the same binding, so a strict and a lenient call over the same source's
        // bbox always meet an identical window.
        let bbox = ((minx, miny), (maxx, maxy));
        let mode = if lenient {
            crate::scene::explored::ScanMode::Lenient
        } else {
            crate::scene::explored::ScanMode::Strict
        };
        let (min, max) = crate::scene::explored::scan_box_for(
            grid,
            src.vp,
            bbox,
            cell,
            crate::scene::explored::MAX_CELLS_PER_POLYGON,
            mode,
        );
        let candidates = match grid.cells_in_bounds(
            min,
            max,
            cell,
            crate::scene::explored::MAX_CELLS_PER_POLYGON,
        ) {
            Some(c) => c,
            None => {
                tracing::warn!("visible_cells scan degenerate; skipping source");
                continue;
            }
        };
        for (i, j) in candidates {
            if out.contains(&(i, j)) {
                continue;
            }
            // Strict: center only. Lenient: center first (so strict cells are always
            // included), then corners if center fails — a cell whose polygon merely clips
            // a corner still qualifies under leniency.
            let center = grid.cell_center((i, j));
            let mut found = false;
            if lenient {
                // Check center first, then corners. `cell_vertices` (the 4 square corners in
                // byte-identical order, or the 6 pointy-top hex vertices) is computed ONLY on this
                // path — the strict movement-gate mask never pays for it (6 sin/cos per hex cell).
                if vision::point_in_poly(&poly, center)
                    && point_qualifies(
                        center,
                        src.vp,
                        &src.floors,
                        settings,
                        li,
                        world_units_per_cell,
                    )
                {
                    found = true;
                }
                if !found {
                    let corners = grid.cell_vertices((i, j), cell);
                    for &corner in &corners {
                        if vision::point_in_poly(&poly, corner)
                            && point_qualifies(
                                corner,
                                src.vp,
                                &src.floors,
                                settings,
                                li,
                                world_units_per_cell,
                            )
                        {
                            found = true;
                            break;
                        }
                    }
                }
            } else {
                // Strict: center only (mirrors player_lit_mask exactly).
                if vision::point_in_poly(&poly, center)
                    && point_qualifies(
                        center,
                        src.vp,
                        &src.floors,
                        settings,
                        li,
                        world_units_per_cell,
                    )
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

/// Per-cell visibility decision shared by `player_lit_mask` (egress/secrecy gate) and
/// `visible_cells` (movement gate). INVARIANT: identical for both so the move gate never
/// forbids a shipped-visible cell nor permits an unshipped one. A cell is visible iff
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
/// is on, else the whole bound box as a rectangle (whole-scene visible). Source: raycast
/// (`vision::visibility_polygon`). `scene_extent` is the scene's WORLD-unit envelope
/// (`GridShape::world_extent` of the authored grid-unit bounds), unioned into the wall-derived
/// bound so a wall-less (or sparsely-walled) scene reveals its own full authored extent instead of
/// a degenerate `viewpoint±VISION_BOUND_MARGIN` box — the same `vision::bound_for_scene` the
/// `player_vision_polygons`/`player_vision_inputs` paths apply, generalized to this shared source
/// (feeds both `player_lit_mask` and `visible_cells`/`visible_cells_cached`, never a forked bound
/// computation).
fn source_los_poly(
    vp: vision::P,
    sight_walls: &[vision::Seg],
    los_restriction: bool,
    scene_extent: grid_shape::WorldExtent,
) -> Vec<vision::P> {
    let b = vision::bound_for_scene(vp, sight_walls, scene_extent, VISION_BOUND_MARGIN);
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
/// accepted so vision and footprints can derive per recipient; the identity
/// payload is non-sensitive and global. `world_defaults` supplies the same
/// world-level capability grants document egress resolves READ against, so the
/// footprints channel cannot disclose a token the recipient's own document
/// stream withholds.
pub fn compute_derived(
    channel: &str,
    ecs: &SceneEcs,
    ctx: &PermissionContext,
    world_defaults: &crate::data::document::WorldCapDefaults,
) -> Option<serde_json::Value> {
    match channel {
        // Debug seam proof (non-sensitive, global); absent in release.
        #[cfg(debug_assertions)]
        "identity" => Some(serde_json::json!({ "entity_count": ecs.entity_count() })),
        // The resolved drawn footprint of every readable token, so the client renders and
        // hit-tests the authoritative geometry instead of re-deriving it from a second formula.
        "footprints" => serde_json::to_value(ecs.resolved_footprints(ctx, world_defaults)).ok(),
        // Per-player vision: the GM sees all; a player gets ONLY their own visibility
        // polygons, per-recipient. A token-less player gets empty polygons → full fog (the
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
                // The secrecy-safe lighting-aware mask — only currently-visible cells, each
                // tagged with its illumination band + tint. Carries the resolved gradation `bands`
                // so the client maps band indices → treatment. Additive: `polygons`/`explored` are
                // unchanged (the client consumes `lit` alongside them).
                // `renderHints` is a deterministic string table (first-seen order over the
                // BTreeMap-ordered mask); each cell emits 5 ints: [i,j,band,tint,hint_idx] where
                // hint_idx is the index into `renderHints`, or -1 for None.
                // The gradation is resolved ONCE here and passed into the mask computation, so
                // the payload's `bands` array and the mask's band indices are the same
                // resolution by construction.
                let bands = ecs.resolved_bands();
                let bands_json: Vec<serde_json::Value> = bands
                    .iter()
                    .map(|b| serde_json::json!({ "name": b.name, "min": b.min_illumination }))
                    .collect();
                // Build the hint table and 5-int cell packing in a plain loop to avoid a
                // mutable borrow of `hints` inside a closure/flat_map borrow conflict.
                let mask = ecs.player_lit_mask(ctx.user_id, &bands);
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
mod tests;
