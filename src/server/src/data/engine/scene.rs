//! `scene`, `world-settings`, `light`, `vision-modes`, `light-gradation`
//! engine bands (M13-0 S1/S3). Field shapes transcribed verbatim from
//! scene-docs.ts (minus `name`, moved to the envelope per S2).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "kebab-case")]
pub enum MovementModel {
    GridStepped,
    Continuous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "lowercase")]
pub enum MovementRestriction {
    Visible,
    Revealed,
    Unrestricted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
pub enum LightMode {
    #[serde(rename = "globalIllumination")]
    GlobalIllumination,
    #[serde(rename = "environmentLight")]
    EnvironmentLight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "lowercase")]
pub enum DiagonalRule {
    Chebyshev,
    Alternating,
    Euclidean,
    Manhattan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
pub enum EasingMode {
    #[serde(rename = "easeInOut")]
    EaseInOut,
    #[serde(rename = "linear")]
    Linear,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct EnvironmentLight {
    pub color: String,
    pub intensity: f64,
}

/// A scene's authored dimensions in GRID UNITS (width × height cells).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct SceneDimensions {
    pub width: f64,
    pub height: f64,
}

/// Distance-per-cell scale for a scene grid. `unit` is a display label
/// (e.g. "ft", "m").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GridDistance {
    pub per_cell: f64,
    pub unit: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Grid {
    /// "square" | "hex" — kept a `String` in v1 (asserted by the battery).
    pub kind: String,
    pub size: f64,
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
    #[serde(default)]
    pub los_restriction: Option<bool>,
    #[serde(default)]
    pub fog: Option<bool>,
    #[serde(default)]
    pub observer_vision: Option<bool>,
    #[serde(default)]
    pub movement_restriction: Option<MovementRestriction>,
    #[serde(default)]
    pub movement_model: Option<MovementModel>,
}

/// Per-scene overrides for lighting; same null-vs-absent equivalence as
/// `SceneVisionOverrides`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SceneLightingOverrides {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub mode: Option<LightMode>,
    #[serde(default)]
    pub environment: Option<EnvironmentLight>,
}

/// A scene's engine-owned config (scene-docs.ts:63-73 `SceneSystem`).
/// `bounds` = the navmesh's outer rectangle in grid units; absent ⇒
/// `DEFAULT_SCENE_BOUNDS_UNITS` (read-side backstop, unchanged).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SceneEngine {
    pub grid: Grid,
    pub background: Option<String>,
    #[serde(default)]
    pub bounds: Option<SceneDimensions>,
    /// Scene-level snap-to-grid toggle, independent of `movementModel`.
    /// Absent ⇒ derived default resolved at read time (false for a
    /// continuous scene, true otherwise) — reading this field alone is NOT
    /// the effective value.
    #[serde(default)]
    pub snap_to_grid: Option<bool>,
    #[serde(default)]
    pub vision: Option<SceneVisionOverrides>,
    #[serde(default)]
    pub lighting: Option<SceneLightingOverrides>,
}

/// The full set of world-level scene defaults that individual scenes may
/// override (scene-docs.ts:78-88 `WorldSceneDefaults`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WorldSceneDefaults {
    pub los_restriction: bool,
    pub fog: bool,
    pub lighting_enabled: bool,
    pub light_mode: LightMode,
    pub environment: EnvironmentLight,
    pub observer_vision: bool,
    pub movement_restriction: MovementRestriction,
    pub movement_model: MovementModel,
    pub partial_cell_leniency: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Pathfinding {
    pub diagonal_rule: DiagonalRule,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AnimationSettings {
    pub speed_cells_per_sec: f64,
    pub easing: EasingMode,
}

/// The engine body of a "world-settings" config document
/// (scene-docs.ts:90-99 `WorldSettingsSystem`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WorldSettingsEngine {
    pub scene: WorldSceneDefaults,
    pub pathfinding: Pathfinding,
    pub animation: AnimationSettings,
    /// The scene players render. `None`/absent/dangling ⇒ the first scene
    /// (legacy behavior). Deliberately NOT part of the structural-
    /// completeness triple below, so a pre-M12d world-settings doc missing
    /// this key is still "complete" and keeps its authored settings.
    #[serde(default)]
    pub active_scene: Option<Uuid>,
}

/// MUST equal the client `DEFAULT_WORLD_SETTINGS` (scene-docs.ts:104-119) —
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
/// falloff curve (scene-docs.ts:532-541 `LightSystem`). `brightRadius`/
/// `dimRadius` are in grid cells.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct LightEngine {
    pub x: f64,
    pub y: f64,
    pub color: String,
    pub intensity: f64,
    pub bright_radius: f64,
    pub dim_radius: f64,
    #[serde(default)]
    pub falloff: Option<Falloff>,
    pub enabled: bool,
}

/// `curve` defaults to "linear" (read-side, unchanged) when absent. Kept a
/// `String` in v1 (asserted by the battery), matching `"linear" | "quadratic"
/// | "none"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Falloff {
    pub curve: String,
}

/// A named vision mode that tokens/actors may possess (scene-docs.ts:494-500
/// `VisionMode`). `illuminationFloor`: the lowest gradation band name a token
/// with this mode can see into. `defaultRange`: effective sight distance in
/// grid cells (0 = unlimited).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct VisionMode {
    pub id: String,
    pub name: String,
    pub illumination_floor: String,
    pub default_range: f64,
    #[serde(default)]
    pub render_hint: Option<String>,
}

/// The engine body of a "vision-modes" config document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct VisionModesEngine {
    pub modes: BTreeMap<String, VisionMode>,
}

/// A named illumination band (scene-docs.ts:456 `GradationBand`).
/// `minIllumination` is the minimum light level [0,1] a cell must reach to
/// qualify; bands are sorted brightest-first at resolution time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GradationBand {
    pub name: String,
    pub min_illumination: f64,
}

/// The engine body of a "light-gradation" config document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct LightGradationEngine {
    pub bands: Vec<GradationBand>,
}
