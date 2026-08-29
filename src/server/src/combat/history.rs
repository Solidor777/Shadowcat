//! Turn-history record/restore seam. `transition::advance`/`start` call
//! through here so the rewind/redo transition lands as a body change to
//! these two functions, never a call-site change in `transition`.
//!
//! `append_record`/`fast_forward` take `&CombatSnapshot` (the transition's
//! PRE-transition state) rather than `transition::Working` (private to that
//! module) — the post-transition combatant/effect state a captured
//! `TurnRecord` needs is derived by replaying the ops already accumulated in
//! the same command on top of `snap`, the same fold a real repository
//! commit performs field-change by field-change.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{json, Value};
use uuid::Uuid;

use crate::data::command::{apply_field_change, FieldChange, Operation};
use crate::data::document::{DocRole, Document, PermissionSet};
use crate::data::engine::combat::{
    CombatEngine, CombatHistoryEngine, CombatantEngine, EffectSnapshot, TurnRecord,
    MAX_TURN_HISTORY,
};
use crate::data::engine::COMBAT_HISTORY_DOC_TYPE;
use crate::data::DataError;

use super::effects::{collect_all_effects, set_effect_field};
use super::ops::{set_engine, whole_engine_replace};
use super::{CombatError, CombatSnapshot, Combatant};

/// Applies `changes` to `doc` in place — the same field-path mutation
/// `apply_field_change` performs against a stored row, run here against an
/// in-memory copy so `post_transition_snapshot` can fold a transition's own
/// ops onto its pre-transition documents. `.expect`s rather than propagating
/// `Result`: `doc` and `changes` both originate from THIS SAME transition
/// (`doc` is one of `snap`'s own documents, `changes` one of its own `ops`),
/// so `Document`'s serialization and `apply_field_change`'s pointer walk
/// cannot fail here without a defect already present elsewhere in the
/// transition that produced them.
fn apply_doc(doc: &mut Document, changes: &[FieldChange]) {
    let mut v = serde_json::to_value(&*doc).expect("Document always serializes");
    for ch in changes {
        apply_field_change(&mut v, ch)
            .expect("a FieldChange produced by this same transition always applies");
    }
    *doc = serde_json::from_value(v).expect("Document always round-trips through Value");
}

/// Reconstructs the combat/combatants/hosts as they stand once every op in
/// `ops` (accumulated so far by the transition calling `append_record`) has
/// applied on top of `snap`'s pre-transition documents. A `Create` is
/// skipped (this transition creates only the history document itself and,
/// for a resolved `Event`, a chat message — neither is a combat/combatant/
/// host document `capture` reads). A `Delete` drops the combatant from the
/// working list, mirroring `transition::Working::commit_delete_combatant`.
fn post_transition_snapshot(snap: &CombatSnapshot, ops: &[Operation]) -> CombatSnapshot {
    let mut combat = snap.combat.clone();
    let mut combatants: Vec<Document> = snap.combatants.iter().map(|c| c.doc.clone()).collect();
    let mut hosts = snap.hosts.clone();
    for op in ops {
        match op {
            Operation::Create { .. } => {}
            Operation::Delete { doc } => combatants.retain(|c| c.id != doc.id),
            Operation::Update { doc_id, changes } => {
                if *doc_id == combat.id {
                    apply_doc(&mut combat, changes);
                } else if let Some(c) = combatants.iter_mut().find(|c| c.id == *doc_id) {
                    apply_doc(c, changes);
                } else if let Some(h) = hosts.get_mut(doc_id) {
                    apply_doc(h, changes);
                }
            }
        }
    }
    let engine: CombatEngine = serde_json::from_value(
        combat
            .engine
            .clone()
            .expect("a combat document always carries an engine body"),
    )
    .expect("a combat engine re-parses after applying this transition's own field changes");
    let combatants: Vec<Combatant> = combatants
        .into_iter()
        .filter_map(|doc| {
            let raw = doc.engine.clone()?;
            let engine: CombatantEngine = serde_json::from_value(raw).ok()?;
            Some(Combatant { doc, engine })
        })
        .collect();
    CombatSnapshot {
        combat,
        engine,
        combatants,
        hosts,
        history: None,
        registry: None,
        other_active: Vec::new(),
        chain: (None, None, None),
    }
}

/// Builds the `TurnRecord` for `post` (a reconstructed post-transition
/// view): every combatant document, and every anchored effect reachable
/// from any combatant's host, deduplicated by `(host, path)` so an
/// unanchored effect shared by two combatants on the same host is captured
/// once — the same dedup `transition::end` applies to its own cleanup pass.
fn capture(post: &CombatSnapshot) -> TurnRecord {
    let mut seen = HashSet::new();
    let effects: Vec<EffectSnapshot> = collect_all_effects(post)
        .into_iter()
        .map(|(_, r)| r)
        .filter(|r| seen.insert((r.host, r.path.clone())))
        .map(|r| EffectSnapshot {
            host: r.host,
            path: r.path,
            engine: r.engine,
        })
        .collect();
    TurnRecord {
        round: post.engine.round,
        turn: post
            .engine
            .turn
            .expect("append_record only runs once a transition has settled a turn"),
        combatants: post.combatants.iter().map(|c| c.doc.clone()).collect(),
        effects,
    }
}

/// Appends a turn-boundary record to the combat's history log: creates the
/// `combat-history` document (`permissions.default: none`) when `snap`
/// carries none yet, otherwise replaces its `/engine` body with the new
/// `records`/`cursor` in one `Update` pre-imaged against the stored history
/// document — nothing else in a transition writes that document, so no
/// coalescing against `ops`'s own prior entries is needed here (contrast
/// `restore`, whose combatant/host targets a same-transition write CAN
/// collide with). Redo history beyond the current cursor is discarded
/// before the new record is pushed (`records.truncate(cursor + 1)`); the
/// oldest record drops once the log exceeds `MAX_TURN_HISTORY`.
pub(crate) fn append_record(snap: &CombatSnapshot, ops: &mut Vec<Operation>) {
    let post = post_transition_snapshot(snap, ops);
    let record = capture(&post);
    match &snap.history {
        None => {
            let history_engine = CombatHistoryEngine {
                records: vec![record],
                cursor: 0,
            };
            // The only timestamp visible to this function's fixed signature
            // (no `now` is threaded through `append_record`'s call sites):
            // mirrors the combat document's own `updated_at`.
            let stamp = snap.combat.updated_at;
            let doc = Document {
                id: Uuid::new_v4(),
                scope: post.combat.scope.clone(),
                doc_type: COMBAT_HISTORY_DOC_TYPE.to_string(),
                schema_version: 1,
                name: None,
                source: None,
                base: None,
                owner: None,
                permissions: PermissionSet {
                    default: DocRole::None,
                    ..Default::default()
                },
                embedded: BTreeMap::new(),
                parent_id: Some(post.combat.id),
                engine: Some(
                    serde_json::to_value(&history_engine)
                        .expect("CombatHistoryEngine always serializes"),
                ),
                system: json!({}),
                created_at: stamp,
                updated_at: stamp,
            };
            ops.push(Operation::Create { doc });
        }
        Some((history_doc, history_engine)) => {
            let mut records = history_engine.records.clone();
            records.truncate(history_engine.cursor as usize + 1);
            records.push(record);
            if records.len() > MAX_TURN_HISTORY {
                records.remove(0);
            }
            let new_cursor = (records.len() - 1) as u32;
            let updated = CombatHistoryEngine {
                records,
                cursor: new_cursor,
            };
            let change = whole_engine_replace(
                history_doc,
                serde_json::to_value(&updated).expect("CombatHistoryEngine always serializes"),
            );
            ops.push(Operation::Update {
                doc_id: history_doc.id,
                changes: vec![change],
            });
        }
    }
}

/// Merges every `Operation::Update` targeting the same document id into
/// ONE `Update`, concatenating `FieldChange`s in arrival order (never
/// merging two changes at the same path — `restore` never emits more than
/// one change per path per document, unlike `transition::Working::
/// coalesce_updates`, which additionally folds same-path duplicates). Keeps
/// `restore`'s ops within the same OCC contract `SqliteRepository::
/// apply_intent` enforces: at most one `Operation::Update` per document id
/// in a batch.
fn coalesce_by_doc(ops: Vec<Operation>) -> Vec<Operation> {
    let mut merged: Vec<Operation> = Vec::new();
    let mut index_by_id: HashMap<Uuid, usize> = HashMap::new();
    for op in ops {
        let Operation::Update { doc_id, changes } = op else {
            merged.push(op);
            continue;
        };
        if let Some(&i) = index_by_id.get(&doc_id) {
            let Operation::Update {
                changes: existing, ..
            } = &mut merged[i]
            else {
                unreachable!("index_by_id only ever indexes an Update slot");
            };
            existing.extend(changes);
        } else {
            index_by_id.insert(doc_id, merged.len());
            merged.push(Operation::Update { doc_id, changes });
        }
    }
    merged
}

/// Builds the ops that snap every live document `record` describes back to
/// its captured state: per combatant, an `Update` replacing `/engine` and
/// `/system` (pre-imaged against the LIVE document) when either differs, or
/// a `Create` of the captured document when the live combatant is gone
/// (an exhausted `Event`, deleted by `transition::resolve_event`); per
/// effect, an `Update` replacing `<path>/engine` (pre-imaged against the
/// live host) when it differs — skipped outright when the host document or
/// the effect's own slot no longer exists live, since there is nothing to
/// write back to. A live document already matching its capture emits no op
/// for it at all.
pub(crate) fn restore(
    snap: &CombatSnapshot,
    record: &TurnRecord,
) -> Result<Vec<Operation>, CombatError> {
    let mut ops = Vec::new();
    for captured in &record.combatants {
        match snap.combatants.iter().find(|c| c.doc.id == captured.id) {
            Some(live) => {
                let mut changes = Vec::new();
                if live.doc.engine != captured.engine {
                    changes.push(set_engine(
                        &live.doc,
                        "/engine",
                        captured.engine.clone().unwrap_or(Value::Null),
                    )?);
                }
                if live.doc.system != captured.system {
                    changes.push(set_engine(&live.doc, "/system", captured.system.clone())?);
                }
                if !changes.is_empty() {
                    ops.push(Operation::Update {
                        doc_id: captured.id,
                        changes,
                    });
                }
            }
            None => ops.push(Operation::Create {
                doc: captured.clone(),
            }),
        }
    }
    for effect in &record.effects {
        let Some(host) = snap.hosts.get(&effect.host) else {
            continue;
        };
        let host_value = serde_json::to_value(host).map_err(DataError::from)?;
        let pointer = format!("{}/engine", effect.path);
        let Some(live_value) = host_value.pointer(&pointer).cloned() else {
            continue;
        };
        let captured_value = serde_json::to_value(&effect.engine).map_err(DataError::from)?;
        if live_value == captured_value {
            continue;
        }
        let change = set_effect_field(host, &effect.path, "/engine", captured_value)?;
        ops.push(Operation::Update {
            doc_id: effect.host,
            changes: vec![change],
        });
    }
    Ok(coalesce_by_doc(ops))
}

/// Whether every document `record` captured already matches its LIVE
/// counterpart in `snap`: every captured combatant exists live with an
/// identical `engine`/`system`, and every captured effect's host still
/// carries an identical engine body at the effect's own path. A missing
/// live combatant, or a captured effect whose host/slot no longer exists,
/// both count as a mismatch (`false`) — `fast_forward` only skips the
/// ordinary transition walk when replaying it would provably reproduce
/// exactly this cached state.
pub(crate) fn live_equals(snap: &CombatSnapshot, record: &TurnRecord) -> bool {
    for captured in &record.combatants {
        let Some(live) = snap.combatants.iter().find(|c| c.doc.id == captured.id) else {
            return false;
        };
        if live.doc.engine != captured.engine || live.doc.system != captured.system {
            return false;
        }
    }
    for effect in &record.effects {
        let Some(host) = snap.hosts.get(&effect.host) else {
            return false;
        };
        let Ok(host_value) = serde_json::to_value(host) else {
            return false;
        };
        let Some(live_value) = host_value.pointer(&format!("{}/engine", effect.path)) else {
            return false;
        };
        let Ok(captured_value) = serde_json::to_value(&effect.engine) else {
            return false;
        };
        if *live_value != captured_value {
            return false;
        }
    }
    true
}

/// Fast-forwards a redo when history already carries the exact next-turn
/// state: only when `forward_restore` is set, the combat is `active`, a
/// history record exists at BOTH the current cursor and one past it, and
/// `live_equals` confirms nothing has diverged from the record at the
/// current cursor since it was captured — in that case replaying the
/// ordinary transition walk would provably reproduce the cached next
/// record, so this restores straight from it (`restore`, then the combat's
/// `/engine/round`+`/engine/turn`, then the history cursor) instead.
/// `None` under any other condition, letting `transition::advance_impl` run
/// its ordinary walk.
pub(crate) fn fast_forward(snap: &CombatSnapshot) -> Result<Option<Vec<Operation>>, CombatError> {
    if !snap.engine.forward_restore || !snap.engine.active {
        return Ok(None);
    }
    let Some((history_doc, history_engine)) = &snap.history else {
        return Ok(None);
    };
    let cursor = history_engine.cursor as usize;
    let Some(current) = history_engine.records.get(cursor) else {
        return Ok(None);
    };
    let Some(next) = history_engine.records.get(cursor + 1) else {
        return Ok(None);
    };
    if !live_equals(snap, current) {
        return Ok(None);
    }

    let mut ops = restore(snap, next)?;
    let round_change = set_engine(&snap.combat, "/engine/round", json!(next.round))?;
    let turn_change = set_engine(&snap.combat, "/engine/turn", json!(next.turn))?;
    ops.push(Operation::Update {
        doc_id: snap.combat.id,
        changes: vec![round_change, turn_change],
    });

    let mut updated = history_engine.clone();
    updated.cursor = (cursor + 1) as u32;
    let history_change = whole_engine_replace(
        history_doc,
        serde_json::to_value(&updated).map_err(DataError::from)?,
    );
    ops.push(Operation::Update {
        doc_id: history_doc.id,
        changes: vec![history_change],
    });

    Ok(Some(ops))
}
