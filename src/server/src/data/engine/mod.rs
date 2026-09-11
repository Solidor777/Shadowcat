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
pub mod note;
pub mod registries;
pub mod scene;
pub mod system_defaults;
pub mod table;
pub mod token;

pub use combat::{
    resolve_combat_rules, CapturedCombatant, CombatDefaults, CombatEngine, CombatHistoryEngine,
    CombatantEngine, CombatantKind, CombatantResource, Duration, DurationUnit, EffectEngine,
    EffectLifecycle, EffectLifecycleDefaults, EffectSnapshot, Enforcement, ExpiryPoint, Formula,
    Interpretation, MovementRules, Recovery, ResolvedCombatRules, Resource, ResourceBinding,
    ResourceRegistryEngine, TurnControl, TurnRecord, MAX_TURN_HISTORY,
};
pub use geometry::{
    DrawingEngine, DrawingShape, Fill, NoticeAudience, RegionEngine, RegionShape, RegionTrigger,
    Seg, Stroke, TemplateEngine, TemplateShape, TriggerEffect, TriggerEvent, WallElevation,
    WallEngine, MAX_TRIGGER_ID_CHARS,
};
pub use note::{NoteEngine, MAX_NOTE_SOURCE_CHARS, MAX_NOTE_SPANS, NOTE_DOC_TYPE};
pub use registries::{
    Channel, ChannelDiceOverride, ChannelRegistryEngine, ChatSettingsEngine, Condition,
    ConditionRegistryEngine, DiceDirectionSetting, DiceModeSetting, DiceSettingsEngine, Faction,
    FactionRegistryEngine, FactionStance,
};
pub use scene::{
    AnimationSettings, DiagonalRule, EasingMode, EnvironmentLight, Falloff, FalloffCurve, Grid,
    GridDistance, LightEmission, LightEngine, LightGradationEngine, LightMode, MovementModel,
    MovementRestriction, Pathfinding, Perception, SceneDimensions, SceneEngine,
    SceneLightingOverrides, SceneVisionOverrides, VisionMode, VisionModesEngine,
    WorldSceneDefaults, WorldSettingsEngine,
};
pub use system_defaults::{
    AnimationOverlay, PathfindingOverlay, SceneDefaultsOverlay, SystemDefaultsEngine,
};
pub use table::{DrawRule, RowRange, TableEngine, TableEntry, TableRow, TABLE_DOC_TYPE};
pub use token::{
    ActorEngine, AnimatedSource, GeneratedBackground, GeneratedBorder, GeneratedCrop, RenderVisual,
    Size, TokenEngine, TokenOverrides, TokenVisual, VisionAssignment,
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
/// Maximum length of a chat channel id, in characters — the one declaration,
/// shared by the message ingest check (`chat::handle_send_message`) and
/// `ChannelRegistryEngine::validate` (a longer key could never be posted to).
pub const MAX_CHANNEL_CHARS: usize = 128;
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

/// Every doc_type carrying a typed `engine` band, the single data source
/// `is_engine_doc_type` and `search_text` both dispatch from — so the
/// registry and either function can never drift apart. `search_text`'s
/// exhaustive match has no arm the compiler can check against this list: a
/// doc_type added here with no corresponding `search_text` arm compiles
/// clean and panics via `unreachable!` only at runtime, the first time that
/// doc_type is indexed. `search_text_is_registered_for_every_engine_doc_type`
/// is the guard — it iterates this list and calls `search_text` on each
/// entry, so the panic surfaces in CI rather than in a production write.
pub(crate) const ENGINE_DOC_TYPES: &[&str] = &[
    "token",
    "scene",
    "wall",
    "region",
    "light",
    "drawing",
    "template",
    "actor",
    "message",
    "world-settings",
    "vision-modes",
    "light-gradation",
    "chat-settings",
    "dice-settings",
    "channel-registry",
    "faction-registry",
    "condition-registry",
    "combat",
    "combatant",
    "resource-registry",
    "effect",
    "system-defaults",
    "combat-history",
    "asset_folder",
    "table",
    "note",
];

/// Every `/engine/<key>` path a `doc_type`'s `normalize_engine` arm can
/// rewrite BEYOND the path a caller's own `FieldChange`s named — the single
/// source `data::validation::derive_engine_side_effects` consults to decide
/// which top-level engine keys to diff at all. The only such derivation
/// today is `NoteEngine::derive_body` (the `"note"` arm of `normalize_engine`
/// above), which rewrites `/engine/body` from `/engine/source`; every other
/// registered doc_type derives nothing beyond what it was asked to write, so
/// it names an empty slice. INVARIANT: any `derive_*` call added to
/// `normalize_engine`'s match MUST register the path(s) it rewrites here —
/// this registry is what surfaces a server-derived value to the broadcast,
/// the `world_events` log, and the author's own optimistic store; a
/// derivation absent from it would sit correctly in the stored row while
/// every live view of the document keeps showing the pre-derivation value.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::derived_engine_paths;
///
/// assert_eq!(derived_engine_paths("note"), &["/engine/body"]);
/// assert!(derived_engine_paths("token").is_empty());
/// ```
pub fn derived_engine_paths(doc_type: &str) -> &'static [&'static str] {
    match doc_type {
        "note" => &["/engine/body"],
        _ => &[],
    }
}

/// Whether `doc_type` carries a typed `engine` band. The registry is a
/// hardcoded list — there is no dynamic registration (the server runs no
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
    ENGINE_DOC_TYPES.contains(&doc_type)
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
        "region" => {
            let typed: RegionEngine = serde_json::from_value(v.clone())
                .map_err(|e| DataError::BadEngine(format!("region: {e}")))?;
            typed
                .validate()
                .map_err(|m| DataError::BadEngine(format!("region: {m}")))?;
            Ok(serde_json::to_value(typed)?)
        }
        "light" => {
            let typed: LightEngine = serde_json::from_value(v.clone())
                .map_err(|e| DataError::BadEngine(format!("light: {e}")))?;
            typed
                .validate()
                .map_err(|m| DataError::BadEngine(format!("light: {m}")))?;
            Ok(serde_json::to_value(typed)?)
        }
        "drawing" => round_trip::<DrawingEngine>(v, "drawing"),
        "template" => round_trip::<TemplateEngine>(v, "template"),
        "actor" => {
            let typed: ActorEngine = serde_json::from_value(v.clone())
                .map_err(|e| DataError::BadEngine(format!("actor: {e}")))?;
            typed
                .validate()
                .map_err(|m| DataError::BadEngine(format!("actor: {m}")))?;
            Ok(serde_json::to_value(typed)?)
        }
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
        "channel-registry" => {
            let typed: ChannelRegistryEngine = serde_json::from_value(v.clone())
                .map_err(|e| DataError::BadEngine(format!("channel-registry: {e}")))?;
            typed
                .validate()
                .map_err(|m| DataError::BadEngine(format!("channel-registry: {m}")))?;
            Ok(serde_json::to_value(typed)?)
        }
        "faction-registry" => round_trip::<FactionRegistryEngine>(v, "faction-registry"),
        "condition-registry" => {
            let typed: ConditionRegistryEngine = serde_json::from_value(v.clone())
                .map_err(|e| DataError::BadEngine(format!("condition-registry: {e}")))?;
            typed
                .validate()
                .map_err(|m| DataError::BadEngine(format!("condition-registry: {m}")))?;
            Ok(serde_json::to_value(typed)?)
        }
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
        "table" => {
            let typed: TableEngine = serde_json::from_value(v.clone())
                .map_err(|e| DataError::BadEngine(format!("table: {e}")))?;
            typed
                .validate()
                .map_err(|m| DataError::BadEngine(format!("table: {m}")))?;
            Ok(serde_json::to_value(typed)?)
        }
        "note" => {
            let mut typed: NoteEngine = serde_json::from_value(v.clone())
                .map_err(|e| DataError::BadEngine(format!("note: {e}")))?;
            typed
                .validate()
                .map_err(|m| DataError::BadEngine(format!("note: {m}")))?;
            typed
                .derive_body()
                .map_err(|m| DataError::BadEngine(format!("note: {m}")))?;
            Ok(serde_json::to_value(typed)?)
        }
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
///     "doc_type": "item",
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

/// Space-joins the non-empty items of `names`, so a registry with no
/// entries (or entries with empty display names) contributes an empty
/// string rather than stray separators.
fn join_names<'a>(names: impl Iterator<Item = &'a str>) -> String {
    names
        .filter(|n| !n.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Reader-facing text over one table's rows: `description`, then per row
/// `label` and every `TableEntry::Text`/`Doc`/`Image`'s reader-facing field.
/// Excludes `DrawRule`/`TableEntry` discriminants and every id (`asset_id`,
/// `table_id`) — a nested `Draw` entry contributes nothing here (its own
/// draw-time text is captured separately, on the executed `Segment::TableDraw`).
fn table_search_text(table: &TableEngine) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if !table.description.is_empty() {
        parts.push(&table.description);
    }
    for row in &table.rows {
        if !row.label.is_empty() {
            parts.push(&row.label);
        }
        for entry in &row.results {
            match entry {
                TableEntry::Text { text } if !text.is_empty() => parts.push(text),
                TableEntry::Doc { label, .. } if !label.is_empty() => parts.push(label),
                TableEntry::Image { alt, .. } if !alt.is_empty() => parts.push(alt),
                TableEntry::Text { .. } | TableEntry::Doc { .. } | TableEntry::Image { .. } => {}
                TableEntry::Draw { .. } => {}
            }
        }
    }
    parts.join(" ")
}

/// Best-effort typed deserialize for the write-path `search_text` posture:
/// a body that fails to parse contributes no text rather than failing the
/// index write (mirrors `index_content_public`'s redaction-failure
/// posture — see that function's doc).
fn typed_or_none<T: serde::de::DeserializeOwned>(engine: &serde_json::Value) -> Option<T> {
    serde_json::from_value(engine.clone()).ok()
}

/// The reader-facing text projection for one engine band, keyed by
/// `doc_type`. Feeds `data::search::index_content`, replacing a leaf sweep
/// of the engine band that would otherwise surface discriminants
/// (`kind`/`stance`), ids (`asset_id`/`table_id`), colors and notation as
/// indexable content. `Some(text)` for every registered engine doc type
/// (`Some(String::new())` for one with no reader-facing text), `None` for a
/// non-engine `doc_type`. Mirrors `normalize_engine`'s exhaustive-match
/// shape so the two dispatch tables are reviewed side by side; a doc_type
/// added to `ENGINE_DOC_TYPES` without a corresponding arm here panics via
/// the `unreachable!` fallback rather than silently indexing nothing.
///
/// # Examples
///
/// ```
/// use serde_json::json;
/// use shadowcat::data::engine::search_text;
///
/// assert_eq!(search_text("item", &json!({})), None);
/// assert_eq!(search_text("token", &json!({})), Some(String::new()));
/// ```
pub fn search_text(doc_type: &str, engine: &serde_json::Value) -> Option<String> {
    if !is_engine_doc_type(doc_type) {
        return None;
    }
    Some(match doc_type {
        "actor" => typed_or_none::<ActorEngine>(engine)
            .map(|a| a.display_name)
            .unwrap_or_default(),
        "note" => typed_or_none::<NoteEngine>(engine)
            .map(|n| crate::chat::segments_search_text(&n.body))
            .unwrap_or_default(),
        "table" => typed_or_none::<TableEngine>(engine)
            .map(|t| table_search_text(&t))
            .unwrap_or_default(),
        "message" => typed_or_none::<crate::chat::MessageEngine>(engine)
            .map(|m| crate::chat::segments_search_text(&m.content))
            .unwrap_or_default(),
        "channel-registry" => typed_or_none::<ChannelRegistryEngine>(engine)
            .map(|r| join_names(r.channels.values().map(|c| c.name.as_str())))
            .unwrap_or_default(),
        "faction-registry" => typed_or_none::<FactionRegistryEngine>(engine)
            .map(|r| join_names(r.factions.values().map(|f| f.name.as_str())))
            .unwrap_or_default(),
        "condition-registry" => typed_or_none::<ConditionRegistryEngine>(engine)
            .map(|r| join_names(r.conditions.values().map(|c| c.name.as_str())))
            .unwrap_or_default(),
        "resource-registry" => typed_or_none::<ResourceRegistryEngine>(engine)
            .map(|r| join_names(r.resources.values().map(|res| res.name.as_str())))
            .unwrap_or_default(),
        "vision-modes" => typed_or_none::<VisionModesEngine>(engine)
            .map(|r| join_names(r.modes.values().map(|m| m.name.as_str())))
            .unwrap_or_default(),
        "light-gradation" => typed_or_none::<LightGradationEngine>(engine)
            .map(|g| join_names(g.bands.iter().map(|b| b.name.as_str())))
            .unwrap_or_default(),
        "token" => typed_or_none::<TokenEngine>(engine)
            .map(|t| t.overrides.and_then(|o| o.name).unwrap_or_default())
            .unwrap_or_default(),
        "combat-history" => typed_or_none::<CombatHistoryEngine>(engine)
            .map(|h| {
                join_names(
                    h.records
                        .iter()
                        .flat_map(|r| r.combatants.iter())
                        .filter_map(|c| c.name.as_deref()),
                )
            })
            .unwrap_or_default(),
        "scene" | "wall" | "region" | "light" | "drawing" | "template" | "world-settings"
        | "chat-settings" | "dice-settings" | "combat" | "combatant" | "effect"
        | "system-defaults" | "asset_folder" => String::new(),
        _ => unreachable!("ENGINE_DOC_TYPES and this match must stay in sync"),
    })
}

#[cfg(test)]
mod tests;
