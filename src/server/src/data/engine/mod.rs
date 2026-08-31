//! Typed `engine` band structs + the doc_type registry: the engine band
//! exists iff `doc_type` is engine-defined, and its stored JSON must
//! deserialize into that doc_type's struct — a strict ingress gate rather
//! than an opaque pointer-walked body.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub mod asset_folder;
pub mod combat;
pub mod geometry;
pub mod registries;
pub mod scene;
pub mod system_defaults;
pub mod token;

pub use combat::{
    resolve_combat_rules, CapturedCombatant, CombatDefaults, CombatEngine, CombatHistoryEngine,
    CombatantEngine, CombatantKind, CombatantResource, Duration, DurationUnit, EffectEngine,
    EffectLifecycle, EffectLifecycleDefaults, EffectSnapshot, Enforcement, ExpiryPoint, Formula,
    Interpretation, MovementRules, Recovery, ResolvedCombatRules, Resource, ResourceBinding,
    ResourceRegistryEngine, TurnControl, TurnRecord, MAX_TURN_HISTORY,
};
pub use geometry::{
    DrawingEngine, DrawingShape, Fill, RegionEngine, RegionShape, Seg, Stroke, TemplateEngine,
    TemplateShape, WallEngine,
};
pub use registries::{
    Channel, ChannelDiceOverride, ChannelRegistryEngine, ChatSettingsEngine, Condition,
    ConditionRegistryEngine, DiceDirectionSetting, DiceModeSetting, DiceSettingsEngine, Faction,
    FactionRegistryEngine, FactionStance,
};
pub use scene::{
    AnimationSettings, DiagonalRule, EasingMode, EnvironmentLight, Falloff, Grid, GridDistance,
    LightEngine, LightGradationEngine, LightMode, MovementModel, MovementRestriction, Pathfinding,
    SceneDimensions, SceneEngine, SceneLightingOverrides, SceneVisionOverrides, VisionMode,
    VisionModesEngine, WorldSceneDefaults, WorldSettingsEngine,
};
pub use system_defaults::{
    AnimationOverlay, PathfindingOverlay, SceneDefaultsOverlay, SystemDefaultsEngine,
};
pub use token::{
    ActorEngine, AnimatedSource, RenderVisual, Size, TokenEngine, TokenOverrides, TokenVisual,
    VisionAssignment,
};

use crate::data::DataError;

/// Doc_type for the world's singleton settings config document.
pub const WORLD_SETTINGS_DOC_TYPE: &str = "world-settings";
/// Doc_type for the world's singleton faction registry config document.
pub const FACTION_REGISTRY_DOC_TYPE: &str = "faction-registry";
/// Doc_type for the world's singleton condition registry config document.
pub const CONDITION_REGISTRY_DOC_TYPE: &str = "condition-registry";
/// Doc_type for the world's singleton channel registry config document.
pub const CHANNEL_REGISTRY_DOC_TYPE: &str = "channel-registry";
/// Doc_type for the world's singleton vision-modes config document.
pub const VISION_MODES_DOC_TYPE: &str = "vision-modes";
/// Doc_type for the world's singleton light-gradation config document.
pub const LIGHT_GRADATION_DOC_TYPE: &str = "light-gradation";
/// Doc_type for a combat: a world document bound to one scene.
pub const COMBAT_DOC_TYPE: &str = "combat";
/// Doc_type for a combatant: always a child (`parent_id`) of a combat.
pub const COMBATANT_DOC_TYPE: &str = "combatant";
/// Doc_type for the world's singleton turn-resource registry.
pub const RESOURCE_REGISTRY_DOC_TYPE: &str = "resource-registry";
/// Doc_type for an effect (embedded under actors/items, or standalone).
pub const EFFECT_DOC_TYPE: &str = "effect";
/// Doc_type for the world's singleton system-declared settings defaults.
pub const SYSTEM_DEFAULTS_DOC_TYPE: &str = "system-defaults";
/// Doc_type for a combat's turn-history log: always a child (`parent_id`) of the combat.
pub const COMBAT_HISTORY_DOC_TYPE: &str = "combat-history";
/// Doc_type for an asset folder (`assets.folder_id` names one); parent = `parent_id`.
pub const ASSET_FOLDER_DOC_TYPE: &str = "asset_folder";

/// Whether `doc_type` carries a typed `engine` band. The registry is a
/// hardcoded match — there is no dynamic registration (the server runs no
/// third-party code).
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::is_engine_doc_type;
///
/// assert!(is_engine_doc_type("token"));
/// assert!(is_engine_doc_type("combat"));
/// assert!(is_engine_doc_type("system-defaults"));
/// assert!(!is_engine_doc_type("item")); // client-only doc_type: opaque system band only
/// ```
pub fn is_engine_doc_type(doc_type: &str) -> bool {
    matches!(
        doc_type,
        "token"
            | "scene"
            | "wall"
            | "region"
            | "light"
            | "drawing"
            | "template"
            | "actor"
            | "message"
            | "world-settings"
            | "vision-modes"
            | "light-gradation"
            | "chat-settings"
            | "dice-settings"
            | "channel-registry"
            | "faction-registry"
            | "condition-registry"
            | "combat"
            | "combatant"
            | "resource-registry"
            | "effect"
            | "system-defaults"
            | "combat-history"
            | "asset_folder"
    )
}

/// `Ok(())` iff `engine` is valid for `doc_type`: engine doc types must carry
/// a body that deserializes into their struct (`deny_unknown_fields`); every
/// other doc type must carry no `engine` at all.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::validate_engine;
///
/// let body = serde_json::json!({ "factions": {} });
/// assert!(validate_engine("faction-registry", Some(&body)).is_ok());
///
/// // deny_unknown_fields: an unknown key is rejected, fail-closed.
/// let smuggled = serde_json::json!({ "factions": {}, "extra": 1 });
/// assert!(validate_engine("faction-registry", Some(&smuggled)).is_err());
///
/// // The gate cuts both ways: non-engine types must carry NO engine band...
/// assert!(validate_engine("item", Some(&body)).is_err());
/// // ...and engine types must carry one.
/// assert!(validate_engine("faction-registry", None).is_err());
/// ```
pub fn validate_engine(
    doc_type: &str,
    engine: Option<&serde_json::Value>,
) -> Result<(), DataError> {
    normalize_engine_opt(doc_type, engine).map(|_| ())
}

/// Validate `engine` for `doc_type` (same contract as `validate_engine`) and
/// return the RE-SERIALIZED validated engine (`None` for non-engine doc
/// types), rather than validating the raw input in place. Single source of
/// truth for the doc_type -> struct dispatch table; `validate_engine` and
/// `data::validation::validate_engine_tree` both build on this.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::normalize_engine_opt;
///
/// // Normalization re-serializes the typed struct: an absent optional field
/// // comes back as an explicit null, never a silently-missing key.
/// let body = serde_json::json!({});
/// let normalized = normalize_engine_opt("chat-settings", Some(&body)).unwrap().unwrap();
/// assert!(normalized.get("markdown").is_some_and(|v| v.is_null()));
/// ```
pub fn normalize_engine_opt(
    doc_type: &str,
    engine: Option<&serde_json::Value>,
) -> Result<Option<serde_json::Value>, DataError> {
    match (is_engine_doc_type(doc_type), engine) {
        (false, None) => Ok(None),
        (false, Some(_)) => Err(DataError::BadEngine(format!(
            "doc_type '{doc_type}' is not engine-defined; `engine` must be absent"
        ))),
        (true, None) => Err(DataError::BadEngine(format!(
            "doc_type '{doc_type}' requires an `engine` body"
        ))),
        (true, Some(v)) => normalize_engine(doc_type, v).map(Some),
    }
}

/// Deserialize `engine` into `doc_type`'s typed struct and re-serialize it,
/// dropping any field the struct didn't retain (see
/// `data::validation::validate_engine_tree` for why re-serialization, not
/// pass-through, is required). `doc_type` MUST be a registered engine doc
/// type (callers go through `normalize_engine_opt`, which enforces this).
///
/// # Examples
///
/// ```text
/// normalize_engine("scene", &raw)? // -> re-serialized SceneEngine JSON
/// ```
fn normalize_engine(doc_type: &str, v: &serde_json::Value) -> Result<serde_json::Value, DataError> {
    fn round_trip<T>(v: &serde_json::Value, t: &str) -> Result<serde_json::Value, DataError>
    where
        T: serde::de::DeserializeOwned + serde::Serialize,
    {
        let typed: T = serde_json::from_value(v.clone())
            .map_err(|e| DataError::BadEngine(format!("{t}: {e}")))?;
        Ok(serde_json::to_value(typed)?)
    }
    match doc_type {
        "token" => {
            let typed: TokenEngine = serde_json::from_value(v.clone())
                .map_err(|e| DataError::BadEngine(format!("token: {e}")))?;
            typed
                .validate()
                .map_err(|m| DataError::BadEngine(format!("token: {m}")))?;
            Ok(serde_json::to_value(typed)?)
        }
        "scene" => {
            let typed: SceneEngine = serde_json::from_value(v.clone())
                .map_err(|e| DataError::BadEngine(format!("scene: {e}")))?;
            typed
                .validate()
                .map_err(|m| DataError::BadEngine(format!("scene: {m}")))?;
            Ok(serde_json::to_value(typed)?)
        }
        "wall" => round_trip::<WallEngine>(v, "wall"),
        "region" => round_trip::<RegionEngine>(v, "region"),
        "light" => round_trip::<LightEngine>(v, "light"),
        "drawing" => round_trip::<DrawingEngine>(v, "drawing"),
        "template" => round_trip::<TemplateEngine>(v, "template"),
        "actor" => round_trip::<ActorEngine>(v, "actor"),
        "message" => round_trip::<crate::chat::MessageEngine>(v, "message"),
        "world-settings" => {
            let typed: WorldSettingsEngine = serde_json::from_value(v.clone())
                .map_err(|e| DataError::BadEngine(format!("world-settings: {e}")))?;
            typed
                .validate()
                .map_err(|m| DataError::BadEngine(format!("world-settings: {m}")))?;
            Ok(serde_json::to_value(typed)?)
        }
        "vision-modes" => round_trip::<VisionModesEngine>(v, "vision-modes"),
        "light-gradation" => round_trip::<LightGradationEngine>(v, "light-gradation"),
        "chat-settings" => round_trip::<ChatSettingsEngine>(v, "chat-settings"),
        "dice-settings" => round_trip::<DiceSettingsEngine>(v, "dice-settings"),
        "channel-registry" => round_trip::<ChannelRegistryEngine>(v, "channel-registry"),
        "faction-registry" => round_trip::<FactionRegistryEngine>(v, "faction-registry"),
        "condition-registry" => round_trip::<ConditionRegistryEngine>(v, "condition-registry"),
        "combat" => {
            let typed: CombatEngine = serde_json::from_value(v.clone())
                .map_err(|e| DataError::BadEngine(format!("combat: {e}")))?;
            typed
                .validate()
                .map_err(|m| DataError::BadEngine(format!("combat: {m}")))?;
            Ok(serde_json::to_value(typed)?)
        }
        "combatant" => {
            let typed: CombatantEngine = serde_json::from_value(v.clone())
                .map_err(|e| DataError::BadEngine(format!("combatant: {e}")))?;
            typed
                .validate()
                .map_err(|m| DataError::BadEngine(format!("combatant: {m}")))?;
            Ok(serde_json::to_value(typed)?)
        }
        "resource-registry" => {
            let typed: ResourceRegistryEngine = serde_json::from_value(v.clone())
                .map_err(|e| DataError::BadEngine(format!("resource-registry: {e}")))?;
            typed
                .validate()
                .map_err(|m| DataError::BadEngine(format!("resource-registry: {m}")))?;
            Ok(serde_json::to_value(typed)?)
        }
        "effect" => {
            let typed: EffectEngine = serde_json::from_value(v.clone())
                .map_err(|e| DataError::BadEngine(format!("effect: {e}")))?;
            typed
                .validate()
                .map_err(|m| DataError::BadEngine(format!("effect: {m}")))?;
            Ok(serde_json::to_value(typed)?)
        }
        "system-defaults" => {
            let typed: SystemDefaultsEngine = serde_json::from_value(v.clone())
                .map_err(|e| DataError::BadEngine(format!("system-defaults: {e}")))?;
            typed
                .validate()
                .map_err(|m| DataError::BadEngine(format!("system-defaults: {m}")))?;
            Ok(serde_json::to_value(typed)?)
        }
        "combat-history" => {
            let typed: CombatHistoryEngine = serde_json::from_value(v.clone())
                .map_err(|e| DataError::BadEngine(format!("combat-history: {e}")))?;
            typed
                .validate()
                .map_err(|m| DataError::BadEngine(format!("combat-history: {m}")))?;
            Ok(serde_json::to_value(typed)?)
        }
        "asset_folder" => round_trip::<asset_folder::AssetFolderEngine>(v, "asset_folder"),
        _ => unreachable!("is_engine_doc_type and this match must stay in sync"),
    }
}

/// Fail-closed typed read: the stored engine (already ingress-validated) or
/// `T::default()` when absent/malformed. Absence is the normal case for
/// non-engine doc types and stays silent; a present-but-undeserializable
/// engine indicates schema drift between ingress validation and this typed
/// read and is logged so it's observable rather than silently masked.
///
/// # Examples
///
/// ```
/// use shadowcat::data::document::Document;
/// use shadowcat::data::engine::engine_of;
/// use shadowcat::data::engine::registries::DiceSettingsEngine;
///
/// let doc: Document = serde_json::from_value(serde_json::json!({
///     "id": "00000000-0000-0000-0000-000000000001",
///     "scope": { "kind": "world", "world_id": "00000000-0000-0000-0000-0000000000aa" },
///     "doc_type": "note",
///     "schema_version": 1,
///     "system": {},
///     "created_at": 0,
///     "updated_at": 0
/// })).unwrap();
///
/// // Absent engine band -> the type's default, silently (fail-closed read).
/// let settings: DiceSettingsEngine = engine_of(&doc);
/// assert_eq!(settings, DiceSettingsEngine::default());
/// ```
pub fn engine_of<T: serde::de::DeserializeOwned + Default>(
    doc: &crate::data::document::Document,
) -> T {
    match &doc.engine {
        None => T::default(),
        Some(v) => match serde_json::from_value(v.clone()) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(
                    doc_id = %doc.id,
                    doc_type = %doc.doc_type,
                    error = %e,
                    "engine_of: stored engine failed to deserialize; falling back to default"
                );
                T::default()
            }
        },
    }
}

#[cfg(test)]
mod tests;
