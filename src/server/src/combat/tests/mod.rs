use std::collections::{BTreeMap, HashMap};

use serde_json::json;
use uuid::Uuid;

use super::*;
use crate::data::command::Operation;
use crate::data::document::{DocRole, Document, PermissionSet, Scope};
use crate::data::engine::combat::*;

pub(super) const WORLD: Uuid = Uuid::from_u128(0xF0);
pub(super) const SCENE: Uuid = Uuid::from_u128(0x5CE);

pub(super) fn doc(
    id: u128,
    doc_type: &str,
    parent: Option<Uuid>,
    engine: serde_json::Value,
) -> Document {
    Document {
        id: Uuid::from_u128(id),
        scope: Scope::World { world_id: WORLD },
        doc_type: doc_type.into(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: None,
        permissions: PermissionSet::default(),
        embedded: BTreeMap::new(),
        parent_id: parent,
        engine: Some(engine),
        system: json!({}),
        created_at: 0,
        updated_at: 0,
    }
}

pub(super) fn combat_engine(
    order: Vec<Uuid>,
    turn: Option<Uuid>,
    round: u32,
    active: bool,
) -> CombatEngine {
    CombatEngine {
        scene_id: SCENE,
        active,
        round,
        turn,
        turn_control: TurnControl::OwnerMayEnd,
        order,
        movement: MovementRules {
            resource: Some("movement".into()),
            interpretation: Interpretation::PerCell,
            enforcement: Enforcement::Hard,
        },
        effect_cleanup: true,
        rewind_restore: true,
        forward_restore: false,
        effect_lifecycle: EffectLifecycleDefaults::default(),
    }
}

pub(super) fn actor_combatant(
    id: u128,
    combat: Uuid,
    actor: u128,
    owner: Option<Uuid>,
    hidden: bool,
    movement: (f64, f64),
) -> Combatant {
    let engine = CombatantEngine {
        kind: CombatantKind::Actor {
            token_id: None,
            actor_id: Some(Uuid::from_u128(actor)),
        },
        initiative: None,
        tiebreak: 0.0,
        resources: BTreeMap::from([(
            "movement".to_string(),
            CombatantResource {
                current: movement.0,
                max: movement.1,
            },
        )]),
    };
    let mut d = doc(
        id,
        "combatant",
        Some(combat),
        serde_json::to_value(&engine).unwrap(),
    );
    d.owner = owner;
    if hidden {
        d.permissions.default = DocRole::None;
    } else {
        d.permissions.default = DocRole::Observer;
    }
    Combatant { doc: d, engine }
}

pub(super) fn event_combatant(
    id: u128,
    combat: Uuid,
    lifespan: Option<u32>,
    message: Option<&str>,
) -> Combatant {
    let engine = CombatantEngine {
        kind: CombatantKind::Event {
            lifespan,
            message: message.map(String::from),
        },
        initiative: Some(30.0),
        tiebreak: 0.0,
        resources: BTreeMap::new(),
    };
    Combatant {
        doc: doc(
            id,
            "combatant",
            Some(combat),
            serde_json::to_value(&engine).unwrap(),
        ),
        engine,
    }
}

/// An actor hosting one effect with a resolved lifecycle and a 2-round duration anchored to `anchor`.
pub(super) fn actor_with_effect(
    id: u128,
    anchor: Option<Uuid>,
    remaining: u32,
    expires: ExpiryPoint,
    unit: DurationUnit,
) -> Document {
    let effect = EffectEngine {
        active: true,
        transfer: false,
        duration: Some(Duration {
            amount: Formula::Number(remaining as f64),
            remaining: Some(remaining),
            unit,
            anchor,
            expires,
        }),
        lifecycle: Some(EffectLifecycle {
            resolved: Some(ResolvedLifecycle {
                on_combat_end: true,
                on_turn_end: false,
                on_advance: true,
            }),
            ..Default::default()
        }),
    };
    let mut actor = doc(id, "actor", None, json!({ "vision": null }));
    actor.embedded.insert(
        "effect".into(),
        vec![doc(
            id + 0x100,
            "effect",
            None,
            serde_json::to_value(&effect).unwrap(),
        )],
    );
    actor
}

pub(super) fn registry_with_movement(turn_start: Formula) -> ResourceRegistryEngine {
    ResourceRegistryEngine {
        resources: BTreeMap::from([(
            "movement".to_string(),
            Resource {
                name: "Movement".into(),
                order: 0,
                binding: ResourceBinding::Tracked {
                    max: Formula::Number(30.0),
                    recover: Recovery {
                        turn_start,
                        ..Default::default()
                    },
                },
            },
        )]),
    }
}

pub(super) fn snapshot(
    engine: CombatEngine,
    combatants: Vec<Combatant>,
    hosts: Vec<Document>,
) -> CombatSnapshot {
    let combat = doc(1, "combat", None, serde_json::to_value(&engine).unwrap());
    CombatSnapshot {
        combat,
        engine,
        combatants,
        hosts: hosts.into_iter().map(|d| (d.id, d)).collect(),
        history: None,
        registry: Some(registry_with_movement(Formula::Number(30.0))),
        other_active: vec![],
        chain: (None, None, None),
    }
}

/// Apply `ops` to the snapshot's documents (Create/Update/Delete over an id map) and return the
/// new map — lets a test chain transitions. Mirrors the real repository's own per-doc field
/// apply (`data::command::apply_field_change`), never re-deriving the mutation semantics.
///
/// Also performs a real OCC check, mirroring `SqliteRepository::apply_intent`'s Phase 1: every
/// `FieldChange.old` is compared against the document's value at that path as it stood at the
/// START of this whole `ops` batch — never a sibling op's already-applied intermediate value,
/// since the real repository loads each `Operation::Update`'s target fresh from the DB before
/// any op in the batch has written anything. A mismatch panics loudly (the shape the real
/// repository would instead reject the whole batch for with `DataError::Conflict`) rather than
/// silently writing `new` over a stale pre-image the way `data::command::apply_field_change`
/// does on its own — that unconditional write is exactly what let a same-batch, same-path
/// double-write pass this harness undetected before this check existed.
pub(super) fn apply(snap: &CombatSnapshot, ops: &[Operation]) -> HashMap<Uuid, Document> {
    let mut docs: HashMap<Uuid, Document> = HashMap::new();
    docs.insert(snap.combat.id, snap.combat.clone());
    for c in &snap.combatants {
        docs.insert(c.doc.id, c.doc.clone());
    }
    for (id, d) in &snap.hosts {
        docs.insert(*id, d.clone());
    }
    let batch_start = docs.clone();
    for op in ops {
        match op {
            Operation::Create { doc } => {
                docs.insert(doc.id, doc.clone());
            }
            Operation::Delete { doc } => {
                docs.remove(&doc.id);
            }
            Operation::Update { doc_id, changes } => {
                if let Some(d) = docs.get_mut(doc_id) {
                    let pre = batch_start.get(doc_id).expect(
                        "an Update target existed pre-batch since it was loaded into docs above",
                    );
                    let pre_value = serde_json::to_value(pre).expect("Document serializes");
                    let mut v = serde_json::to_value(&*d).expect("Document serializes");
                    for ch in changes {
                        let current = pre_value
                            .pointer(&ch.path)
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);
                        assert_eq!(
                            current, ch.old,
                            "OCC violation: FieldChange at {} on {doc_id} carries a pre-image \
                             that does not match the batch-start document value — the real \
                             repository would reject this whole batch with DataError::Conflict",
                            ch.path
                        );
                        crate::data::command::apply_field_change(&mut v, ch)
                            .expect("field change applies to a fixture doc");
                    }
                    *d = serde_json::from_value(v).expect("Document round-trips");
                }
            }
        }
    }
    docs
}

pub(super) fn engine_of<T: serde::de::DeserializeOwned>(
    docs: &HashMap<Uuid, Document>,
    id: Uuid,
) -> T {
    serde_json::from_value(docs[&id].engine.clone().unwrap()).unwrap()
}

mod effects;
mod transition;
