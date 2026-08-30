//! Engine band for `system-defaults`: the active system module's declared
//! defaults for every world setting. Resolved as the innermost non-engine
//! layer of every settings chain (engine literal → system → world → scene).
//! Every leaf is optional; absent and `null` both mean "fall through".

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::combat::CombatDefaults;
use super::scene::{
    DiagonalRule, EasingMode, EnvironmentLight, LightMode, MovementModel, MovementRestriction,
};

/// `Option`-lifted twin of `WorldSceneDefaults`; a field added there without
/// a twin here fails `world_scene_defaults_and_overlay_share_a_field_set`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct SceneDefaultsOverlay {
    /// Overrides `WorldSceneDefaults.los_restriction`.
    #[ts(optional = nullable)]
    pub los_restriction: Option<bool>,
    /// Overrides `WorldSceneDefaults.fog`.
    #[ts(optional = nullable)]
    pub fog: Option<bool>,
    /// Overrides `WorldSceneDefaults.lighting_enabled`.
    #[ts(optional = nullable)]
    pub lighting_enabled: Option<bool>,
    /// Overrides `WorldSceneDefaults.light_mode`.
    #[ts(optional = nullable)]
    pub light_mode: Option<LightMode>,
    /// Overrides `WorldSceneDefaults.environment`.
    #[ts(optional = nullable)]
    pub environment: Option<EnvironmentLight>,
    /// Overrides `WorldSceneDefaults.observer_vision`.
    #[ts(optional = nullable)]
    pub observer_vision: Option<bool>,
    /// Overrides `WorldSceneDefaults.movement_restriction`.
    #[ts(optional = nullable)]
    pub movement_restriction: Option<MovementRestriction>,
    /// Overrides `WorldSceneDefaults.movement_model`.
    #[ts(optional = nullable)]
    pub movement_model: Option<MovementModel>,
    /// Overrides `WorldSceneDefaults.partial_cell_leniency`.
    #[ts(optional = nullable)]
    pub partial_cell_leniency: Option<bool>,
}

/// `Option`-lifted twin of `Pathfinding`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct PathfindingOverlay {
    /// Overrides `Pathfinding.diagonal_rule`.
    #[ts(optional = nullable)]
    pub diagonal_rule: Option<DiagonalRule>,
}

/// `Option`-lifted twin of `AnimationSettings`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct AnimationOverlay {
    /// Overrides `AnimationSettings.speed_cells_per_sec`; must be finite and `> 0`.
    #[ts(optional = nullable)]
    pub speed_cells_per_sec: Option<f64>,
    /// Overrides `AnimationSettings.easing`.
    #[ts(optional = nullable)]
    pub easing: Option<EasingMode>,
}

/// The engine body of the `system-defaults` singleton (mirrors the client's
/// `SystemDefaultsEngine`). Written by the GM's client from the active system
/// module's declaration, never edited by hand; `active_scene` is world state,
/// not a setting, and has no overlay.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct SystemDefaultsEngine {
    /// Scene-defaults overlay.
    #[ts(optional = nullable)]
    pub scene: Option<SceneDefaultsOverlay>,
    /// Pathfinding overlay.
    #[ts(optional = nullable)]
    pub pathfinding: Option<PathfindingOverlay>,
    /// Animation overlay.
    #[ts(optional = nullable)]
    pub animation: Option<AnimationOverlay>,
    /// Combat-rule overlay (the same partial shape world and scene carry).
    #[ts(optional = nullable)]
    pub combat: Option<CombatDefaults>,
}

impl SystemDefaultsEngine {
    /// Numeric overlays are finite and positive where the world struct requires
    /// it, and every combat lifecycle formula parses.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if let Some(c) = &self.combat {
            c.validate("combat")?;
        }
        if let Some(a) = &self.animation {
            if let Some(s) = a.speed_cells_per_sec {
                if !s.is_finite() || s <= 0.0 {
                    return Err("animation.speedCellsPerSec must be finite and > 0".into());
                }
            }
        }
        if let Some(s) = &self.scene {
            if let Some(e) = &s.environment {
                if !(0.0..=1.0).contains(&e.intensity) {
                    return Err("scene.environment.intensity must be within 0..=1".into());
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
