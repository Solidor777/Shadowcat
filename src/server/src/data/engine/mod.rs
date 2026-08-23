//! Typed `engine` band structs + the doc_type registry: the engine band
//! exists iff `doc_type` is engine-defined, and its stored JSON must
//! deserialize into that doc_type's struct — a strict ingress gate rather
//! than an opaque pointer-walked body.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub mod geometry;
pub mod registries;
pub mod scene;
pub mod token;

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
        "scene" => round_trip::<SceneEngine>(v, "scene"),
        "wall" => round_trip::<WallEngine>(v, "wall"),
        "region" => round_trip::<RegionEngine>(v, "region"),
        "light" => round_trip::<LightEngine>(v, "light"),
        "drawing" => round_trip::<DrawingEngine>(v, "drawing"),
        "template" => round_trip::<TemplateEngine>(v, "template"),
        "actor" => round_trip::<ActorEngine>(v, "actor"),
        "message" => round_trip::<crate::chat::MessageEngine>(v, "message"),
        "world-settings" => round_trip::<WorldSettingsEngine>(v, "world-settings"),
        "vision-modes" => round_trip::<VisionModesEngine>(v, "vision-modes"),
        "light-gradation" => round_trip::<LightGradationEngine>(v, "light-gradation"),
        "chat-settings" => round_trip::<ChatSettingsEngine>(v, "chat-settings"),
        "dice-settings" => round_trip::<DiceSettingsEngine>(v, "dice-settings"),
        "channel-registry" => round_trip::<ChannelRegistryEngine>(v, "channel-registry"),
        "faction-registry" => round_trip::<FactionRegistryEngine>(v, "faction-registry"),
        "condition-registry" => round_trip::<ConditionRegistryEngine>(v, "condition-registry"),
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
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    // --- (a) minimal valid bodies deserialize for every doc_type ---

    #[test]
    fn token_minimal_body_is_valid() {
        let v = json!({ "x": 1.0, "y": 2.0, "w": 100.0, "h": 100.0, "rotation": 0.0 });
        assert!(validate_engine("token", Some(&v)).is_ok());
    }

    #[test]
    fn scene_minimal_body_is_valid() {
        let v = json!({ "grid": { "kind": "square", "size": 100.0 }, "background": null });
        assert!(validate_engine("scene", Some(&v)).is_ok());
    }

    #[test]
    fn wall_minimal_body_is_valid() {
        let v = json!({ "seg": { "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 1.0 } });
        assert!(validate_engine("wall", Some(&v)).is_ok());
    }

    #[test]
    fn region_minimal_body_is_valid() {
        let v = json!({
            "shape": { "kind": "rect", "points": [0.0, 0.0, 1.0, 1.0] },
            "behavior": "terrain", "cost": 1.0, "enabled": true
        });
        assert!(validate_engine("region", Some(&v)).is_ok());
    }

    #[test]
    fn light_minimal_body_is_valid() {
        let v = json!({
            "x": 0.0, "y": 0.0, "color": "#fff", "intensity": 1.0,
            "brightRadius": 5.0, "dimRadius": 10.0, "enabled": true
        });
        assert!(validate_engine("light", Some(&v)).is_ok());
    }

    #[test]
    fn drawing_minimal_body_is_valid() {
        let v = json!({
            "shape": { "kind": "rect", "points": [0.0, 0.0, 1.0, 1.0] },
            "stroke": null, "fill": null
        });
        assert!(validate_engine("drawing", Some(&v)).is_ok());
    }

    #[test]
    fn template_minimal_body_is_valid() {
        let v = json!({
            "shape": { "kind": "cone", "x": 0.0, "y": 0.0, "size": 5.0, "direction": 0.0 },
            "color": "#f00"
        });
        assert!(validate_engine("template", Some(&v)).is_ok());
    }

    #[test]
    fn actor_minimal_body_is_valid() {
        let v = json!({
            "displayName": "Goblin", "visual": { "kind": "image", "asset": "a" },
            "size": { "w": 1.0, "h": 1.0 }, "shape": "square",
            "faction": null, "conditions": [], "prototype": true
        });
        assert!(validate_engine("actor", Some(&v)).is_ok());
    }

    #[test]
    fn message_minimal_body_is_valid() {
        let v = json!({
            "channel": "ic", "user_owner": "00000000-0000-0000-0000-000000000000",
            "kind": "normal", "content": []
        });
        assert!(validate_engine("message", Some(&v)).is_ok());
    }

    #[test]
    fn world_settings_minimal_body_is_valid() {
        let v = serde_json::to_value(WorldSettingsEngine::default()).unwrap();
        assert!(validate_engine("world-settings", Some(&v)).is_ok());
    }

    #[test]
    fn vision_modes_minimal_body_is_valid() {
        let v = json!({ "modes": {} });
        assert!(validate_engine("vision-modes", Some(&v)).is_ok());
    }

    #[test]
    fn light_gradation_minimal_body_is_valid() {
        let v = json!({ "bands": [] });
        assert!(validate_engine("light-gradation", Some(&v)).is_ok());
    }

    #[test]
    fn chat_settings_minimal_body_is_valid() {
        assert!(validate_engine("chat-settings", Some(&json!({}))).is_ok());
    }

    #[test]
    fn dice_settings_minimal_body_is_valid() {
        assert!(validate_engine("dice-settings", Some(&json!({}))).is_ok());
    }

    #[test]
    fn channel_registry_minimal_body_is_valid() {
        assert!(validate_engine("channel-registry", Some(&json!({ "channels": {} }))).is_ok());
    }

    #[test]
    fn faction_registry_minimal_body_is_valid() {
        assert!(validate_engine("faction-registry", Some(&json!({ "factions": {} }))).is_ok());
    }

    #[test]
    fn condition_registry_minimal_body_is_valid() {
        assert!(validate_engine("condition-registry", Some(&json!({ "conditions": {} }))).is_ok());
    }

    // --- (b) unknown fields rejected (struct-level; skip tagged-enum-only bodies) ---

    #[test]
    fn token_unknown_field_is_rejected() {
        let v = json!({ "x": 1.0, "y": 2.0, "w": 1.0, "h": 1.0, "rotation": 0.0, "bogus": 1 });
        assert!(validate_engine("token", Some(&v)).is_err());
    }

    #[test]
    fn scene_unknown_field_is_rejected() {
        let v =
            json!({ "grid": { "kind": "square", "size": 100.0 }, "background": null, "bogus": 1 });
        assert!(validate_engine("scene", Some(&v)).is_err());
    }

    #[test]
    fn wall_unknown_field_is_rejected() {
        let v = json!({ "seg": { "x1": 0.0, "y1": 0.0, "x2": 1.0, "y2": 1.0 }, "bogus": 1 });
        assert!(validate_engine("wall", Some(&v)).is_err());
    }

    #[test]
    fn region_unknown_field_is_rejected() {
        let v = json!({
            "shape": { "kind": "rect", "points": [] },
            "behavior": "terrain", "cost": 1.0, "enabled": true, "bogus": 1
        });
        assert!(validate_engine("region", Some(&v)).is_err());
    }

    #[test]
    fn light_unknown_field_is_rejected() {
        let v = json!({
            "x": 0.0, "y": 0.0, "color": "#fff", "intensity": 1.0,
            "brightRadius": 5.0, "dimRadius": 10.0, "enabled": true, "bogus": 1
        });
        assert!(validate_engine("light", Some(&v)).is_err());
    }

    #[test]
    fn drawing_unknown_field_is_rejected() {
        let v = json!({
            "shape": { "kind": "rect", "points": [] }, "stroke": null, "fill": null, "bogus": 1
        });
        assert!(validate_engine("drawing", Some(&v)).is_err());
    }

    #[test]
    fn template_unknown_field_is_rejected() {
        let v = json!({
            "shape": { "kind": "cone", "x": 0.0, "y": 0.0, "size": 5.0, "direction": 0.0 },
            "color": "#f00", "bogus": 1
        });
        assert!(validate_engine("template", Some(&v)).is_err());
    }

    #[test]
    fn actor_unknown_field_is_rejected() {
        let v = json!({
            "displayName": "Goblin", "visual": { "kind": "image", "asset": "a" },
            "size": { "w": 1.0, "h": 1.0 }, "shape": "square",
            "faction": null, "conditions": [], "prototype": true, "bogus": 1
        });
        assert!(validate_engine("actor", Some(&v)).is_err());
    }

    #[test]
    fn actor_missing_display_name_is_rejected() {
        let v = json!({
            "visual": { "kind": "image", "asset": "a" },
            "size": { "w": 1.0, "h": 1.0 }, "shape": "square",
            "faction": null, "conditions": [], "prototype": true
        });
        assert!(validate_engine("actor", Some(&v)).is_err());
    }

    #[test]
    fn actor_missing_conditions_is_rejected() {
        let v = json!({
            "displayName": "Goblin", "visual": { "kind": "image", "asset": "a" },
            "size": { "w": 1.0, "h": 1.0 }, "shape": "square",
            "faction": null, "prototype": true
        });
        assert!(validate_engine("actor", Some(&v)).is_err());
    }

    #[test]
    fn actor_missing_faction_key_accepted_as_none() {
        // INVARIANT: `Option<T>` accepts an absent key as `None` regardless
        // of `#[serde(default)]` — pins the true ingress contract on
        // `ActorEngine.faction` (see its doc comment).
        let v = json!({
            "displayName": "Goblin", "visual": { "kind": "image", "asset": "a" },
            "size": { "w": 1.0, "h": 1.0 }, "shape": "square",
            "conditions": [], "prototype": true
        });
        assert!(validate_engine("actor", Some(&v)).is_ok());
        let engine: ActorEngine = serde_json::from_value(v).unwrap();
        assert_eq!(engine.faction, None);
    }

    #[test]
    fn world_settings_unknown_field_is_rejected() {
        let mut v = serde_json::to_value(WorldSettingsEngine::default()).unwrap();
        v.as_object_mut().unwrap().insert("bogus".into(), json!(1));
        assert!(validate_engine("world-settings", Some(&v)).is_err());
    }

    #[test]
    fn vision_modes_unknown_field_is_rejected() {
        assert!(
            validate_engine("vision-modes", Some(&json!({ "modes": {}, "bogus": 1 }))).is_err()
        );
    }

    #[test]
    fn light_gradation_unknown_field_is_rejected() {
        assert!(
            validate_engine("light-gradation", Some(&json!({ "bands": [], "bogus": 1 }))).is_err()
        );
    }

    #[test]
    fn chat_settings_unknown_field_is_rejected() {
        assert!(validate_engine("chat-settings", Some(&json!({ "bogus": 1 }))).is_err());
    }

    #[test]
    fn dice_settings_unknown_field_is_rejected() {
        assert!(validate_engine("dice-settings", Some(&json!({ "bogus": 1 }))).is_err());
    }

    #[test]
    fn channel_registry_unknown_field_is_rejected() {
        assert!(validate_engine(
            "channel-registry",
            Some(&json!({ "channels": {}, "bogus": 1 }))
        )
        .is_err());
    }

    #[test]
    fn faction_registry_unknown_field_is_rejected() {
        assert!(validate_engine(
            "faction-registry",
            Some(&json!({ "factions": {}, "bogus": 1 }))
        )
        .is_err());
    }

    #[test]
    fn condition_registry_unknown_field_is_rejected() {
        assert!(validate_engine(
            "condition-registry",
            Some(&json!({ "conditions": {}, "bogus": 1 }))
        )
        .is_err());
    }

    #[test]
    fn message_unknown_field_is_rejected() {
        assert!(validate_engine(
            "message",
            Some(&json!({
                "channel": "ic", "user_owner": "00000000-0000-0000-0000-000000000000",
                "kind": "normal", "content": [], "bogus": 1
            }))
        )
        .is_err());
    }

    // --- (c) wrong-typed field rejected (all 17 registered doc_types) ---

    #[test]
    fn token_wrong_typed_field_is_rejected() {
        let v = json!({ "x": "12", "y": 2.0, "w": 1.0, "h": 1.0, "rotation": 0.0 });
        assert!(validate_engine("token", Some(&v)).is_err());
    }

    #[test]
    fn scene_wrong_typed_grid_size_is_rejected() {
        let v = json!({ "grid": { "kind": "square", "size": "100" }, "background": null });
        assert!(validate_engine("scene", Some(&v)).is_err());
    }

    #[test]
    fn wall_wrong_typed_field_is_rejected() {
        let v = json!({ "seg": { "x1": "0", "y1": 0.0, "x2": 1.0, "y2": 1.0 } });
        assert!(validate_engine("wall", Some(&v)).is_err());
    }

    #[test]
    fn region_wrong_typed_field_is_rejected() {
        let v = json!({
            "shape": { "kind": "rect", "points": [0.0, 0.0, 1.0, 1.0] },
            "behavior": "terrain", "cost": "1.0", "enabled": true
        });
        assert!(validate_engine("region", Some(&v)).is_err());
    }

    #[test]
    fn light_wrong_typed_intensity_is_rejected() {
        let v = json!({
            "x": 0.0, "y": 0.0, "color": "#fff", "intensity": "1",
            "brightRadius": 5.0, "dimRadius": 10.0, "enabled": true
        });
        assert!(validate_engine("light", Some(&v)).is_err());
    }

    #[test]
    fn drawing_wrong_typed_field_is_rejected() {
        let v = json!({
            "shape": { "kind": "rect", "points": [0.0, 0.0, 1.0, 1.0] },
            "stroke": null, "fill": { "color": 1, "alpha": null }
        });
        assert!(validate_engine("drawing", Some(&v)).is_err());
    }

    #[test]
    fn template_wrong_typed_field_is_rejected() {
        let v = json!({
            "shape": { "kind": "cone", "x": 0.0, "y": 0.0, "size": 5.0, "direction": 0.0 },
            "color": 1
        });
        assert!(validate_engine("template", Some(&v)).is_err());
    }

    #[test]
    fn actor_wrong_typed_field_is_rejected() {
        let v = json!({
            "displayName": "Goblin", "visual": { "kind": "image", "asset": "a" },
            "size": { "w": 1.0, "h": 1.0 }, "shape": "square",
            "faction": null, "conditions": [], "prototype": "true"
        });
        assert!(validate_engine("actor", Some(&v)).is_err());
    }

    #[test]
    fn message_wrong_typed_field_is_rejected() {
        let v = json!({
            "channel": 1, "user_owner": "00000000-0000-0000-0000-000000000000",
            "kind": "normal", "content": []
        });
        assert!(validate_engine("message", Some(&v)).is_err());
    }

    #[test]
    fn world_settings_wrong_typed_field_is_rejected() {
        let mut v = serde_json::to_value(WorldSettingsEngine::default()).unwrap();
        v["animation"]["speedCellsPerSec"] = json!("6");
        assert!(validate_engine("world-settings", Some(&v)).is_err());
    }

    #[test]
    fn vision_modes_wrong_typed_field_is_rejected() {
        let v = json!({
            "modes": {
                "darkvision": {
                    "id": "darkvision", "name": "Darkvision",
                    "illuminationFloor": "dark", "defaultRange": "60"
                }
            }
        });
        assert!(validate_engine("vision-modes", Some(&v)).is_err());
    }

    #[test]
    fn light_gradation_wrong_typed_field_is_rejected() {
        let v = json!({ "bands": [{ "name": "dim", "minIllumination": "0.5" }] });
        assert!(validate_engine("light-gradation", Some(&v)).is_err());
    }

    #[test]
    fn chat_settings_wrong_typed_field_is_rejected() {
        let v = json!({ "markdown": "true" });
        assert!(validate_engine("chat-settings", Some(&v)).is_err());
    }

    #[test]
    fn dice_settings_wrong_typed_field_is_rejected() {
        let v = json!({ "mode": 1 });
        assert!(validate_engine("dice-settings", Some(&v)).is_err());
    }

    #[test]
    fn channel_registry_wrong_typed_field_is_rejected() {
        let v = json!({ "channels": { "ic": { "name": 1 } } });
        assert!(validate_engine("channel-registry", Some(&v)).is_err());
    }

    #[test]
    fn faction_registry_wrong_typed_field_is_rejected() {
        let v = json!({
            "factions": {
                "goblins": { "name": "Goblins", "color": "#fff", "stance": 1 }
            }
        });
        assert!(validate_engine("faction-registry", Some(&v)).is_err());
    }

    #[test]
    fn condition_registry_wrong_typed_field_is_rejected() {
        let v = json!({ "conditions": { "prone": { "name": "Prone", "icon": 1 } } });
        assert!(validate_engine("condition-registry", Some(&v)).is_err());
    }

    // --- (d)/(e)/(f): registry membership + None handling ---

    #[test]
    fn non_engine_doc_type_with_engine_body_is_rejected() {
        assert!(validate_engine("item", Some(&json!({}))).is_err());
        assert!(validate_engine("custom-thing", Some(&json!({}))).is_err());
    }

    #[test]
    fn non_engine_doc_type_without_engine_is_ok() {
        assert!(validate_engine("item", None).is_ok());
        assert!(validate_engine("custom-thing", None).is_ok());
    }

    #[test]
    fn engine_doc_type_missing_engine_is_rejected() {
        assert!(validate_engine("token", None).is_err());
    }

    // --- token/actor visual union round-trips ---

    #[test]
    fn token_visual_image_round_trips() {
        let v: TokenVisual =
            serde_json::from_value(json!({ "kind": "image", "asset": "a.png" })).unwrap();
        assert!(matches!(v, TokenVisual::Image { .. }));
        assert_eq!(serde_json::to_value(&v).unwrap()["kind"], "image");
    }

    #[test]
    fn token_visual_animated_round_trips() {
        let src = json!({ "kind": "animated", "source": { "type": "frames", "frames": ["a"] }, "fps": 12.0, "loop": true });
        let v: TokenVisual = serde_json::from_value(src.clone()).unwrap();
        assert!(matches!(v, TokenVisual::Animated { .. }));
        assert_eq!(serde_json::to_value(&v).unwrap(), src);
    }

    #[test]
    fn token_visual_faces_round_trips() {
        let mut faces = BTreeMap::new();
        faces.insert(
            "default".to_string(),
            RenderVisual::Image {
                asset: "a.png".into(),
            },
        );
        let v = TokenVisual::Faces {
            faces,
            default: "default".into(),
            face_map: None,
        };
        let json = serde_json::to_value(&v).unwrap();
        let back: TokenVisual = serde_json::from_value(json).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn animated_source_frames_round_trips() {
        let src: AnimatedSource =
            serde_json::from_value(json!({ "type": "frames", "frames": ["a", "b"] })).unwrap();
        assert!(matches!(src, AnimatedSource::Frames { .. }));
    }

    #[test]
    fn animated_source_sheet_round_trips() {
        let src: AnimatedSource = serde_json::from_value(
            json!({ "type": "sheet", "asset": "s.png", "rows": 2, "cols": 3 }),
        )
        .unwrap();
        assert!(matches!(src, AnimatedSource::Sheet { .. }));
    }

    // --- literal-set assertions (client writers emit these strings today) ---

    #[test]
    fn shape_literal_set_deserializes() {
        for shape in ["square", "circle"] {
            let v = json!({
                "displayName": "Goblin", "visual": { "kind": "image", "asset": "a" },
                "size": { "w": 1.0, "h": 1.0 }, "shape": shape,
                "faction": null, "conditions": [], "prototype": true
            });
            assert!(
                validate_engine("actor", Some(&v)).is_ok(),
                "shape '{shape}' must be accepted"
            );
        }
    }

    #[test]
    fn region_shape_kind_literal_set_deserializes() {
        for (kind, points) in [
            ("rect", json!([0.0, 0.0, 1.0, 1.0])),
            ("circle", json!([0.0, 0.0, 1.0])),
            ("polygon", json!([0.0, 0.0, 1.0, 0.0, 1.0, 1.0])),
        ] {
            let v = json!({
                "shape": { "kind": kind, "points": points },
                "behavior": "terrain", "cost": 1.0, "enabled": true
            });
            assert!(
                validate_engine("region", Some(&v)).is_ok(),
                "region shape kind '{kind}' must be accepted"
            );
        }
    }

    #[test]
    fn region_behavior_literal_set_deserializes() {
        for behavior in ["terrain", "impassable", "arrest"] {
            let v = json!({
                "shape": { "kind": "rect", "points": [0.0, 0.0, 1.0, 1.0] },
                "behavior": behavior, "cost": 1.0, "enabled": true
            });
            assert!(
                validate_engine("region", Some(&v)).is_ok(),
                "region behavior '{behavior}' must be accepted"
            );
        }
    }

    /// Drift guard: `WorldSettingsEngine::default()` must serialize to the
    /// SAME values as the client's `DEFAULT_WORLD_SETTINGS`, field-by-field.
    #[test]
    fn world_settings_default_matches_client_default() {
        let v = serde_json::to_value(WorldSettingsEngine::default()).unwrap();
        assert_eq!(
            v,
            json!({
                "scene": {
                    "losRestriction": true,
                    "fog": true,
                    "lightingEnabled": true,
                    "lightMode": "environmentLight",
                    "environment": { "color": "#0a0e1a", "intensity": 0.0 },
                    "observerVision": false,
                    "movementRestriction": "visible",
                    "movementModel": "grid-stepped",
                    "partialCellLeniency": true,
                },
                "pathfinding": { "diagonalRule": "chebyshev" },
                "animation": { "speedCellsPerSec": 6.0, "easing": "easeInOut" },
                "activeScene": null,
            })
        );
    }

    #[test]
    fn engine_of_defaults_on_absent_or_malformed() {
        let doc = crate::data::document::tests::world_scoped_doc(
            uuid::Uuid::from_u128(1),
            uuid::Uuid::from_u128(2),
            "world-settings",
        );
        let ws: WorldSettingsEngine = engine_of(&doc);
        assert_eq!(ws, WorldSettingsEngine::default());
    }
}
