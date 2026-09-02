//! Server-side world-config seeding: the ops-builder that creates absent
//! config singletons with their engine-defined default bodies and keeps the
//! `system-defaults` singleton mirroring the enabled system package's
//! manifest declaration. The DECISION of what to seed lives here alone;
//! callers commit the returned ops under `WriteOrigin::ConfigSeed` through
//! their own channel — `Repository::apply_intent` where no room exists,
//! `Room::publish` where one does (publish wraps the same `apply_intent`
//! plus broadcast, so the transport split is not a forked decision).

use std::collections::BTreeSet;
use std::path::Path;

use uuid::Uuid;

use crate::data::command::{FieldChange, Operation};
use crate::data::document::{DocRole, Document, PermissionSet, Scope, WorldRole};
use crate::data::engine::{
    ChannelRegistryEngine, ChatSettingsEngine, ConditionRegistryEngine, DiceSettingsEngine,
    FactionRegistryEngine, LightGradationEngine, ResourceRegistryEngine, SystemDefaultsEngine,
    VisionModesEngine, WorldSettingsEngine, CHANNEL_REGISTRY_DOC_TYPE, CONDITION_REGISTRY_DOC_TYPE,
    FACTION_REGISTRY_DOC_TYPE, LIGHT_GRADATION_DOC_TYPE, RESOURCE_REGISTRY_DOC_TYPE,
    SYSTEM_DEFAULTS_DOC_TYPE, VISION_MODES_DOC_TYPE, WORLD_SETTINGS_DOC_TYPE,
};
use crate::data::membership::PermissionContext;
use crate::data::repository::Repository;
use crate::data::sqlite::SqliteRepository;
use crate::modules::scan_installed_modules;

/// Every world-config singleton doc_type the seed pass owns, in seed order.
/// One list, read by the ops-builder and by callers querying a world's
/// current config set — never re-enumerated elsewhere.
pub const CONFIG_SINGLETON_DOC_TYPES: [&str; 10] = [
    WORLD_SETTINGS_DOC_TYPE,
    VISION_MODES_DOC_TYPE,
    LIGHT_GRADATION_DOC_TYPE,
    crate::chat::CHAT_SETTINGS_DOC_TYPE,
    crate::chat::DICE_SETTINGS_DOC_TYPE,
    CHANNEL_REGISTRY_DOC_TYPE,
    FACTION_REGISTRY_DOC_TYPE,
    CONDITION_REGISTRY_DOC_TYPE,
    RESOURCE_REGISTRY_DOC_TYPE,
    SYSTEM_DEFAULTS_DOC_TYPE,
];

/// Build the ops that bring a world's config-singleton set current: a
/// `Create` (fresh UUID; absence is keyed by `doc_type`, and the singleton
/// ingress gate backstops any server-internal race) for each
/// `CONFIG_SINGLETON_DOC_TYPES` entry absent from `existing`, plus — when a
/// stored `system-defaults` body differs from what the enabled system
/// declares (`system_defaults`, `None` ⇒ the empty default) — one OCC'd
/// `/engine` Update refreshing it: the stored copy is a server-owned mirror
/// of the manifest and must not drift.
///
/// # Examples
///
/// ```
/// use shadowcat::data::world_seed::missing_config_ops;
///
/// let ops = missing_config_ops(&[], uuid::Uuid::nil(), None, 0);
/// assert_eq!(ops.len(), 10);
/// ```
pub fn missing_config_ops(
    existing: &[Document],
    world_id: Uuid,
    system_defaults: Option<&SystemDefaultsEngine>,
    now: i64,
) -> Vec<Operation> {
    let present: BTreeSet<&str> = existing.iter().map(|d| d.doc_type.as_str()).collect();
    let mut ops = Vec::new();
    for ty in CONFIG_SINGLETON_DOC_TYPES {
        if !present.contains(ty) {
            ops.push(Operation::Create {
                doc: config_doc(world_id, ty, seed_engine_body(ty, system_defaults), now),
            });
        }
    }
    if let Some(doc) = existing
        .iter()
        .find(|d| d.doc_type == SYSTEM_DEFAULTS_DOC_TYPE)
    {
        let desired = serde_json::to_value(system_defaults.cloned().unwrap_or_default())
            .expect("SystemDefaultsEngine serializes");
        let stored = doc.engine.clone().unwrap_or(serde_json::Value::Null);
        if stored != desired {
            ops.push(Operation::Update {
                doc_id: doc.id,
                changes: vec![FieldChange {
                    remove: false,
                    path: "/engine".into(),
                    old: stored,
                    new: desired,
                }],
            });
        }
    }
    ops
}

/// The engine-default body a fresh config singleton of `doc_type` is seeded
/// with; `system_defaults` supplies the `system-defaults` body (the enabled
/// system's declaration, else the empty default). Every arm serializes a
/// validated engine struct, so a seed Create clears `validate_engine_tree`
/// by construction.
fn seed_engine_body(
    doc_type: &str,
    system_defaults: Option<&SystemDefaultsEngine>,
) -> serde_json::Value {
    let v = match doc_type {
        WORLD_SETTINGS_DOC_TYPE => serde_json::to_value(WorldSettingsEngine::default()),
        VISION_MODES_DOC_TYPE => serde_json::to_value(VisionModesEngine::seed()),
        LIGHT_GRADATION_DOC_TYPE => serde_json::to_value(LightGradationEngine::seed()),
        CHANNEL_REGISTRY_DOC_TYPE => serde_json::to_value(ChannelRegistryEngine::seed()),
        FACTION_REGISTRY_DOC_TYPE => serde_json::to_value(FactionRegistryEngine::seed()),
        CONDITION_REGISTRY_DOC_TYPE => serde_json::to_value(ConditionRegistryEngine::seed()),
        RESOURCE_REGISTRY_DOC_TYPE => serde_json::to_value(ResourceRegistryEngine::default()),
        SYSTEM_DEFAULTS_DOC_TYPE => {
            serde_json::to_value(system_defaults.cloned().unwrap_or_default())
        }
        crate::chat::CHAT_SETTINGS_DOC_TYPE => serde_json::to_value(ChatSettingsEngine::default()),
        crate::chat::DICE_SETTINGS_DOC_TYPE => serde_json::to_value(DiceSettingsEngine::default()),
        other => unreachable!("not a config singleton doc_type: {other}"),
    };
    v.expect("engine seed bodies serialize")
}

/// A fresh world-config singleton document: parentless, unnamed, unowned,
/// `default: Observer` permissions (mirroring the shipped client envelope
/// shape, so every member may read config), and the given engine body.
fn config_doc(world_id: Uuid, doc_type: &str, engine: serde_json::Value, now: i64) -> Document {
    Document {
        id: Uuid::new_v4(),
        scope: Scope::World { world_id },
        doc_type: doc_type.to_string(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: None,
        permissions: PermissionSet {
            default: DocRole::Observer,
            ..Default::default()
        },
        embedded: Default::default(),
        parent_id: None,
        engine: Some(engine),
        system: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    }
}

/// The enabled system package's declared world-setting defaults for `world`:
/// reads the enabled set, scans `modules_dir`, and returns the single
/// system-providing enabled module's validated declaration. `None` when no
/// system is enabled, none is declared, or the enabled-set read fails
/// (logged — a config-seed pass must degrade, never block its caller).
///
/// # Examples
///
/// ```text
/// // async; exercised by this module's own tests over an in-memory repo.
/// let sd = enabled_system_defaults(&repo, world_id, modules_dir).await;
/// ```
pub async fn enabled_system_defaults(
    repo: &dyn Repository,
    world_id: Uuid,
    modules_dir: &Path,
) -> Option<SystemDefaultsEngine> {
    let enabled = match repo.world_enabled_modules(world_id).await {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(world = %world_id, error = %e, "enabled-module read failed; config seed proceeds without a system layer");
            return None;
        }
    };
    if enabled.is_empty() {
        return None;
    }
    scan_installed_modules(modules_dir)
        .into_iter()
        .find(|m| m.provides_system && enabled.iter().any(|id| id == &m.id))
        .and_then(|m| m.system_defaults)
}

/// Test-only fixture seed: persist JUST the channel-registry singleton for
/// `world_id` — the production seed body plus any `extra` channel ids a
/// fixture exercises (each registered under its own id as the display name;
/// fixture assertions never render channel names). Production worlds receive
/// this doc from `missing_config_ops` at world create / join reseed, so an
/// in-process fixture that never passes through either must seed it
/// explicitly or `chat::settings::channel_registered` fail-closes every
/// send. Goes through `SqliteRepository::seed_document_unvalidated` (fixtures
/// retain no GM `PermissionContext` to drive an ingress Create), so this
/// stays `#[cfg(test)]`-only exactly like that method.
#[cfg(test)]
pub(crate) async fn seed_test_channel_registry(
    repo: &SqliteRepository,
    world_id: Uuid,
    extra: &[&str],
) {
    let mut registry = ChannelRegistryEngine::seed();
    for id in extra {
        registry.channels.insert(
            (*id).to_string(),
            crate::data::engine::Channel {
                name: (*id).to_string(),
            },
        );
    }
    let engine = serde_json::to_value(registry).expect("ChannelRegistryEngine serializes");
    repo.seed_document_unvalidated(&config_doc(world_id, CHANNEL_REGISTRY_DOC_TYPE, engine, 0))
        .await
        .expect("channel-registry seed persists");
}

/// The `PermissionContext` a config-seed commit is attributed to: the
/// world's first GM member by sorted user id (`Command.author` is a required
/// `Uuid`, so seeds are attributed to a real member deterministically).
/// `None` when the world has no GM member — the seed pass is skipped there
/// (`create_world_owned` always seats one, so this arises only in
/// legacy/test fixtures). Takes the concrete `SqliteRepository` rather than
/// `dyn Repository` because `list_members` is an inherent method the trait
/// does not carry.
///
/// # Examples
///
/// ```text
/// // async; exercised by this module's own tests over an in-memory repo.
/// let ctx = seed_author(&repo, world_id).await;
/// ```
pub async fn seed_author(repo: &SqliteRepository, world_id: Uuid) -> Option<PermissionContext> {
    let members = match repo.list_members(world_id).await {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(world = %world_id, error = %e, "member read failed; config seed skipped");
            return None;
        }
    };
    members
        .into_iter()
        .filter(|(_, _, role)| *role == WorldRole::Gm)
        .map(|(id, _, _)| id)
        .min()
        .map(|user_id| PermissionContext {
            user_id,
            world_role: WorldRole::Gm,
        })
}

#[cfg(test)]
mod tests;
