//! Per-world derived scene ECS. Hydrated from documents (#5); never persisted,
//! never authoritative. Holds one hecs entity per scene-entity document so
//! engine-owned systems (M9 vision, M10 pathfinding) can query spatial state.

pub mod explored;
pub mod lighting;
pub mod movement;
pub mod vision;

use std::collections::{BTreeMap, HashMap};

use uuid::Uuid;

use crate::data::command::{set_pointer, Operation};
use crate::data::document::Document;
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

/// Parse a movement-restriction string; any unknown/missing value fails closed to `Visible`
/// (the most restrictive non-frozen mode — never silently widens to `Unrestricted`).
fn parse_movement_restriction(s: &str) -> MovementRestriction {
    match s {
        "revealed" => MovementRestriction::Revealed,
        "unrestricted" => MovementRestriction::Unrestricted,
        _ => MovementRestriction::Visible,
    }
}

/// The resolved per-scene lighting/vision/movement settings (subset of the client
/// `ResolvedSceneSettings`; pathfinding/animation fields are resolved in later checkpoints).
#[derive(Clone, Debug)]
pub struct ResolvedScene {
    pub los_restriction: bool,
    pub fog: bool,
    pub observer_vision: bool,
    pub lighting_enabled: bool,
    pub light_mode: LightMode,
    pub env_color: u32,
    pub env_intensity: f64,
    pub movement_restriction: MovementRestriction,
    pub partial_cell_leniency: bool,
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

/// Read a bool from a `system` JSON pointer; `null`/absent/non-bool ⇒ `None` (⇒ inherit).
fn opt_bool(v: &serde_json::Value, ptr: &str) -> Option<bool> {
    v.pointer(ptr).and_then(|x| x.as_bool())
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
        }
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
    }

    /// Resolve a scene's effective lighting/vision settings: built-in defaults < world-settings doc
    /// < per-scene override. Fail-closed and `null ⇒ inherit` (mirrors `resolveSceneSettings`).
    pub fn resolve_scene(&self, scene: Uuid) -> ResolvedScene {
        // World layer — structural guard: a partial world-settings doc falls back to built-ins.
        let ws = self.world_settings.as_ref().map(|d| &d.system);
        // Structural guard: each required key must be a non-null object, mirroring the TS
        // `ws?.scene && ws?.pathfinding && ws?.animation` check (falsy for null values).
        // A partial or null-valued key falls back to built-in defaults rather than panicking.
        let ws_scene = ws.and_then(|s| {
            if s.get("scene").and_then(|v| v.as_object()).is_some()
                && s.get("pathfinding").and_then(|v| v.as_object()).is_some()
                && s.get("animation").and_then(|v| v.as_object()).is_some()
            {
                s.pointer("/scene")
            } else {
                None
            }
        });
        // Built-in defaults (mirror DEFAULT_WORLD_SETTINGS.scene).
        let d_los = ws_scene
            .and_then(|s| s.get("losRestriction"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let d_fog = ws_scene
            .and_then(|s| s.get("fog"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let d_obs = ws_scene
            .and_then(|s| s.get("observerVision"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let d_lit = ws_scene
            .and_then(|s| s.get("lightingEnabled"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let d_mode = ws_scene
            .and_then(|s| s.get("lightMode"))
            .and_then(|v| v.as_str())
            .unwrap_or("environmentLight");
        // A pointer on a `null` `environment` value returns `None`, so both sub-fields
        // inherit the world default (same behaviour as an absent `environment` key).
        let d_env_color = ws_scene
            .and_then(|s| s.pointer("/environment/color"))
            .and_then(|v| v.as_str())
            .unwrap_or("#0a0e1a");
        let d_env_int = ws_scene
            .and_then(|s| s.pointer("/environment/intensity"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        // movementRestriction: scene `vision.movementRestriction` ?? world ?? "visible".
        let d_move = ws_scene
            .and_then(|s| s.get("movementRestriction"))
            .and_then(|v| v.as_str())
            .unwrap_or("visible");
        // partialCellLeniency: world-only (no per-scene override; mirrors `d.scene.partialCellLeniency`).
        let d_lenient = ws_scene
            .and_then(|s| s.get("partialCellLeniency"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        // Scene override layer (per-scene `vision`/`lighting`; null/absent ⇒ inherit).
        let scene_sys = self
            .index
            .get(&scene)
            .and_then(|&e| self.world.get::<&SceneEntity>(e).ok())
            .map(|c| c.doc.system.clone());
        let s = scene_sys.as_ref();
        let los = s
            .and_then(|s| opt_bool(s, "/vision/losRestriction"))
            .unwrap_or(d_los);
        let fog = s.and_then(|s| opt_bool(s, "/vision/fog")).unwrap_or(d_fog);
        let obs = s
            .and_then(|s| opt_bool(s, "/vision/observerVision"))
            .unwrap_or(d_obs);
        let lit = s
            .and_then(|s| opt_bool(s, "/lighting/enabled"))
            .unwrap_or(d_lit);
        let mode_str = s
            .and_then(|s| s.pointer("/lighting/mode"))
            .and_then(|v| v.as_str())
            .unwrap_or(d_mode);
        let env_color = s
            .and_then(|s| s.pointer("/lighting/environment/color"))
            .and_then(|v| v.as_str())
            .unwrap_or(d_env_color);
        let env_int = s
            .and_then(|s| s.pointer("/lighting/environment/intensity"))
            .and_then(|v| v.as_f64())
            .unwrap_or(d_env_int);
        // Scene may override movementRestriction (string); null/absent ⇒ inherit world. Mirrors
        // `v.movementRestriction ?? d.scene.movementRestriction`. partialCellLeniency has no scene override.
        let move_str = s
            .and_then(|s| s.pointer("/vision/movementRestriction"))
            .and_then(|v| v.as_str())
            .unwrap_or(d_move);

        ResolvedScene {
            los_restriction: los,
            fog,
            observer_vision: obs,
            lighting_enabled: lit,
            light_mode: if mode_str == "globalIllumination" {
                LightMode::GlobalIllumination
            } else {
                LightMode::EnvironmentLight
            },
            env_color: parse_hex_color(env_color),
            env_intensity: env_int.clamp(0.0, 1.0),
            movement_restriction: parse_movement_restriction(move_str),
            partial_cell_leniency: d_lenient,
        }
    }

    /// Resolved gradation bands, brightest-first. Fail-closed to the built-in three-band default.
    pub fn resolved_bands(&self) -> Vec<Band> {
        let bands = self
            .gradation
            .as_ref()
            .and_then(|d| d.system.pointer("/bands"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|b| {
                        Some(Band {
                            name: b.get("name")?.as_str()?.to_string(),
                            min_illumination: b.get("minIllumination")?.as_f64()?,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        crate::scene::lighting::sorted_bands(bands)
    }

    /// Resolved vision-mode registry. Returns a `BTreeMap` for deterministic key order (mirrors
    /// the plan's Global Constraint on determinism; `.get(id)` works identically for callers).
    /// Fail-closed to the built-in `normal`+`darkvision` seed ONLY when no doc/`modes` is present
    /// (mirrors TS `sys?.modes ?? SEED`). A GM-authored modes doc with all-malformed entries is
    /// returned as-is rather than silently re-granting built-in modes the GM may have removed.
    pub fn resolved_vision_modes(&self) -> BTreeMap<String, VisionMode> {
        let mut out = BTreeMap::new();
        // Seed only on the None (absent) branch — a present doc's modes being all malformed
        // must not silently replace a GM-authored registry with the built-in seed.
        let parsed = self
            .vision_modes
            .as_ref()
            .and_then(|d| d.system.pointer("/modes"))
            .and_then(|v| v.as_object());
        match parsed {
            Some(modes) => {
                for (id, m) in modes {
                    if let Some(floor) = m.get("illuminationFloor").and_then(|v| v.as_str()) {
                        let range = m
                            .get("defaultRange")
                            .and_then(|v| v.as_f64())
                            .unwrap_or(0.0);
                        out.insert(
                            id.clone(),
                            VisionMode {
                                illumination_floor: floor.to_string(),
                                default_range: range,
                                render_hint: m
                                    .get("renderHint")
                                    .and_then(|v| v.as_str())
                                    .map(str::to_string),
                            },
                        );
                    }
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

    /// Resolve a token move from an `Update`'s `changes`: `(scene, committed_start,
    /// post_image_end)`. The end is the committed `system` with **all** changes applied in
    /// array order (last-write-wins) — exactly what `apply_intent` commits — so a wholesale
    /// `/system` write or duplicate `/system/x` changes cannot evade the collision check by
    /// presenting a safe target while committing an unsafe one. `None` if `token_id` is not a
    /// token with `(x,y)`. Reads the authoritative ECS state, never the client's `old`.
    pub fn token_move(
        &self,
        token_id: Uuid,
        changes: &[crate::data::command::FieldChange],
    ) -> Option<TokenMove> {
        let &e = self.index.get(&token_id)?;
        let tok = self.world.get::<&SceneEntity>(e).ok()?;
        if tok.doc.doc_type != "token" {
            return None;
        }
        let scene = tok.doc.parent_id?;
        let cx = sys_f64(&tok.doc, "/x")?;
        let cy = sys_f64(&tok.doc, "/y")?;
        let mut v = serde_json::to_value(&tok.doc).ok()?;
        for ch in changes {
            let _ = set_pointer(&mut v, &ch.path, ch.new.clone());
        }
        let nx = v.pointer("/system/x").and_then(|x| x.as_f64())?;
        let ny = v.pointer("/system/y").and_then(|x| x.as_f64())?;
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
            if let (Some(x), Some(y), Some(scene)) = (
                sys_f64(&e.doc, "/x"),
                sys_f64(&e.doc, "/y"),
                e.doc.parent_id,
            ) {
                viewpoints.push((scene, (x, y)));
            }
        }
        let mut out = Vec::with_capacity(viewpoints.len());
        for (scene, vp) in viewpoints {
            let walls = self.sight_walls(scene);
            let bound = vision::bound_for(vp, &walls, VISION_BOUND_MARGIN);
            out.push((scene, vision::visibility_polygon(vp, &walls, bound)));
        }
        out
    }

    /// Each scene's grid cell size (`system.grid.size`), defaulting to 100 — the unit the M9c
    /// explored-fog accumulation quantizes vision into. Read once per dispatch (cheap doc scan).
    pub fn scene_grid_sizes(&self) -> std::collections::HashMap<Uuid, f64> {
        let mut out = std::collections::HashMap::new();
        for e in self.world.query::<&SceneEntity>().iter() {
            if e.doc.doc_type != "scene" {
                continue;
            }
            let size = e
                .doc
                .system
                .pointer("/grid/size")
                .and_then(|v| v.as_f64())
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
            if w.doc
                .system
                .pointer("/blocksSight")
                .and_then(|v| v.as_bool())
                != Some(true)
            {
                continue;
            }
            if let (Some(x1), Some(y1), Some(x2), Some(y2)) = (
                sys_f64(&w.doc, "/seg/x1"),
                sys_f64(&w.doc, "/seg/y1"),
                sys_f64(&w.doc, "/seg/x2"),
                sys_f64(&w.doc, "/seg/y2"),
            ) {
                out.push(vision::Seg {
                    a: (x1, y1),
                    b: (x2, y2),
                });
            }
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
            if w.doc
                .system
                .pointer("/blocksLight")
                .and_then(|v| v.as_bool())
                != Some(true)
            {
                continue;
            }
            if let (Some(x1), Some(y1), Some(x2), Some(y2)) = (
                sys_f64(&w.doc, "/seg/x1"),
                sys_f64(&w.doc, "/seg/y1"),
                sys_f64(&w.doc, "/seg/x2"),
                sys_f64(&w.doc, "/seg/y2"),
            ) {
                out.push(vision::Seg {
                    a: (x1, y1),
                    b: (x2, y2),
                });
            }
        }
        out
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
            if e.doc.system.pointer("/enabled").and_then(|v| v.as_bool()) != Some(true) {
                continue;
            }
            let (Some(x), Some(y)) = (sys_f64(&e.doc, "/x"), sys_f64(&e.doc, "/y")) else {
                continue;
            };
            let color = e
                .doc
                .system
                .pointer("/color")
                .and_then(|v| v.as_str())
                .map(parse_hex_color)
                .unwrap_or(0xFFFFFF);
            let falloff = match e
                .doc
                .system
                .pointer("/falloff/curve")
                .and_then(|v| v.as_str())
            {
                Some("quadratic") => Falloff::Quadratic,
                Some("none") => Falloff::None,
                _ => Falloff::Linear,
            };
            out.push(Light {
                pos: (x, y),
                color,
                intensity: e
                    .doc
                    .system
                    .pointer("/intensity")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1.0)
                    .clamp(0.0, 1.0),
                bright_radius: e
                    .doc
                    .system
                    .pointer("/brightRadius")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
                dim_radius: e
                    .doc
                    .system
                    .pointer("/dimRadius")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
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
    /// `embedded.actor[0].system.vision` without overrides. An unknown mode id is dropped
    /// (fail-closed: it contributes no vision floor). Always returns ≥1 triple (normal fallback
    /// with `render_hint: None`).
    pub fn token_vision_floors(&self, token: &Document) -> Vec<(f64, f64, Option<String>)> {
        let modes = self.resolved_vision_modes();
        let bands = self.resolved_bands();

        // Mirror actor.ts resolveTokenActor: a LINKED token (actor_id) resolves the shared actor and
        // applies the per-token override whitelist (overrides.vision REPLACES the actor's vision); a
        // dangling link (actor absent) yields normal, ignoring overrides. An INSTANCED token (no
        // actor_id) uses its embedded copy's vision; overrides do not apply to instanced tokens.
        let assignments: Option<&serde_json::Value> = match token
            .system
            .pointer("/actor_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
        {
            Some(id) => match self.actors.get(&id) {
                Some(actor) => token
                    .system
                    .pointer("/overrides/vision")
                    .filter(|v| v.is_array())
                    .or_else(|| actor.system.pointer("/vision").filter(|v| v.is_array())),
                None => None, // dangling link → normal (overrides ignored, per resolveTokenActor)
            },
            None => token
                .embedded
                .get("actor")
                .and_then(|v| v.first())
                .and_then(|a| a.system.pointer("/vision"))
                .filter(|v| v.is_array()),
        };

        let mut out: Vec<(f64, f64, Option<String>)> = Vec::new();
        if let Some(arr) = assignments.and_then(|v| v.as_array()) {
            for a in arr {
                let Some(mode_id) = a.get("mode").and_then(|v| v.as_str()) else {
                    continue;
                };
                let Some(vm) = modes.get(mode_id) else {
                    continue;
                }; // unknown mode → drop (fail-closed)
                let range = a
                    .get("range")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(vm.default_range);
                out.push((
                    crate::scene::lighting::floor_min(&bands, &vm.illumination_floor),
                    range,
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
    pub(crate) fn lighting_inputs(&self, scene: Uuid, settings: &ResolvedScene) -> LightingInputs {
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
        let lit_polys: Vec<Vec<vision::P>> = lights
            .iter()
            .map(|l| {
                let b = vision::bound_for(l.pos, &light_walls, VISION_BOUND_MARGIN);
                vision::visibility_polygon(l.pos, &light_walls, b)
            })
            .collect();
        LightingInputs {
            all_bright,
            lights,
            lit_polys,
            sight_walls: self.sight_walls(scene),
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
            if let (Some(x), Some(y)) = (sys_f64(&e.doc, "/x"), sys_f64(&e.doc, "/y")) {
                sources.push(Src {
                    scene,
                    vp: (x, y),
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
            let li = self.lighting_inputs(scene, settings);

            let entry = per_scene
                .entry(scene)
                .or_insert_with(|| (cell, BTreeMap::new()));
            for src in sources.iter().filter(|s| s.scene == scene) {
                // LOS polygon for this source (or, LOS off, the whole bound box as a polygon).
                let poly = source_los_poly(src.vp, &li.sight_walls, settings.los_restriction);
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

        // Gather this user's vision sources in THIS scene (owner ∪ observer-tier when
        // observerVision). Mirrors player_lit_mask's source gather, scene-filtered.
        struct Src {
            vp: vision::P,
            floors: Vec<(f64, f64, Option<String>)>,
        }
        let mut sources: Vec<Src> = Vec::new();
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
            if let (Some(x), Some(y)) = (sys_f64(&e.doc, "/x"), sys_f64(&e.doc, "/y")) {
                sources.push(Src {
                    vp: (x, y),
                    floors: self.token_vision_floors(&e.doc),
                });
            }
        }
        if sources.is_empty() {
            return out;
        }

        // Scene-shared lighting inputs (once), then per-source per-cell test.
        let li = self.lighting_inputs(scene, &settings);
        for src in &sources {
            let poly = source_los_poly(src.vp, &li.sight_walls, settings.los_restriction);
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
                    // Strict: center only. Lenient: center + 4 corners (first passing sample wins).
                    let center = ((i as f64 + 0.5) * cell, (j as f64 + 0.5) * cell);
                    let corners = [
                        (i as f64 * cell, j as f64 * cell),
                        ((i + 1) as f64 * cell, j as f64 * cell),
                        (i as f64 * cell, (j + 1) as f64 * cell),
                        ((i + 1) as f64 * cell, (j + 1) as f64 * cell),
                    ];
                    let samples: &[(f64, f64)] = if lenient {
                        // SAFETY: reborrow as slice; corners is a local array in scope.
                        // Cannot use a mixed slice literal because corner array and single-element
                        // inline have incompatible sizes; build as two separate slices and chain.
                        &corners
                    } else {
                        std::slice::from_ref(&center)
                    };
                    // For lenient, center must be checked first so the §13 strict cells are always
                    // included; then fall through to corners only if center fails.
                    let mut found = false;
                    if lenient {
                        // Check center first, then corners.
                        if vision::point_in_poly(&poly, center) {
                            let cl = if li.all_bright {
                                crate::scene::lighting::CellLight {
                                    level: 1.0,
                                    tint: 0,
                                }
                            } else {
                                crate::scene::lighting::cell_illumination(
                                    center,
                                    settings.env_intensity,
                                    settings.env_color,
                                    &li.lights,
                                    &li.lit_polys,
                                    cell,
                                )
                            };
                            let dist_cells = (((center.0 - src.vp.0).powi(2)
                                + (center.1 - src.vp.1).powi(2))
                            .sqrt())
                                / cell;
                            if cell_visible(&src.floors, cl.level, dist_cells) {
                                found = true;
                            }
                        }
                        if !found {
                            for &(sx, sy) in &corners {
                                if !vision::point_in_poly(&poly, (sx, sy)) {
                                    continue;
                                }
                                let cl = if li.all_bright {
                                    crate::scene::lighting::CellLight {
                                        level: 1.0,
                                        tint: 0,
                                    }
                                } else {
                                    crate::scene::lighting::cell_illumination(
                                        (sx, sy),
                                        settings.env_intensity,
                                        settings.env_color,
                                        &li.lights,
                                        &li.lit_polys,
                                        cell,
                                    )
                                };
                                let dist_cells =
                                    (((sx - src.vp.0).powi(2) + (sy - src.vp.1).powi(2)).sqrt())
                                        / cell;
                                if cell_visible(&src.floors, cl.level, dist_cells) {
                                    found = true;
                                    break;
                                }
                            }
                        }
                    } else {
                        // Strict: center only (mirrors player_lit_mask exactly).
                        let _ = samples; // samples is &[center] but we use the named var directly.
                        if vision::point_in_poly(&poly, center) {
                            let cl = if li.all_bright {
                                crate::scene::lighting::CellLight {
                                    level: 1.0,
                                    tint: 0,
                                }
                            } else {
                                crate::scene::lighting::cell_illumination(
                                    center,
                                    settings.env_intensity,
                                    settings.env_color,
                                    &li.lights,
                                    &li.lit_polys,
                                    cell,
                                )
                            };
                            let dist_cells = (((center.0 - src.vp.0).powi(2)
                                + (center.1 - src.vp.1).powi(2))
                            .sqrt())
                                / cell;
                            if cell_visible(&src.floors, cl.level, dist_cells) {
                                found = true;
                            }
                        }
                    }
                    if found {
                        out.insert((i, j));
                    }
                }
            }
        }
        out
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
            if w.doc
                .system
                .pointer("/blocksMove")
                .and_then(|v| v.as_bool())
                != Some(true)
            {
                continue;
            }
            let (Some(x1), Some(y1), Some(x2), Some(y2)) = (
                sys_f64(&w.doc, "/seg/x1"),
                sys_f64(&w.doc, "/seg/y1"),
                sys_f64(&w.doc, "/seg/x2"),
                sys_f64(&w.doc, "/seg/y2"),
            ) else {
                continue;
            };
            if segments_cross(a0, a1, (x1, y1), (x2, y2)) {
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
    pub(crate) sight_walls: Vec<vision::Seg>,
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
/// (`vision::visibility_polygon`).
fn source_los_poly(
    vp: vision::P,
    sight_walls: &[vision::Seg],
    los_restriction: bool,
) -> Vec<vision::P> {
    let b = vision::bound_for(vp, sight_walls, VISION_BOUND_MARGIN);
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

/// Read an `f64` from a document's opaque `system` body via JSON pointer (ints coerce).
fn sys_f64(doc: &Document, pointer: &str) -> Option<f64> {
    doc.system.pointer(pointer).and_then(|v| v.as_f64())
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
fn segments_cross(p1: (f64, f64), p2: (f64, f64), p3: (f64, f64), p4: (f64, f64)) -> bool {
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
    fn apply_op_create_update_delete() {
        let mut ecs = SceneEcs::new();
        ecs.apply_op(&Operation::Create {
            doc: doc(11, Some(10), "token"),
        });
        assert_eq!(ecs.entity_count(), 1);
        ecs.apply_op(&Operation::Update {
            doc_id: Uuid::from_u128(11),
            changes: vec![crate::data::command::FieldChange {
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

    fn entity_doc(id: u128, parent: u128, ty: &str, system: serde_json::Value) -> Document {
        let mut d = doc(id, Some(parent), ty);
        d.system = system;
        d
    }

    fn entity_doc_top(id: u128, ty: &str, system: serde_json::Value) -> Document {
        let mut d = doc(id, None, ty);
        d.system = system;
        d
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
                entity_doc(12, 10, "wall", cross.clone()),
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
                entity_doc(24, 20, "wall", cross.clone()),
            ],
            0,
        );
        assert!(ecs_scope.blocks_move(other, (0.0, 0.0), (10.0, 10.0))); // blocks in scene 20
        assert!(!ecs_scope.blocks_move(scene, (0.0, 0.0), (10.0, 10.0))); // not in scene 10

        // A scene whose only crossing wall is blocksMove:false must not block movement.
        let ecs2 = SceneEcs::from_documents(
            vec![
                doc(10, None, "scene"),
                entity_doc(
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
                entity_doc(11, 10, "token", json!({ "x": 0, "y": 0 })),
            ],
            0,
        );
        let id = Uuid::from_u128(11);
        // A normal two-axis move.
        let (s, a0, a1) = ecs
            .token_move(
                id,
                &[fc("/system/x", json!(10)), fc("/system/y", json!(10))],
            )
            .unwrap();
        assert_eq!(s, Uuid::from_u128(10));
        assert_eq!(a0, (0.0, 0.0));
        assert_eq!(a1, (10.0, 10.0));
        // Bypass A: a wholesale `/system` write — the post-image reads the new x/y.
        let whole = fc("/system", json!({ "x": 50, "y": 50 }));
        assert_eq!(ecs.token_move(id, &[whole]).unwrap().2, (50.0, 50.0));
        // Bypass B: duplicate `/system/x` — last write wins, mirroring apply_intent.
        let dup = ecs
            .token_move(id, &[fc("/system/x", json!(5)), fc("/system/x", json!(50))])
            .unwrap();
        assert_eq!(dup.2 .0, 50.0);
        // A non-position update is a no-op move (committed == post-image).
        let noop = ecs.token_move(id, &[fc("/system/hp", json!(5))]).unwrap();
        assert_eq!(noop.1, noop.2);
        // A non-token id resolves to nothing.
        assert!(ecs.token_move(Uuid::from_u128(99), &[]).is_none());
    }

    #[test]
    fn vision_channel_is_per_recipient() {
        use crate::data::document::WorldRole;
        let player = Uuid::from_u128(7);
        let mut token = entity_doc(11, 10, "token", json!({ "x": 0, "y": 0 }));
        token.owner = Some(player);
        let ecs = SceneEcs::from_documents(
            vec![
                doc(10, None, "scene"),
                token,
                entity_doc(
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
        let mut tok = entity_doc(11, 10, "token", json!({ "x": 50, "y": 50 }));
        tok.owner = Some(player);
        let light = entity_doc(
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
        let mut tok = entity_doc(11, 10, "token", json!({ "x": 50, "y": 50 }));
        tok.owner = Some(player);
        tok.embedded.insert(
            "actor".into(),
            vec![{
                let mut a = doc(99, None, "actor");
                a.system = json!({ "vision": [{ "mode": "darkvision", "range": 6 }] });
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
        ws.system = json!({
            "scene": { "lightingEnabled": false, "lightMode": "globalIllumination",
                       "environment": { "color": "#0a0e1a", "intensity": 0.25 } },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        });
        ecs.set_world_config(Some(ws), None, None);
        let r1 = ecs.resolve_scene(scene_id);
        assert!(!r1.lighting_enabled);
        assert!(matches!(r1.light_mode, LightMode::GlobalIllumination));
        assert_eq!(r1.env_color, 0x0A0E1A);
        assert!((r1.env_intensity - 0.25).abs() < 1e-9);

        // Per-scene override re-enables lighting (null/absent ⇒ inherit; a present value wins).
        let mut scene = doc(10, None, "scene");
        scene.system = json!({ "grid": { "kind": "square", "size": 100 },
                               "lighting": { "enabled": true } });
        ecs.apply_op(&Operation::Update {
            doc_id: scene_id,
            changes: vec![crate::data::command::FieldChange {
                path: "/system".into(),
                old: json!(null),
                new: scene.system.clone(),
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
        vm.system = json!({ "modes": { "blindsight": { "illuminationFloor": "dark", "defaultRange": 4 } } });
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
        ecs.set_actors(vec![entity_doc_top(
            200,
            "actor",
            json!({ "vision": [{ "mode": "darkvision", "range": 6 }] }),
        )]);

        // Linked token referencing the actor → darkvision floor (dark=0.0), range 6.
        let mut linked = entity_doc(
            11,
            10,
            "token",
            json!({ "x": 0, "y": 0, "actor_id": Uuid::from_u128(200).to_string() }),
        );
        let floors = ecs.token_vision_floors(&linked);
        assert_eq!(floors.len(), 1);
        assert_eq!(floors[0], (0.0, 6.0, Some("desaturate".to_string()))); // dark floor, 6-cell range, darkvision hint

        // A per-token override REPLACES the actor's vision entirely.
        linked.system["overrides"] = json!({ "vision": [{ "mode": "normal", "range": 0 }] });
        let f2 = ecs.token_vision_floors(&linked);
        assert_eq!(f2[0], (0.34, 0.0, None)); // dim floor, unlimited range, no hint (normal mode has render_hint: None)

        // An actorless token → normal only.
        let raw = entity_doc(12, 10, "token", json!({ "x": 0, "y": 0 }));
        assert_eq!(ecs.token_vision_floors(&raw), vec![(0.34, 0.0, None)]);

        // An explicit EMPTY override REPLACES (no fall-through to the linked actor → normal).
        let mut linked_empty = entity_doc(
            13,
            10,
            "token",
            json!({ "x": 0, "y": 0, "actor_id": Uuid::from_u128(200).to_string(),
                    "overrides": { "vision": [] } }),
        );
        assert_eq!(
            ecs.token_vision_floors(&linked_empty),
            vec![(0.34, 0.0, None)]
        );

        // A token with BOTH actor_id AND an embedded actor resolves the LINKED actor (matches the
        // client's actor_id-first resolveTokenActor), NOT the embedded copy.
        linked_empty.system["overrides"] = json!({}); // no vision override
        linked_empty.embedded.insert(
            "actor".into(),
            vec![entity_doc_top(
                201,
                "actor",
                json!({ "vision": [{ "mode": "normal", "range": 0 }] }),
            )],
        );
        // actor 200 grants darkvision range 6 → linked wins → (0.0, 6.0), not the embedded normal.
        assert_eq!(
            ecs.token_vision_floors(&linked_empty),
            vec![(0.0, 6.0, Some("desaturate".to_string()))]
        );

        // A DANGLING link (actor_id with no matching actor) + an overrides.vision is normal — the
        // client ignores overrides when the linked actor is absent.
        let dangling = entity_doc(
            14,
            10,
            "token",
            json!({ "x": 0, "y": 0, "actor_id": Uuid::from_u128(999).to_string(),
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
                entity_doc(
                    20,
                    10,
                    "light",
                    json!({
                        "x": 50.0, "y": 50.0, "color": "#ffeeaa", "intensity": 1.0,
                        "brightRadius": 2.0, "dimRadius": 6.0, "enabled": true
                    }),
                ),
                entity_doc(
                    21,
                    10,
                    "light",
                    json!({ "x": 0.0, "y": 0.0, "color": "#fff",
                    "intensity": 1.0, "brightRadius": 1.0, "dimRadius": 2.0, "enabled": false }),
                ),
                entity_doc(
                    22,
                    10,
                    "wall",
                    json!({ "seg": {"x1":0,"y1":0,"x2":10,"y2":0}, "blocksLight": true }),
                ),
                entity_doc(
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
                entity_doc(
                    20,
                    10,
                    "light",
                    json!({
                        "x": 50.0, "y": 50.0, "color": "#ffeeaa", "intensity": 1.0,
                        "brightRadius": 2.0, "dimRadius": 6.0, "enabled": true
                    }),
                ),
                entity_doc(
                    22,
                    10,
                    "wall",
                    json!({ "seg": {"x1":0,"y1":0,"x2":10,"y2":0}, "blocksLight": true }),
                ),
                doc(30, None, "scene"), // scene id 20 (doc id 30 → Uuid 30; parent is None)
                entity_doc(
                    31,
                    30,
                    "light",
                    json!({
                        "x": 10.0, "y": 10.0, "color": "#ffffff", "intensity": 0.8,
                        "brightRadius": 3.0, "dimRadius": 8.0, "enabled": true
                    }),
                ),
                entity_doc(
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
        let mut tok = entity_doc(11, 10, "token", json!({ "x": 50, "y": 50 }));
        tok.owner = Some(player);
        let dark = SceneEcs::from_documents(vec![doc(10, None, "scene"), tok.clone()], 0);
        assert!(
            dark.player_lit_mask(player)
                .iter()
                .all(|s| s.cells.is_empty()),
            "dark scene + normal vision → empty lit mask"
        );

        // Add a bright light covering the token's cell → that cell becomes visible at the bright band.
        let light = entity_doc(
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
        bright_scene.system = json!({ "grid": { "kind": "square", "size": 100 },
                                      "lighting": { "enabled": false } });
        let mut ntok = entity_doc(11, 10, "token", json!({ "x": 50, "y": 50 }));
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
        // linked tokens with a resolved actor_id; an instanced token reads embedded.actor[0].system.vision.
        let mut dv = entity_doc(12, 10, "token", json!({ "x": 50, "y": 50 }));
        dv.embedded.insert(
            "actor".into(),
            vec![entity_doc_top(
                900,
                "actor",
                json!({ "vision": [{ "mode": "darkvision", "range": 6 }] }),
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
        let mut tok = entity_doc(11, 10, "token", json!({ "x": 50, "y": 50 }));
        tok.owner = Some(player);
        tok.embedded.insert(
            "actor".into(),
            vec![{
                let mut a = doc(99, None, "actor");
                a.system = json!({ "vision": [{ "mode": "darkvision", "range": 6 }] });
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
        let mut tok2 = entity_doc(12, 10, "token", json!({ "x": 50, "y": 50 }));
        tok2.owner = Some(player2); // no embedded vision → normal fallback
        let light = entity_doc(
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

    #[test]
    fn config_and_actor_side_tables_track_ops() {
        use serde_json::json;
        let mut ecs = SceneEcs::new();
        // Seed via setters (the room-hydration path).
        let mut ws = doc(100, None, "world-settings");
        ws.system = json!({ "scene": { "lightingEnabled": false } });
        ecs.set_world_config(Some(ws), None, None);
        ecs.set_actors(vec![entity_doc_top(200, "actor", json!({ "vision": [] }))]);
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
        let mut vm = entity_doc(30, 10, "vision-modes", json!({}));
        vm.doc_type = "vision-modes".into();
        vm.parent_id = None;
        vm.system = json!({ "modes": {
            "truesight": { "illuminationFloor": "dark", "defaultRange": 8, "renderHint": "outline" },
            "plain":     { "illuminationFloor": "dim",  "defaultRange": 0 }
        }});
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
        let mut tok = entity_doc(11, 10, "token", json!({ "x": 0, "y": 0 }));
        tok.embedded.insert(
            "actor".into(),
            vec![{
                let mut a = doc(99, None, "actor");
                a.system = json!({ "vision": [
                    { "mode": "normal", "range": 0 },
                    { "mode": "darkvision", "range": 6 }
                ]});
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

    /// Set `world_settings` to a doc whose `system` is `json_system` (test-only).
    /// Mirrors how `room.rs` builds a world-settings config doc.
    #[cfg(test)]
    impl SceneEcs {
        pub(crate) fn set_world_settings_for_test(&mut self, json_system: serde_json::Value) {
            let mut d = crate::data::document::tests::world_scoped_doc(
                Uuid::from_u128(9),
                Uuid::from_u128(100),
                "world-settings",
            );
            d.system = json_system;
            self.world_settings = Some(d);
        }

        pub(crate) fn insert_scene_for_test(
            &mut self,
            scene_id: Uuid,
            json_system: serde_json::Value,
        ) {
            let mut d = crate::data::document::tests::world_scoped_doc(
                Uuid::from_u128(9),
                scene_id,
                "scene",
            );
            d.system = json_system;
            // Remove stale entity if re-inserting.
            if let Some(old_e) = self.index.remove(&scene_id) {
                let _ = self.world.despawn(old_e);
            }
            let e = self.world.spawn((SceneEntity { doc: d },));
            self.index.insert(scene_id, e);
        }
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
        // A complete world-settings system (scene+pathfinding+animation) so the structural guard passes.
        ecs.set_world_settings_for_test(json!({
            "scene": { "losRestriction": true, "fog": true, "lightingEnabled": true,
                       "lightMode": "environmentLight", "environment": {"color":"#0a0e1a","intensity":0.0},
                       "observerVision": false, "movementRestriction": "revealed", "partialCellLeniency": false },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        }));
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
        ecs.set_world_settings_for_test(json!({
            "scene": { "movementRestriction": "visible", "partialCellLeniency": true },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        }));
        // Scene overrides vision.movementRestriction to "unrestricted".
        ecs.insert_scene_for_test(
            scene_id,
            json!({
                "grid": { "kind": "square", "size": 100 },
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
        ecs.set_world_settings_for_test(json!({
            "scene": { "movementRestriction": "revealed", "partialCellLeniency": true },
            "pathfinding": { "diagonalRule": "chebyshev" },
            "animation": { "speedCellsPerSec": 6, "easing": "easeInOut" }
        }));
        // null clears the override → inherit world "revealed" (mirrors `?? d.scene.movementRestriction`).
        ecs.insert_scene_for_test(
            scene_id,
            json!({
                "grid": { "kind": "square", "size": 100 },
                "vision": { "movementRestriction": null }
            }),
        );
        let r = ecs.resolve_scene(scene_id);
        assert_eq!(r.movement_restriction, MovementRestriction::Revealed);
    }

    #[test]
    fn lit_mask_suppresses_hint_when_normal_floor_wins_in_bright_cell() {
        use serde_json::json;
        // Combined-token suppression (buddy-check A1): an owned token whose embedded actor has
        // BOTH normal (floor=dim 0.34) AND darkvision (floor=dark 0.0).  Standing in a brightly-lit
        // cell (light placed at the token), normal's floor (0.34) is higher than darkvision's (0.0),
        // so normal is the highest-admitting mode → its hint (None) wins → lit cells carry no hint.
        let player = Uuid::from_u128(42);
        let mut tok = entity_doc(11, 10, "token", json!({ "x": 50, "y": 50 }));
        tok.owner = Some(player);
        tok.embedded.insert(
            "actor".into(),
            vec![{
                let mut a = doc(99, None, "actor");
                a.system = json!({ "vision": [
                    { "mode": "normal",     "range": 0 },
                    { "mode": "darkvision", "range": 6 }
                ]});
                a
            }],
        );
        // A bright light at the token location illuminates the cell at (0,0) above dim threshold.
        let light = entity_doc(
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
        let mut tok = entity_doc(11, 10, "token", json!({ "x": 50, "y": 50 }));
        tok.owner = Some(user);
        let light = entity_doc(
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

    #[test]
    fn visible_cells_lenient_is_a_superset_of_strict() {
        let (ecs, user, scene) = scene_with_lit_player_token();
        let strict = ecs.visible_cells(user, scene, false);
        let lenient = ecs.visible_cells(user, scene, true);
        assert!(
            strict.iter().all(|c| lenient.contains(c)),
            "lenient ⊇ strict"
        );
        assert!(lenient.len() >= strict.len());
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
}
