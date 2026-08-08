//! `scene`, `world-settings`, `light`, `vision-modes`, `light-gradation`
//! engine bands. Field shapes mirror the client's re-exported
//! `SceneEngine`, `WorldSceneDefaults`, `WorldSettingsEngine`, `LightEngine`,
//! `VisionMode`, and `GradationBand` (minus `name`, which lives on the
//! envelope instead).

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two deny attributes below.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// Which movement engine a scene uses — the dispatch axis between the grid
/// A* pathfinder and the continuous/navmesh router (`SceneEcs::pathfind`).
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
/// `bounds` = the navmesh's outer rectangle in grid units; absent ⇒
/// `DEFAULT_SCENE_BOUNDS_UNITS` (read-side backstop, unchanged).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SceneEngine {
    /// The scene's grid geometry.
    pub grid: Grid,
    /// Background image asset id; wire-required but nullable.
    pub background: Option<String>,
    /// Navmesh outer rectangle in grid units; absent = the read-side default.
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
}

/// The full set of world-level scene defaults that individual scenes may
/// override (mirrors the client's `WorldSceneDefaults`).
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

/// World pathfinding settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Pathfinding {
    /// Diagonal-step cost rule for the grid pathfinder.
    pub diagonal_rule: DiagonalRule,
}

/// Token move-animation settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AnimationSettings {
    /// Playback speed, grid cells per second.
    pub speed_cells_per_sec: f64,
    /// Easing curve.
    pub easing: EasingMode,
}

/// The engine body of a "world-settings" config document
/// (mirrors the client's `WorldSettingsEngine`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WorldSettingsEngine {
    /// World-level scene defaults (scenes override per field).
    pub scene: WorldSceneDefaults,
    /// Pathfinding settings.
    pub pathfinding: Pathfinding,
    /// Move-animation settings.
    pub animation: AnimationSettings,
    /// The scene players render. `None`/absent/dangling ⇒ the first scene
    /// (legacy behavior). Deliberately NOT part of the structural-
    /// completeness triple below, so a world-settings doc written before
    /// this field existed is still "complete" and keeps its authored
    /// settings.
    #[serde(default)]
    pub active_scene: Option<Uuid>,
}

/// MUST equal the client's `DEFAULT_WORLD_SETTINGS` —
/// asserted by a unit test (`server-mirrors-client` rule).
impl Default for WorldSettingsEngine {
    fn default() -> Self {
        WorldSettingsEngine {
            scene: WorldSceneDefaults {
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
            },
            pathfinding: Pathfinding {
                diagonal_rule: DiagonalRule::Chebyshev,
            },
            animation: AnimationSettings {
                speed_cells_per_sec: 6.0,
                easing: EasingMode::EaseInOut,
            },
            active_scene: None,
        }
    }
}

/// A placed light source: position, photometric properties, and an optional
/// falloff curve (mirrors the client's `LightEngine`). `brightRadius`/
/// `dimRadius` are in grid cells.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LightEngine {
    /// Position x, scene units.
    pub x: f64,
    /// Position y, scene units.
    pub y: f64,
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
    /// GM toggle; a disabled light emits nothing.
    pub enabled: bool,
}

/// `curve` defaults to "linear" (read-side, unchanged) when absent. Kept a
/// `String` in v1 (asserted by the battery), matching `"linear" | "quadratic"
/// | "none"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Falloff {
    /// "linear" | "quadratic" | "none" — kept a `String` in v1.
    pub curve: String,
}

/// A named vision mode that tokens/actors may possess (mirrors the client's
/// `VisionMode`). `illuminationFloor`: the lowest gradation band name a token
/// with this mode can see into. `defaultRange`: effective sight distance in
/// grid cells (0 = unlimited).
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
    /// Optional client render treatment tag (e.g. a tint); never interpreted
    /// server-side.
    #[serde(default)]
    pub render_hint: Option<String>,
}

/// The engine body of a "vision-modes" config document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct VisionModesEngine {
    /// Vision modes keyed by mode id.
    pub modes: BTreeMap<String, VisionMode>,
}

/// A named illumination band (mirrors the client's `GradationBand`).
/// `minIllumination` is the minimum light level `[0,1]` a cell must reach to
/// qualify; bands are sorted brightest-first at resolution time.
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct LightGradationEngine {
    /// The world's illumination bands (sorted brightest-first at resolution).
    pub bands: Vec<GradationBand>,
}
