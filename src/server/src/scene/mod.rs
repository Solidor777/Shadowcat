//! Per-world derived scene ECS. Hydrated from documents (#5); never persisted,
//! never authoritative. Holds one hecs entity per scene-entity document so
//! engine-owned systems (M9 vision, M10 pathfinding) can query spatial state.

pub mod explored;
pub mod lighting;
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

/// The resolved per-scene lighting/vision settings the mask needs (subset of the client
/// `ResolvedSceneSettings`; movement/pathfinding/animation fields are resolved in later checkpoints).
#[derive(Clone, Debug)]
pub struct ResolvedScene {
    pub los_restriction: bool,
    pub fog: bool,
    pub observer_vision: bool,
    pub lighting_enabled: bool,
    pub light_mode: LightMode,
    pub env_color: u32,
    pub env_intensity: f64,
}

/// A resolved vision mode (subset of the client `VisionMode`). `default_range` is in cells.
#[derive(Clone, Debug)]
pub struct VisionMode {
    pub illumination_floor: String,
    pub default_range: f64,
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

/// One scene's visible cells for a player: `cells` are `(i, j, band_index, tint 0xRRGGBB)`.
#[derive(Debug)]
pub struct LitScene {
    pub scene: Uuid,
    pub cell: f64,
    pub cells: Vec<(i32, i32, usize, u32)>,
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
                            },
                        );
                    }
                }
            }
            None => {
                out.insert(
                    "normal".into(),
                    VisionMode {
                        illumination_floor: "dim".into(),
                        default_range: 0.0,
                    },
                );
                out.insert(
                    "darkvision".into(),
                    VisionMode {
                        illumination_floor: "dark".into(),
                        default_range: 12.0,
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

    /// The token's effective vision modes as `(floor_min_illumination, range_cells)` pairs.
    /// `range_cells == 0.0` ⇒ unlimited. Precedence (mirrors `resolveTokenActor` in actor.ts):
    /// a LINKED token (`actor_id` present) resolves the shared actor and applies
    /// `overrides.vision` as a wholesale replacement when present; a dangling link (actor absent)
    /// yields normal, ignoring overrides. An INSTANCED token (no `actor_id`) uses its
    /// `embedded.actor[0].system.vision` without overrides. An unknown mode id is dropped
    /// (fail-closed: it contributes no vision floor). Always returns ≥1 pair (normal fallback).
    pub fn token_vision_floors(&self, token: &Document) -> Vec<(f64, f64)> {
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

        let mut out: Vec<(f64, f64)> = Vec::new();
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
            ));
        }
        out
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
            floors: Vec<(f64, f64)>,
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
        // (i, j) -> (best_level, band_index, tint): best illumination seen from any source.
        type CellEntry = BTreeMap<(i32, i32), (f64, usize, u32)>;
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
            let sight_walls = self.sight_walls(scene);
            // Lighting inputs: under globalIllumination or lighting-off, every LOS cell is bright;
            // else compute per-cell from lights (occluded by blocksLight) + environment.
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

            let entry = per_scene
                .entry(scene)
                .or_insert_with(|| (cell, BTreeMap::new()));
            for src in sources.iter().filter(|s| s.scene == scene) {
                // LOS polygon for this source (or, LOS off, the whole bound box as a polygon).
                let b = vision::bound_for(src.vp, &sight_walls, VISION_BOUND_MARGIN);
                let poly = if settings.los_restriction {
                    vision::visibility_polygon(src.vp, &sight_walls, b)
                } else {
                    vec![
                        (b.minx, b.miny),
                        (b.maxx, b.miny),
                        (b.maxx, b.maxy),
                        (b.minx, b.maxy),
                    ]
                };
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
                        let cl = if all_bright {
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
                                &lights,
                                &lit_polys,
                                cell,
                            )
                        };
                        // Darkvision lowers the floor within range; pick the lowest applicable floor.
                        let dist_cells =
                            (((cx - src.vp.0).powi(2) + (cy - src.vp.1).powi(2)).sqrt()) / cell;
                        let mut floor = f64::INFINITY;
                        for &(fmin, range) in &src.floors {
                            if range == 0.0 || dist_cells <= range {
                                floor = floor.min(fmin);
                            }
                        }
                        if floor.is_finite() && cl.level >= floor {
                            let band = crate::scene::lighting::band_index(&bands, cl.level);
                            let slot = entry.1.entry((i, j)).or_insert((cl.level, band, cl.tint));
                            if cl.level > slot.0 {
                                *slot = (cl.level, band, cl.tint); // brightest source wins the band/tint
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
                    .map(|((i, j), (_lvl, band, tint))| (i, j, band, tint))
                    .collect(),
            })
            .collect()
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
                // TODO: thread the bands player_lit_mask already resolved to avoid this second resolve.
                let bands_json: Vec<serde_json::Value> = ecs
                    .resolved_bands()
                    .into_iter()
                    .map(|b| serde_json::json!({ "name": b.name, "min": b.min_illumination }))
                    .collect();
                let lit: Vec<serde_json::Value> = ecs
                    .player_lit_mask(ctx.user_id)
                    .into_iter()
                    .map(|s| {
                        let flat: Vec<i64> = s
                            .cells
                            .into_iter()
                            .flat_map(|(i, j, band, tint)| {
                                [i as i64, j as i64, band as i64, tint as i64]
                            })
                            .collect();
                        serde_json::json!({ "scene": s.scene, "cell": s.cell, "cells": flat })
                    })
                    .collect();
                Some(
                    serde_json::json!({ "mode": "masked", "polygons": polygons, "bands": bands_json, "lit": lit }),
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
            cells.len() % 4,
            0,
            "cells packed 4 ints/cell (i,j,band,tint)"
        );
        assert!(!pv["bands"].as_array().unwrap().is_empty()); // bands now top-level
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
        assert_eq!(floors[0], (0.0, 6.0)); // dark floor, 6-cell range

        // A per-token override REPLACES the actor's vision entirely.
        linked.system["overrides"] = json!({ "vision": [{ "mode": "normal", "range": 0 }] });
        let f2 = ecs.token_vision_floors(&linked);
        assert_eq!(f2[0], (0.34, 0.0)); // dim floor, unlimited range

        // An actorless token → normal only.
        let raw = entity_doc(12, 10, "token", json!({ "x": 0, "y": 0 }));
        assert_eq!(ecs.token_vision_floors(&raw), vec![(0.34, 0.0)]);

        // An explicit EMPTY override REPLACES (no fall-through to the linked actor → normal).
        let mut linked_empty = entity_doc(
            13,
            10,
            "token",
            json!({ "x": 0, "y": 0, "actor_id": Uuid::from_u128(200).to_string(),
                    "overrides": { "vision": [] } }),
        );
        assert_eq!(ecs.token_vision_floors(&linked_empty), vec![(0.34, 0.0)]);

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
        assert_eq!(ecs.token_vision_floors(&linked_empty), vec![(0.0, 6.0)]);

        // A DANGLING link (actor_id with no matching actor) + an overrides.vision is normal — the
        // client ignores overrides when the linked actor is absent.
        let dangling = entity_doc(
            14,
            10,
            "token",
            json!({ "x": 0, "y": 0, "actor_id": Uuid::from_u128(999).to_string(),
                    "overrides": { "vision": [{ "mode": "darkvision", "range": 9 }] } }),
        );
        assert_eq!(ecs.token_vision_floors(&dangling), vec![(0.34, 0.0)]);
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
                .any(|&(i, j, band, _)| i == 0 && j == 0 && band == 0),
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
                .any(|&(i, j, band, _)| i == 0 && j == 0 && band == 0),
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
}
