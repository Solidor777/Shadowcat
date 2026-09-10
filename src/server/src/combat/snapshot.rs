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
///
/// # Examples
///
/// ```
/// use shadowcat::combat::Combatant;
/// use shadowcat::data::document::{Document, PermissionSet, Scope};
/// use shadowcat::data::engine::combat::{CombatantEngine, CombatantKind};
/// use std::collections::BTreeMap;
/// use uuid::Uuid;
///
/// let engine = CombatantEngine {
///     kind: CombatantKind::Event { lifespan: None, message: None },
///     initiative: Some(10.0),
///     tiebreak: 0.0,
///     resources: BTreeMap::new(),
/// };
/// let doc = Document {
///     id: Uuid::new_v4(),
///     scope: Scope::World { world_id: Uuid::new_v4() },
///     doc_type: "combatant".to_string(),
///     schema_version: 1,
///     name: Some("Goblin".to_string()),
///     source: None,
///     base: None,
///     owner: None,
///     permissions: PermissionSet::default(),
///     embedded: Default::default(),
///     parent_id: Some(Uuid::new_v4()),
///     engine: Some(serde_json::to_value(&engine).unwrap()),
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
/// let combatant = Combatant { doc, engine };
/// assert_eq!(combatant.engine.initiative, Some(10.0));
/// ```
#[derive(Clone)]
pub struct Combatant {
    /// The stored document.
    pub doc: Document,
    /// Its parsed engine band.
    pub engine: CombatantEngine,
}

/// Everything a pure `transition` needs for one command against one combat,
/// read once.
///
/// # Examples
///
/// ```
/// use shadowcat::combat::CombatSnapshot;
/// use shadowcat::data::document::{Document, PermissionSet, Scope};
/// use shadowcat::data::engine::combat::{
///     CombatEngine, EffectLifecycleDefaults, Enforcement, Interpretation, MovementRules,
///     TurnControl,
/// };
/// use std::collections::HashMap;
/// use uuid::Uuid;
///
/// let engine = CombatEngine {
///     scene_id: Uuid::new_v4(),
///     active: false,
///     round: 0,
///     turn: None,
///     turn_control: TurnControl::OwnerMayEnd,
///     order: Vec::new(),
///     movement: MovementRules {
///         resource: None,
///         interpretation: Interpretation::PerCell,
///         enforcement: Enforcement::None,
///     },
///     effect_cleanup: true,
///     rewind_restore: true,
///     forward_restore: false,
///     effect_lifecycle: EffectLifecycleDefaults::default(),
/// };
/// let combat = Document {
///     id: Uuid::new_v4(),
///     scope: Scope::World { world_id: Uuid::new_v4() },
///     doc_type: "combat".to_string(),
///     schema_version: 1,
///     name: None,
///     source: None,
///     base: None,
///     owner: None,
///     permissions: PermissionSet::default(),
///     embedded: Default::default(),
///     parent_id: None,
///     engine: Some(serde_json::to_value(&engine).unwrap()),
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
/// let snapshot = CombatSnapshot {
///     combat,
///     engine,
///     combatants: Vec::new(),
///     hosts: HashMap::new(),
///     history: None,
///     registry: None,
///     other_active: Vec::new(),
///     chain: (None, None, None),
/// };
/// assert!(snapshot.combatants.is_empty());
/// assert!(!snapshot.engine.active);
/// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::auth::role::ServerRole;
/// use shadowcat::combat::load_snapshot;
/// use shadowcat::data::command::{Operation, WriteOrigin};
/// use shadowcat::data::document::{Document, PermissionSet, Scope, WorldRole};
/// use shadowcat::data::engine::combat::{
///     CombatEngine, EffectLifecycleDefaults, Enforcement, Interpretation, MovementRules,
///     TurnControl,
/// };
/// use shadowcat::data::membership::PermissionContext;
/// use shadowcat::data::repository::Repository;
/// use shadowcat::data::sqlite::SqliteRepository;
/// use uuid::Uuid;
///
/// # #[tokio::main]
/// # async fn main() {
/// let repo = SqliteRepository::connect("sqlite::memory:").await.unwrap();
/// let gm = repo
///     .create_user("gm", None, ServerRole::User, 0)
///     .await
///     .unwrap();
/// let world = repo.create_world_owned("test", gm, 0).await.unwrap();
///
/// let engine = CombatEngine {
///     scene_id: Uuid::new_v4(),
///     active: false,
///     round: 0,
///     turn: None,
///     turn_control: TurnControl::OwnerMayEnd,
///     order: Vec::new(),
///     movement: MovementRules {
///         resource: None,
///         interpretation: Interpretation::PerCell,
///         enforcement: Enforcement::None,
///     },
///     effect_cleanup: true,
///     rewind_restore: true,
///     forward_restore: false,
///     effect_lifecycle: EffectLifecycleDefaults::default(),
/// };
/// let combat_id = Uuid::new_v4();
/// let doc = Document {
///     id: combat_id,
///     scope: Scope::World { world_id: world.id },
///     doc_type: "combat".to_string(),
///     schema_version: 1,
///     name: None,
///     source: None,
///     base: None,
///     owner: None,
///     permissions: PermissionSet::default(),
///     embedded: Default::default(),
///     parent_id: None,
///     engine: Some(serde_json::to_value(&engine).unwrap()),
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
/// let ctx = PermissionContext {
///     user_id: gm,
///     world_role: WorldRole::Gm,
/// };
/// repo.apply_intent(
///     &ctx,
///     world.id,
///     vec![Operation::Create { doc }],
///     0,
///     WriteOrigin::Client,
/// )
/// .await
/// .unwrap();
///
/// let snapshot = load_snapshot(&repo, world.id, combat_id).await.unwrap();
/// assert_eq!(snapshot.combat.id, combat_id);
/// assert!(snapshot.combatants.is_empty());
/// # }
/// ```
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
