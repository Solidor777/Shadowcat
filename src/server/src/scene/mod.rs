//! Per-world derived scene ECS. Hydrated from documents; never persisted,
//! never authoritative. Holds one hecs entity per scene-entity document so
//! engine-owned systems (vision, pathfinding) can query spatial state.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

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
// Keeping the two distinct avoids widening this file's already-declared public enum surface.
use crate::data::engine as eng;
use crate::data::membership::PermissionContext;
use crate::scene::lighting::Band;

/// Resolved per-scene lighting mode. The client's wire twin is generated from
/// `eng::LightMode` (see the module-header alias note).
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
/// `ResolvedSceneSettings`; pathfinding/animation fields are resolved in later checkpoints).
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
#[derive(Clone, Debug)]
pub struct VisionMode {
    /// Minimum illumination band name the mode can see under.
    pub illumination_floor: String,
    /// Default vision range in cells (used when a token authors none).
    pub default_range: f64,
    /// Client render treatment (e.g. `"desaturate"`); `None` = plain.
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

/// Wire (`eng::LightMode`) → resolved bridge; see the module-header alias note.
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
/// Holds the full `blocksSight` wall set and the visibility polygons for every stationary
/// owned token (all owned tokens in the scene except the moving one). Computed once per move
/// via `SceneEcs::player_vision_inputs`; each sample then calls the cheaper `polygons_at`
/// (one moving-token raycast only, no repeated O(entities) ECS or wall scan).
pub(crate) struct VisionMoveInputs {
    /// Full `blocksSight` wall set (includes `gm_only` walls — full-wall-set invariant).
    walls: Vec<vision::Seg>,
    /// Vision polygons for every owned token in the scene EXCEPT the moving token, at their
    /// committed (stationary) positions. Constant across all samples of one move.
    static_polys: Vec<Vec<vision::P>>,
    /// The scene's own WORLD-unit rectangle (`SceneEcs::scene_world_extent`) — so `polygons_at`'s
    /// per-sample bound stays scene-extent-aware identically to `player_vision_polygons` (no
    /// fork). Never the raw authored bounds, which are measured in grid units (cells),
    /// continuous — never world units, and not required to be integral.
    scene_extent: (f64, f64),
    /// True when the user owns no token in this scene: `polygons_at` returns empty (fail-closed).
    empty: bool,
}

impl VisionMoveInputs {
    /// Per-sample: compute the moving token's visibility polygon at `viewpoint` and prepend it
    /// to the precomputed static polygons. Returns empty when `empty == true` (no owned token
    /// in this scene — fail-closed). Uses the same `sight_walls` set and raycast primitives as
    /// `player_vision_polygons` (full-wall-set invariant; no fork).
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
    /// The `light-gradation` singleton config-doc, or `None` (built-in bands).
    gradation: Option<Document>,
    /// The `vision-modes` singleton config-doc, or `None` (seed modes).
    vision_modes: Option<Document>,
    /// Point-lookup table keyed by actor doc id. Used only for `actors.get(id)` joins; must
    /// not be iterated for ordered or wire output (HashMap iteration order is non-deterministic).
    actors: HashMap<Uuid, Document>,
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
    /// `visible_cells_cached`'s per-`(user, scene)` mask cache for the movement gate.
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

/// Whether the changes being mirrored have cleared the authoritative write path.
/// A failed pointer op means something categorically different on each side, so the
/// two are never reported at the same level.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MirrorInput {
    /// Already committed and broadcast (`apply_op`). BOTH authoritative loops —
    /// `apply_intent` AND `apply_command` (the trusted chat/settings seeding path,
    /// which does NOT run `validate_field_change`) — apply the same change through
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

/// The single authority for the `/engine`-tier per-requester visibility decision: can `viewer`
/// see `doc`'s `/engine` property override? `viewer: None` is the AUTHORITATIVE caller (a GM, or
/// the execution path) and always sees everything — `true` unconditionally. `viewer: Some(user)`
/// resolves the tier declared at `permissions.property_overrides["/engine"]` (default `All` when
/// absent) against `user`'s access, via `permission::resolve_access` + `effective_owner(doc,
/// None)` — the no-actor-join form, exact for any doc type that never carries an actor link
/// (wall, region). `move_walls` and `region_field` both call this rather than keep a private
/// copy: two paths that must agree on the same decision share one symbol rather than each keeping
/// its own copy (anti-fork). Do not re-inline this at a new call site.
fn engine_tier_visible(doc: &Document, viewer: Option<Uuid>) -> bool {
    let Some(user) = viewer else {
        return true;
    };
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
        crate::data::permission::effective_owner(doc, None),
    );
    access.can_see(tier)
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

/// The footprint radius used when no effective actor resolves. Mirrors the client's
/// `resolveFootprint` fallback.
/// PARITY-BOUND, not a fail-closed choice: it is more permissive than a 1×1 square's 0.707, and
/// changing it here without changing the client re-forks the router and the gate. Change both or
/// neither.
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
                Self::apply_config_update(&mut self.gradation, *doc_id, changes);
                Self::apply_config_update(&mut self.vision_modes, *doc_id, changes);
                if let Some(a) = self.actors.get_mut(doc_id) {
                    // Same store-equal mutation rule: an actor's `/owner` is an authz
                    // input for every token linked to it, so a forked `remove` here
                    // re-owns tokens the store considers unowned.
                    reapply_changes(a, changes);
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
    /// `engine` to be a complete, `deny_unknown_fields`-checked `WorldSettingsEngine` — this
    /// enforces, at write time, the same `scene`+`pathfinding`+`animation`-all-present structural
    /// completeness the TS mirror (`ws?.scene && ws?.pathfinding && ws?.animation`) still checks
    /// at read time. A doc that never passed that ingress gate (e.g. a
    /// test fixture built directly) falls back to built-in
    /// defaults. Used by every resolver that reads world-settings so partial/
    /// malformed-doc handling stays consistent across all of them.
    fn validated_world_settings_engine(&self) -> Option<eng::WorldSettingsEngine> {
        let doc = self.world_settings.as_ref()?;
        self.engine_as_cached::<eng::WorldSettingsEngine>(doc.id, doc)
    }

    /// Resolve a scene's effective lighting/vision settings: built-in defaults < world-settings doc
    /// < per-scene override. Fail-closed and `null ⇒ inherit` (mirrors `resolveSceneSettings`).
    pub fn resolve_scene(&self, scene: Uuid) -> ResolvedScene {
        // World layer: `validated_world_settings_engine` already enforces the
        // scene+pathfinding+animation-all-present structural guard at write time (ingress),
        // so a `None` here means the same "fall back to built-ins" case this guard covers.
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
    /// overrides only vision/lighting/grid — parent §5.2). Reads `world-settings.pathfinding.diagonalRule`.
    /// Uses `validated_world_settings_engine` so a structurally incomplete/absent doc falls back to
    /// `Chebyshev`, consistent with `resolve_scene`'s handling of the same partial-doc case.
    pub(crate) fn resolved_diagonal_rule(&self) -> pathfinding::DiagonalRule {
        self.validated_world_settings_engine()
            .map(|w| conv_diagonal_rule(w.pathfinding.diagonal_rule))
            .unwrap_or(pathfinding::DiagonalRule::Chebyshev)
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
    pub(crate) fn scene_world_extent(&self, scene: Uuid) -> (f64, f64) {
        self.world_extent_from(&self.scene_grid_sizes(), scene)
    }

    /// The vision paths' REFUSAL policy over `scene_world_extent_at`: the conversion against an
    /// ALREADY-READ `scene_grid_sizes` map, substituting the zero rectangle for a scene that map
    /// does not carry. `scene_world_extent` and `player_vision_polygons`' per-scene memo both
    /// reach the conversion through this, so the two cannot drift into disagreeing about either
    /// the extent or what an absent scene means.
    ///
    /// `(0.0, 0.0)` when `grid_sizes` has no entry for the scene: it carries one for every live
    /// scene, so an absent entry means the scene is gone and no extent may be synthesised. A zero
    /// extent contributes nothing to `vision::bound_for_scene`'s union, leaving the wall-derived
    /// bound — the under-reveal direction. `navmesh_for` shares the conversion but NOT this
    /// policy: it refuses with `None`, because a navmesh cannot be triangulated over a rectangle
    /// that contributes nothing.
    fn world_extent_from(
        &self,
        grid_sizes: &std::collections::HashMap<Uuid, f64>,
        scene: Uuid,
    ) -> (f64, f64) {
        grid_sizes
            .get(&scene)
            .copied()
            .map_or((0.0, 0.0), |cell| self.scene_world_extent_at(scene, cell))
    }

    /// The conversion itself, and its ONLY expression: `scene`'s authored bounds through its own
    /// resolved `GridShape`, at a grid size the caller has already resolved. Refuses nothing —
    /// the caller that looked `cell` up owns the policy for a scene that has none, and the two
    /// policies genuinely differ (`world_extent_from` substitutes the zero rectangle every extent
    /// guard already refuses; `navmesh_for` returns `None`).
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
    fn scene_world_extent_at(&self, scene: Uuid, cell: f64) -> (f64, f64) {
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

    /// Resolved vision-mode registry. Returns a `BTreeMap` for deterministic key order
    /// (`.get(id)` works identically for callers).
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
                // Mirrors the client's `SEED_VISION_MODES`: normal has no hint;
                // darkvision desaturates.
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
        let mut viewpoints: Vec<(Uuid, vision::P)> = Vec::new();
        for e in self.world.query::<&SceneEntity>().iter() {
            if e.doc.doc_type != "token" || self.token_effective_owner(&e.doc) != Some(user_id) {
                continue;
            }
            if let (Some(t), Some(scene)) = (
                self.engine_as_cached::<eng::TokenEngine>(e.doc.id, &e.doc),
                e.doc.parent_id,
            ) {
                viewpoints.push((scene, (t.x, t.y)));
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
        let mut extents: std::collections::HashMap<Uuid, (f64, f64)> =
            std::collections::HashMap::new();
        let mut out = Vec::with_capacity(viewpoints.len());
        for (scene, vp) in viewpoints {
            let walls = self.sight_walls(scene);
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
        // Collect static-token viewpoints (non-moving owned tokens in `scene`). Drop the query
        // borrow before wall queries — mirrors player_vision_polygons collect-then-query order.
        let mut static_vps: Vec<vision::P> = Vec::new();
        let mut has_owned = false;
        for e in self.world.query::<&SceneEntity>().iter() {
            if e.doc.doc_type != "token"
                || e.doc.parent_id != Some(scene)
                || self.token_effective_owner(&e.doc) != Some(user)
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
        let scene_extent = self.scene_world_extent(scene);
        if !has_owned {
            return VisionMoveInputs {
                walls: Vec::new(),
                static_polys: Vec::new(),
                scene_extent,
                empty: true,
            };
        }
        // Full wall set: computed once for the entire move (same as player_vision_polygons).
        let walls = self.sight_walls(scene);
        // Static polygons: one per stationary owned token; constant across all samples.
        let static_polys = static_vps
            .iter()
            .map(|&vp| {
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
        // The footprint radius is authored against the INDEXING scale (a square block's
        // half-diagonal in cells), not the per-cell world distance — see
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
        // Hoisted above the engine dispatch so BOTH engines receive the SAME slice — never a forked
        // wall computation (the same discipline `mask` follows below).
        let walls = self.move_walls(scene, if is_gm { None } else { Some(user) });
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
                        },
                    )?;
                    // `find` reports cost in CELLS; the continuous engine reports SCENE UNITS
                    // (parity with the polyanya path below, which measures Euclidean length).
                    // The conversion is the shape's own per-cell world distance, not the cell
                    // size: on hex those differ by the √3 factor between a hex's circumradius
                    // and the distance to its neighbours.
                    let weighted = pathfinding::PathOutcome {
                        cost: weighted.cost * grid_shape.world_units_per_cell(),
                        ..weighted
                    };
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
                    Ok(navmesh::truncate_at_arrest(
                        clipped,
                        &regions,
                        cell,
                        &*grid_shape,
                    ))
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
    /// (fail-closed: it contributes no vision floor). Always returns ≥1 triple (normal fallback
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

    /// A token's effective `(shape, size)`, joined through the SAME actor precedence
    /// `token_vision_floors` implements: a LINKED token (`actor_id` present) resolves the shared
    /// actor and applies `overrides.shape`/`overrides.size` (each independently, per-field) over
    /// the actor's own value; a dangling link yields `None` (overrides ignored, mirroring
    /// `resolveTokenActor`); an INSTANCED token (no `actor_id`) reads its embedded copy through the
    /// deliberately-uncached direct `engine_as` path — an embedded actor's own `id` differs from
    /// the token's, so caching under either key would go stale on an `/embedded/actor/0/...` write.
    fn token_shape_and_size(&self, token: Uuid) -> Option<(String, eng::Size)> {
        let &e = self.index.get(&token)?;
        let tok = self.world.get::<&SceneEntity>(e).ok()?;
        let doc = &tok.doc;
        let token_eng = self.engine_as_cached::<eng::TokenEngine>(token, doc);

        match token_eng.as_ref().and_then(|t| t.actor_id) {
            Some(id) => {
                let actor = self.actors.get(&id)?; // dangling link → None (overrides ignored)
                let actor_eng = self.engine_as_cached::<eng::ActorEngine>(actor.id, actor)?;
                let overrides = token_eng.as_ref().and_then(|t| t.overrides.as_ref());
                let shape = overrides
                    .and_then(|o| o.shape.clone())
                    .unwrap_or(actor_eng.shape);
                let size = overrides.and_then(|o| o.size).unwrap_or(actor_eng.size);
                Some((shape, size))
            }
            None => doc
                .embedded
                .get("actor")
                .and_then(|v| v.first())
                .and_then(engine_as::<eng::ActorEngine>)
                .map(|a| (a.shape, a.size)),
        }
    }

    /// A token's bounding-disc radius in GRID UNITS (cells). Mirrors the client's `footprintRadius`
    /// formula: a circle uses `max(w,h)/2`, any other shape
    /// its half-diagonal `hypot(w,h)/2` (conservative enclosure). Effective-actor resolution
    /// mirrors `resolveTokenActor` via the SAME join `token_vision_floors` implements: a LINKED
    /// token resolves the shared actor and applies the per-token override whitelist; a dangling
    /// link ignores overrides; an INSTANCED token uses its embedded copy and overrides do not
    /// apply.
    ///
    /// `None` means REFUSE — the derived radius is outside `[0, MAX_FOOTPRINT_CELLS]`, or the
    /// stored size is degenerate. Callers must fail closed, never substitute a default: clamping an
    /// oversized token to the bound would route and gate it as a smaller disc, letting it enter
    /// gaps its real footprint cannot (a geometric fail-open).
    ///
    /// DELIBERATE DIVERGENCE from the client on degenerate input: the client's `footprintRadius`
    /// has no finite/sign guard and propagates `NaN` (rejected later by `find`'s range check),
    /// whereas this refuses. Both fail closed; only the mechanism differs.
    pub(crate) fn resolve_token_footprint(&self, token: Uuid) -> Option<f64> {
        let Some((shape, size)) = self.token_shape_and_size(token) else {
            return Some(DEFAULT_FOOTPRINT_RADIUS_CELLS);
        };
        let (w, h) = (size.w, size.h);
        if !w.is_finite() || !h.is_finite() || w < 0.0 || h < 0.0 {
            tracing::warn!(
                ?token,
                w,
                h,
                "token size is degenerate; refusing a footprint"
            );
            return None;
        }
        let r = if shape == "circle" {
            w.max(h) / 2.0
        } else {
            w.hypot(h) / 2.0
        };
        if !(0.0..=pathfinding::MAX_FOOTPRINT_CELLS).contains(&r) {
            tracing::warn!(
                ?token,
                r,
                "token footprint exceeds MAX_FOOTPRINT_CELLS; refusing"
            );
            return None;
        }
        Some(r)
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
            self.light_walls(scene)
        };
        Self::lighting_inputs_from(
            all_bright,
            lights,
            &light_walls,
            self.sight_walls(scene),
            self.resolve_grid_shape(scene, cell)
                .world_extent(settings.bounds),
            cell,
        )
    }

    /// Raycast step of `lighting_inputs`, split out so `visible_cells_cached` can gather the
    /// pre-raycast `lights`/`light_walls`/`sight_walls` (cheap: cached document decodes only, no
    /// geometry) to build its invalidation fingerprint WITHOUT paying for `lit_polys`' raycasts,
    /// then call this to do the raycast only on a fingerprint mismatch. `lighting_inputs` itself
    /// is unchanged behavior — it always gathers then immediately raycasts, same as before this
    /// split.
    ///
    /// `extent` is the scene's WORLD-unit rectangle `(0,0)–extent`, produced by
    /// `GridShape::world_extent` from the scene's authored bounds — those are measured in grid
    /// units (cells), continuous, and must never reach `env_light_polys`, which measures against
    /// wall coordinates in world units.
    fn lighting_inputs_from(
        all_bright: bool,
        lights: Vec<lighting::Light>,
        light_walls: &[vision::Seg],
        sight_walls: Vec<vision::Seg>,
        extent: (f64, f64),
        cell: f64,
    ) -> LightingInputs {
        let lit_polys: Vec<Vec<vision::P>> = lights
            .iter()
            .map(|l| {
                let b = vision::bound_for(l.pos, light_walls, VISION_BOUND_MARGIN);
                vision::visibility_polygon(l.pos, light_walls, b)
            })
            .collect();
        // Boundary-projected environment occlusion. Empty under all_bright (env is not
        // the mechanism there); occluded by the SAME blocksLight walls as the placed lights.
        let env_polys = if all_bright {
            Vec::new()
        } else {
            lighting::env_light_polys(extent, cell, light_walls)
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
            // An absent entry means no scene document — skip rather than synthesize a grid.
            let Some(cell) = grid.get(&scene).copied() else {
                continue;
            };
            if cell <= 0.0 {
                continue;
            }
            let cell_grid = self.resolve_grid_shape(scene, cell);
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
                            cell,
                        )
                    };
                    let dist_cells =
                        (((cx - src.vp.0).powi(2) + (cy - src.vp.1).powi(2)).sqrt()) / cell;
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

        let grid = self.resolve_grid_shape(scene, cell);
        let li = Self::lighting_inputs_from(
            all_bright,
            lights,
            &light_walls,
            sight_walls,
            grid.world_extent(settings.bounds),
            cell,
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
                    floors: self.token_vision_floors(&e.doc),
                });
            }
        }
        sources
    }

    /// Engine-owned movement collision. True if the move segment `a0→a1` crosses any `blocksMove`
    /// wall in `scene`.
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
/// under lighting-off or globalIllumination.
pub(crate) struct LightingInputs {
    /// Skip per-light raycasts: lighting off or `GlobalIllumination`.
    pub(crate) all_bright: bool,
    /// Resolved scene lights (empty under `all_bright`).
    pub(crate) lights: Vec<lighting::Light>,
    /// Per-light visibility polygons, index-aligned with `lights`.
    pub(crate) lit_polys: Vec<Vec<vision::P>>,
    /// Scene-boundary visibility polygons occluding the environment ambient (`env_light_polys`).
    /// Empty under `all_bright` (env is not the mechanism there — every LOS cell is forced bright).
    pub(crate) env_polys: Vec<Vec<vision::P>>,
    /// `blocksSight` wall segments (LOS raycast input).
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
    /// Source token id (snapshot ordering only; see the struct doc).
    id: Uuid,
    /// Viewpoint in scene units.
    vp: vision::P,
    /// Resolved vision floors: `(illumination floor, range cells, render hint)`.
    floors: Vec<(f64, f64, Option<String>)>,
}

/// One `sources` entry in `VisibilityInputsSnapshot`: `(token id, viewpoint, vision floors)`.
type VisSrcSnapshot = (Uuid, vision::P, Vec<(f64, f64, Option<String>)>);

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
    /// `blocksLight` wall segments.
    light_walls: Vec<vision::Seg>,
    /// `blocksSight` wall segments.
    sight_walls: Vec<vision::Seg>,
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
    for src in sources {
        let poly = source_los_poly(
            src.vp,
            &li.sight_walls,
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
            // Strict: center only. Lenient: center first (so §13 strict cells are always
            // included), then corners if center fails — a cell whose polygon merely clips
            // a corner still qualifies under leniency.
            let center = grid.cell_center((i, j));
            let mut found = false;
            if lenient {
                // Check center first, then corners. `cell_vertices` (the 4 square corners in
                // byte-identical order, or the 6 pointy-top hex vertices) is computed ONLY on this
                // path — the strict movement-gate mask never pays for it (6 sin/cos per hex cell).
                if vision::point_in_poly(&poly, center)
                    && point_qualifies(center, src.vp, &src.floors, settings, li, cell)
                {
                    found = true;
                }
                if !found {
                    let corners = grid.cell_vertices((i, j), cell);
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
/// (`vision::visibility_polygon`). `scene_extent` is the scene's WORLD-unit rectangle
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
    scene_extent: (f64, f64),
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
/// accepted so vision can derive per recipient; the identity payload is
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
    use grid_shape::GridShape as _;
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

    /// The grid size every hex fixture in this module declares, and the size every test that
    /// derives hex COORDINATES from a `HexGrid` builds that shape at — `hex_open_scene`,
    /// `hex_env_lit_scene_with_room`, `hex_continuous_scene_docs`, and the three continuous hex
    /// tests that author their own scene inline. A test whose expectations come from
    /// `cell_center`/`cell_vertices` is measuring the scene it declared only while the two agree,
    /// and nothing else makes them agree.
    ///
    /// The two `resolve_grid_shape_*` tests deliberately do NOT read it: their subject is that
    /// shape resolution keys on `grid.kind` and takes its SIZE from the caller's parameter, so the
    /// scene's declared size has to be stated independently of the shape they compare against — it
    /// is in fact never read by the code under test, and a mismatch between the parameter and the
    /// expected shape fails their `cell_center` comparison outright rather than silently.
    const HEX_FIXTURE_SIZE: f64 = 50.0;

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
    fn resolve_grid_shape_selects_hex_grid_for_hex_kind_scenes() {
        let scene_id = Uuid::from_u128(10);
        let scene = entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "hex", "size": 50.0 }, "background": null }),
        );
        let ecs = SceneEcs::from_documents(vec![scene], 0);
        let shape = ecs.resolve_grid_shape(scene_id, 50.0);
        let want = grid_shape::HexGrid { size: 50.0 };
        assert_eq!(shape.cell_center((1, 0)), want.cell_center((1, 0)));
        assert_ne!(
            shape.cell_center((1, 0)),
            grid_shape::SquareGrid {
                cell: 50.0,
                rule: pathfinding::DiagonalRule::Chebyshev
            }
            .cell_center((1, 0)),
            "hex and square cell centers must differ for the same cell index/size"
        );
    }

    #[test]
    fn resolve_grid_shape_falls_back_to_square_grid_for_unrecognized_kind() {
        let scene_id = Uuid::from_u128(10);
        let scene = entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "triangle", "size": 50.0 }, "background": null }),
        );
        let ecs = SceneEcs::from_documents(vec![scene], 0);
        let shape = ecs.resolve_grid_shape(scene_id, 50.0);
        let want = grid_shape::SquareGrid {
            cell: 50.0,
            rule: ecs.resolved_diagonal_rule(),
        };
        assert_eq!(shape.cell_center((1, 0)), want.cell_center((1, 0)));
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
    fn pathfind_refuses_a_scene_with_no_document() {
        // Anti-drift with the two movement gates (`Room::publish`, `Room::execute_move`), which
        // both refuse this input: a router that substituted a 100-unit default would happily
        // return a route through a scene whose grid it invented, for a scene the gate that must
        // later authorize the move rejects outright. GM requester, so no mask or presence check
        // can account for the refusal — with the default restored this routes successfully.
        let ecs = SceneEcs::new();
        let out = ecs.pathfind(
            RouteRequester {
                user: Uuid::from_u128(7),
                is_gm: true,
                explored: None,
            },
            Uuid::from_u128(404),
            (50.0, 50.0),
            &[(450.0, 50.0)],
            0.1,
        );
        assert!(
            matches!(out, Err(pathfinding::PathFail::Invalid)),
            "a scene with no document is not routable"
        );
    }

    #[test]
    fn user_owns_token_in_scene_follows_the_actor_join_and_is_scene_scoped() {
        use serde_json::json;
        // The pathfind presence gate keys on this. It must agree with
        // `token_effective_owner` (never a raw `doc.owner` read) and must be scoped to the
        // named scene, or a player is either locked out of a scene they hold an
        // actor-inherited token in, or admitted to one they hold nothing in.
        let player = Uuid::from_u128(7);
        let stranger = Uuid::from_u128(8);
        let mut actor = entity_doc_top_eng(200, "actor", actor_body(json!([])));
        actor.owner = Some(player);

        // Scene 10 holds a token linked to the actor with NO per-token owner.
        let inherited = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                    "actor_id": Uuid::from_u128(200).to_string() }),
        );
        // Scene 20 holds only a token nobody in this test owns.
        let unowned = entity_doc_eng(
            21,
            20,
            "token",
            json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );

        let mut ecs = SceneEcs::from_documents(
            vec![
                doc(10, None, "scene"),
                doc(20, None, "scene"),
                inherited,
                unowned,
            ],
            0,
        );
        ecs.set_actors(vec![actor.clone()]);

        assert!(
            ecs.user_owns_token_in_scene(player, Uuid::from_u128(10)),
            "an actor-inherited owner controls a token in scene 10"
        );
        assert!(
            !ecs.user_owns_token_in_scene(player, Uuid::from_u128(20)),
            "presence is scene-scoped: scene 20 holds no token of theirs"
        );
        assert!(
            !ecs.user_owns_token_in_scene(stranger, Uuid::from_u128(10)),
            "a stranger controls nothing"
        );

        // Re-assigning the ACTOR moves presence with it, with no write to any token.
        let mut reassigned = actor;
        reassigned.owner = Some(stranger);
        ecs.set_actors(vec![reassigned]);
        assert!(
            !ecs.user_owns_token_in_scene(player, Uuid::from_u128(10)),
            "presence follows the actor live — nothing is stamped on the token"
        );
        assert!(ecs.user_owns_token_in_scene(stranger, Uuid::from_u128(10)));
    }

    /// The derived ECS must reach the SAME document state the authoritative store
    /// reaches for every `FieldChange`, `remove: true` included. `remove` deletes the
    /// key and `new` is unused (conventionally Null) — an ECS that only ever calls
    /// `set_pointer` writes `ch.new` where the DB wrote absence, and `new` is
    /// unconstrained by the OCC/capability checks. Routed through ownership because
    /// that is where the divergence is exploitable: `/engine/actor_id` is a
    /// WRITE_FIELDS path, so a token's effective owner can submit this, leaving the
    /// write path saying "unowned" while the vision/lit-mask family says "owned by
    /// whoever `new` names" — vision widening exactly where write refuses.
    #[test]
    fn ecs_and_db_agree_on_ownership_after_a_remove_change_carrying_a_non_null_new() {
        use crate::data::command::FieldChange;
        use serde_json::json;

        let p = Uuid::from_u128(7); // owns actor A
        let q = Uuid::from_u128(8); // owns actor B — the injected target
        let a_id = Uuid::from_u128(200);
        let b_id = Uuid::from_u128(201);
        let mut actor_a = entity_doc_top_eng(200, "actor", actor_body(json!([])));
        actor_a.owner = Some(p);
        let mut actor_b = entity_doc_top_eng(201, "actor", actor_body(json!([])));
        actor_b.owner = Some(q);

        let token = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                    "actor_id": a_id.to_string() }),
        );

        // The change a token's effective owner can legitimately submit: `/engine/actor_id`
        // maps to WRITE_FIELDS, the OCC pre-image matches, and an absent optional
        // `actor_id` clears `validate_engine_tree`.
        let changes = vec![FieldChange {
            remove: true,
            path: "/engine/actor_id".into(),
            old: json!(a_id.to_string()),
            new: json!(b_id.to_string()), // unconstrained; ignored by the authoritative path
        }];

        // What the AUTHORITATIVE store reaches. Calls the hoisted rule
        // (`command::apply_field_change`) that `apply_intent` Phase 2 itself calls, so
        // this oracle cannot drift from the store — hand-copying the remove/set branch
        // here would silently go stale the moment `apply_intent` changed. NOT tautological
        // with respect to what this test pins: reverting the ECS mirror to an
        // unconditional `set_pointer` still diverges from this oracle and still fails.
        let db_token: Document = {
            let mut v = serde_json::to_value(&token).unwrap();
            for ch in &changes {
                apply_field_change(&mut v, ch).unwrap();
            }
            serde_json::from_value(v).unwrap()
        };
        let mut actor_index = std::collections::HashMap::new();
        actor_index.insert(a_id, actor_a.clone());
        actor_index.insert(b_id, actor_b.clone());
        let db_owner = crate::data::permission::effective_owner(
            &db_token,
            crate::data::permission::token_actor_link(&db_token)
                .and_then(|id| actor_index.get(&id)),
        );
        assert_eq!(
            db_owner, None,
            "the link is gone in the authoritative store"
        );

        let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), token.clone()], 0);
        ecs.set_actors(vec![actor_a, actor_b]);
        ecs.apply_op(&Operation::Update {
            doc_id: token.id,
            changes,
        });

        let e = ecs.index[&token.id];
        let ecs_doc = ecs.world.get::<&SceneEntity>(e).unwrap().doc.clone();
        assert_eq!(
            ecs.token_effective_owner(&ecs_doc),
            db_owner,
            "derived ECS ownership must equal authoritative ownership after a remove change"
        );

        // The exploitable observable: the injected actor's owner must NOT gain the token
        // as a vision source for a token the write path considers unowned.
        assert!(
            ecs.player_vision_polygons(q).is_empty(),
            "a removed actor link must not hand the token's vision to the injected `new` owner"
        );
        assert!(ecs.player_vision_polygons(p).is_empty());
    }

    /// Control for the test above: with `remove: false` the ECS and the store agree
    /// (both `set_pointer`), so the assertion pair there is about `remove`, not about
    /// re-linking in general.
    #[test]
    fn ecs_and_db_agree_when_a_set_change_relinks_a_token() {
        use crate::data::command::FieldChange;
        use serde_json::json;

        let p = Uuid::from_u128(7);
        let q = Uuid::from_u128(8);
        let a_id = Uuid::from_u128(200);
        let b_id = Uuid::from_u128(201);
        let mut actor_a = entity_doc_top_eng(200, "actor", actor_body(json!([])));
        actor_a.owner = Some(p);
        let mut actor_b = entity_doc_top_eng(201, "actor", actor_body(json!([])));
        actor_b.owner = Some(q);

        let token = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                    "actor_id": a_id.to_string() }),
        );
        let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), token.clone()], 0);
        ecs.set_actors(vec![actor_a, actor_b]);
        ecs.apply_op(&Operation::Update {
            doc_id: token.id,
            changes: vec![FieldChange {
                remove: false,
                path: "/engine/actor_id".into(),
                old: json!(a_id.to_string()),
                new: json!(b_id.to_string()),
            }],
        });

        let e = ecs.index[&token.id];
        let ecs_doc = ecs.world.get::<&SceneEntity>(e).unwrap().doc.clone();
        assert_eq!(
            ecs.token_effective_owner(&ecs_doc),
            Some(q),
            "a plain set DOES re-link, and both paths see it"
        );
        assert_eq!(ecs.player_vision_polygons(q).len(), 1);
        assert!(ecs.player_vision_polygons(p).is_empty());
    }

    /// The same divergence on the `self.actors` index (`apply_op`'s second
    /// `set_pointer` loop): removing an actor's `/owner` must leave the actor
    /// UNOWNED in the ECS, matching the store — not owned by the injected `new`.
    #[test]
    fn ecs_actor_index_honors_a_remove_change_on_owner() {
        use crate::data::command::FieldChange;
        use serde_json::json;

        let p = Uuid::from_u128(7);
        let q = Uuid::from_u128(8);
        let a_id = Uuid::from_u128(200);
        let mut actor_a = entity_doc_top_eng(200, "actor", actor_body(json!([])));
        actor_a.owner = Some(p);
        let token = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                    "actor_id": a_id.to_string() }),
        );
        let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), token.clone()], 0);
        ecs.set_actors(vec![actor_a]);
        ecs.apply_op(&Operation::Update {
            doc_id: a_id,
            changes: vec![FieldChange {
                remove: true,
                path: "/owner".into(),
                old: json!(p.to_string()),
                new: json!(q.to_string()),
            }],
        });
        assert_eq!(ecs.actor(&a_id).unwrap().owner, None);
        let e = ecs.index[&token.id];
        let ecs_doc = ecs.world.get::<&SceneEntity>(e).unwrap().doc.clone();
        assert_eq!(ecs.token_effective_owner(&ecs_doc), None);
        assert!(ecs.player_vision_polygons(q).is_empty());
    }

    /// Collects the `Level` of every event emitted on the current thread, so a test can
    /// assert what a code path is allowed to log — not merely what it computes.
    #[derive(Default, Clone)]
    struct LevelCapture(std::sync::Arc<std::sync::Mutex<Vec<tracing::Level>>>);

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for LevelCapture {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            self.0.lock().unwrap().push(*event.metadata().level());
        }
    }

    /// Run `f` with a thread-local capturing subscriber and return the levels emitted.
    fn captured_levels(f: impl FnOnce()) -> Vec<tracing::Level> {
        use tracing_subscriber::layer::SubscriberExt;
        let cap = LevelCapture::default();
        let subscriber = tracing_subscriber::registry().with(cap.clone());
        tracing::subscriber::with_default(subscriber, f);
        let out = cap.0.lock().unwrap().clone();
        out
    }

    /// `Room::publish` reaches `token_move` with RAW client changes, strictly before
    /// `apply_intent` runs `validate_field_change`, so a malformed path is untrusted
    /// input any authenticated client can send at will. It must (a) fail closed and
    /// (b) NOT emit at `error` — otherwise a client can flood on demand the channel
    /// that exists to surface real store/mirror divergence.
    #[test]
    fn a_malformed_proposed_path_fails_closed_without_an_error_level_log() {
        use crate::data::command::FieldChange;
        use serde_json::json;

        let token = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 10.0, "y": 20.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), token.clone()], 0);

        // Descends through a scalar (`/engine/x` is a number) -> BadPath.
        let malformed = vec![FieldChange {
            remove: false,
            path: "/engine/x/y/z".into(),
            old: json!(null),
            new: json!(1.0),
        }];

        let mut out = None;
        let levels = captured_levels(|| out = Some(ecs.token_move(token.id, &malformed)));

        // Fails closed: the position is untouched, so the projected target equals the
        // committed one — the gate derives nothing from a malformed change.
        assert_eq!(
            out.unwrap().map(|(_, from, to)| (from, to)),
            Some(((10.0, 20.0), (10.0, 20.0))),
            "a malformed proposed path must not move the projected target"
        );
        assert!(
            !levels.contains(&tracing::Level::ERROR),
            "client-proposed malformed input must not emit at error level, got {levels:?}"
        );
        assert!(
            levels.contains(&tracing::Level::DEBUG),
            "the divergence must still be reported, at debug: got {levels:?}"
        );
    }

    /// The counterpart: at a COMMITTED mirror the same failure is a should-never-happen
    /// invariant breach — the store applied a change the mirror could not — and must
    /// stay at `error`. Same helper, two meanings; this pins that they do not collapse.
    #[test]
    fn a_failed_committed_mirror_change_logs_at_error_level() {
        use crate::data::command::FieldChange;
        use serde_json::json;

        let token = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 10.0, "y": 20.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), token.clone()], 0);

        let levels = captured_levels(|| {
            ecs.apply_op(&Operation::Update {
                doc_id: token.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/engine/x/y/z".into(),
                    old: json!(null),
                    new: json!(1.0),
                }],
            })
        });
        assert!(
            levels.contains(&tracing::Level::ERROR),
            "a committed change the mirror cannot apply is a real divergence: got {levels:?}"
        );
    }

    /// `apply_config_update` (the world-settings / gradation / vision-modes singleton
    /// mirror) is the third site that must obey the store-equal mutation rule. It had
    /// no coverage: reverting it to an unconditional `set_pointer` survived the whole
    /// suite. A `remove` on a config field must leave the ECS singleton matching the
    /// store's ABSENCE, not holding `ch.new`.
    #[test]
    fn config_singleton_mirror_honors_a_remove_change() {
        use crate::data::command::{apply_field_change, FieldChange};
        use serde_json::json;

        // A vision-modes doc with two modes; the change removes one of them while
        // naming a replacement in `new` (the value an unconditional set would land).
        let vm_id = Uuid::from_u128(300);
        let mut vm = doc(300, None, "vision-modes");
        vm.engine = Some(json!({ "modes": {
            "darkvision": { "id": "darkvision", "name": "Darkvision",
                            "illuminationFloor": "dark", "defaultRange": 6 },
            "blindsight": { "id": "blindsight", "name": "Blindsight",
                            "illuminationFloor": "dark", "defaultRange": 4 }
        }}));

        let change = FieldChange {
            remove: true,
            path: "/engine/modes/blindsight".into(),
            old: json!(null),
            // Unconstrained by OCC/capability checks: what a forked mirror would store.
            new: json!({ "id": "smuggled", "name": "Smuggled",
                         "illuminationFloor": "bright", "defaultRange": 99 }),
        };

        // Oracle: the hoisted rule the authoritative store itself calls.
        let store_engine = {
            let mut v = serde_json::to_value(&vm).unwrap();
            apply_field_change(&mut v, &change).unwrap();
            serde_json::from_value::<Document>(v)
                .unwrap()
                .engine
                .unwrap()
        };

        let mut ecs = SceneEcs::new();
        ecs.set_world_config(None, None, Some(vm));
        ecs.apply_op(&Operation::Update {
            doc_id: vm_id,
            changes: vec![change],
        });

        let mirrored = ecs.vision_modes_doc().unwrap().engine.clone().unwrap();
        assert_eq!(
            mirrored, store_engine,
            "the config singleton mirror must reach the store's value for a remove change"
        );
        assert!(
            mirrored["modes"].get("blindsight").is_none(),
            "the removed mode must be ABSENT, not replaced by `new`"
        );
        // The derived read-through agrees: the removed mode is gone, the other remains.
        let modes = ecs.resolved_vision_modes();
        assert!(!modes.contains_key("blindsight"));
        assert!(
            !modes.contains_key("smuggled"),
            "`new` must never be stored"
        );
        assert!(modes.contains_key("darkvision"));
    }

    /// `token_move` (the pre-commit move projection) is the fourth site. It had no
    /// coverage either: an unconditional `set_pointer` there survived the suite. The
    /// projected target must be derived by the same rule the store uses — so a `remove`
    /// clears the coordinate rather than yielding a target taken from `new`.
    ///
    /// Hardening, not a reachable committed divergence: `TokenEngine.x`/`y` are required
    /// `f64`, so a `/engine/x` removal fails `validate_engine_tree` on the post-image and
    /// never commits. This pins store-equality by construction at the site rather than
    /// relying on that downstream rejection to stay true.
    #[test]
    fn token_move_projection_honors_a_remove_change() {
        use crate::data::command::FieldChange;
        use serde_json::json;

        let token = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 10.0, "y": 20.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), token.clone()], 0);

        // Control: a plain set projects the new position from `new`.
        let moved = ecs.token_move(
            token.id,
            &[FieldChange {
                remove: false,
                path: "/engine/x".into(),
                old: json!(10.0),
                new: json!(70.0),
            }],
        );
        assert_eq!(
            moved.map(|(_, from, to)| (from, to)),
            Some(((10.0, 20.0), (70.0, 20.0))),
            "a set change projects `new` as the target"
        );

        // A REMOVE of the same path must clear `/engine/x`, so no target is derivable —
        // never a target built from `new`. An unconditional `set_pointer` here yields
        // (99.0, 20.0) instead.
        let removed = ecs.token_move(
            token.id,
            &[FieldChange {
                remove: true,
                path: "/engine/x".into(),
                old: json!(10.0),
                new: json!(99.0),
            }],
        );
        assert_eq!(
            removed, None,
            "a removed coordinate yields no projected move, never one derived from `new`"
        );
    }

    #[test]
    fn token_ownership_resolves_through_the_actor_join_for_vision() {
        use serde_json::json;
        // Vision/mask ownership MUST be the same rule the write-authz path uses
        // (`permission::effective_owner`), or a player could move a token that
        // contributes no vision — or see through one they cannot move.
        let player = Uuid::from_u128(7);
        let other = Uuid::from_u128(8);
        let mut actor = entity_doc_top_eng(200, "actor", actor_body(json!([])));
        actor.owner = Some(player);

        // Linked, NO per-token owner: inherits the actor's owner.
        let inherited = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                    "actor_id": Uuid::from_u128(200).to_string() }),
        );
        // Linked to the SAME actor but overridden to `other`: the override wins.
        let mut overridden = entity_doc_eng(
            12,
            10,
            "token",
            json!({ "x": 200.0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                    "actor_id": Uuid::from_u128(200).to_string() }),
        );
        overridden.owner = Some(other);
        // Linked to an actor that does not exist: fails closed to no owner.
        let dangling = entity_doc_eng(
            13,
            10,
            "token",
            json!({ "x": 400.0, "y": 0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                    "actor_id": Uuid::from_u128(999).to_string() }),
        );

        let mut ecs = SceneEcs::from_documents(
            vec![
                doc(10, None, "scene"),
                inherited.clone(),
                overridden.clone(),
                dangling.clone(),
            ],
            0,
        );
        ecs.set_actors(vec![actor.clone()]);

        assert_eq!(
            ecs.token_effective_owner(&inherited),
            Some(player),
            "a linked token with no override inherits the actor's owner"
        );
        assert_eq!(
            ecs.token_effective_owner(&overridden),
            Some(other),
            "the per-token override supersedes the actor's owner"
        );
        assert_eq!(
            ecs.token_effective_owner(&dangling),
            None,
            "a dangling link fails closed"
        );

        // The vision channel agrees: the inheriting player gets exactly the one
        // token they effectively own, and the override holder gets exactly theirs.
        let polys = ecs.player_vision_polygons(player);
        assert_eq!(
            polys.len(),
            1,
            "only the inherited token is a vision source for its inheriting owner"
        );
        assert_eq!(ecs.player_vision_polygons(other).len(), 1);
        assert!(
            ecs.player_vision_polygons(Uuid::from_u128(99)).is_empty(),
            "a stranger owns nothing"
        );

        // Re-assigning the ACTOR moves vision ownership with no write to any token.
        let mut reassigned = actor;
        reassigned.owner = Some(other);
        ecs.set_actors(vec![reassigned]);
        assert_eq!(
            ecs.token_effective_owner(&inherited),
            Some(other),
            "ownership follows the actor live — nothing is stamped on the token"
        );
        assert!(ecs.player_vision_polygons(player).is_empty());
        assert_eq!(ecs.player_vision_polygons(other).len(), 2);
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

    /// A minimal, structurally-complete `ActorEngine` body with the caller's `shape`/`size`, for
    /// this file's footprint tests.
    fn actor_body_shaped(shape: &str, w: f64, h: f64) -> serde_json::Value {
        json!({
            "displayName": "Fixture Actor",
            "visual": { "kind": "image", "asset": "a.png" },
            "size": { "w": w, "h": h },
            "shape": shape,
            "conditions": [],
            "prototype": true,
        })
    }

    /// A hydrated ECS with one LINKED token (id 11) referencing an actor (id 200) of the given
    /// `shape`/`size`, no overrides.
    fn scene_with_linked_token_sized(shape: &str, w: f64, h: f64) -> (SceneEcs, Uuid) {
        let token_id = Uuid::from_u128(11);
        let mut ecs = SceneEcs::from_documents(
            vec![
                doc(10, None, "scene"),
                entity_doc_eng(
                    11,
                    10,
                    "token",
                    json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                            "actor_id": Uuid::from_u128(200).to_string() }),
                ),
            ],
            0,
        );
        ecs.set_actors(vec![entity_doc_top_eng(
            200,
            "actor",
            actor_body_shaped(shape, w, h),
        )]);
        (ecs, token_id)
    }

    /// A hydrated ECS with one raw (actorless) token (id 12) — no `actor_id`, no embedded actor.
    fn scene_with_raw_token_no_actor() -> (SceneEcs, Uuid) {
        let token_id = Uuid::from_u128(12);
        let ecs = SceneEcs::from_documents(
            vec![
                doc(10, None, "scene"),
                entity_doc_eng(
                    12,
                    10,
                    "token",
                    json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
                ),
            ],
            0,
        );
        (ecs, token_id)
    }

    /// A hydrated ECS with one LINKED token (id 11) referencing a "square" 1x1 actor (id 200),
    /// with a per-token `overrides.size` of the caller's `shape`/`(w, h)`.
    fn scene_with_linked_token_overriding_size(shape: &str, w: f64, h: f64) -> (SceneEcs, Uuid) {
        let token_id = Uuid::from_u128(11);
        let mut ecs = SceneEcs::from_documents(
            vec![
                doc(10, None, "scene"),
                entity_doc_eng(
                    11,
                    10,
                    "token",
                    json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0,
                            "actor_id": Uuid::from_u128(200).to_string(),
                            "overrides": { "shape": shape, "size": { "w": w, "h": h } } }),
                ),
            ],
            0,
        );
        ecs.set_actors(vec![entity_doc_top_eng(
            200,
            "actor",
            actor_body_shaped("square", 1.0, 1.0),
        )]);
        (ecs, token_id)
    }

    #[test]
    fn footprint_radius_mirrors_the_client_formula() {
        // Mirrors the client's `footprintRadius`:
        //   circle ⇒ max(w,h)/2 ; square (and any other shape) ⇒ hypot(w,h)/2
        // Representative + boundary cases; `Size` is a free {w,h} pair, so there is no finite
        // domain to enumerate exhaustively.
        let cases = [
            ("square", 1.0, 1.0, std::f64::consts::SQRT_2 / 2.0),
            ("square", 2.0, 2.0, std::f64::consts::SQRT_2),
            ("square", 1.0, 2.0, 5.0f64.sqrt() / 2.0),
            ("circle", 1.0, 1.0, 0.5),
            ("circle", 2.0, 3.0, 1.5),
            // A shape outside {"circle","square"} takes the square branch, mirroring the client's
            // `shape === "circle" ? … : hypot(…)` fallthrough.
            ("blob", 1.0, 1.0, std::f64::consts::SQRT_2 / 2.0),
        ];
        for (shape, w, h, expected) in cases {
            let (ecs, token) = scene_with_linked_token_sized(shape, w, h);
            let got = ecs.resolve_token_footprint(token).expect("in-range");
            assert!(
                (got - expected).abs() < 1e-12,
                "shape={shape} w={w} h={h}: want {expected}, got {got}"
            );
        }
    }

    #[test]
    fn footprint_radius_falls_back_to_the_client_default_for_an_actorless_token() {
        let (ecs, token) = scene_with_raw_token_no_actor();
        assert_eq!(
            ecs.resolve_token_footprint(token),
            Some(DEFAULT_FOOTPRINT_RADIUS_CELLS),
            "an actorless token uses the same 0.4 default the client's resolveFootprint uses"
        );
    }

    #[test]
    fn footprint_radius_honors_a_per_token_size_override() {
        let (ecs, token) = scene_with_linked_token_overriding_size("circle", 4.0, 4.0);
        assert!((ecs.resolve_token_footprint(token).expect("in-range") - 2.0).abs() < 1e-12);
    }

    #[test]
    fn footprint_radius_refuses_an_oversized_token_rather_than_clamping() {
        // w=h=1000 ⇒ ~707 cells, far over MAX_FOOTPRINT_CELLS (64.0). Clamping would gate a
        // map-scale token as a 64-cell disc — a geometric fail-open.
        let (ecs, token) = scene_with_linked_token_sized("square", 1000.0, 1000.0);
        assert_eq!(
            ecs.resolve_token_footprint(token),
            None,
            "an out-of-range footprint is refused"
        );
    }

    #[test]
    fn footprint_radius_admits_a_token_exactly_at_the_bound() {
        let at = pathfinding::MAX_FOOTPRINT_CELLS; // 64.0
        let (ecs, token) = scene_with_linked_token_sized("circle", at * 2.0, at * 2.0);
        assert_eq!(
            ecs.resolve_token_footprint(token),
            Some(at),
            "AT the bound is admissible"
        );
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
        // band even for a normal-vision token with NO lights present.
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
        // Absent doc → built-in seed mirrors the client's `SEED_VISION_MODES`: darkvision desaturates, normal does not.
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
    /// Mirrors `document::tests::world_scoped_doc`, the same test helper `ws::room`'s own tests
    /// use to build a world-settings config doc.
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
        use crate::data::command::set_pointer;
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
    fn the_resolved_shape_reports_the_resolved_kind() {
        // The three readers of the same decision — the shape a scene resolves to, the kind its
        // settings carry, and the ECS resolver — must not be able to disagree.
        // Discrimination: fails if `resolve_grid_shape_with_rule` stops constructing its shape
        // from `resolve_grid_kind`, or if `resolve_scene` stops reading the same pure helper,
        // which are the only ways the three can diverge. The unrecognised spelling pins the
        // fail-closed default in the same loop.
        for (engine, expect) in [
            (
                json!({ "grid": { "kind": "hex", "size": 50 }, "background": null }),
                GridKind::Hex,
            ),
            (
                json!({ "grid": { "kind": "square", "size": 50 }, "background": null }),
                GridKind::Square,
            ),
            (
                json!({ "grid": { "kind": "wobbly", "size": 50 }, "background": null }),
                GridKind::Square,
            ),
        ] {
            let ecs = SceneEcs::from_documents(vec![entity_doc_top_eng(10, "scene", engine)], 0);
            let scene = Uuid::from_u128(10);
            assert_eq!(ecs.resolve_grid_kind(scene), expect);
            assert_eq!(ecs.resolve_scene(scene).grid_kind, expect);
            assert_eq!(ecs.resolve_grid_shape(scene, 50.0).kind(), expect);
        }
    }

    #[test]
    fn changing_a_scenes_grid_kind_invalidates_the_cached_visibility_mask() {
        // The cache is value-comparison based, so any input the mask depends on must appear in
        // the snapshot. The grid kind decides every cell index in the mask.
        // Discrimination: fails if `ResolvedScene` carries no kind or the snapshot omits it,
        // because the snapshot then compares equal across the mutation and the stale mask is
        // returned. It cannot pass vacuously: the fixture asserts the two masks differ, so a
        // scene whose kind change produced no geometric difference fails the guard.
        let user = Uuid::from_u128(7);
        let scene_id = Uuid::from_u128(10);
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

        let before = ecs.visible_cells_cached(user, scene_id, false);

        // Mutate the scene document's grid kind through apply_op, matching how a real write
        // would reach the ECS.
        ecs.apply_op(&Operation::Update {
            doc_id: scene_id,
            changes: vec![FieldChange {
                path: "/engine/grid/kind".to_string(),
                old: json!("square"),
                new: json!("hex"),
                remove: false,
            }],
        });

        let after = ecs.visible_cells_cached(user, scene_id, false);
        assert_ne!(
            before, after,
            "a grid-kind change must produce a different mask"
        );
    }

    #[test]
    fn lit_mask_suppresses_hint_when_normal_floor_wins_in_bright_cell() {
        use serde_json::json;
        // Combined-token suppression: an owned token whose embedded actor has
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
        let scene_id = Uuid::from_u128(10);
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok, light], 0);
        let mask = ecs.player_lit_mask(player);
        let lit_cells: Vec<_> = mask.iter().flat_map(|s| s.cells.iter()).collect();
        assert!(
            !lit_cells.is_empty(),
            "token with normal+darkvision under bright light must see at least one cell"
        );
        // Which cells are BRIGHT is read from the data, not restated: the light sits at the
        // token's own position, so that cell's own illumination band names the brightest band
        // present, and every cell sharing it is a brightly-lit cell.
        let cell = *ecs
            .scene_grid_sizes()
            .get(&scene_id)
            .expect("the fixture's scene has a grid size");
        let token_cell = ecs.resolve_grid_shape(scene_id, cell).cell_of((50.0, 50.0));
        let bright_band = lit_cells
            .iter()
            .find(|(i, j, _, _, _)| (*i, *j) == token_cell)
            .map(|(_, _, b, _, _)| *b)
            .expect("the cell holding both the token and the light is lit");
        // Derived, not merely read back: bands are ordered brightest-first and the fixture's light
        // sits on the token, so the brightest band is index 0. Without this the comparison below
        // would be output-against-output and a uniform band shift would stay green.
        assert_eq!(
            bright_band, 0,
            "the cell holding the light must resolve to the brightest band"
        );
        // Every brightly-lit cell must carry None: normal's floor (0.34) > darkvision's floor
        // (0.0), so normal is the highest-admitting mode there and its None hint suppresses
        // desaturate.
        assert!(
            lit_cells
                .iter()
                .filter(|(_, _, b, _, _)| *b == bright_band)
                .all(|(_, _, _, _, h)| h.is_none()),
            "normal-floor wins in bright cell: desaturate hint must be suppressed (None)"
        );
        // The suppression above is a per-cell DECISION, not an inert hint field: the same mask's
        // darker cells — which only darkvision admits, since normal's floor excludes them — do
        // carry the hint. Without this the assertion would hold just as well if hints were never
        // emitted at all.
        assert!(
            lit_cells
                .iter()
                .any(|(_, _, b, _, h)| *b != bright_band && h.is_some()),
            "a darker cell admitted only by darkvision must carry its hint, so the bright cell's \
             None is a suppression rather than an absence"
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

    /// The hex analogue of `env_lit_scene_with_room`: an `environmentLight` scene on a pointy-top
    /// hex grid, with a player-owned normal-vision token at hex `(0,0)` and the six edges of hex
    /// `HEX_SEALED_CELL` walled off. The seal's segments are derived from `HexGrid::cell_vertices`
    /// rather than restated, so the box is exactly that hex's own boundary on both toggles.
    ///
    /// This is the only fixture that reaches `lighting::env_light_polys` on a hex scene: the
    /// remaining hex fixtures either disable lighting or route as an unrestricted GM, so no mask
    /// is built and the environment-light path never runs on hex without it. The extent that path
    /// walks the perimeter of is grid-kind-dependent, and grid kind and movement model are
    /// independent axes that combine.
    fn hex_env_lit_scene_with_room(blocks_light: bool) -> (SceneEcs, Uuid, Uuid) {
        let user = Uuid::from_u128(7);
        let scene_id = Uuid::from_u128(10);
        let g = grid_shape::HexGrid {
            size: HEX_FIXTURE_SIZE,
        };
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        let verts = grid_shape::GridShape::cell_vertices(&g, HEX_SEALED_CELL, g.size);
        let mut docs = vec![
            entity_doc_top_eng(
                10,
                "scene",
                json!({ "grid": { "kind": "hex", "size": g.size }, "background": null,
                        "bounds": { "width": 6.0, "height": 4.0 } }),
            ),
            tok,
        ];
        for (k, a) in verts.iter().enumerate() {
            let b = verts[(k + 1) % verts.len()];
            docs.push(entity_doc_eng(
                31 + k as u128,
                10,
                "wall",
                json!({ "seg": {"x1": a.0, "y1": a.1, "x2": b.0, "y2": b.1},
                        "blocksSight": true, "blocksMove": false, "blocksLight": blocks_light }),
            ));
        }
        let mut ecs = SceneEcs::from_documents(docs, 0);
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

    /// The hex whose six edges `hex_env_lit_scene_with_room` seals, and whose drop-out the test
    /// asserts. Named once so the fixture and its assertion cannot name different cells.
    const HEX_SEALED_CELL: (i32, i32) = (3, 1);

    #[test]
    fn hex_env_light_occlusion_seals_the_interior_like_the_square_path() {
        // The hex arm of the environment-occlusion property, which no other fixture reaches. It
        // depends on the hex extent: `env_light_polys` walks the perimeter of the rectangle
        // `world_extent` produces, so a wrong hex extent moves every boundary sample and changes
        // which cells the environment reaches.
        // Discrimination: the baseline assertion fails if the sealed hex is never lit in the first
        // place (a vacuous seal), the sealed assertion fails if `blocksLight` stops occluding, and
        // the subset assertion fails if occlusion ever ADDS a cell. Under a mutation of the hex
        // extent formula the perimeter walk changes and this test moves.
        let (ecs_after, user, scene) = hex_env_lit_scene_with_room(true);
        let (ecs_before, _, _) = hex_env_lit_scene_with_room(false);
        let after = mask_cells(&ecs_after, user, scene);
        let before = mask_cells(&ecs_before, user, scene);
        assert!(
            before.contains(&HEX_SEALED_CELL),
            "baseline (blocksLight:false) still lights hex {HEX_SEALED_CELL:?}"
        );
        assert!(
            !after.contains(&HEX_SEALED_CELL),
            "a blocksLight-sealed hex must drop out of a normal-vision player's mask"
        );
        assert!(
            after.is_subset(&before),
            "env occlusion is strictly narrowing on hex too: it only removes cells"
        );
        assert!(
            after.contains(&(0, 0)),
            "the open exterior (the token's own hex) stays lit and visible"
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

        let authoritative = ecs.region_field(scene_id, None).expect("scene exists");
        assert!(
            authoritative.is_impassable((0, 0)),
            "authoritative field includes the secret region"
        );
        assert_eq!(authoritative.terrain_multiplier((2, 0)), 2.0);

        let player_field = ecs
            .region_field(scene_id, Some(player))
            .expect("scene exists");
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
        assert!(!ecs
            .region_field(scene_id, None)
            .expect("scene exists")
            .is_impassable((0, 0)));
    }

    #[test]
    fn move_walls_returns_only_blocks_move_segments_for_the_scene() {
        // A scene with one blocksMove wall and one non-blocksMove wall yields exactly the blocking segment.
        let (ecs, scene) = scene_with_two_walls_one_blocking();
        let walls = ecs.move_walls(scene, None);
        assert_eq!(walls.len(), 1, "only the blocksMove wall is returned");
        let w = walls[0];
        assert_eq!((w.a, w.b), ((100.0, 0.0), (100.0, 200.0)));
    }

    /// A scene with grid size `cell` and no walls.
    fn scene_with_grid(cell: f64) -> (SceneEcs, Uuid) {
        let scene_id = Uuid::from_u128(10);
        let ecs = SceneEcs::from_documents(
            vec![entity_doc_top_eng(
                10,
                "scene",
                json!({ "grid": { "kind": "square", "size": cell }, "background": null }),
            )],
            0,
        );
        (ecs, scene_id)
    }

    /// A `wall` doc parented to `scene`, blocksMove+blocksSight+blocksLight all true.
    fn wall_doc_eng(scene: Uuid, a: (f64, f64), b: (f64, f64)) -> Document {
        let mut d = crate::data::document::tests::world_scoped_doc(
            Uuid::from_u128(9),
            Uuid::new_v4(),
            "wall",
        );
        d.parent_id = Some(scene);
        d.engine = Some(json!({
            "seg": { "x1": a.0, "y1": a.1, "x2": b.0, "y2": b.1 },
            "blocksMove": true,
            "blocksSight": true,
            "blocksLight": true,
        }));
        d
    }

    /// One public blocksMove wall at x=100 and one `gm_only` blocksMove wall at x=150.
    /// Both also carry blocksSight+blocksLight so the wall-set-parity test below can observe them
    /// in the vision sets.
    fn scene_with_public_and_secret_move_walls() -> (SceneEcs, Uuid, Uuid) {
        let (mut ecs, scene) = scene_with_grid(100.0);
        let player = Uuid::new_v4();
        let public = wall_doc_eng(scene, (100.0, 0.0), (100.0, 200.0));
        let mut secret = wall_doc_eng(scene, (150.0, 0.0), (150.0, 200.0));
        secret
            .permissions
            .property_overrides
            .insert("/engine".into(), crate::data::document::Visibility::GmOnly);
        ecs.apply_op(&Operation::Create { doc: public });
        ecs.apply_op(&Operation::Create { doc: secret });
        (ecs, scene, player)
    }

    /// One wall with blocksSight:false, blocksMove:true, default permissions.
    fn scene_with_invisible_barrier_wall() -> (SceneEcs, Uuid, Uuid) {
        let (mut ecs, scene) = scene_with_grid(100.0);
        let player = Uuid::new_v4();
        let mut barrier = wall_doc_eng(scene, (100.0, 0.0), (100.0, 200.0));
        barrier.engine = Some(json!({
            "seg": { "x1": 100.0, "y1": 0.0, "x2": 100.0, "y2": 200.0 },
            "blocksMove": true,
            "blocksSight": false,
            "blocksLight": true,
        }));
        ecs.apply_op(&Operation::Create { doc: barrier });
        (ecs, scene, player)
    }

    #[test]
    fn move_walls_omits_a_gm_only_wall_for_a_player_viewer() {
        let (ecs, scene, player) = scene_with_public_and_secret_move_walls();
        assert_eq!(
            ecs.move_walls(scene, None).len(),
            2,
            "authoritative view carries every blocksMove wall"
        );
        let visible = ecs.move_walls(scene, Some(player));
        assert_eq!(
            visible.len(),
            1,
            "a gm_only wall is omitted from a player's routing set"
        );
        assert_eq!(
            (visible[0].a, visible[0].b),
            ((100.0, 0.0), (100.0, 200.0)),
            "the surviving wall is the public one"
        );
    }

    #[test]
    fn move_walls_keeps_a_blocks_sight_false_wall_for_a_player() {
        // An invisible BARRIER (blocksSight:false, blocksMove:true) is a PUBLIC document: the router
        // must honor it. Only document-level secrecy filters — the two kinds are not the same axis.
        let (ecs, scene, player) = scene_with_invisible_barrier_wall();
        assert_eq!(
            ecs.move_walls(scene, Some(player)).len(),
            1,
            "a blocksSight:false wall is public geometry and stays in the player's routing set"
        );
    }

    /// Anti-drift: vision and lighting keep the FULL wall set; only routing filters. This is a
    /// must-NOT-converge constraint, so it gets a test rather than only a doc comment.
    #[test]
    fn vision_and_lighting_keep_a_gm_only_wall_that_routing_drops() {
        let (ecs, scene, player) = scene_with_public_and_secret_move_walls();
        assert_eq!(
            ecs.sight_walls(scene).len(),
            2,
            "sight_walls keeps the gm_only wall"
        );
        assert_eq!(
            ecs.light_walls(scene).len(),
            2,
            "light_walls keeps the gm_only wall"
        );
        assert_eq!(
            ecs.move_walls(scene, Some(player)).len(),
            1,
            "only the ROUTING set filters per requester"
        );
    }

    /// Anti-drift pin for `engine_tier_visible`, the single symbol `move_walls` and
    /// `region_field` both call for the `/engine`-tier per-requester decision. A re-inline at
    /// either call site, or a semantic change to the predicate itself, fails this test.
    #[test]
    fn engine_tier_visible_admits_authoritative_and_rejects_a_non_owner_player_on_gm_only() {
        let mut doc = crate::data::document::tests::world_scoped_doc(
            Uuid::from_u128(9),
            Uuid::from_u128(50),
            "wall",
        );
        doc.permissions
            .property_overrides
            .insert("/engine".into(), crate::data::document::Visibility::GmOnly);
        let player = Uuid::from_u128(99);
        assert!(
            engine_tier_visible(&doc, None),
            "the authoritative (None) viewer sees a gm_only doc"
        );
        assert!(
            !engine_tier_visible(&doc, Some(player)),
            "a non-owner player is rejected from a gm_only doc"
        );
    }

    #[test]
    fn absent_scene_yields_empty_visible_cells_not_a_synthesized_grid() {
        let (ecs, user, _scene) = scene_with_lit_player_token();
        let ghost_scene = Uuid::from_u128(0xDEAD);
        assert!(ecs.visible_cells(user, ghost_scene, false).is_empty());
        assert!(ecs.visible_cells(user, ghost_scene, true).is_empty());
        assert!(ecs
            .visible_cells_cached(user, ghost_scene, false)
            .is_empty());
    }

    #[test]
    fn absent_scene_region_field_is_none() {
        let (ecs, _user, _scene) = scene_with_lit_player_token();
        assert!(ecs.region_field(Uuid::from_u128(0xDEAD), None).is_none());
    }

    #[test]
    fn absent_scene_navmesh_for_is_none() {
        let (ecs, _user, _scene) = scene_with_lit_player_token();
        assert!(ecs.navmesh_for(Uuid::from_u128(0xDEAD), 0.5, &[]).is_none());
    }

    #[test]
    fn navmesh_for_is_memoized_across_calls() {
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
        let scene = Uuid::from_u128(10);
        let a = ecs.navmesh_for(scene, 0.4, &[]).expect("navmesh builds");
        let b = ecs.navmesh_for(scene, 0.4, &[]).expect("navmesh builds");
        assert!(
            std::sync::Arc::ptr_eq(&a, &b),
            "same (scene, radius, walls) must return the SAME cached Arc, not rebuild"
        );
    }

    #[test]
    fn navmesh_for_distinguishes_footprint_radii() {
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
        let scene = Uuid::from_u128(10);
        let a = ecs.navmesh_for(scene, 0.4, &[]).expect("navmesh builds");
        let b = ecs.navmesh_for(scene, 0.9, &[]).expect("navmesh builds");
        assert!(
            !std::sync::Arc::ptr_eq(&a, &b),
            "distinct footprint radii must get distinct cached meshes"
        );
    }

    #[test]
    fn navmesh_for_rejects_degenerate_radius_even_after_cache_primed_at_zero() {
        let ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
        let scene = Uuid::from_u128(10);
        // Prime the cache at footprint_radius_cells == 0.0: quantized key (scene, 0, []).
        let primed = ecs.navmesh_for(scene, 0.0, &[]);
        assert!(
            primed.is_some(),
            "radius 0.0 must build and cache successfully"
        );

        // f64 as i64 saturates NaN to 0, colliding with the primed key above. Without an
        // upfront validation guard this would return the CACHED radius-0.0 mesh instead of
        // failing closed.
        assert!(
            ecs.navmesh_for(scene, f64::NAN, &[]).is_none(),
            "NaN footprint radius must fail closed, not reuse the cached radius-0.0 mesh"
        );

        // A small negative rounds to -0 under `(x * 1000.0).round() as i64`, which also casts
        // to the same colliding key.
        assert!(
            ecs.navmesh_for(scene, -0.0001, &[]).is_none(),
            "negative footprint radius must fail closed, not reuse the cached radius-0.0 mesh"
        );
    }

    #[test]
    fn navmesh_for_does_not_share_a_mesh_across_differing_wall_sets() {
        let (ecs, scene, player) = scene_with_public_and_secret_move_walls();
        let gm_walls = ecs.move_walls(scene, None);
        let player_walls = ecs.move_walls(scene, Some(player));
        let gm_mesh = ecs
            .navmesh_for(scene, 0.4, &gm_walls)
            .expect("gm mesh builds");
        let player_mesh = ecs
            .navmesh_for(scene, 0.4, &player_walls)
            .expect("player mesh builds");
        assert!(
            !std::sync::Arc::ptr_eq(&gm_mesh, &player_mesh),
            "a differing wall set must not be served a mesh built from another set"
        );
    }

    #[test]
    fn navmesh_for_shares_a_mesh_across_identical_wall_sets() {
        let (ecs, scene, _player) = scene_with_public_and_secret_move_walls();
        let walls = ecs.move_walls(scene, None);
        let a = ecs.navmesh_for(scene, 0.4, &walls).expect("first build");
        let b = ecs.navmesh_for(scene, 0.4, &walls).expect("second build");
        assert!(
            std::sync::Arc::ptr_eq(&a, &b),
            "an identical wall set reuses the memoized mesh"
        );
    }

    #[test]
    fn navmesh_for_wall_key_is_order_independent() {
        // `hecs` iteration order is not stable, so the same set produced in a different order must
        // still hit the cache.
        let (ecs, scene, _player) = scene_with_public_and_secret_move_walls();
        let walls = ecs.move_walls(scene, None);
        let mut reversed = walls.clone();
        reversed.reverse();
        let a = ecs.navmesh_for(scene, 0.4, &walls).expect("first build");
        let b = ecs
            .navmesh_for(scene, 0.4, &reversed)
            .expect("reordered lookup");
        assert!(
            std::sync::Arc::ptr_eq(&a, &b),
            "wall-set key is order-independent"
        );
    }

    #[test]
    fn wall_mutation_invalidates_the_navmesh_cache() {
        let mut ecs = SceneEcs::from_documents(vec![doc(10, None, "scene")], 0);
        let scene = Uuid::from_u128(10);
        let walls = ecs.move_walls(scene, None);
        let a = ecs.navmesh_for(scene, 0.4, &walls).expect("navmesh builds");
        ecs.apply_op(&Operation::Create {
            doc: entity_doc_eng(
                20,
                10,
                "wall",
                json!({ "seg": { "x1": 10.0, "y1": 0.0, "x2": 10.0, "y2": 50.0 },
                        "blocksMove": true, "blocksSight": false, "blocksLight": false }),
            ),
        });
        let walls = ecs.move_walls(scene, None);
        let b = ecs
            .navmesh_for(scene, 0.4, &walls)
            .expect("navmesh rebuilds");
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
        let a = ecs.navmesh_for(scene, 0.4, &[]).expect("navmesh builds");
        ecs.apply_op(&Operation::Update {
            doc_id: scene,
            changes: vec![crate::data::command::FieldChange {
                remove: false,
                path: "/engine/bounds".into(),
                old: json!(null),
                new: json!({ "width": 40, "height": 40 }),
            }],
        });
        let b = ecs.navmesh_for(scene, 0.4, &[]).expect("navmesh rebuilds");
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
            RouteRequester {
                user: Uuid::from_u128(1),
                is_gm: true,
                explored: None,
            },
            scene,
            (50.0, 50.0),
            &[(250.0, 50.0)],
            0.1,
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
                RouteRequester {
user: Uuid::from_u128(1),
is_gm: true,
explored: // GM: unrestricted mask
                None,
},
                Uuid::from_u128(10),
                (50.0, 50.0),
                &[(950.0, 50.0)],
                0.1,
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
                RouteRequester {
user: Uuid::from_u128(1),
is_gm: true,
explored: // GM: unrestricted mask
                None,
},
                Uuid::from_u128(10),
                (50.0, 50.0),
                &[(50.0, 50.0)],
                0.1,
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

    /// A rect region's corners in scene units, ordered as the `points` array the
    /// `"rect"` shape expects.
    struct RegionRect {
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
    }

    fn region_doc_top(
        id: u128,
        parent: u128,
        behavior: &str,
        cost: f64,
        rect: RegionRect,
    ) -> Document {
        entity_doc_eng(
            id,
            parent,
            "region",
            json!({ "shape": { "kind": "rect", "points": [rect.x0, rect.y0, rect.x1, rect.y1] },
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
            12,
            10,
            "terrain",
            5.0,
            RegionRect {
                x0: 100.0,
                y0: 0.0,
                x1: 200.0,
                y1: 100.0,
            },
        ));
        let mut ecs = SceneEcs::from_documents(docs, 0);
        ecs.set_world_settings_for_test(continuous_world_settings());
        let out = ecs
            .pathfind(
                RouteRequester {
                    user: Uuid::from_u128(1),
                    is_gm: true,
                    explored: None,
                },
                Uuid::from_u128(10),
                (50.0, 50.0),
                &[(250.0, 50.0)],
                0.1,
            )
            .expect("weighted continuous route");
        // Tight pin (not a loose range): the forced-Euclidean detour is exactly 2 diagonal steps
        // (each √2 cells) around the mult-5 cell, so the cost is 2·√2·cell = ~282.84 scene units. A
        // loose bound here would silently pass a regression to the world diagonal rule (Chebyshev
        // diagonals cost 1 → 200 units) — that reversion is precisely the forced-Euclidean gap
        // this pin guards, so the expected value must be the Euclidean one, epsilon-tight.
        let expected = 2.0 * std::f64::consts::SQRT_2 * 100.0;
        assert!(
            (out.cost - expected).abs() < 0.5,
            "forced-Euclidean detour cost is 2·√2·cell ≈ {expected:.3} scene units, got {}",
            out.cost
        );
        assert!(
            out.path.iter().any(|p| p.1 > 90.0),
            "route bends off the y=50 line to avoid the terrain: {:?}",
            out.path
        );
    }

    /// Hex + continuous: grid kind and movement model are INDEPENDENT axes, so this scene is
    /// reachable through the ordinary authoring path (`resolve_grid_shape` keys only on
    /// `grid.kind`; `pathfind` dispatches only on `movement_model`).
    fn hex_continuous_scene_docs() -> Vec<crate::data::document::Document> {
        vec![entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "hex", "size": HEX_FIXTURE_SIZE }, "background": null,
                    "vision": { "movementModel": "continuous" } }),
        )]
    }

    #[test]
    fn pathfind_hex_continuous_arrest_truncates_at_the_axial_hex_not_the_square_cell() {
        // Call-site wiring proof for `navmesh::truncate_at_arrest`: `pathfind` must hand the
        // continuous engine the SAME `resolve_grid_shape`-derived shape `region_field` rasterized
        // the arrest region with. Arrest-only ⇒ `has_terrain_or_impassable()` is false ⇒ the pure
        // polyanya branch. Route runs along the r=1 hex row from hex (0,1) to hex (4,1); the arrest
        // region covers ONLY hex (3,1) (center x ≈303.1), whose entry boundary from (2,1) is
        // x ≈259.8. Reading the same axial key (3,1) as a SQUARE cell would place it at
        // x∈[150,200) — a different location — cutting the preview roughly a full hex early.
        let g = grid_shape::HexGrid {
            size: HEX_FIXTURE_SIZE,
        };
        let mut docs = hex_continuous_scene_docs();
        docs.push(region_doc_top(
            12,
            10,
            "arrest",
            1.0,
            RegionRect {
                x0: 285.0,
                y0: 55.0,
                x1: 320.0,
                y1: 95.0,
            },
        ));
        let mut ecs = SceneEcs::from_documents(docs, 0);
        ecs.set_world_settings_for_test(continuous_world_settings());
        // Fixture guard: exactly one hex arrests, and it is the axial cell the assertions name.
        let field = ecs
            .region_field(Uuid::from_u128(10), None)
            .expect("scene exists");
        assert!(field.is_arrest((3, 1)), "fixture: arrest is on axial (3,1)");
        assert!(
            !field.is_arrest((2, 1)) && !field.is_arrest((4, 1)),
            "fixture: exactly one hex arrests"
        );

        let out = ecs
            .pathfind(
                RouteRequester {
                    user: Uuid::from_u128(1),
                    is_gm: true,
                    explored: None,
                },
                Uuid::from_u128(10),
                g.cell_center((0, 1)),
                &[g.cell_center((4, 1))],
                0.1,
            )
            .expect("hex continuous route");
        assert!(out.arrested, "the arrest hex truncates the preview");
        let last = *out.path.last().unwrap();
        assert_eq!(
            g.cell_of(last),
            (3, 1),
            "truncation lands on the arrest hex itself, last = {last:?}"
        );
        assert!(
            last.0 > 259.8,
            "truncation is at the hex (2,1)/(3,1) boundary, not the square cell (3,1) at \
             x∈[150,200), last x = {}",
            last.0
        );
    }

    #[test]
    fn pathfind_continuous_no_region_is_a_straight_polyanya_route() {
        // Same scene WITHOUT a region: the pure polyanya path is taken — a straight 200px route.
        let mut ecs = SceneEcs::from_documents(continuous_scene_docs(), 0);
        ecs.set_world_settings_for_test(continuous_world_settings());
        let out = ecs
            .pathfind(
                RouteRequester {
                    user: Uuid::from_u128(1),
                    is_gm: true,
                    explored: None,
                },
                Uuid::from_u128(10),
                (50.0, 50.0),
                &[(250.0, 50.0)],
                0.1,
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
            RegionRect {
                x0: 100.0,
                y0: 0.0,
                x1: 200.0,
                y1: 300.0,
            },
        ));
        let mut ecs = SceneEcs::from_documents(docs, 0);
        ecs.set_world_settings_for_test(continuous_world_settings());
        let out = ecs
            .pathfind(
                RouteRequester {
                    user: Uuid::from_u128(1),
                    is_gm: true,
                    explored: None,
                },
                Uuid::from_u128(10),
                (50.0, 50.0),
                &[(250.0, 350.0)],
                0.1,
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
        let mut secret = region_doc_top(
            12,
            10,
            "terrain",
            5.0,
            RegionRect {
                x0: 100.0,
                y0: 0.0,
                x1: 200.0,
                y1: 100.0,
            },
        );
        // Mark the region gm_only via the SAME `/engine` property-visibility override
        // `region_field`'s per-requester filter checks
        // (`move_exec::authoritative_field_springs_a_secret_region_a_player_was_routed_through`
        // uses the identical convention for its own gm_only region fixture).
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
                RouteRequester {
                    user: player,
                    is_gm: false,
                    explored: None,
                },
                Uuid::from_u128(10),
                (50.0, 50.0),
                &[(250.0, 50.0)],
                0.1,
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
                RouteRequester {
                    user: Uuid::from_u128(1),
                    is_gm: true,
                    explored: None,
                },
                Uuid::from_u128(10),
                (50.0, 50.0),
                &[(250.0, 50.0)],
                0.1,
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
            .pathfind(
                RouteRequester {
                    user,
                    is_gm: false,
                    explored: None,
                },
                scene,
                (50.0, 50.0),
                &[far_goal],
                0.1,
            )
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
        // The test above only drives the PURE-POLYANYA
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
        let terrain = region_doc_top(
            12,
            10,
            "terrain",
            5.0,
            RegionRect {
                x0: 5000.0,
                y0: 5000.0,
                x1: 5100.0,
                y1: 5100.0,
            },
        );
        let ecs = SceneEcs::from_documents(vec![scene, tok, light, terrain], 0);
        let scene_id = Uuid::from_u128(10);
        let cell = 100.0;

        let lenient = ecs.resolve_scene(scene_id).partial_cell_leniency;
        let mask = ecs.visible_cells(user, scene_id, lenient);
        assert!(!mask.is_empty(), "the lit token has a non-empty mask");
        assert!(
            ecs.region_field(scene_id, Some(user))
                .expect("scene exists")
                .has_terrain_or_impassable(),
            "the terrain region flips the Continuous dispatch to the weighted sub-path"
        );

        // Near goal, still within the small visible mask: the weighted route must succeed and
        // stay entirely inside the mask (the grid A* mask check IS the enforcement mechanism for
        // this sub-path, so a route can never even be found outside the mask).
        let near_goal = (150.0, 50.0);
        let near = ecs
            .pathfind(
                RouteRequester {
                    user,
                    is_gm: false,
                    explored: None,
                },
                scene_id,
                (50.0, 50.0),
                &[near_goal],
                0.1,
            )
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
        let far = ecs.pathfind(
            RouteRequester {
                user,
                is_gm: false,
                explored: None,
            },
            scene_id,
            (50.0, 50.0),
            &[far_goal],
            0.1,
        );
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
        let mut secret = region_doc_top(
            12,
            10,
            "arrest",
            1.0,
            RegionRect {
                x0: 200.0,
                y0: 0.0,
                x1: 300.0,
                y1: 100.0,
            },
        );
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
                RouteRequester {
                    user: player,
                    is_gm: false,
                    explored: None,
                },
                scene,
                (50.0, 50.0),
                &[(450.0, 50.0)],
                0.1,
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
                RouteRequester {
                    user: Uuid::from_u128(1),
                    is_gm: true,
                    explored: None,
                },
                scene,
                (50.0, 50.0),
                &[(450.0, 50.0)],
                0.1,
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
            crate::scene::move_exec::MoveGateInputs {
                scene,
                restriction: MovementRestriction::Unrestricted,
                visible: &visible,
                cell: 100.0,
            },
            token,
            &p.path,
            false,
            0.4,
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

    /// A scene whose corridor from (50,50) to (250,50) is crossed by a FINITE `gm_only`
    /// blocksMove wall at x=150 spanning y∈[0,100]. Continuous movement model (so the router
    /// goes through `navmesh_for`'s per-requester obstacle set — the mechanism this fixture
    /// exercises). `movement_restriction: unrestricted` so the visibility mask is not the
    /// variable under test. The authored 4x4 block of cells at cell 100 gives a 400x400 world
    /// rectangle, wide enough that a detour around the wall's y=100
    /// endpoint exists. Returns `(ecs, scene, user, token)`; `owner_is_gm` only selects which
    /// fixed user id is returned (routing GM-ness is the separate `is_gm` argument callers pass
    /// to `pathfind` directly — this fixture places no GM/player distinction on the token or
    /// wall doc itself, mirroring `scene_with_public_and_secret_move_walls`).
    fn scene_with_secret_wall_between_two_cells(owner_is_gm: bool) -> (SceneEcs, Uuid, Uuid, Uuid) {
        let scene = entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                    "bounds": { "width": 4.0, "height": 4.0 },
                    "vision": { "movementModel": "continuous" } }),
        );
        let scene_id = Uuid::from_u128(10);
        let user = Uuid::from_u128(if owner_is_gm { 1 } else { 2 });
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 50.0, "y": 50.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        let token_id = Uuid::from_u128(11);
        let mut wall = wall_doc_eng(scene_id, (150.0, 0.0), (150.0, 100.0));
        wall.permissions
            .property_overrides
            .insert("/engine".into(), crate::data::document::Visibility::GmOnly);
        let mut ecs = SceneEcs::from_documents(vec![scene, tok, wall], 0);
        ecs.set_world_settings_for_test(continuous_world_settings());
        (ecs, scene_id, user, token_id)
    }

    #[test]
    fn non_gm_route_crosses_a_gm_only_wall_that_springs_at_execution() {
        // The router cannot see the secret wall, so it routes straight through it; the executor reads
        // the authoritative set and stops there. Same spring-at-execution shape as a secret region.
        let (ecs, scene, player, token) = scene_with_secret_wall_between_two_cells(false);
        let out = ecs
            .pathfind(
                RouteRequester {
                    user: player,
                    is_gm: false,
                    explored: None,
                },
                scene,
                (50.0, 50.0),
                &[(250.0, 50.0)],
                0.4,
            )
            .expect("the player's route ignores a wall it cannot see");
        assert!(
            out.path.len() >= 2,
            "a route is produced despite the secret wall across it"
        );

        let visible = ecs.visible_cells(player, scene, false);
        let exec = crate::scene::move_exec::execute_move(
            &ecs,
            crate::scene::move_exec::MoveGateInputs {
                scene,
                restriction: MovementRestriction::Unrestricted,
                visible: &visible,
                cell: 100.0,
            },
            token,
            &out.path,
            false,
            0.4,
        )
        .expect("execution is admissible");
        assert!(
            exec.truncated,
            "the secret wall springs at execution and truncates the move"
        );
    }

    #[test]
    fn gm_route_does_not_cross_a_gm_only_wall() {
        // A GM passes viewer=None, so the secret wall IS in their routing set and no route SEGMENT
        // may cross the wall segment. Asserted structurally via segments_cross — NOT by testing
        // distance from the wall's x-line, which a legitimate detour around a finite wall's endpoint
        // necessarily crosses (and which, at cell size 100, every column-1 cell center sits exactly on).
        let (ecs, scene, gm, _token) = scene_with_secret_wall_between_two_cells(true);
        let out = ecs
            .pathfind(
                RouteRequester {
                    user: gm,
                    is_gm: true,
                    explored: None,
                },
                scene,
                (50.0, 50.0),
                &[(250.0, 50.0)],
                0.4,
            )
            .expect("a GM route exists (bounds admit a detour around the wall's endpoint)");
        let wall = ((150.0, 0.0), (150.0, 100.0));
        for seg in out.path.windows(2) {
            assert!(
                !crate::scene::segments_cross(seg[0], seg[1], wall.0, wall.1),
                "no GM route segment crosses the wall it can see: {:?}",
                seg
            );
        }
    }

    #[test]
    fn pathfind_grid_stepped_scene_is_byte_for_byte_unchanged() {
        // Same fixture/assertions as the existing `pathfind_gm_unconstrained_routes_without_a_mask`
        // test, proving the default (grid-stepped) dispatch branch is unaffected by the
        // continuous-engine dispatch.
        let (ecs, _user, scene) = scene_with_lit_player_token();
        let r = ecs.pathfind(
            RouteRequester {
                user: Uuid::from_u128(1),
                is_gm: true,
                explored: None,
            },
            scene,
            (50.0, 50.0),
            &[(250.0, 50.0)],
            0.1,
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
            RouteRequester {
                user,
                is_gm: false,
                explored: None,
            },
            scene,
            (50.0, 50.0),
            &[(5000.0, 5000.0)],
            0.1,
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
        let grid = crate::scene::grid_shape::SquareGrid {
            cell,
            rule: crate::scene::pathfinding::DiagonalRule::Chebyshev,
        };
        explored.mark_polygons(
            &[vec![0.0, 0.0, 4.0 * cell, 0.0, 4.0 * cell, cell, 0.0, cell]],
            &grid,
            cell,
        );
        let r = ecs.pathfind(
            RouteRequester {
                user,
                is_gm: false,
                explored: Some(&explored),
            },
            scene,
            (50.0, 50.0),
            &[(350.0, 50.0)],
            0.1,
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

    // --- wall-less scene full intrascene vision ---

    /// A wall-less scene authored as a 5x5 block of cells at cell 100 — a 500x500 world
    /// rectangle — must reveal its own full extent, not a small `VISION_BOUND_MARGIN` box around
    /// the viewpoint.
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
                    "bounds": { "width": 5.0, "height": 5.0 } }),
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

    /// Each scene's vision bound uses ITS OWN extent, never a neighbour's. The viewpoint loop
    /// spans every scene the user owns a token in, so an extent resolved once above that loop
    /// would measure one scene's bound against another scene's rectangle — and the memoisation
    /// that avoids re-scanning the entity table per viewpoint is exactly where that mistake fits.
    #[test]
    fn each_scenes_vision_bound_uses_its_own_extent_not_a_neighbours() {
        // Two wall-less scenes with deliberately mismatched extents: scene 10 is a 5x5 block at
        // cell 100 (a 500-unit square), scene 20 a 1x1 block at cell 100 (a 100-unit square). The
        // probe points are read from each scene's OWN resolved extent, so neither is a literal.
        // Discrimination: with a single hoisted extent both scenes answer with the same rectangle,
        // so whichever scene is not the source of that value fails one of its two assertions —
        // the small scene reveals a point beyond its own extent, or the large scene stops short of
        // one inside its own.
        let user = Uuid::from_u128(7);
        let mut docs = Vec::new();
        for (scene_id, token_id, block) in [(10u128, 11u128, 5.0_f64), (20, 21, 1.0)] {
            let mut tok = entity_doc_eng(
                token_id,
                scene_id,
                "token",
                json!({ "x": 5.0, "y": 5.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
            );
            tok.owner = Some(user);
            docs.push(entity_doc_top_eng(
                scene_id,
                "scene",
                json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                        "bounds": { "width": block, "height": block } }),
            ));
            docs.push(tok);
        }
        let ecs = SceneEcs::from_documents(docs, 0);
        let polys = ecs.player_vision_polygons(user);
        assert_eq!(
            polys.len(),
            2,
            "one polygon per scene the user owns a token in"
        );

        let extents: Vec<(f64, f64)> = [10u128, 20]
            .iter()
            .map(|&s| ecs.scene_world_extent(Uuid::from_u128(s)))
            .collect();
        assert!(
            extents[0].0 > extents[1].0,
            "fixture: the two scenes must have different extents, got {extents:?}"
        );

        for (i, scene_id) in [10u128, 20].iter().enumerate() {
            let (_, poly) = polys
                .iter()
                .find(|(sid, _)| *sid == Uuid::from_u128(*scene_id))
                .expect("scene present");
            let (ex, ey) = extents[i];
            // Just inside this scene's own extent, on the diagonal from the viewpoint.
            let inside = (ex - 10.0, ey - 10.0);
            assert!(
                vision::point_in_poly(poly, inside),
                "scene {scene_id} must reveal {inside:?}, inside its own extent {:?}",
                extents[i]
            );
            // Beyond this scene's own extent AND beyond the wall-less margin box around (5,5).
            let outside = (
                ex + VISION_BOUND_MARGIN + 10.0,
                ey + VISION_BOUND_MARGIN + 10.0,
            );
            assert!(
                !vision::point_in_poly(poly, outside),
                "scene {scene_id} must not reveal {outside:?}, beyond its own extent {:?}",
                extents[i]
            );
        }
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
                    "bounds": { "width": 5.0, "height": 5.0 } }),
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
                    "bounds": { "width": 5.0, "height": 5.0 } }),
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

    // --- source_los_poly wall-less degenerate box (player_lit_mask/visible_cells) ---

    /// A wall-less scene authored as a 5x5 block of cells at cell 100 — a 500x500 world
    /// rectangle — with all-bright lighting (isolates the bound-box defect from
    /// illumination), `losRestriction` off (so `source_los_poly` takes the plain-rectangle branch,
    /// the branch susceptible to the bound-box defect). Cell (4,4) — center (450,450) — lies within the
    /// scene's authored extent but strictly outside a degenerate
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
                    "bounds": { "width": 5.0, "height": 5.0 } }),
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
    /// bounds, not a degenerate box around the viewpoint — the same bound-box defect class fixed in
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
    /// wall-less scene — closing the "two/three vision paths diverge" defect class, generalized to
    /// this third path.
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

    /// Pins the exact movement-gate cell set for an open all-bright scene. `accumulate_visible_cells`
    /// computes each candidate cell's CENTER via `GridShape::cell_center`; `SquareGrid::cell_center`
    /// equals the hardcoded `((i+0.5)*cell,(j+0.5)*cell)` square formula, so a regression to
    /// non-square center math in that function diverges from this frozen set immediately, without
    /// depending on the broader frozen-fixture parity battery. Reuses
    /// `wall_less_large_scene_all_bright` (one owned token, no walls, all-bright, a 5x5
    /// block at cell 100).
    #[test]
    fn accumulate_visible_cells_routes_through_grid_shape_cell_center_not_hardcoded() {
        let (ecs, user, scene_id) = wall_less_large_scene_all_bright();
        let got = ecs.visible_cells(user, scene_id, false);
        let expected: std::collections::BTreeSet<(i32, i32)> = (-1..=4)
            .flat_map(|i| (-1..=4).map(move |j| (i, j)))
            .collect();
        assert_eq!(got, expected);
    }

    /// Pins the exact secrecy-egress cell set `player_lit_mask` emits for an open all-bright scene.
    /// `player_lit_mask` computes each candidate cell's CENTER via `GridShape::cell_center`;
    /// `SquareGrid::cell_center` equals the hardcoded `((i+0.5)*cell,(j+0.5)*cell)` square formula,
    /// so a regression to non-square center math in that function diverges from this frozen set
    /// immediately. Companion to `accumulate_visible_cells_routes_through_grid_shape_cell_center_not_hardcoded`,
    /// applied to the OTHER (separate) secrecy-egress call site; the pinned set matches the strict
    /// movement-gate set (`visible_cells` strict ≡ `player_lit_mask` cells). Reuses
    /// `wall_less_large_scene_all_bright` (one owned token, no walls, all-bright, a 5x5
    /// block at cell 100).
    #[test]
    fn player_lit_mask_routes_through_grid_shape_cell_center_not_hardcoded() {
        let (ecs, user, scene_id) = wall_less_large_scene_all_bright();
        let got: std::collections::BTreeSet<(i32, i32)> = ecs
            .player_lit_mask(user)
            .into_iter()
            .filter(|s| s.scene == scene_id)
            .flat_map(|s| s.cells.into_iter().map(|(i, j, _b, _t, _h)| (i, j)))
            .collect();
        let expected: std::collections::BTreeSet<(i32, i32)> = (-1..=4)
            .flat_map(|i| (-1..=4).map(move |j| (i, j)))
            .collect();
        assert_eq!(got, expected);
    }

    /// A wall-less pointy-top hex scene (size 50), all-bright, LOS off, one owned instanced token
    /// at hex (0,0) = pixel (0,0) with unlimited "normal" vision.
    ///
    /// The authored block is 3.2 x 3.0 hexes, which is fractional because a hex block's world
    /// rectangle is a shear-dependent function of the block rather than a per-axis product.
    /// `HexGrid { size: 50 }::world_extent((3.2, 3.0))` evaluates
    /// `(√3·50·(2.2 + 1.0) + √3/2·50, 50·1.5·2 + 50)` to `(320.429…, 200)`, so `source_los_poly`
    /// is the rectangle `[-100, 320.429…] x [-100, 200]` — `bound_for_scene` takes
    /// `min(0-100, 0) = -100` on each low edge and `max(0+100, extent) = extent` on each high edge.
    fn hex_open_scene() -> (SceneEcs, Uuid, Uuid) {
        let user = Uuid::from_u128(7);
        let scene_id = Uuid::from_u128(10);
        let mut tok = entity_doc_eng(
            11,
            10,
            "token",
            json!({ "x": 0.0, "y": 0.0, "w": 100.0, "h": 100.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        let scene = entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "hex", "size": HEX_FIXTURE_SIZE }, "background": null,
                    "bounds": { "width": 3.2, "height": 3.0 } }),
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

    /// REJECT direction on a hex scene: a hex cell whose HEX CENTER falls outside the vision mask
    /// is excluded from `visible_cells`. Hex (2,0) center (~173.2, 0) is inside the
    /// [-100, 320.429…] x [-100, 200] LOS rectangle and visible; hex (5,0) center (~433.0, 0) is
    /// well outside (x > 320.429…) — and its nearest (left) vertices at x ~389.7 are also outside —
    /// so it is excluded under BOTH strict and lenient sampling. Guards that the hex candidate
    /// enumeration cannot admit an out-of-mask hex cell.
    #[test]
    fn visible_cells_hex_excludes_cell_whose_center_is_outside_the_mask() {
        let (ecs, user, scene) = hex_open_scene();
        let strict = ecs.visible_cells(user, scene, false);
        assert!(
            strict.contains(&(2, 0)),
            "hex (2,0) center is inside the LOS rectangle"
        );
        assert!(
            !strict.contains(&(5, 0)),
            "hex (5,0) center is outside the mask -> excluded"
        );
        // Even leniency (corner sampling) cannot pull (5,0) in: its nearest vertex is still outside.
        let lenient = ecs.visible_cells(user, scene, true);
        assert!(
            !lenient.contains(&(5, 0)),
            "hex (5,0) has no vertex inside the mask either"
        );
    }

    /// Leniency on a hex scene samples the SIX hex vertices (`GridShape::cell_vertices`), not four
    /// square corners. Hex (4,0) center (~346.4, 0) is just outside the
    /// [-100, 320.429…] x [-100, 200] LOS rectangle (x > 320.429…), so strict excludes it; its left
    /// vertices (~303.1, ±25) are inside, so lenient includes it. The strict->lenient flip proves
    /// the hex corner geometry is wired.
    #[test]
    fn visible_cells_hex_lenient_includes_cell_whose_vertex_clips_the_mask() {
        let (ecs, user, scene) = hex_open_scene();
        let strict = ecs.visible_cells(user, scene, false);
        assert!(
            !strict.contains(&(4, 0)),
            "hex (4,0) center is outside -> strict excludes"
        );
        let lenient = ecs.visible_cells(user, scene, true);
        assert!(
            lenient.contains(&(4, 0)),
            "hex (4,0) vertex clips the mask -> lenient includes"
        );
    }

    /// End-to-end composition of the hex leniency path: `GridShape::cell_vertices` (six hex corners)
    /// widens `visible_cells`, and the widened mask is what the authoritative executor gates
    /// against. The SAME move into hex (4,0) — whose center is outside the LOS rectangle but whose
    /// left vertices clip it — completes under leniency and truncates under strict center sampling.
    /// This is the composed behavior no per-site test covers: leniency is only meaningful if the
    /// executor consumes the widened mask.
    #[test]
    fn hex_lenient_mask_lets_the_executor_enter_a_cell_the_strict_mask_stops_at() {
        let (ecs, user, scene) = hex_open_scene();
        let cell = HEX_FIXTURE_SIZE;
        let token = Uuid::from_u128(11);
        let grid = ecs.resolve_grid_shape(scene, cell);
        let dest = grid.cell_center((4, 0));

        let lenient_mask = ecs.visible_cells(user, scene, true);
        let out = crate::scene::move_exec::execute_move(
            &ecs,
            crate::scene::move_exec::MoveGateInputs {
                scene,
                restriction: MovementRestriction::Visible,
                visible: &lenient_mask,
                cell,
            },
            token,
            &[(0.0, 0.0), dest],
            false,
            0.4,
        )
        .expect("a token move on a hex scene executes");
        assert!(
            !out.truncated,
            "the lenient mask admits every traversed hex cell"
        );
        assert_eq!(grid.cell_of(out.stop), (4, 0), "the move reaches hex (4,0)");

        let strict_mask = ecs.visible_cells(user, scene, false);
        let out = crate::scene::move_exec::execute_move(
            &ecs,
            crate::scene::move_exec::MoveGateInputs {
                scene,
                restriction: MovementRestriction::Visible,
                visible: &strict_mask,
                cell,
            },
            token,
            &[(0.0, 0.0), dest],
            false,
            0.4,
        )
        .expect("a token move on a hex scene executes");
        assert!(out.truncated, "strict center sampling excludes hex (4,0)");
        assert_ne!(
            grid.cell_of(out.stop),
            (4, 0),
            "the strict move never enters hex (4,0)"
        );
    }

    /// REQUIREMENT this scene has to satisfy, which is what every test reading it depends on: a
    /// single source's candidate scan must exceed `MAX_CELLS_PER_POLYGON`. The width supplies that
    /// over-cap product with an enormous margin — the authored block is measured in grid units
    /// (cells), which `GridShape::world_extent` multiplies by the cell size, so the scan clears
    /// the cap by two further orders of magnitude than the authored number alone suggests. A scan
    /// under the cap never engages the clamp, and the assertions would then hold for a reason they
    /// do not name. The height is small so the CLAMPED scan is a few thousand cells and the tests
    /// run in a unit suite. Wall-less, all-bright, LOS off, one owned token at the origin cell, so
    /// the whole scan is a single source's.
    fn over_cap_scan_scene() -> (SceneEcs, Uuid, Uuid) {
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
                    "bounds": { "width": 200_000_000.0, "height": 5.0 } }),
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

    #[test]
    fn an_over_cap_visibility_scan_yields_a_bounded_mask_not_an_empty_one() {
        // Under `MovementRestriction::Visible` an empty mask refuses every move, so the
        // over-cap outcome must be a bounded neighbourhood of the source rather than nothing.
        // Discrimination: fails if `accumulate_visible_cells` hands the unclamped bbox to
        // `cells_in_bounds`, because the cap then returns `None` and the source is skipped,
        // leaving the mask empty. It cannot pass vacuously: the second assertion requires the
        // mask to STOP somewhere, so a scan that ignored the cap entirely also fails.
        let (ecs, user, scene) = over_cap_scan_scene();
        let mask = ecs.visible_cells(user, scene, false);
        assert!(mask.contains(&(0, 0)), "the source's own cell is visible");
        let outside = crate::scene::explored::SCAN_WINDOW_HALF_CELLS as i32 + 10;
        assert!(
            !mask.contains(&(outside, 0)),
            "a cell beyond the scan window is not in the mask"
        );
    }

    #[test]
    fn an_over_cap_lit_mask_scan_yields_a_bounded_cell_set_not_an_empty_one() {
        // The egress half of the same scan, which is a separate call site and would otherwise be
        // converted independently. Discrimination: identical to the mask test, applied to
        // `player_lit_mask`'s own scan.
        let (ecs, user, scene) = over_cap_scan_scene();
        let cells: std::collections::BTreeSet<(i32, i32)> = ecs
            .player_lit_mask(user)
            .into_iter()
            .filter(|s| s.scene == scene)
            .flat_map(|s| s.cells.into_iter().map(|(i, j, _b, _t, _h)| (i, j)))
            .collect();
        assert!(cells.contains(&(0, 0)), "the source's own cell is lit");
        let outside = crate::scene::explored::SCAN_WINDOW_HALF_CELLS as i32 + 10;
        assert!(
            !cells.contains(&(outside, 0)),
            "a cell beyond the scan window is not shipped"
        );
    }

    /// A scene sized so the STRICT (unpadded) candidate scan's own span sits exactly at
    /// `explored::MAX_CELLS_PER_POLYGON` (2000×2000 cells, product 4,000,000, returned unclamped
    /// by itself) while the LENIENT (one-cell-padded) scan's own span exceeds it (2002×2002,
    /// product 4,008,004, clamped by itself) — the band where the two invocations' own spans
    /// straddle the cap on either side of it. Wall-less, all-bright, LOS off, one owned token at
    /// `(100, 100)`. The grid size is 1, so a `1999 × 1999` authored block converts to a
    /// `1999 × 1999` world rectangle — the one grid size at which a block measured in grid units
    /// and its world span coincide, which is what keeps the two candidate spans this doc names
    /// exactly at the cap.
    /// `source_los_poly`'s bound rectangle is therefore exactly `[0, 0]–[1999, 1999]`
    /// (`VISION_BOUND_MARGIN` cancels against the token's own offset on the low edge; the scene's
    /// extent dominates the high edge).
    fn strict_lenient_clamp_band_scene() -> (SceneEcs, Uuid, Uuid, Uuid) {
        let user = Uuid::from_u128(8);
        let scene_id = Uuid::from_u128(20);
        let token_id = Uuid::from_u128(21);
        let mut tok = entity_doc_eng(
            21,
            20,
            "token",
            json!({ "x": 100.0, "y": 100.0, "w": 1.0, "h": 1.0, "rotation": 0.0 }),
        );
        tok.owner = Some(user);
        let scene = entity_doc_top_eng(
            20,
            "scene",
            json!({ "grid": { "kind": "square", "size": 1 }, "background": null,
                    "bounds": { "width": 1999.0, "height": 1999.0 } }),
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
        (ecs, user, scene_id, token_id)
    }

    /// The scan box `strict_lenient_clamp_band_scene`'s single source produces, and the strict/
    /// lenient spans that box's candidate scan enumerates. Every input is READ from the fixture —
    /// the resolved scene settings, the scene's own grid size, the resolved grid shape, the
    /// token's own position, and `source_los_poly` itself — rather than restated as a literal, so
    /// a change to `VISION_BOUND_MARGIN`, the fixture's authored bounds, its token position, or
    /// its grid size changes what this computes too, instead of leaving it stale.
    fn strict_lenient_band_span(ecs: &SceneEcs, scene: Uuid, token: Uuid) -> (i64, i64) {
        let settings = ecs.resolve_scene(scene);
        let cell = *ecs
            .scene_grid_sizes()
            .get(&scene)
            .expect("the fixture's scene has a grid size");
        let grid = ecs.resolve_grid_shape(scene, cell);
        let vp = ecs
            .token_position(token)
            .expect("the fixture's token has a position");
        let walls = ecs.sight_walls(scene);
        let poly = source_los_poly(
            vp,
            &walls,
            settings.los_restriction,
            grid.world_extent(settings.bounds),
        );
        let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for &(x, y) in &poly {
            minx = minx.min(x);
            miny = miny.min(y);
            maxx = maxx.max(x);
            maxy = maxy.max(y);
        }
        let bbox = ((minx, miny), (maxx, maxy));
        let padded = crate::scene::explored::pad_box(bbox, cell);
        let strict_span = grid_shape::candidate_span(grid.cell_bounds(bbox.0, bbox.1, cell));
        let lenient_span = grid_shape::candidate_span(grid.cell_bounds(padded.0, padded.1, cell));
        (strict_span, lenient_span)
    }

    #[test]
    fn lenient_visibility_scan_stays_a_superset_of_strict_at_the_clamp_boundary() {
        // The strict scan's own span sits exactly at the cap; the lenient scan's own (padded)
        // span exceeds it. Discrimination: fails if the clamp decision for each invocation is
        // computed from that invocation's own box instead of a box shared across both — the
        // unclamped strict result then reaches a candidate column the clamped lenient result's
        // own (independently-decided) window never enumerates, and `is_subset` catches it.
        //
        // The fixture's whole value depends on actually landing in the band — `strict_lenient_band_span`
        // reads every input from the fixture itself rather than restating one, so a change to
        // `VISION_BOUND_MARGIN`, the authored bounds, the token position, or the grid size that
        // moves the scene out of the band fails the two assertions below instead of leaving them
        // vacuously true.
        let (ecs, user, scene, token) = strict_lenient_clamp_band_scene();
        let (strict_span, lenient_span) = strict_lenient_band_span(&ecs, scene, token);
        assert!(
            strict_span <= crate::scene::explored::MAX_CELLS_PER_POLYGON,
            "fixture: the strict span must sit at or under the cap ({strict_span})"
        );
        assert!(
            lenient_span > crate::scene::explored::MAX_CELLS_PER_POLYGON,
            "fixture: the padded span must exceed the cap ({lenient_span})"
        );
        let strict = ecs.visible_cells(user, scene, false);
        let lenient = ecs.visible_cells(user, scene, true);
        assert!(
            !strict.is_empty(),
            "the strict scan must reach at least one cell"
        );
        assert!(
            strict.is_subset(&lenient),
            "strict must never see a cell the lenient scan does not"
        );
        let outside = crate::scene::explored::SCAN_WINDOW_HALF_CELLS as i32 + 200;
        assert!(
            !strict.contains(&(outside, 0)),
            "a cell well beyond the window is not in the strict mask — proves the clamp binds"
        );
    }

    #[test]
    fn parity_holds_inside_the_clamp_band() {
        // `player_lit_mask` and the strict `visible_cells` scan must enumerate identical candidate
        // sets for the same source (`cell_visible`'s own doc states this as an invariant); pinned
        // specifically inside the clamp band, not only in the scenes outside it the other
        // `assert_strict_parity` call sites already cover.
        let (ecs, user, scene, _token) = strict_lenient_clamp_band_scene();
        assert_strict_parity(&ecs, user, scene);
    }

    #[test]
    fn scene_world_extent_agrees_with_the_shapes_own_conversion() {
        // Two call shapes exist — the ECS helper for callers holding only a scene id, and the
        // inline `grid.world_extent(settings.bounds)` for callers already holding both — and a
        // divergence between them would fork the vision bound from the lit mask.
        // Discrimination: fails if either shape starts reading a different bounds value or a
        // different shape, which is the only way the two can disagree.
        let (ecs, _user, scene) = hex_open_scene();
        // The cell is read from the grid-size lookup, not restated: that is the resolution the
        // production sites perform, so an inline arm using a literal would not be the arm that
        // runs in production.
        let cell = *ecs
            .scene_grid_sizes()
            .get(&scene)
            .expect("the fixture's scene has a grid size");
        let inline = ecs
            .resolve_grid_shape(scene, cell)
            .world_extent(ecs.resolve_scene(scene).bounds);
        assert_eq!(ecs.scene_world_extent(scene), inline);
    }

    #[test]
    fn hex_continuous_routes_along_axial_row_zero_including_the_mesh_corner() {
        // Every hex in axial row `r = 0` has centre `y` exactly `0`, which is the triangulated
        // rectangle's bottom EDGE, and `cell_center((0,0))` is the corner vertex itself. Those
        // centres are on-mesh only because the mesh's point-in-polygon test admits an
        // exactly-on-boundary point — a containment convention of the routing library, not of this
        // codebase. Pinned rather than assumed: without this, a change to that convention would
        // make an entire authored hex row unroutable with nothing in the tree failing.
        // Discrimination: the endpoints are `cell_center` values with `y == 0.0` asserted, so the
        // test cannot drift onto an interior row and keep passing; and the cost is bounded on both
        // sides by the straight-line distance, so a route that detoured off the edge fails too.
        let g = grid_shape::HexGrid {
            size: HEX_FIXTURE_SIZE,
        };
        let docs = vec![entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "hex", "size": g.size }, "background": null,
                    "bounds": { "width": 20.0, "height": 20.0 },
                    "vision": { "movementModel": "continuous" } }),
        )];
        let mut ecs = SceneEcs::from_documents(docs, 0);
        ecs.set_world_settings_for_test(continuous_world_settings());
        let corner = g.cell_center((0, 0));
        let far = g.cell_center((5, 0));
        assert_eq!(
            (corner.1, far.1),
            (0.0, 0.0),
            "fixture: both endpoints must sit on the rectangle's bottom edge"
        );
        assert_eq!(
            corner,
            (0.0, 0.0),
            "fixture: the origin hex IS the mesh corner"
        );
        let straight = far.0 - corner.0;
        for (from, to, label) in [
            (corner, far, "the corner vertex outward"),
            (far, corner, "inward to the corner vertex"),
        ] {
            let out = ecs
                .pathfind(
                    RouteRequester {
                        user: Uuid::from_u128(1),
                        is_gm: true,
                        explored: None,
                    },
                    Uuid::from_u128(10),
                    from,
                    &[to],
                    0.1,
                )
                .unwrap_or_else(|e| panic!("routing {label} along row 0 must succeed, got {e:?}"));
            assert!(
                out.cost >= straight * 0.99 && out.cost <= straight * 1.01,
                "routing {label} must cost the straight-line distance {straight}, got {}",
                out.cost
            );
        }
    }

    #[test]
    fn hex_continuous_navmesh_spans_the_authored_play_area() {
        // A hex scene authored 20 × 20 grid units at size 50 must route to a hex near the far
        // edge of that authored area. Hex (18,1)'s centre sits well beyond the product of the
        // authored bound and the cell size, so a rectangle built from that product excludes the
        // destination and the route reports unreachable.
        // Discrimination: fails if `world_extent` returns the bounds×size product on hex, because
        // the destination is derived from `cell_center`, not from the extent.
        let g = grid_shape::HexGrid {
            size: HEX_FIXTURE_SIZE,
        };
        let docs = vec![entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "hex", "size": g.size }, "background": null,
                    "bounds": { "width": 20.0, "height": 20.0 },
                    "vision": { "movementModel": "continuous" } }),
        )];
        let mut ecs = SceneEcs::from_documents(docs, 0);
        ecs.set_world_settings_for_test(continuous_world_settings());
        let dest = g.cell_center((18, 1));
        assert!(
            dest.0 > 20.0 * 50.0,
            "fixture: the destination must sit beyond the bounds×size product ({}), got {}",
            20.0 * 50.0,
            dest.0
        );
        let out = ecs
            .pathfind(
                RouteRequester {
                    user: Uuid::from_u128(1),
                    is_gm: true,
                    explored: None,
                },
                Uuid::from_u128(10),
                g.cell_center((1, 1)),
                &[dest],
                0.1,
            )
            .expect("a hex cell inside the authored bounds must be routable");
        assert!(
            out.path.len() >= 2,
            "route must reach the destination, got {:?}",
            out.path
        );
    }

    #[test]
    fn hex_continuous_weighted_cost_is_reported_in_scene_units() {
        // A terrain region flips the continuous dispatch to the weighted grid sub-path, whose
        // cost is converted from cells to scene units. On hex one grid step is √3·size scene
        // units, so the reported cost must be at least the straight-line distance between the
        // endpoints; a conversion through the size itself cannot reach that.
        // Discrimination: the expectation is bounded below by the straight-line distance between
        // the two endpoints, computed from `cell_center`, not from the router's own output.
        let g = grid_shape::HexGrid {
            size: HEX_FIXTURE_SIZE,
        };
        let mut docs = vec![entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "hex", "size": g.size }, "background": null,
                    "bounds": { "width": 20.0, "height": 20.0 },
                    "vision": { "movementModel": "continuous" } }),
        )];
        // A terrain region well away from the route: present only to select the weighted path.
        docs.push(region_doc_top(
            13,
            10,
            "terrain",
            5.0,
            RegionRect {
                x0: 1200.0,
                y0: 600.0,
                x1: 1260.0,
                y1: 660.0,
            },
        ));
        let mut ecs = SceneEcs::from_documents(docs, 0);
        ecs.set_world_settings_for_test(continuous_world_settings());
        // The whole test is about the WEIGHTED sub-path, which only runs when the dispatch
        // predicate fires. Asserted rather than assumed: with an empty field the pure-polyanya
        // path runs instead and the cost below would be measuring a different function.
        let field = ecs
            .region_field(Uuid::from_u128(10), None)
            .expect("the fixture's scene resolves a region field");
        assert!(
            field.has_terrain_or_impassable(),
            "fixture: the terrain region must select the weighted sub-path"
        );
        let a = g.cell_center((1, 1));
        let b = g.cell_center((10, 1));
        let straight = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let out = ecs
            .pathfind(
                RouteRequester {
                    user: Uuid::from_u128(1),
                    is_gm: true,
                    explored: None,
                },
                Uuid::from_u128(10),
                a,
                &[b],
                0.1,
            )
            .expect("hex continuous weighted route");
        // Bounded on BOTH sides. The endpoints are nine collinear hex steps apart with no terrain
        // between them, so the true scene-unit cost is exactly the straight-line distance; a
        // lower bound alone also passes for any wrong-but-larger factor, `2·size` included.
        assert!(
            out.cost >= straight * 0.99 && out.cost <= straight * 1.01,
            "cost {} must equal the straight-line scene distance {straight}",
            out.cost
        );
    }

    #[test]
    fn a_degenerate_authored_grid_size_never_reaches_the_extent_conversion() {
        // Why the degenerate-`cell` refusal has no expression at `navmesh_for`: `scene_grid_sizes`
        // filters a non-positive authored size and substitutes the positive default, so the value
        // `navmesh_for` converts through `world_extent` is always positive and the resulting extent
        // is never degenerate. The refusal itself lives on `build_navmesh`'s extent parameter,
        // pinned by `navmesh::tests::degenerate_extent_fails_closed` at the level that value
        // enters.
        // Discrimination: fails if `scene_grid_sizes` ever starts passing a non-positive authored
        // size through, which would make a collapsed rectangle reachable from a scene document —
        // and the second assertion fails if the substituted size stops producing a usable mesh.
        let scene = entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 0.0 }, "background": null,
                    "bounds": { "width": 10.0, "height": 10.0 } }),
        );
        let ecs = SceneEcs::from_documents(vec![scene], 0);
        let cell = ecs
            .scene_grid_sizes()
            .get(&Uuid::from_u128(10))
            .copied()
            .expect("a live scene always carries a grid size");
        assert!(
            cell > 0.0,
            "a non-positive authored grid size must be hardened before it converts, got {cell}"
        );
        let (ex, ey) = ecs.scene_world_extent(Uuid::from_u128(10));
        assert!(
            ex > 0.0 && ey > 0.0,
            "the converted extent is therefore never degenerate, got ({ex}, {ey})"
        );
        assert!(ecs.navmesh_for(Uuid::from_u128(10), 0.4, &[]).is_some());
    }

    #[test]
    fn navmesh_for_refuses_a_radius_over_the_footprint_cap() {
        // The radius-RANGE refusal, pinned at the level it now lives: `build_navmesh` receives an
        // already-converted world distance and refuses only on that distance's magnitude, so an
        // over-cap radius whose converted distance stays under `MAX_NAVMESH_COORD` would build a
        // mesh if `navmesh_for` stopped checking the range.
        // Discrimination: the radius is derived from `MAX_FOOTPRINT_CELLS` itself, and the
        // in-range sibling assertion below fails if the guard is widened into rejecting
        // legitimate radii.
        let scene = entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 100 }, "background": null,
                    "bounds": { "width": 10.0, "height": 10.0 } }),
        );
        let ecs = SceneEcs::from_documents(vec![scene], 0);
        let over_cap = crate::scene::pathfinding::MAX_FOOTPRINT_CELLS + 1.0;
        assert!(ecs
            .navmesh_for(Uuid::from_u128(10), over_cap, &[])
            .is_none());
        assert!(ecs
            .navmesh_for(
                Uuid::from_u128(10),
                crate::scene::pathfinding::MAX_FOOTPRINT_CELLS,
                &[]
            )
            .is_some());
    }

    #[test]
    fn navmesh_for_refuses_a_scene_whose_converted_extent_is_over_magnitude() {
        // The magnitude bound on the CONVERSION, pinned where the conversion now happens: neither
        // the authored bound nor the cell size alone is oversized, but `world_extent`'s product
        // exceeds `navmesh::MAX_NAVMESH_COORD`, which saturates on the `f64 -> f32` cast and
        // panics inside the triangulation.
        // Discrimination: the sibling assertion uses the same cell size with a bound small enough
        // to keep the product under the ceiling, so a guard that refused on the cell size alone
        // fails it.
        let over = entity_doc_top_eng(
            10,
            "scene",
            json!({ "grid": { "kind": "square", "size": 1e10 }, "background": null,
                    "bounds": { "width": 1e10, "height": 100.0 } }),
        );
        let ecs = SceneEcs::from_documents(vec![over], 0);
        assert!(ecs.navmesh_for(Uuid::from_u128(10), 0.4, &[]).is_none());

        let under = entity_doc_top_eng(
            11,
            "scene",
            json!({ "grid": { "kind": "square", "size": 1e10 }, "background": null,
                    "bounds": { "width": 10.0, "height": 10.0 } }),
        );
        let ecs = SceneEcs::from_documents(vec![under], 0);
        assert!(ecs.navmesh_for(Uuid::from_u128(11), 0.4, &[]).is_some());
    }
}
