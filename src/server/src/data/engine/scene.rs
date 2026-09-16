//! `scene`, `world-settings`, `light`, `vision-modes`, `light-gradation`
//! engine bands. Field shapes mirror the client's re-exported
//! `SceneEngine`, `WorldSceneDefaults`, `WorldSettingsEngine`, `LightEngine`,
//! `VisionMode`, and `GradationBand` (minus `name`, which lives on the
//! envelope instead).

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// Which movement engine a scene uses — the dispatch axis between the grid
/// A* pathfinder and the continuous/navmesh router (`SceneEcs::pathfind`).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::MovementModel;
///
/// assert_ne!(MovementModel::GridStepped, MovementModel::Continuous);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "kebab-case")]
pub enum MovementModel {
    /// Cell-to-cell movement on the grid (A* over cells).
    GridStepped,
    /// Free continuous movement routed on the navmesh (polyanya).
    Continuous,
}

/// How far a player-driven token may reach. The per-cell traversal gate is
/// `scene::move_exec::execute_move`/`gate_walk` (the sole traversal decision);
/// `Room::publish` additionally consults this for its Create-placement gate
/// (center cell only).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::MovementRestriction;
///
/// let restriction = MovementRestriction::Visible;
/// assert_eq!(restriction, MovementRestriction::Visible);
/// assert_ne!(restriction, MovementRestriction::Unrestricted);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "lowercase")]
pub enum MovementRestriction {
    /// Only cells the mover can currently see.
    Visible,
    /// Anywhere already revealed (explored fog), seen now or before.
    Revealed,
    /// No visibility gating (walls/regions still apply).
    Unrestricted,
}

/// Scene lighting mode.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::LightMode;
///
/// assert_ne!(LightMode::GlobalIllumination, LightMode::EnvironmentLight);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
pub enum LightMode {
    /// Everything fully lit; placed lights are cosmetic.
    #[serde(rename = "globalIllumination")]
    GlobalIllumination,
    /// The scene's ambient `EnvironmentLight` + placed lights drive
    /// illumination (and therefore lighting-aware vision).
    #[serde(rename = "environmentLight")]
    EnvironmentLight,
}

/// Diagonal-step cost rule for the grid pathfinder (`pathfinding::find`).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::DiagonalRule;
///
/// let rule = DiagonalRule::Euclidean;
/// assert_eq!(rule, DiagonalRule::Euclidean);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "lowercase")]
pub enum DiagonalRule {
    /// Diagonals cost the same as orthogonals (D&D 5e-style).
    Chebyshev,
    /// Diagonals alternate 1-2-1-2 cells (PF/3.5-style).
    Alternating,
    /// Diagonals cost sqrt(2).
    Euclidean,
    /// Diagonals cost the same as two orthogonal steps (no diagonal
    /// shortcut; diagonal moves themselves stay legal).
    Manhattan,
}

/// Token move-animation easing.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::EasingMode;
///
/// assert_ne!(EasingMode::EaseInOut, EasingMode::Linear);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
pub enum EasingMode {
    /// Accelerate then decelerate.
    #[serde(rename = "easeInOut")]
    EaseInOut,
    /// Constant speed.
    #[serde(rename = "linear")]
    Linear,
}

/// Ambient scene light for `LightMode::EnvironmentLight`.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::EnvironmentLight;
///
/// let light = EnvironmentLight { color: "#0a0e1a".to_string(), intensity: 0.2 };
/// assert_eq!(light.intensity, 0.2);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct EnvironmentLight {
    /// `#rrggbb` ambient color.
    pub color: String,
    /// Ambient level 0..=1 (0 = darkness, placed lights only).
    pub intensity: f64,
}

/// A scene's authored dimensions in GRID UNITS (width × height cells).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::SceneDimensions;
///
/// let dims = SceneDimensions { width: 30.0, height: 20.0 };
/// assert_eq!(dims.width * dims.height, 600.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct SceneDimensions {
    /// Width in grid cells.
    pub width: f64,
    /// Height in grid cells.
    pub height: f64,
}

/// Distance-per-cell scale for a scene grid. `unit` is a display label
/// (e.g. "ft", "m").
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::GridDistance;
///
/// let distance = GridDistance { per_cell: 5.0, unit: "ft".to_string() };
/// assert_eq!(distance.per_cell, 5.0);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GridDistance {
    /// Distance one cell represents, in `unit`s.
    pub per_cell: f64,
    /// Display label (e.g. "ft", "m") — never interpreted.
    pub unit: String,
}

/// A scene's grid geometry. There is NO fallback size anywhere downstream:
/// consumers refuse (`None`/empty) on an absent grid rather than synthesizing
/// a default (`scene_grid_sizes` is the sole defaulting source).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::Grid;
///
/// let grid = Grid { kind: "square".to_string(), size: 50.0, distance: None };
/// assert!(grid.distance.is_none());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Grid {
    /// "square" | "hex" — kept a `String` in v1 (asserted by the battery).
    pub kind: String,
    /// Cell size in scene units. For hex grids this is the OUTER radius
    /// (center-to-vertex circumradius) — `HexGrid` and the client's `GridSpec.size`
    /// share this convention so cell indices always agree.
    pub size: f64,
    /// Real-world distance scale; absent = unitless.
    #[serde(default)]
    pub distance: Option<GridDistance>,
}

/// Per-scene overrides for vision behaviour; absent fields fall back to world
/// defaults. `null` is a valid wire value (the UI writes null to clear an
/// override); resolvers use `??`, so `null`/absent are semantically
/// identical — a stored explicit null re-serializes as absent, which is
/// semantically lossless.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::SceneVisionOverrides;
///
/// let overrides = SceneVisionOverrides {
///     los_restriction: Some(true),
///     fog: None,
///     observer_vision: None,
///     movement_restriction: None,
///     movement_model: None,
/// };
/// assert_eq!(overrides.los_restriction, Some(true));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SceneVisionOverrides {
    /// Line-of-sight restriction override (walls occlude sight).
    #[serde(default)]
    pub los_restriction: Option<bool>,
    /// Fog-of-war override.
    #[serde(default)]
    pub fog: Option<bool>,
    /// Whether observers (non-owners) contribute vision override.
    #[serde(default)]
    pub observer_vision: Option<bool>,
    /// Movement-gate policy override.
    #[serde(default)]
    pub movement_restriction: Option<MovementRestriction>,
    /// Movement-engine override.
    #[serde(default)]
    pub movement_model: Option<MovementModel>,
}

/// Per-scene overrides for lighting; same null-vs-absent equivalence as
/// `SceneVisionOverrides`.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::SceneLightingOverrides;
///
/// let overrides = SceneLightingOverrides { enabled: Some(false), mode: None, environment: None };
/// assert_eq!(overrides.enabled, Some(false));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SceneLightingOverrides {
    /// Lighting on/off override.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Light-mode override.
    #[serde(default)]
    pub mode: Option<LightMode>,
    /// Ambient-light override.
    #[serde(default)]
    pub environment: Option<EnvironmentLight>,
}

/// A scene's engine-owned config (mirrors the client's `SceneEngine`).
/// `bounds` = the authored play-area rectangle in grid units, which the
/// continuous router and the per-player vision/lighting path both read;
/// absent ⇒ `DEFAULT_SCENE_BOUNDS_UNITS` (read-side backstop, unchanged).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::{Grid, SceneEngine};
///
/// let scene = SceneEngine {
///     grid: Grid { kind: "square".to_string(), size: 50.0, distance: None },
///     background: None,
///     bounds: None,
///     snap_to_grid: None,
///     vision: None,
///     lighting: None,
///     combat: None,
///     ambience: None,
/// };
/// assert_eq!(scene.grid.size, 50.0);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SceneEngine {
    /// The scene's grid geometry.
    pub grid: Grid,
    /// Background image asset id; wire-required but nullable.
    pub background: Option<String>,
    /// The authored play-area rectangle in grid units; absent = the read-side
    /// default. `GridShape::world_extent` converts it to world units for TWO
    /// consumer families, not one: `navmesh::build_navmesh` triangulates that
    /// rectangle directly, and reaches the vision paths through
    /// `vision::bound_for_scene`. So a change here moves what a player is told
    /// they can see, not only where a route may run.
    #[serde(default)]
    pub bounds: Option<SceneDimensions>,
    /// Scene-level snap-to-grid toggle, independent of `movementModel`.
    /// Absent ⇒ derived default resolved at read time (false for a
    /// continuous scene, true otherwise) — reading this field alone is NOT
    /// the effective value.
    #[serde(default)]
    pub snap_to_grid: Option<bool>,
    /// Per-scene vision overrides; absent fields fall back to world defaults.
    #[serde(default)]
    pub vision: Option<SceneVisionOverrides>,
    /// Per-scene lighting overrides; absent fields fall back to world defaults.
    #[serde(default)]
    pub lighting: Option<SceneLightingOverrides>,
    /// Combat-rule overrides (movement resource / interpretation / enforcement /
    /// turn control); absent fields fall through the chain
    /// (`combat::resolve_combat_rules`).
    #[serde(default)]
    pub combat: Option<super::combat::CombatDefaults>,
    /// This scene's ambient playlist override; `None` = no ambience plays when this scene
    /// becomes active. See `SceneAmbience`.
    #[serde(default)]
    pub ambience: Option<SceneAmbience>,
}

/// The full set of world-level scene defaults that individual scenes may
/// override (mirrors the client's `WorldSceneDefaults`).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::WorldSceneDefaults;
///
/// let defaults = WorldSceneDefaults::default();
/// assert!(defaults.los_restriction);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WorldSceneDefaults {
    /// Walls occlude sight by default.
    pub los_restriction: bool,
    /// Fog of war on by default.
    pub fog: bool,
    /// Lighting simulation on by default.
    pub lighting_enabled: bool,
    /// Default light mode.
    pub light_mode: LightMode,
    /// Default ambient light.
    pub environment: EnvironmentLight,
    /// Whether non-owner observers contribute vision by default.
    pub observer_vision: bool,
    /// Default movement-gate policy.
    pub movement_restriction: MovementRestriction,
    /// Default movement engine.
    pub movement_model: MovementModel,
    /// Grid gate counts a cell partially inside vision as reachable.
    pub partial_cell_leniency: bool,
}

/// A scene's ambient playlist override: when this scene becomes the world's active scene
/// (`world-settings.activeScene` commits), `audio::transport::on_active_scene` stops the
/// previous scene's ambience entries and starts this one — one server rule, no client
/// involvement.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::scene::SceneAmbience;
///
/// let ambience = SceneAmbience { playlist: uuid::Uuid::new_v4(), gain: 0.6 };
/// assert_eq!(ambience.gain, 0.6);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SceneAmbience {
    /// The playlist document to play as this scene's ambience.
    pub playlist: Uuid,
    /// Ambience gain multiplier, `0..=1` (presentation range; ingress validates finiteness).
    pub gain: f64,
}

/// World pathfinding settings.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::{DiagonalRule, Pathfinding};
///
/// let pf = Pathfinding::default();
/// assert_eq!(pf.diagonal_rule, DiagonalRule::Chebyshev);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Pathfinding {
    /// Diagonal-step cost rule for the grid pathfinder.
    pub diagonal_rule: DiagonalRule,
}

/// Token move-animation settings.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::AnimationSettings;
///
/// let anim = AnimationSettings::default();
/// assert_eq!(anim.speed_cells_per_sec, 6.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AnimationSettings {
    /// Playback speed, grid cells per second.
    pub speed_cells_per_sec: f64,
    /// Easing curve.
    pub easing: EasingMode,
}

impl SceneEngine {
    /// Every combat lifecycle formula present parses, and the ambience override's gain is
    /// finite when present.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if let Some(a) = &self.ambience {
            if !a.gain.is_finite() {
                return Err("ambience gain must be finite".to_string());
            }
        }
        match &self.combat {
            Some(c) => c.validate("combat"),
            None => Ok(()),
        }
    }
}

/// Wall-occlusion policy for the `"audibility"` derived channel.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::scene::Occlusion;
///
/// assert_ne!(Occlusion::Walls, Occlusion::None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "camelCase")]
pub enum Occlusion {
    /// `los`-band walls attenuate an emitter to `throughWallGain`.
    Walls,
    /// No occlusion: every emitter's gain is falloff-only, walls ignored.
    None,
}

/// World-level audio overlay (mirrors the client's `AudioOverlay`). Every leaf is optional;
/// absent means the engine-literal default listed on each field.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::scene::AudioOverlay;
///
/// let overlay = AudioOverlay::default();
/// assert!(overlay.spatial.is_none());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct AudioOverlay {
    /// Spatial attenuation + occlusion on/off; absent = `true`. `false` ⇒ every emitter
    /// mixes flat at channel gain (`scene::audibility`'s `spatial=false` short-circuit).
    pub spatial: Option<bool>,
    /// Wall-occlusion policy; absent = `Occlusion::Walls`.
    pub occlusion: Option<Occlusion>,
    /// Gain an occluded emitter is reduced to — never silenced entirely, so a player still
    /// learns something is behind the door; absent = `0.25`.
    #[serde(rename = "throughWallGain")]
    pub through_wall_gain: Option<f64>,
}

impl AudioOverlay {
    /// Ingress validation beyond serde shape: `throughWallGain`, when present, is finite.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if let Some(g) = self.through_wall_gain {
            if !g.is_finite() {
                return Err("throughWallGain must be finite".to_string());
            }
        }
        Ok(())
    }
}

/// The engine body of a "world-settings" config document: the world layer of
/// the settings chain, an `Option`-lifted overlay (mirrors the client's
/// `WorldSettingsEngine`). Every leaf is optional; absent and `null` both
/// mean "fall through" to `system-defaults`, then to the engine literals on
/// `WorldSceneDefaults::default`/`Pathfinding::default`/
/// `AnimationSettings::default`. Derived `Default` is the empty overlay —
/// what the world-config seed authors.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::WorldSettingsEngine;
///
/// let overlay = WorldSettingsEngine::default();
/// assert!(overlay.scene.is_none());
/// assert!(overlay.active_scene.is_none());
/// assert!(overlay.audio.is_none());
/// ```
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct WorldSettingsEngine {
    /// World-level scene-defaults overlay (scenes override per field).
    #[ts(optional = nullable)]
    pub scene: Option<super::system_defaults::SceneDefaultsOverlay>,
    /// Pathfinding overlay.
    #[ts(optional = nullable)]
    pub pathfinding: Option<super::system_defaults::PathfindingOverlay>,
    /// Move-animation overlay.
    #[ts(optional = nullable)]
    pub animation: Option<super::system_defaults::AnimationOverlay>,
    /// The scene players render. `None`/absent/dangling ⇒ the first scene.
    #[ts(optional = nullable)]
    pub active_scene: Option<Uuid>,
    /// Combat-rule overrides (movement resource / interpretation / enforcement /
    /// turn control); absent fields fall through the chain
    /// (`combat::resolve_combat_rules`).
    #[ts(optional = nullable)]
    pub combat: Option<super::combat::CombatDefaults>,
    /// World-level audio overlay (spatial/occlusion/through-wall gain); absent fields fall
    /// back to the engine literals on `AudioOverlay`'s own doc.
    #[ts(optional = nullable)]
    pub audio: Option<AudioOverlay>,
}

impl WorldSettingsEngine {
    /// The overlay range checks shared with `SystemDefaultsEngine::validate`
    /// (animation speed, environment intensity), plus every combat lifecycle
    /// formula present parses and the audio overlay's own field checks.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if let Some(c) = &self.combat {
            c.validate("combat")?;
        }
        if let Some(a) = &self.animation {
            a.validate()?;
        }
        if let Some(s) = &self.scene {
            s.validate()?;
        }
        if let Some(a) = &self.audio {
            a.validate()?;
        }
        Ok(())
    }
}

/// The engine-literal innermost fallback of the settings chain — the ONE
/// source every resolver's final `unwrap_or` reads. The client's
/// `DEFAULT_WORLD_SETTINGS` mirrors these values, asserted by a unit test.
impl Default for WorldSceneDefaults {
    fn default() -> Self {
        WorldSceneDefaults {
            los_restriction: true,
            fog: true,
            lighting_enabled: true,
            light_mode: LightMode::EnvironmentLight,
            environment: EnvironmentLight {
                color: "#0a0e1a".to_string(),
                intensity: 0.0,
            },
            observer_vision: false,
            movement_restriction: MovementRestriction::Visible,
            movement_model: MovementModel::GridStepped,
            partial_cell_leniency: true,
        }
    }
}

/// Engine-literal pathfinding fallback (see `WorldSceneDefaults::default`).
impl Default for Pathfinding {
    fn default() -> Self {
        Pathfinding {
            diagonal_rule: DiagonalRule::Chebyshev,
        }
    }
}

/// Engine-literal animation fallback (see `WorldSceneDefaults::default`).
impl Default for AnimationSettings {
    fn default() -> Self {
        AnimationSettings {
            speed_cells_per_sec: 6.0,
            easing: EasingMode::EaseInOut,
        }
    }
}

/// A placed standalone light source: a position plus its emission payload
/// (mirrors the client's `LightEngine`). The emission shape lives exactly
/// once, in `LightEmission` — a carried emission is the same payload resolved
/// at a token's live position.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::{LightEmission, LightEngine};
///
/// let light = LightEngine {
///     x: 10.0,
///     y: 10.0,
///     elevation: None,
///     emission: LightEmission {
///         color: "#ffffff".to_string(),
///         intensity: 1.0,
///         bright_radius: 5.0,
///         dim_radius: 10.0,
///         falloff: None,
///         enabled: true,
///     },
/// };
/// assert_eq!(light.emission.bright_radius, 5.0);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LightEngine {
    /// Position x, scene units.
    pub x: f64,
    /// Position y, scene units.
    pub y: f64,
    /// Elevation above the scene's ground plane (`None`/absent = 0, grounded);
    /// decides which walls' elevation bands occlude this light. A carried
    /// emission takes its token's elevation instead (`TokenEngine::elevation`).
    #[serde(default)]
    pub elevation: Option<f64>,
    /// The light's photometric payload.
    pub emission: LightEmission,
}

/// A light emitter's photometric payload — everything about a light except
/// where it is: shared by standalone `light` documents (`LightEngine.emission`)
/// and token/actor-carried emissions (`ActorEngine.light`,
/// `TokenOverrides.light`). `brightRadius`/`dimRadius` are in grid cells.
/// Every carrier validates it at ingress through `LightEmission::validate`.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::LightEmission;
///
/// let emission = LightEmission {
///     color: "#ffcc88".to_string(),
///     intensity: 0.8,
///     bright_radius: 4.0,
///     dim_radius: 8.0,
///     falloff: None,
///     enabled: true,
/// };
/// assert!(emission.enabled);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LightEmission {
    /// `#rrggbb` light color.
    pub color: String,
    /// Emission strength 0..=1 at the source.
    pub intensity: f64,
    /// Full-brightness radius, grid cells.
    pub bright_radius: f64,
    /// Dim-light outer radius, grid cells.
    pub dim_radius: f64,
    /// Brightness falloff curve; absent = linear (read-side default).
    #[serde(default)]
    pub falloff: Option<Falloff>,
    /// GM toggle; a disabled emission contributes nothing (the suppress path
    /// for a carried emission, the on/off switch for a standalone light).
    pub enabled: bool,
}

impl LightEmission {
    /// Ingress validation beyond serde shape, the posture every engine struct
    /// carries: `intensity` finite (its `0..=1` range is a read-side clamp),
    /// both radii finite, non-negative and bounded by the shared cell cap
    /// `scene::pathfinding::MAX_FOOTPRINT_CELLS` — the bound the aura and the
    /// footprint already share — so no authored glow can reach a scan bound
    /// the egress clip would otherwise have to fail closed on
    /// (`ws::move_clip::glow_reaches`).
    ///
    /// # Examples
    ///
    /// ```text
    /// emission.validate()  // Err("dimRadius exceeds 64") past the cap
    /// ```
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !self.intensity.is_finite() {
            return Err("intensity must be finite".to_string());
        }
        let cap = crate::scene::pathfinding::MAX_FOOTPRINT_CELLS;
        for (name, r) in [
            ("brightRadius", self.bright_radius),
            ("dimRadius", self.dim_radius),
        ] {
            if !r.is_finite() || r < 0.0 {
                return Err(format!("{name} must be finite and non-negative"));
            }
            if r > cap {
                return Err(format!("{name} exceeds {cap}"));
            }
        }
        Ok(())
    }
}

impl LightEngine {
    /// Ingress validation beyond serde shape: position and elevation finite,
    /// and the emission through `LightEmission::validate`.
    pub(crate) fn validate(&self) -> Result<(), String> {
        for (name, v) in [("x", self.x), ("y", self.y)] {
            if !v.is_finite() {
                return Err(format!("{name} must be finite"));
            }
        }
        if let Some(e) = self.elevation {
            if !e.is_finite() {
                return Err("elevation must be finite".to_string());
            }
        }
        self.emission.validate()
    }
}

/// A falloff curve wrapper. `curve` defaults to `FalloffCurve::Linear`
/// (read-side) when the whole `falloff` key is absent from `LightEmission`.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::{Falloff, FalloffCurve};
///
/// let falloff = Falloff { curve: FalloffCurve::Quadratic };
/// assert_eq!(falloff.curve, FalloffCurve::Quadratic);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Falloff {
    /// The taper curve across the dim band.
    pub curve: FalloffCurve,
}

/// Photometric falloff curve identifier across the dim band
/// `(brightRadius, dimRadius]`, mirroring `lighting::Falloff`'s variants.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::FalloffCurve;
///
/// assert_ne!(FalloffCurve::Linear, FalloffCurve::None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "camelCase")]
pub enum FalloffCurve {
    /// Smooth linear taper from full intensity at the bright edge to 0 at the dim edge.
    Linear,
    /// Smooth quadratic taper (faster than linear).
    Quadratic,
    /// No gradient: a flat dim-band step (`0.5 × intensity`) — bright/dim radii feed the
    /// gradation bands directly.
    None,
}

/// What a vision mode perceives. Terrain senses (normal sight, darkvision)
/// see the SCENE: their reach is `LOS ∩ illumination ≥ floor`, range-limited.
/// Creature senses (tremorsense) perceive TOKENS: grounded tokens within
/// range, ignoring illumination and — when `VisionMode::requires_los` is
/// false — walls. The lit-mask pipeline reads terrain senses only; creature
/// senses feed the `perceived` token list (`SceneEcs::player_perceived_tokens`).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::Perception;
///
/// assert_eq!(Perception::default(), Perception::Terrain);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "lowercase")]
pub enum Perception {
    /// The ordinary LOS ∩ illumination-floor mask.
    #[default]
    Terrain,
    /// Perceives grounded tokens within range, not terrain.
    Creatures,
}

/// Serde default for `VisionMode::requires_los` — a mode's reach is
/// wall-bounded unless it declares otherwise.
fn default_requires_los() -> bool {
    true
}

/// A named vision mode that tokens/actors may possess (mirrors the client's
/// `VisionMode`). `illuminationFloor`: the lowest gradation band name a token
/// with this mode can see into (inert for creature senses — creature
/// perception never reads the illumination field). `defaultRange`: effective
/// sight distance in grid cells (0 = unlimited).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::{Perception, VisionMode};
///
/// let mode = VisionMode {
///     id: "normal".to_string(),
///     name: "Normal".to_string(),
///     illumination_floor: "dim".to_string(),
///     default_range: 0.0,
///     perceives: Perception::Terrain,
///     requires_los: true,
///     render_hint: None,
/// };
/// assert_eq!(mode.perceives, Perception::Terrain);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct VisionMode {
    /// Stable id `VisionAssignment.mode` references.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Lowest gradation band name this mode can see into.
    pub illumination_floor: String,
    /// Default sight distance in grid cells (0 = unlimited).
    pub default_range: f64,
    /// What the mode perceives; absent = `terrain`, so every mode authored
    /// before this field existed is unchanged.
    #[serde(default)]
    pub perceives: Perception,
    /// Whether sight walls bound the mode's reach; absent = `true`. Consulted
    /// by creature senses (tremorsense declares `false`); terrain senses are
    /// always LOS-gated by the mask pipeline itself.
    #[serde(default = "default_requires_los")]
    pub requires_los: bool,
    /// Optional client render treatment tag (e.g. a tint); never interpreted
    /// server-side.
    #[serde(default)]
    pub render_hint: Option<String>,
}

/// The engine body of a "vision-modes" config document.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::VisionModesEngine;
///
/// let engine = VisionModesEngine::seed();
/// assert!(engine.modes.contains_key("normal"));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct VisionModesEngine {
    /// Vision modes keyed by mode id.
    pub modes: BTreeMap<String, VisionMode>,
}

impl VisionModesEngine {
    /// Default world seed: `normal` (dim floor, unlimited range), `darkvision`
    /// (dark floor, 12 cells, desaturate render hint), and `tremorsense`
    /// (creature sense: grounded tokens within 12 cells, ignoring walls and
    /// illumination; its illumination floor is inert). The engine definition —
    /// the client's `SEED_VISION_MODES` mirrors this.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::engine::{Perception, VisionModesEngine};
    ///
    /// let s = VisionModesEngine::seed();
    /// assert_eq!(s.modes["darkvision"].default_range, 12.0);
    /// assert_eq!(s.modes["tremorsense"].perceives, Perception::Creatures);
    /// ```
    pub fn seed() -> Self {
        let mut modes = BTreeMap::new();
        modes.insert(
            "normal".to_string(),
            VisionMode {
                id: "normal".to_string(),
                name: "Normal".to_string(),
                illumination_floor: "dim".to_string(),
                default_range: 0.0,
                perceives: Perception::Terrain,
                requires_los: true,
                render_hint: None,
            },
        );
        modes.insert(
            "darkvision".to_string(),
            VisionMode {
                id: "darkvision".to_string(),
                name: "Darkvision".to_string(),
                illumination_floor: "dark".to_string(),
                default_range: 12.0,
                perceives: Perception::Terrain,
                requires_los: true,
                render_hint: Some("desaturate".to_string()),
            },
        );
        modes.insert(
            "tremorsense".to_string(),
            VisionMode {
                id: "tremorsense".to_string(),
                name: "Tremorsense".to_string(),
                // Inert: a creature sense never reads the illumination field.
                illumination_floor: "dark".to_string(),
                default_range: 12.0,
                perceives: Perception::Creatures,
                requires_los: false,
                render_hint: None,
            },
        );
        Self { modes }
    }
}

/// A named illumination band (mirrors the client's `GradationBand`).
/// `minIllumination` is the minimum light level `[0,1]` a cell must reach to
/// qualify; bands are sorted brightest-first at resolution time.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::scene::GradationBand;
///
/// let band = GradationBand { name: "bright".to_string(), min_illumination: 0.67 };
/// assert_eq!(band.min_illumination, 0.67);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GradationBand {
    /// Band name (`VisionMode.illumination_floor` references it).
    pub name: String,
    /// Minimum light level `[0,1]` a cell must reach to qualify.
    pub min_illumination: f64,
}

/// The engine body of a "light-gradation" config document.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::LightGradationEngine;
///
/// let engine = LightGradationEngine::seed();
/// assert_eq!(engine.bands[0].name, "bright");
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct LightGradationEngine {
    /// The world's illumination bands (sorted brightest-first at resolution).
    pub bands: Vec<GradationBand>,
}

impl LightGradationEngine {
    /// Default three-band world seed (bright ≥ 0.67, dim ≥ 0.34, dark ≥ 0).
    /// The engine definition — the client's `DEFAULT_GRADATION` mirrors this.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::data::engine::LightGradationEngine;
    ///
    /// assert_eq!(LightGradationEngine::seed().bands.len(), 3);
    /// ```
    pub fn seed() -> Self {
        Self {
            bands: vec![
                GradationBand {
                    name: "bright".to_string(),
                    min_illumination: 0.67,
                },
                GradationBand {
                    name: "dim".to_string(),
                    min_illumination: 0.34,
                },
                GradationBand {
                    name: "dark".to_string(),
                    min_illumination: 0.0,
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests;
