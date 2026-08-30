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

use serde_json::json;
use uuid::Uuid;

use crate::data::command::{apply_field_change, FieldChange, Operation};
use crate::data::document::{DocRole, Document, PermissionSet};
use crate::data::engine::combat::{
    CapturedCombatant, CombatEngine, CombatHistoryEngine, CombatantEngine, EffectSnapshot,
    TurnRecord, MAX_TURN_HISTORY,
};
use crate::data::engine::{COMBATANT_DOC_TYPE, COMBAT_HISTORY_DOC_TYPE};
use crate::data::migrate::CURRENT_SCHEMA_VERSION;
use crate::data::validation::MAX_SYSTEM_BYTES;
use crate::data::DataError;

use super::effects::{collect_all_effects, set_effect_field};
use super::ops::{set_engine, whole_engine_replace};
use super::transition::rebuild_order;
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

/// Narrows one live combatant to the bands a record keeps — see
/// `CapturedCombatant`'s own INVARIANT for why a whole `Document` is not
/// captured.
fn capture_combatant(c: &Combatant) -> CapturedCombatant {
    CapturedCombatant {
        id: c.doc.id,
        parent_id: c.doc.parent_id,
        name: c.doc.name.clone(),
        permissions: c.doc.permissions.clone(),
        owner: c.doc.owner,
        engine: c.engine.clone(),
        system: c.doc.system.clone(),
    }
}

/// Rebuilds the combatant document a `CapturedCombatant` describes, for the
/// one path that needs a whole document back (`restore` re-`Create`ing a
/// combatant deleted since the boundary). `scope`/`doc_type` are DERIVED
/// from `combat` rather than stored a second time in the record — a
/// combatant is always a `combatant`-typed child of its combat, so a second
/// stored copy would be a forked decision with nothing keeping the two in
/// agreement. `created_at`/`updated_at` are stamped `now`: the document is
/// genuinely new to the store at this instant. `.expect`s rather than
/// propagating: `engine` was parsed FROM stored JSON, so it re-serializes.
fn rebuild_document(combat: &Document, captured: &CapturedCombatant, now: i64) -> Document {
    Document {
        id: captured.id,
        scope: combat.scope.clone(),
        doc_type: COMBATANT_DOC_TYPE.to_string(),
        schema_version: CURRENT_SCHEMA_VERSION,
        name: captured.name.clone(),
        source: None,
        base: None,
        owner: captured.owner,
        permissions: captured.permissions.clone(),
        embedded: BTreeMap::new(),
        parent_id: captured.parent_id,
        engine: Some(
            serde_json::to_value(&captured.engine)
                .expect("a CombatantEngine parsed from stored JSON always re-serializes"),
        ),
        system: captured.system.clone(),
        created_at: now,
        updated_at: now,
    }
}

/// Builds the `TurnRecord` for `post` (a reconstructed post-transition
/// view): every combatant narrowed by `capture_combatant`, and every
/// anchored effect reachable from any combatant's host, deduplicated by
/// `(host, path)` so an unanchored effect shared by two combatants on the
/// same host is captured once — the same dedup `transition::end` applies to
/// its own cleanup pass.
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
            .expect("append_record only runs once a transition has set the entering turn"),
        combatants: post.combatants.iter().map(capture_combatant).collect(),
        effects,
    }
}

/// Serialized-byte ceiling the retained records are evicted down to.
/// `validate_system_size` refuses an `/engine` band over `MAX_SYSTEM_BYTES`
/// and the refusal rolls back the WHOLE combat transition, so eviction
/// targets 90% of the cap rather than the cap itself: the headroom is what
/// the NEXT append writes into before eviction runs again, and a bound with
/// no headroom would leave the log sitting exactly at the refusal threshold.
const HISTORY_BYTE_BUDGET: usize = MAX_SYSTEM_BYTES / 10 * 9;

/// Bytes a `CombatHistoryEngine` serialization adds around its records:
/// `{"records":[` (12) + `],"cursor":` (11) + at most ten digits of a `u32`
/// cursor + `}` (1) = 34, plus one comma per record after the first.
/// Deliberately an OVER-estimate of the cursor's width, since undercounting
/// the envelope is what lets the evicted result still breach the cap.
const HISTORY_ENVELOPE_BYTES: usize = 34;

/// Drops oldest records until the serialized `CombatHistoryEngine` fits
/// `HISTORY_BYTE_BUDGET`, keeping at least the newest record (evicting the
/// record a transition just captured would leave the clock with no boundary
/// to rewind to). Sizes each record ONCE and subtracts as it walks, rather
/// than re-serializing the whole log per eviction. RESIDUAL: a single record
/// larger than the budget on its own cannot be evicted away — an
/// individual boundary that big means a combat with thousands of combatants,
/// and dropping the only record would trade a refused write for a silently
/// empty history.
fn evict_to_fit(records: &mut Vec<TurnRecord>) {
    let sizes: Vec<usize> = records
        .iter()
        .map(|r| {
            serde_json::to_vec(r)
                .expect("a TurnRecord built from stored JSON always serializes")
                .len()
        })
        .collect();
    let mut total: usize =
        HISTORY_ENVELOPE_BYTES + sizes.iter().sum::<usize>() + sizes.len().saturating_sub(1);
    let mut drop_count = 0usize;
    while total > HISTORY_BYTE_BUDGET && drop_count + 1 < records.len() {
        // Removing the oldest of two-or-more records removes its own bytes
        // and exactly one separating comma.
        total -= sizes[drop_count] + 1;
        drop_count += 1;
    }
    records.drain(..drop_count);
}

/// Pushes `record` onto `records` and applies BOTH retention bounds — the
/// `MAX_TURN_HISTORY` count cap and `evict_to_fit`'s serialized-byte cap —
/// returning the resulting cursor (always the newest record). Neither bound
/// implies the other: a count cap does not bound serialized size (which is
/// what `validate_system_size` refuses on), and the byte cap alone would let
/// a combat with tiny records retain an unbounded number of them.
fn bounded_push(records: &mut Vec<TurnRecord>, record: TurnRecord) -> u32 {
    records.push(record);
    if records.len() > MAX_TURN_HISTORY {
        records.drain(..records.len() - MAX_TURN_HISTORY);
    }
    evict_to_fit(records);
    (records.len() - 1) as u32
}

/// The history write THIS transition has already staged, if any: its index
/// in `ops` and the `CombatHistoryEngine` it carries. `settle_turn` calls
/// `append_record` once per turn boundary it crosses, so a single transition
/// can reach this function several times — every call after the first must
/// fold into the op the first one wrote, never add a second write to the
/// same document (`SqliteRepository::apply_intent` permits at most one
/// `Operation::Update` per document per batch, and a second `Create` of a
/// document already created in the batch has no valid pre-image at all).
fn staged_history(
    ops: &[Operation],
    snap: &CombatSnapshot,
) -> Option<(usize, CombatHistoryEngine)> {
    ops.iter().enumerate().find_map(|(i, op)| match op {
        Operation::Create { doc } if doc.doc_type == COMBAT_HISTORY_DOC_TYPE => {
            let engine = serde_json::from_value(doc.engine.clone()?).ok()?;
            Some((i, engine))
        }
        Operation::Update { doc_id, changes } => {
            let (history_doc, _) = snap.history.as_ref()?;
            if *doc_id != history_doc.id {
                return None;
            }
            let change = changes.iter().find(|c| c.path == "/engine")?;
            serde_json::from_value(change.new.clone())
                .ok()
                .map(|engine| (i, engine))
        }
        _ => None,
    })
}

/// Appends a turn-boundary record to the combat's history log: creates the
/// `combat-history` document (`permissions.default: none`) when `snap`
/// carries none yet, otherwise replaces its `/engine` body with the new
/// `records`/`cursor` in one `Update` pre-imaged against the stored history
/// document. Redo history beyond the current cursor is discarded before the
/// FIRST record of a transition is pushed (`records.truncate(cursor + 1)`);
/// both retention bounds then apply via `bounded_push`. `now` stamps a
/// freshly created history document's `created_at`/`updated_at` — the call
/// site (`transition::settle_turn`) already carries `now` for its own
/// transition.
///
/// Called once per turn boundary a transition crosses, including every
/// auto-resolved intermediate entry, so a rewind can land on an `Event`'s or
/// a hidden combatant's turn. Later calls in the same transition FOLD into
/// the op the first one staged (`staged_history`) rather than emitting a
/// second write to the same document — the OCC/one-Update-per-document
/// contract `restore`'s `coalesce_by_doc` satisfies for its own targets.
pub(crate) fn append_record(snap: &CombatSnapshot, ops: &mut Vec<Operation>, now: i64) {
    let post = post_transition_snapshot(snap, ops);
    let record = capture(&post);

    let staged = staged_history(ops, snap);
    let mut engine = match (&staged, &snap.history) {
        (Some((_, already)), _) => already.clone(),
        (None, Some((_, stored))) => {
            let mut e = stored.clone();
            e.records.truncate(stored.cursor as usize + 1);
            e
        }
        (None, None) => CombatHistoryEngine::default(),
    };
    engine.cursor = bounded_push(&mut engine.records, record);
    let value = serde_json::to_value(&engine).expect("CombatHistoryEngine always serializes");

    if let Some((i, _)) = staged {
        match &mut ops[i] {
            Operation::Create { doc } => doc.engine = Some(value),
            Operation::Update { changes, .. } => {
                for change in changes.iter_mut().filter(|c| c.path == "/engine") {
                    // Keeps the batch-start pre-image `whole_engine_replace`
                    // recorded and takes the later post-image, the same fold
                    // `transition::merge_field_changes` performs.
                    change.new = value.clone();
                }
            }
            Operation::Delete { .. } => {
                unreachable!("staged_history only ever indexes a Create or an Update slot")
            }
        }
        return;
    }

    match &snap.history {
        None => {
            let doc = Document {
                id: Uuid::new_v4(),
                scope: post.combat.scope.clone(),
                doc_type: COMBAT_HISTORY_DOC_TYPE.to_string(),
                schema_version: CURRENT_SCHEMA_VERSION,
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
                engine: Some(value),
                system: json!({}),
                created_at: now,
                updated_at: now,
            };
            ops.push(Operation::Create { doc });
        }
        Some((history_doc, _)) => {
            let change = whole_engine_replace(history_doc, value);
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
/// (an exhausted `Event`, deleted by `transition::resolve_event`) — stamped
/// with `now` rather than the stale `created_at`/`updated_at` `capture`
/// recorded, since a re-`Create`d document is genuinely new to the store at
/// this instant; per effect, an `Update` replacing `<path>/engine`
/// (pre-imaged against the live host) when it differs — skipped outright
/// when the host document or the effect's own slot no longer exists live,
/// since there is nothing to write back to. A live document already
/// matching its capture emits no op for it at all.
pub(crate) fn restore(
    snap: &CombatSnapshot,
    record: &TurnRecord,
    now: i64,
) -> Result<Vec<Operation>, CombatError> {
    let mut ops = Vec::new();
    for captured in &record.combatants {
        match snap.combatants.iter().find(|c| c.doc.id == captured.id) {
            Some(live) => {
                let mut changes = Vec::new();
                if live.engine != captured.engine {
                    changes.push(set_engine(
                        &live.doc,
                        "/engine",
                        serde_json::to_value(&captured.engine).map_err(DataError::from)?,
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
                doc: rebuild_document(&snap.combat, captured, now),
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
        if live.engine != captured.engine || live.doc.system != captured.system {
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

/// Reconstructs the combatant list implied once `restore`'s own ops for
/// `record` have applied: every combatant `record` captured (its own,
/// post-restore engine), plus any LIVE combatant `snap` carries that
/// `record` never captured — untouched by `restore`, since there is nothing
/// to write back for a combatant that did not exist at the record's own
/// turn boundary. Feeds `transition::rebuild_order`, so a combatant
/// `restore` re-`Create`s (an `Event` deleted since the record was
/// captured) does not stay absent from `/engine/order`.
pub(crate) fn resulting_combatants(snap: &CombatSnapshot, record: &TurnRecord) -> Vec<Combatant> {
    let mut out: Vec<Combatant> = record
        .combatants
        .iter()
        .map(|captured| Combatant {
            // `rebuild_order` reads only `doc.id` plus the engine's
            // `initiative`/`tiebreak`; `now` never reaches a stored row from
            // here, since a document `restore` genuinely re-`Create`s is
            // built by `restore`'s own call with its own `now`.
            doc: rebuild_document(&snap.combat, captured, snap.combat.updated_at),
            engine: captured.engine.clone(),
        })
        .collect();
    for c in &snap.combatants {
        if !out.iter().any(|o| o.doc.id == c.doc.id) {
            out.push(Combatant {
                doc: c.doc.clone(),
                engine: c.engine.clone(),
            });
        }
    }
    out
}

/// Fast-forwards a redo when history already carries the exact next-turn
/// state: only when `forward_restore` is set, the combat is `active`, a
/// history record exists at BOTH the current cursor and one past it, and
/// `live_equals` confirms nothing has diverged from the record at the
/// current cursor since it was captured — in that case replaying the
/// ordinary transition walk would provably reproduce the cached next
/// record, so this restores straight from it (`restore`, then the combat's
/// `/engine/round`+`/engine/turn`+`/engine/order`, then the history cursor)
/// instead. `now` stamps any combatant `restore` re-`Create`s. `None` under
/// any other condition, letting `transition::advance_impl` run its ordinary
/// walk.
pub(crate) fn fast_forward(
    snap: &CombatSnapshot,
    now: i64,
) -> Result<Option<Vec<Operation>>, CombatError> {
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

    let mut ops = restore(snap, next, now)?;
    let mut combat_changes = vec![
        set_engine(&snap.combat, "/engine/round", json!(next.round))?,
        set_engine(&snap.combat, "/engine/turn", json!(next.turn))?,
    ];
    let resulting = resulting_combatants(snap, next);
    let new_order = rebuild_order(&resulting, &snap.engine.order);
    if new_order != snap.engine.order {
        combat_changes.push(set_engine(&snap.combat, "/engine/order", json!(new_order))?);
    }
    ops.push(Operation::Update {
        doc_id: snap.combat.id,
        changes: combat_changes,
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
