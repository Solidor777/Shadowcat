//! Loads a `CombatSnapshot`: one combat document, its combatants, their
//! hosts, optional history/registry, sibling active combats on the scene,
//! and the resolved-rules override chain — everything a pure `transition`
//! needs, gathered in one read.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::HashMap;

use uuid::Uuid;

use crate::data::document::{world_of, Document};
use crate::data::engine::combat::{
    CombatDefaults, CombatEngine, CombatHistoryEngine, CombatantEngine, CombatantKind,
    ResourceRegistryEngine,
};
use crate::data::engine::{
    SceneEngine, SystemDefaultsEngine, WorldSettingsEngine, COMBATANT_DOC_TYPE, COMBAT_DOC_TYPE,
    COMBAT_HISTORY_DOC_TYPE, RESOURCE_REGISTRY_DOC_TYPE, SYSTEM_DEFAULTS_DOC_TYPE,
    WORLD_SETTINGS_DOC_TYPE,
};
use crate::data::repository::Repository;

use super::CombatError;

/// One combatant: its document and parsed `CombatantEngine`, kept together
/// so a transition never re-parses the same JSON twice.
#[derive(Clone)]
pub struct Combatant {
    /// The stored document.
    pub doc: Document,
    /// Its parsed engine band.
    pub engine: CombatantEngine,
}

/// Everything a pure `transition` needs for one command against one combat,
/// read once.
pub struct CombatSnapshot {
    /// The combat document.
    pub combat: Document,
    /// Its parsed engine band.
    pub engine: CombatEngine,
    /// Every combatant child, parsed.
    pub combatants: Vec<Combatant>,
    /// Every token/actor document a combatant names, keyed by its own id;
    /// present only when the host document actually exists.
    pub hosts: HashMap<Uuid, Document>,
    /// The combat's turn-history document and parsed engine, when one exists.
    pub history: Option<(Document, CombatHistoryEngine)>,
    /// The world's turn-resource registry, when one exists.
    pub registry: Option<ResourceRegistryEngine>,
    /// Other combats active on the same scene (`start`'s pre-empt step reads this).
    pub other_active: Vec<Document>,
    /// The resolved-rules override chain: (system-defaults, world-settings, scene).
    pub chain: (
        Option<CombatDefaults>,
        Option<CombatDefaults>,
        Option<CombatDefaults>,
    ),
}

/// Loads a `CombatSnapshot` for `combat_id` in `world`. `NotFound` when the
/// document is absent, is not a `combat`, or is scoped to a different world
/// — the three cases collapse to one variant so a caller can never use the
/// distinction to probe existence of a combat outside its own world.
pub async fn load_snapshot(
    repo: &dyn Repository,
    world: Uuid,
    combat_id: Uuid,
) -> Result<CombatSnapshot, CombatError> {
    let combat = repo
        .get_document(combat_id)
        .await?
        .filter(|d| d.doc_type == COMBAT_DOC_TYPE && world_of(d) == Some(world))
        .ok_or(CombatError::NotFound)?;
    let engine: CombatEngine = combat
        .engine
        .clone()
        .and_then(|v| serde_json::from_value(v).ok())
        .ok_or(CombatError::NotFound)?;

    let children = repo.query_children(combat_id).await?;
    let mut combatants = Vec::new();
    let mut history = None;
    for child in children {
        match child.doc_type.as_str() {
            COMBATANT_DOC_TYPE => {
                let Some(raw) = child.engine.clone() else {
                    tracing::warn!(id = %child.id, "combatant with no engine body; skipped");
                    continue;
                };
                match serde_json::from_value::<CombatantEngine>(raw) {
                    Ok(engine) => combatants.push(Combatant { doc: child, engine }),
                    Err(e) => tracing::warn!(
                        id = %child.id,
                        error = %e,
                        "unparseable combatant engine; skipped"
                    ),
                }
            }
            COMBAT_HISTORY_DOC_TYPE if history.is_none() => {
                if let Some(raw) = child.engine.clone() {
                    match serde_json::from_value::<CombatHistoryEngine>(raw) {
                        Ok(h) => history = Some((child, h)),
                        Err(e) => tracing::warn!(
                            id = %child.id,
                            error = %e,
                            "unparseable combat-history engine; skipped"
                        ),
                    }
                }
            }
            _ => {}
        }
    }

    let mut hosts = HashMap::new();
    for c in &combatants {
        if let CombatantKind::Actor { token_id, actor_id } = &c.engine.kind {
            for id in [token_id, actor_id].into_iter().flatten() {
                if let std::collections::hash_map::Entry::Vacant(e) = hosts.entry(*id) {
                    if let Some(doc) = repo.get_document(*id).await? {
                        e.insert(doc);
                    }
                }
            }
        }
    }

    let registry = repo
        .query_documents(world, RESOURCE_REGISTRY_DOC_TYPE)
        .await?
        .into_iter()
        .next()
        .and_then(|d| d.engine)
        .and_then(|v| serde_json::from_value(v).ok());

    let other_active = repo
        .query_documents(world, COMBAT_DOC_TYPE)
        .await?
        .into_iter()
        .filter(|d| {
            d.id != combat_id
                && d.engine
                    .as_ref()
                    .and_then(|v| serde_json::from_value::<CombatEngine>(v.clone()).ok())
                    .is_some_and(|e| e.active && e.scene_id == engine.scene_id)
        })
        .collect();

    let defaults = repo
        .query_documents_by_types(world, &[SYSTEM_DEFAULTS_DOC_TYPE, WORLD_SETTINGS_DOC_TYPE])
        .await?;
    let system = defaults
        .iter()
        .find(|d| d.doc_type == SYSTEM_DEFAULTS_DOC_TYPE)
        .and_then(|d| d.engine.clone())
        .and_then(|v| serde_json::from_value::<SystemDefaultsEngine>(v).ok())
        .and_then(|e| e.combat);
    let world_defaults = defaults
        .iter()
        .find(|d| d.doc_type == WORLD_SETTINGS_DOC_TYPE)
        .and_then(|d| d.engine.clone())
        .and_then(|v| serde_json::from_value::<WorldSettingsEngine>(v).ok())
        .and_then(|e| e.combat);
    let scene = match repo.get_document(engine.scene_id).await? {
        Some(d) => d
            .engine
            .and_then(|v| serde_json::from_value::<SceneEngine>(v).ok())
            .and_then(|e| e.combat),
        None => None,
    };

    Ok(CombatSnapshot {
        combat,
        engine,
        combatants,
        hosts,
        history,
        registry,
        other_active,
        chain: (system, world_defaults, scene),
    })
}
