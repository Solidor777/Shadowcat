//! Pure combat-clock transitions: each public function takes a
//! `CombatSnapshot` and returns the `Operation`s for ONE command. Nothing
//! here touches a `Repository` or chooses a `WriteOrigin` — a later
//! caller commits the returned ops.
//!
//! `advance`/`start` share one core mechanism (`settle_turn`): entering a
//! combatant runs its `turn_start` boundary; an `Event` combatant, or a
//! hidden combatant under `TurnControl::OwnerMayEnd`, immediately resolves
//! and the walk continues to the next entry. The step budget starts at
//! `order.len()` and grows by exactly one for every entry the walk
//! deletes (an exhausted `Event`), rather than resetting to a fresh
//! full-length guard on each deletion — so an all-auto-resolving order
//! (e.g. every entry an infinite `Event`) still terminates, in a number of
//! steps linear in `order`'s initial length, and the walk's last allowed
//! step always settles the turn wherever it lands, rather than erroring.
//! INVARIANT: nothing this module does for a hidden combatant is
//! observable from outside its own document (no message, no distinguishing
//! op shape) — the SAME `recover`/`tick`/`expire_by_policy` calls run for a
//! hidden entry as for a visible one, writing only to that combatant's own
//! (permission-gated) document and its own hosts' effect documents. Every
//! `Operation::Update` a transition accumulates is folded, by document id,
//! through `Working::coalesce_updates` once at the end — so a document
//! written from more than one boundary/phase within the same command
//! (an auto-resolved entry's start+end pair, or more than one `resolve_event` deletion in
//! one `settle_turn` walk) always reaches the caller as ONE `Update` per
//! document, with a correct cumulative OCC pre-image.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::{HashMap, HashSet};

use serde_json::json;
use uuid::Uuid;

use crate::chat::{build_message_doc, ActorOwnerRef, Audience, MessageDraft, MessageKind, Segment};
use crate::data::command::{FieldChange, Operation};
use crate::data::document::{DocRole, Document};
use crate::data::engine::combat::{
    resolve_combat_rules, CombatEngine, CombatantKind, DurationUnit, ExpiryPoint, Formula,
    Recovery, ResourceBinding, ResourceRegistryEngine, TurnControl,
};
use crate::data::DataError;
use crate::dice::{RawRoll, RollOutcome, RollSpec};

use super::effects::{collect_all_effects, collect_effects, expire_by_policy, tick, EffectRef};
use super::eval;
use super::history;
use super::ops::{set_engine, whole_engine_replace};
use super::{CombatError, CombatSnapshot, Combatant};

/// A resource mutation intent for `resource`.
pub enum ResourceOp {
    /// Add (or, with a negative amount, subtract) from the current value.
    Delta {
        /// The signed amount to add.
        amount: f64,
    },
    /// Overwrite the current value outright.
    Set {
        /// The new value, clamped to `[0, max]`.
        value: f64,
    },
}

/// A single already-executed roll result to post via `roll`. `formula` is
/// the display formula; `outcome`/`spec`/`raw` are the full deterministic
/// result, kept so a GM can later recalculate it (mirrors `Segment::RollEmbed`).
pub struct RollPost {
    /// The formula as the caller wrote it.
    pub formula: String,
    /// The deterministic outcome.
    pub outcome: RollOutcome,
    /// The parsed spec, for later recalculation.
    pub spec: RollSpec,
    /// The natural-face roll log, for later recalculation.
    pub raw: RawRoll,
}

impl RollPost {
    /// Test seam: builds a `RollPost` whose `outcome.total` is exactly
    /// `total`, by executing the constant expression `"<total>"` (a bare
    /// integer literal parses with no dice groups and evaluates
    /// deterministically) rather than any real dice. `formula` is stored
    /// as given — display-only, independent of how the outcome was faked.
    #[cfg(test)]
    pub(crate) fn test_with_total(formula: &str, total: i64) -> Self {
        let (_, outcome, spec, raw) = crate::chat::rolls::execute_roll_with_seed(
            &total.to_string(),
            crate::dice::notation::ParseContext::default(),
            0,
        )
        .expect("a bare integer constant always parses and evaluates deterministically");
        RollPost {
            formula: formula.to_string(),
            outcome,
            spec,
            raw,
        }
    }
}

/// Whether `c`'s document is hidden from the world AT LARGE: its
/// `permissions.default` grants no role, so any reader carrying no per-user
/// entry of their own receives nothing.
///
/// This is a WORLD-DEFAULT readability test and answers a DIFFERENT question
/// from the per-caller whole-document `cap::READ` gate `combat::authorize`
/// resolves (`resolve_access_world`, via `combat::combatant_access`). Neither
/// implies the other, in either direction: a `default: none` combatant
/// carrying `permissions.users[player] = Owner` is hidden here yet genuinely
/// readable by that player, and a `default: observer` combatant carrying
/// `permissions.users[player] = None` is not hidden here yet genuinely
/// unreadable by them. Authorizing a caller from this predicate is therefore
/// an authorization hole in one direction and a refusal of a legitimate owner
/// in the other; authorization uses `combat::combatant_access` instead.
///
/// The world-default scope is what BOTH consumer classes here need, because
/// neither asks about a particular caller:
/// - the broadcast `Audience` of a message a transition posts (`resolve_event`,
///   `roll`) — `Audience` names a whole-world tier, so the question is whether
///   the entry is public at all, not whether some individual may read it;
/// - `settle_turn`'s auto-resolve rule under `TurnControl::OwnerMayEnd` — one
///   decision for the whole order walk, not a per-recipient one.
pub(crate) fn is_hidden(c: &Combatant) -> bool {
    c.doc.permissions.default == DocRole::None
}

/// Which per-boundary `Recovery` formula applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// A combatant's turn starting.
    TurnStart,
    /// A combatant's turn ending.
    TurnEnd,
    /// A round starting.
    RoundStart,
    /// A round ending.
    RoundEnd,
}

/// Picks `phase`'s formula out of `recover`.
fn phase_formula(recover: &Recovery, phase: Phase) -> &Formula {
    match phase {
        Phase::TurnStart => &recover.turn_start,
        Phase::TurnEnd => &recover.turn_end,
        Phase::RoundStart => &recover.round_start,
        Phase::RoundEnd => &recover.round_end,
    }
}

/// The chain-field name of `phase`, used in evaluation-failure details.
fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::TurnStart => "turn_start",
        Phase::TurnEnd => "turn_end",
        Phase::RoundStart => "round_start",
        Phase::RoundEnd => "round_end",
    }
}

/// Appends the resource-recovery `FieldChange`s `c` earns at `phase` to
/// `out`. Every `Tracked` REGISTRY resource applies: the phase amount and
/// the binding's `max` are evaluated over the combatant's formula host
/// (`eval::formula_host`), and the result clamps to `[0, max]`. An ABSENT
/// stored entry reads as full (`eval::resolved_resource`'s lazy-full rule)
/// and is materialized only when the clamped result differs from full —
/// uniformly for every combatant kind (an `Event` has no host, so a text
/// formula for it resolves through the no-host path). An evaluation failure
/// records a detail line in `failures` — prefixed with the combatant's name
/// or id, so one broken shared formula cannot dedup-collapse across distinct
/// combatants — and applies nothing for that resource; a recovery
/// that would leave `current` unchanged emits no change (there is nothing
/// to write).
fn recover(
    view: &CombatSnapshot,
    c: &Combatant,
    phase: Phase,
    out: &mut Vec<FieldChange>,
    failures: &mut Vec<String>,
) -> Result<(), CombatError> {
    let Some(registry) = &view.registry else {
        return Ok(());
    };
    let host = eval::formula_host(&view.hosts, &c.engine.kind);
    let who = c.doc.name.clone().unwrap_or_else(|| c.doc.id.to_string());
    for (key, res) in &registry.resources {
        let ResourceBinding::Tracked { recover, .. } = &res.binding else {
            continue;
        };
        let stored = c.engine.resources.get(key).map(|r| r.current);
        let delta = match eval::eval_formula(phase_formula(recover, phase), host) {
            Ok(v) => v,
            Err(e) => {
                failures.push(format!("{who}: {key} {}: {}", phase_name(phase), e.detail));
                continue;
            }
        };
        if delta == 0.0 {
            continue;
        }
        let nums = match eval::resolved_resource(&res.binding, stored, host) {
            Ok(n) => n,
            Err(e) => {
                failures.push(format!("{who}: {key} max: {}", e.detail));
                continue;
            }
        };
        let new = (nums.current + delta).clamp(0.0, nums.max);
        if stored.map_or(new == nums.max, |s| new == s) {
            continue;
        }
        out.push(set_engine(
            &c.doc,
            &format!("/engine/resources/{key}/current"),
            json!(new),
        )?);
    }
    Ok(())
}

/// The mutable state one transition threads through its phases: the combat
/// document/engine, every combatant's document/engine, every host document,
/// and the ops accumulated so far. `registry` is read-only for the whole
/// transition (recoveries only ever READ it). Every mutation goes through
/// `commit_combat`/`commit_combatant`/`apply_to_host`, which apply the
/// change to the LOCAL copy before recording (or, for a host, before
/// `commit_host_ops` records) the op — so a later `set_engine`/
/// `set_effect_field` call in the same transition reads a pre-image that
/// reflects every prior write in this same command. This progressive local
/// application is NOT, by itself, what the real repository's own
/// `Operation::Update` OCC contract expects (see `coalesce_updates`'s own
/// doc comment) — `advance`/`start` reconcile the two by running
/// `coalesce_updates` once, at the very end of the transition.
struct Working {
    /// The combat document (kept live).
    combat: Document,
    /// The combat's parsed engine, re-derived from `combat` after every commit.
    engine: CombatEngine,
    /// Every combatant, kept live.
    combatants: Vec<Combatant>,
    /// Every host document, kept live.
    hosts: HashMap<Uuid, Document>,
    /// Read-only resource registry.
    registry: Option<ResourceRegistryEngine>,
    /// Ops accumulated so far, in commit order.
    ops: Vec<Operation>,
    /// Every formula-evaluation failure detail this transition recorded (a
    /// skipped write each); drained into ONE GM-only chat notice by
    /// `flush_eval_notices`.
    eval_failures: Vec<String>,
    /// Total `settle_turn` loop iterations taken. Test-only instrumentation
    /// for asserting the step budget stays linear in `order`'s initial
    /// length (see `settle_turn`'s own doc comment) — never read outside a
    /// test build.
    #[cfg(test)]
    settle_turn_steps: usize,
}

impl Working {
    /// Builds a working copy from an immutable snapshot.
    fn from_snapshot(snap: &CombatSnapshot) -> Self {
        Working {
            combat: snap.combat.clone(),
            engine: snap.engine.clone(),
            combatants: snap
                .combatants
                .iter()
                .map(|c| Combatant {
                    doc: c.doc.clone(),
                    engine: c.engine.clone(),
                })
                .collect(),
            hosts: snap.hosts.clone(),
            registry: snap.registry.clone(),
            ops: Vec::new(),
            eval_failures: Vec::new(),
            #[cfg(test)]
            settle_turn_steps: 0,
        }
    }

    /// A read-only `CombatSnapshot` view of the CURRENT working state, for
    /// calling `recover`/`collect_effects` (which take a `&CombatSnapshot`
    /// by their own public contract, exercised directly by `effects.rs`'s
    /// own tests). `history`/`other_active`/`chain` are inert for every
    /// reader this view is passed to within a transition (none of them
    /// read those fields once a transition is under way).
    fn view(&self) -> CombatSnapshot {
        CombatSnapshot {
            combat: self.combat.clone(),
            engine: self.engine.clone(),
            combatants: self
                .combatants
                .iter()
                .map(|c| Combatant {
                    doc: c.doc.clone(),
                    engine: c.engine.clone(),
                })
                .collect(),
            hosts: self.hosts.clone(),
            history: None,
            registry: self.registry.clone(),
            other_active: Vec::new(),
            chain: (None, None, None),
        }
    }

    /// Applies `changes` to a serialized `Value` in place.
    fn apply_changes(
        v: &mut serde_json::Value,
        changes: &[FieldChange],
    ) -> Result<(), CombatError> {
        for ch in changes {
            crate::data::command::apply_field_change(v, ch)?;
        }
        Ok(())
    }

    /// Commits `changes` against the combat document, re-deriving `engine`,
    /// and records the `Update` op. No-op (records nothing) when `changes`
    /// is empty. INVARIANT: routes ONLY to the combat document — never
    /// resolved by searching `combatants`/`hosts` by id, so a test fixture
    /// whose combatant id happens to numerically collide with an unrelated
    /// host id (never possible with real, randomly-generated UUIDs, but
    /// deliberately exercised by some fixtures here) can never mutate the
    /// wrong document.
    fn commit_combat(&mut self, changes: Vec<FieldChange>) -> Result<(), CombatError> {
        if changes.is_empty() {
            return Ok(());
        }
        let mut v = serde_json::to_value(&self.combat).map_err(DataError::from)?;
        Self::apply_changes(&mut v, &changes)?;
        self.combat = serde_json::from_value(v).map_err(DataError::from)?;
        self.engine =
            serde_json::from_value(self.combat.engine.clone().ok_or(CombatError::NotFound)?)
                .map_err(DataError::from)?;
        self.ops.push(Operation::Update {
            doc_id: self.combat.id,
            changes,
        });
        Ok(())
    }

    /// Commits `changes` against combatant `id` (searched ONLY in
    /// `combatants`, never `hosts` — see `commit_combat`'s INVARIANT note),
    /// re-deriving its engine, and records the `Update` op. No-op when
    /// `changes` is empty.
    fn commit_combatant(&mut self, id: Uuid, changes: Vec<FieldChange>) -> Result<(), CombatError> {
        if changes.is_empty() {
            return Ok(());
        }
        let c = self
            .combatants
            .iter_mut()
            .find(|c| c.doc.id == id)
            .ok_or(CombatError::NotFound)?;
        let mut v = serde_json::to_value(&c.doc).map_err(DataError::from)?;
        Self::apply_changes(&mut v, &changes)?;
        c.doc = serde_json::from_value(v).map_err(DataError::from)?;
        c.engine = serde_json::from_value(c.doc.engine.clone().ok_or(CombatError::NotFound)?)
            .map_err(DataError::from)?;
        self.ops.push(Operation::Update {
            doc_id: id,
            changes,
        });
        Ok(())
    }

    /// Applies `changes` to host `id`'s local copy (searched ONLY in
    /// `hosts`, never `combatants` — see `commit_combat`'s INVARIANT note).
    fn apply_to_host(&mut self, id: Uuid, changes: &[FieldChange]) -> Result<(), CombatError> {
        let h = self.hosts.get_mut(&id).ok_or(CombatError::NotFound)?;
        let mut v = serde_json::to_value(&*h).map_err(DataError::from)?;
        Self::apply_changes(&mut v, changes)?;
        *h = serde_json::from_value(v).map_err(DataError::from)?;
        Ok(())
    }

    /// Commits every op `effects::tick`/`effects::expire_by_policy` produced
    /// — always `Update`s targeting a HOST document id, never a combatant.
    fn commit_host_ops(&mut self, ops: Vec<Operation>) -> Result<(), CombatError> {
        for op in ops {
            if let Operation::Update { doc_id, changes } = &op {
                self.apply_to_host(*doc_id, changes)?;
            }
            self.ops.push(op);
        }
        Ok(())
    }

    /// Records a `Create` op. No local mutation: nothing later in a
    /// transition reads a document it just created.
    fn commit_create(&mut self, doc: Document) {
        self.ops.push(Operation::Create { doc });
    }

    /// Records a `Delete` of a combatant, removing it from the working
    /// combatant list so later phases in the same transition never see it.
    fn commit_delete_combatant(&mut self, doc: Document) {
        self.combatants.retain(|c| c.doc.id != doc.id);
        self.ops.push(Operation::Delete { doc });
    }

    /// Sets `/engine/turn` to `id`.
    fn set_turn(&mut self, id: Uuid) -> Result<(), CombatError> {
        let change = set_engine(&self.combat, "/engine/turn", json!(id))?;
        self.commit_combat(vec![change])
    }

    /// Merges every `Operation::Update` recorded since index `from` that
    /// targets the SAME document id into ONE `Operation::Update` per id, at
    /// that id's first-appearance position. `FieldChange`s at DIFFERENT
    /// paths on the same document are kept side by side, in their original
    /// relative order. `FieldChange`s at the SAME path — a resource
    /// recovered at more than one boundary in this transition (e.g. both
    /// `recover.turn_start` and `recover.turn_end` on one key across an
    /// auto-resolved entry's `enter_turn`+`run_turn_end` pair), `/engine/turn`
    /// written once per entry the walk enters, or `/engine/order` rewritten by
    /// more than one `resolve_event` deletion in one walk — are folded
    /// into ONE `FieldChange` via `merge_field_changes`, never left as two
    /// separate entries. INVARIANT this exists to satisfy:
    /// `SqliteRepository::apply_intent`'s Phase 1 snapshots ONE whole-document
    /// `Value` per `Operation::Update` and checks every `FieldChange` in its
    /// `changes` list against that SAME fixed snapshot — so two separate
    /// entries at one path would have the second checked against the
    /// untouched original value rather than the first entry's `new`, even
    /// though `Working`'s own progressive local application (see `Working`'s
    /// own doc comment) made the second `FieldChange.old` correct RELATIVE
    /// to the first. Concatenating instead of merging would silently
    /// reproduce that exact conflict.
    fn coalesce_updates(&mut self, from: usize) {
        let mut merged: Vec<Operation> = Vec::new();
        let mut index_by_id: HashMap<Uuid, usize> = HashMap::new();
        for op in self.ops.drain(from..) {
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
                merge_field_changes(existing, changes);
            } else {
                index_by_id.insert(doc_id, merged.len());
                merged.push(Operation::Update { doc_id, changes });
            }
        }
        self.ops.extend(merged);
    }

    /// Drains `eval_failures` into ONE GM-only notice op (`eval_notice`), or
    /// does nothing when no failure was recorded.
    fn flush_eval_notices(&mut self, world: Uuid, author: Uuid, now: i64) {
        let failures = std::mem::take(&mut self.eval_failures);
        self.ops.extend(eval_notice(failures, world, author, now));
    }
}

/// ONE GM-only chat notice carrying every DISTINCT failure detail in
/// `failures` (order-preserving dedupe), or `None` when it is empty. The
/// clock proceeds past a bad formula — the affected write is skipped — and
/// this notice is how that skip surfaces instead of silently vanishing.
fn eval_notice(failures: Vec<String>, world: Uuid, author: Uuid, now: i64) -> Option<Operation> {
    if failures.is_empty() {
        return None;
    }
    let mut seen = HashSet::new();
    let lines: Vec<String> = failures
        .into_iter()
        .filter(|f| seen.insert(f.clone()))
        .collect();
    let doc = build_message_doc(
        world,
        author,
        MessageDraft {
            channel: "combat".to_string(),
            actor_owner: None,
            audience: Audience::GmOnly,
            kind: MessageKind::System,
            content: vec![Segment::Text {
                text: format!(
                    "Combat formula evaluation failed — affected values were left unchanged: {}",
                    lines.join("; ")
                ),
            }],
            source: None,
        },
        now,
    );
    Some(Operation::Create { doc })
}

/// Folds `incoming` into `existing` in place, in favor of `existing`
/// (called only from `Working::coalesce_updates`, whose own doc comment
/// carries the full invariant this satisfies): a `FieldChange` at a path
/// already present keeps `existing`'s `old` — the earlier, batch-start-
/// accurate pre-image — but takes `incoming`'s `new`/`remove` — the later,
/// cumulative post-image; a `FieldChange` at a new path is appended,
/// preserving arrival order.
fn merge_field_changes(existing: &mut Vec<FieldChange>, incoming: Vec<FieldChange>) {
    for ch in incoming {
        if let Some(prior) = existing.iter_mut().find(|e| e.path == ch.path) {
            prior.new = ch.new;
            prior.remove = ch.remove;
        } else {
            existing.push(ch);
        }
    }
}

/// Runs `recover(phase)` + `tick(boundary, unit)` for combatant `id`. `seen`
/// dedupes an UNANCHORED effect (`duration.anchor: None`, attributed to
/// EVERY combatant sharing its host) across a multi-combatant sweep of this
/// same boundary (`round_wrap`, `start`'s `RoundStart` pass): the effect is
/// ticked against the FIRST combatant in the sweep that reaches its
/// `(host, path)`, and every later visitor within the SAME sweep skips it —
/// without this, each visitor would re-collect the effect from `w.hosts`
/// (already mutated by the prior visitor's tick) and decrement it again. A
/// single-combatant call passes a fresh, empty set (nothing to dedupe against).
fn run_boundary(
    w: &mut Working,
    id: Uuid,
    phase: Phase,
    boundary: ExpiryPoint,
    unit: DurationUnit,
    seen: &mut HashSet<(Uuid, String)>,
) -> Result<(), CombatError> {
    let mut changes = Vec::new();
    let mut failures = Vec::new();
    {
        let view = w.view();
        let c = w
            .combatants
            .iter()
            .find(|c| c.doc.id == id)
            .ok_or(CombatError::NotFound)?;
        recover(&view, c, phase, &mut changes, &mut failures)?;
    }
    w.eval_failures.append(&mut failures);
    w.commit_combatant(id, changes)?;
    let refs: Vec<EffectRef> = {
        let view = w.view();
        let c = w
            .combatants
            .iter()
            .find(|c| c.doc.id == id)
            .ok_or(CombatError::NotFound)?;
        collect_effects(&view, c)
    };
    let refs: Vec<EffectRef> = refs
        .into_iter()
        .filter(|r| seen.insert((r.host, r.path.clone())))
        .collect();
    let defaults = w.engine.effect_lifecycle.clone();
    let (tick_ops, mut tick_failures) = tick(&w.hosts, &refs, boundary, unit, &defaults)?;
    w.eval_failures.append(&mut tick_failures);
    w.commit_host_ops(tick_ops)?;
    Ok(())
}

/// A combatant's turn ending: `TurnEnd` recovery/tick, plus
/// `on_turn_end` policy expiry when `effect_cleanup` is set.
fn run_turn_end(w: &mut Working, id: Uuid) -> Result<(), CombatError> {
    run_boundary(
        w,
        id,
        Phase::TurnEnd,
        ExpiryPoint::TurnEnd,
        DurationUnit::Turns,
        &mut HashSet::new(),
    )?;
    if w.engine.effect_cleanup {
        let refs: Vec<EffectRef> = {
            let view = w.view();
            let c = w
                .combatants
                .iter()
                .find(|c| c.doc.id == id)
                .ok_or(CombatError::NotFound)?;
            collect_effects(&view, c)
        };
        let defaults = w.engine.effect_lifecycle.clone();
        let (expire_ops, mut expire_failures) =
            expire_by_policy(&w.hosts, &refs, |f| f.on_turn_end, &defaults)?;
        w.eval_failures.append(&mut expire_failures);
        w.commit_host_ops(expire_ops)?;
    }
    Ok(())
}

/// A combatant's turn starting: `TurnStart` recovery/tick.
fn run_turn_start(w: &mut Working, id: Uuid) -> Result<(), CombatError> {
    run_boundary(
        w,
        id,
        Phase::TurnStart,
        ExpiryPoint::TurnStart,
        DurationUnit::Turns,
        &mut HashSet::new(),
    )
}

/// `id` ENTERING its turn: its own `turn_start` boundary, `/engine/turn` set
/// to it, and the turn-boundary history record captured at that point.
/// IDENTICAL for an entry that settles here and one that auto-resolves and
/// is walked past — which is what lets a rewind land on an `Event`'s or a
/// hidden combatant's turn and see the state as that turn began, rather than
/// only on the turn the walk finally stopped at. Capturing BEFORE the
/// auto-resolution that may follow is load-bearing, not incidental: an
/// exhausted `Event` is deleted by its own resolution, and a record captured
/// after that deletion would carry `turn` pointing at a combatant the record
/// no longer holds — `rewind` would then write a `/engine/turn` absent from
/// the rebuilt `/engine/order` and be refused by `CombatEngine::validate`.
///
/// INVARIANT (hidden-combatant secrecy): every `/engine/turn` write this
/// makes for an auto-resolving entry is folded, by `coalesce_updates`, into
/// the ONE combat-document `Update` the transition emits, carrying the
/// batch-start pre-image and the FINAL settled turn — no intermediate value,
/// and no extra operation, reaches any recipient. The history document it
/// writes is `permissions.default: none` (GM-only) and is written exactly
/// once per transition regardless of how many boundaries were crossed.
fn enter_turn(
    w: &mut Working,
    snap: &CombatSnapshot,
    id: Uuid,
    now: i64,
) -> Result<(), CombatError> {
    run_turn_start(w, id)?;
    w.set_turn(id)?;
    history::append_record(snap, &mut w.ops, now);
    Ok(())
}

/// A round boundary: `round += 1`, then `RoundEnd` then `RoundStart`
/// recovery/tick for every combatant in the CURRENT order. Each boundary
/// category (`RoundEnd`, `RoundStart`) gets its OWN dedup set spanning the
/// whole sweep, so a shared unanchored effect is ticked once per boundary
/// category, never once per combatant that happens to share its host.
fn round_wrap(w: &mut Working) -> Result<(), CombatError> {
    let new_round = w.engine.round + 1;
    let change = set_engine(&w.combat, "/engine/round", json!(new_round))?;
    w.commit_combat(vec![change])?;
    let mut end_seen = HashSet::new();
    let mut start_seen = HashSet::new();
    for id in w.engine.order.clone() {
        run_boundary(
            w,
            id,
            Phase::RoundEnd,
            ExpiryPoint::RoundEnd,
            DurationUnit::Rounds,
            &mut end_seen,
        )?;
        run_boundary(
            w,
            id,
            Phase::RoundStart,
            ExpiryPoint::RoundStart,
            DurationUnit::Rounds,
            &mut start_seen,
        )?;
    }
    Ok(())
}

/// Computes the next `order` index after `resolved_entry` finished settling
/// at `idx`. `removed` is true when `resolved_entry` was deleted from
/// `order` (an exhausted `Event`) — the entry that shifted into `idx` IS the
/// next one; otherwise the next index is `idx + 1`. Wrapping past the end
/// of `order` runs `round_wrap` and returns `0`.
fn advance_from(w: &mut Working, idx: usize, removed: bool) -> Result<usize, CombatError> {
    let next = if removed { idx } else { idx + 1 };
    if next >= w.engine.order.len() {
        round_wrap(w)?;
        Ok(0)
    } else {
        Ok(next)
    }
}

/// Resolves an `Event` combatant's own turn in full: posts its message (if
/// any), decrements `lifespan`, and deletes it (removing it from `order`)
/// once `lifespan` reaches `0`. Returns whether it was deleted.
fn resolve_event(
    w: &mut Working,
    id: Uuid,
    world: Uuid,
    author: Uuid,
    now: i64,
) -> Result<bool, CombatError> {
    let (lifespan, message, hidden) = {
        let c = w
            .combatants
            .iter()
            .find(|c| c.doc.id == id)
            .ok_or(CombatError::NotFound)?;
        let CombatantKind::Event { lifespan, message } = &c.engine.kind else {
            return Err(CombatError::Data(DataError::OpFailed(
                "resolve_event called on a non-event combatant".into(),
            )));
        };
        (*lifespan, message.clone(), is_hidden(c))
    };
    if let Some(text) = message {
        let doc = build_message_doc(
            world,
            author,
            MessageDraft {
                channel: "combat".to_string(),
                actor_owner: None,
                audience: if hidden {
                    Audience::GmOnly
                } else {
                    Audience::Public
                },
                kind: MessageKind::Normal,
                content: vec![Segment::Text { text }],
                source: None,
            },
            now,
        );
        w.commit_create(doc);
    }
    match lifespan {
        None => Ok(false),
        Some(n) => {
            let remaining = n.saturating_sub(1);
            if remaining == 0 {
                let doc = w
                    .combatants
                    .iter()
                    .find(|c| c.doc.id == id)
                    .ok_or(CombatError::NotFound)?
                    .doc
                    .clone();
                let mut order = w.engine.order.clone();
                order.retain(|&x| x != id);
                let change = set_engine(&w.combat, "/engine/order", json!(order))?;
                w.commit_combat(vec![change])?;
                w.commit_delete_combatant(doc);
                Ok(true)
            } else {
                let change = {
                    let c = w
                        .combatants
                        .iter()
                        .find(|c| c.doc.id == id)
                        .ok_or(CombatError::NotFound)?;
                    set_engine(&c.doc, "/engine/kind/lifespan", json!(remaining))?
                };
                w.commit_combatant(id, vec![change])?;
                Ok(false)
            }
        }
    }
}

/// Walks `order` starting at `idx`, entering each combatant's turn
/// (`enter_turn`: `turn_start`, `/engine/turn`, history record) and, when it
/// is an `Event` or a hidden combatant under `TurnControl::OwnerMayEnd`,
/// auto-resolving it in full and continuing to the next entry — every
/// auto-resolving entry gets its own `run_turn_end` (a hidden actor
/// unconditionally, an `Event` unless its own `resolve_event` deleted it,
/// leaving no document to write), so the start/end boundary pair is
/// symmetric for every entry the walk touches. The step budget
/// starts at `order.len()` (a single full pass over every entry present at
/// the walk's start) and grows by exactly ONE for every step that deletes
/// an exhausted `Event`, rather than resetting to a fresh full-length guard
/// on each deletion — a deletion can happen at most `order.len()` times in
/// total, since `order` strictly shrinks and never regrows within one walk,
/// so the total budget across the whole walk is bounded by `2 *
/// order.len()`: LINEAR in the walk's starting size, never quadratic — a
/// budget that instead reset to a fresh full length on every removal would
/// produce `O(order.len()^2)` total steps for a descending-lifespan `Event`
/// order, where each cycle expires exactly one entry right as its own
/// cycle's budget runs out. INVARIANT: whichever entry ends up holding
/// `turn` — whether it stopped genuinely or the budget ran out with every entry still
/// auto-resolving — has ALWAYS already completed its own resolution (an
/// auto-resolving entry is never left mid-turn holding `turn`); the
/// budget-exhaustion case parks `turn` on the last entry visited, per the
/// "an all-hidden/all-event order terminates with `turn` on the last visited
/// entry and a round advanced" case.
fn settle_turn(
    w: &mut Working,
    snap: &CombatSnapshot,
    mut idx: usize,
    world: Uuid,
    author: Uuid,
    now: i64,
) -> Result<(), CombatError> {
    if w.engine.order.is_empty() {
        return Err(CombatError::Empty);
    }
    let mut budget = w.engine.order.len();
    let mut steps = 0usize;
    loop {
        if w.engine.order.is_empty() {
            return Err(CombatError::Empty);
        }
        idx %= w.engine.order.len();
        let entry_id = w.engine.order[idx];
        let (is_event, hidden) = {
            let c = w
                .combatants
                .iter()
                .find(|c| c.doc.id == entry_id)
                .ok_or(CombatError::NotFound)?;
            (
                matches!(c.engine.kind, CombatantKind::Event { .. }),
                is_hidden(c),
            )
        };
        let auto_resolves =
            is_event || (hidden && w.engine.turn_control == TurnControl::OwnerMayEnd);

        enter_turn(w, snap, entry_id, now)?;
        if !auto_resolves {
            return Ok(());
        }

        let removed = if is_event {
            let removed = resolve_event(w, entry_id, world, author, now)?;
            if !removed {
                run_turn_end(w, entry_id)?;
            }
            removed
        } else {
            run_turn_end(w, entry_id)?;
            false
        };

        steps += 1;
        #[cfg(test)]
        {
            w.settle_turn_steps = steps;
        }

        if removed {
            // `order` just shrank by one; grant exactly one more step of
            // budget for the shrink, rather than resetting to a fresh
            // full-length guard — the source of the quadratic blowup this
            // budget replaces.
            budget += 1;
        } else if steps >= budget {
            // entry_id fully resolved (turn_start + turn_end/event
            // resolution both ran) but nothing genuinely stopping was found
            // within the budget — `turn` already parks on it, set by this
            // step's own `enter_turn`.
            return Ok(());
        }
        idx = advance_from(w, idx, removed)?;
    }
}

/// Starts (or resumes) a combat. Preempts any other active combat on the
/// same scene FIRST (release before claim, so a batch-level singleton gate
/// sees the release ahead of this combat's own claim). When the combat has
/// no current turn, snapshots the resolved rules chain onto the combat
/// document, sets `round = 1`, runs `RoundStart` recovery/ticks for every
/// combatant, settles the first entry's turn (auto-resolving past any
/// leading `Event`/hidden-`OwnerMayEnd` entries), and creates the combat's
/// first `combat-history` record. A combat that already has a turn (a
/// pause/resume) is resumed as-is: no re-snapshot, no recovery, just
/// `active = true`.
pub fn start(
    snap: &CombatSnapshot,
    now: i64,
    world: Uuid,
    author: Uuid,
) -> Result<Vec<Operation>, CombatError> {
    let mut w = Working::from_snapshot(snap);
    for other in &snap.other_active {
        let change = set_engine(other, "/engine/active", json!(false))?;
        w.ops.push(Operation::Update {
            doc_id: other.id,
            changes: vec![change],
        });
    }
    if w.engine.turn.is_none() {
        let resolved = resolve_combat_rules(
            snap.chain.0.as_ref(),
            snap.chain.1.as_ref(),
            snap.chain.2.as_ref(),
        );
        let first = *w.engine.order.first().ok_or(CombatError::Empty)?;
        let changes = vec![
            set_engine(
                &w.combat,
                "/engine/turn_control",
                json!(resolved.turn_control),
            )?,
            set_engine(
                &w.combat,
                "/engine/movement",
                serde_json::to_value(&resolved.movement).map_err(DataError::from)?,
            )?,
            set_engine(
                &w.combat,
                "/engine/effect_cleanup",
                json!(resolved.effect_cleanup),
            )?,
            set_engine(
                &w.combat,
                "/engine/rewind_restore",
                json!(resolved.rewind_restore),
            )?,
            set_engine(
                &w.combat,
                "/engine/forward_restore",
                json!(resolved.forward_restore),
            )?,
            set_engine(
                &w.combat,
                "/engine/effect_lifecycle",
                serde_json::to_value(&resolved.effect_lifecycle).map_err(DataError::from)?,
            )?,
            set_engine(&w.combat, "/engine/round", json!(1u32))?,
            set_engine(&w.combat, "/engine/turn", json!(first))?,
        ];
        w.commit_combat(changes)?;

        // One dedup set spans the whole sweep — see `round_wrap`'s identical
        // note: an unanchored effect shared by two combatants on the same
        // host is ticked once for the sweep, not once per combatant.
        let mut seen = HashSet::new();
        for id in w.engine.order.clone() {
            run_boundary(
                &mut w,
                id,
                Phase::RoundStart,
                ExpiryPoint::RoundStart,
                DurationUnit::Rounds,
                &mut seen,
            )?;
        }
        settle_turn(&mut w, snap, 0, world, author, now)?;
    }
    let active_change = set_engine(&w.combat, "/engine/active", json!(true))?;
    w.commit_combat(vec![active_change])?;
    w.flush_eval_notices(world, author, now);
    w.coalesce_updates(0);
    Ok(w.ops)
}

/// Shared implementation for `advance`: builds the ops for ending the
/// current turn, advancing to the next combatant (wrapping the round when
/// the order is exhausted), and settling the new turn. Returns the
/// `settle_turn` step count alongside the ops so `advance_with_step_count`
/// (test-only) can assert its linear bound without re-deriving `advance`'s
/// own wiring separately.
fn advance_impl(
    snap: &CombatSnapshot,
    world: Uuid,
    author: Uuid,
    now: i64,
) -> Result<(Vec<Operation>, usize), CombatError> {
    if let Some(ops) = history::fast_forward(snap, now)? {
        return Ok((ops, 0));
    }
    if !snap.engine.active {
        return Err(CombatError::NotRunning);
    }
    if snap.engine.order.is_empty() {
        return Err(CombatError::Empty);
    }
    let current_id = snap.engine.turn.ok_or(CombatError::Empty)?;
    let mut w = Working::from_snapshot(snap);
    let start_idx = w
        .engine
        .order
        .iter()
        .position(|&id| id == current_id)
        .ok_or(CombatError::NotFound)?;
    run_turn_end(&mut w, current_id)?;
    let next_idx = advance_from(&mut w, start_idx, false)?;
    settle_turn(&mut w, snap, next_idx, world, author, now)?;
    w.flush_eval_notices(world, author, now);
    w.coalesce_updates(0);
    #[cfg(test)]
    let steps = w.settle_turn_steps;
    #[cfg(not(test))]
    let steps = 0usize;
    Ok((w.ops, steps))
}

/// Ends the current turn, advances to the next combatant (wrapping the
/// round when the order is exhausted), and settles the new turn — see the
/// module doc for the auto-resolve/termination guarantee.
pub fn advance(
    snap: &CombatSnapshot,
    world: Uuid,
    author: Uuid,
    now: i64,
) -> Result<Vec<Operation>, CombatError> {
    advance_impl(snap, world, author, now).map(|(ops, _)| ops)
}

/// Test-only seam: identical to `advance`, but also returns the number of
/// `settle_turn` loop iterations taken — lets a test assert the walk's
/// removal-adjusted step budget stays linear in `order`'s starting length
/// rather than inferring iteration counts from an external proxy (wall
/// clock, op count) that a `coalesce_updates` pass would otherwise obscure.
#[cfg(test)]
pub(crate) fn advance_with_step_count(
    snap: &CombatSnapshot,
    world: Uuid,
    author: Uuid,
    now: i64,
) -> Result<(Vec<Operation>, usize), CombatError> {
    advance_impl(snap, world, author, now)
}

/// Pauses a running combat: clears `active` only. `NotRunning` if it is not
/// currently active.
pub fn pause(snap: &CombatSnapshot) -> Result<Vec<Operation>, CombatError> {
    if !snap.engine.active {
        return Err(CombatError::NotRunning);
    }
    let change = set_engine(&snap.combat, "/engine/active", json!(false))?;
    Ok(vec![Operation::Update {
        doc_id: snap.combat.id,
        changes: vec![change],
    }])
}

/// Moves the combat clock back to the turn boundary recorded immediately
/// before the current one: `Unrewindable` when there is no history yet, or
/// the cursor already sits at the oldest retained record. Restores every
/// document `history::restore` can reach from the target record when
/// `rewind_restore` is set (skipped entirely, moving only the clock,
/// when it is not); always writes the combat's `/engine/round` and
/// `/engine/turn` back to the target record's own. When `rewind_restore` is
/// set, `/engine/order` is also rebuilt (via `rebuild_order`) from the
/// post-restore combatant set (`history::resulting_combatants`) — a
/// combatant `restore` re-`Create`s (an exhausted `Event`, deleted since the
/// target record was captured) would otherwise stay live but absent from
/// `order`, unreachable by any future turn walk. `now` stamps any combatant
/// `restore` re-`Create`s. Every later record is dropped from history unless
/// `forward_restore` is set, in which case the redo history survives for
/// `history::fast_forward` to later replay.
///
/// `RewindUnreachable` when the combat engine this rewind would write is not
/// a valid one — checked BEFORE any op is built, by running the prospective
/// post-image through `CombatEngine::validate` itself rather than restating
/// its rules here, so the two can never disagree about what a valid clock
/// state is. The reachable case is `rewind_restore` off with a target
/// boundary whose `turn` names a combatant since deleted and dropped from
/// `order` (an exhausted `Event`): nothing restores it, so `turn` would name
/// an id absent from `order`. Refusing honestly is the only correct outcome —
/// rebuilding `order` without restoring the document would leave a phantom
/// entry naming a document that does not exist, and clamping or skipping the
/// `/engine/turn` write would move the clock somewhere the GM never asked for.
pub fn rewind(snap: &CombatSnapshot, now: i64) -> Result<Vec<Operation>, CombatError> {
    let Some((history_doc, history_engine)) = &snap.history else {
        return Err(CombatError::Unrewindable);
    };
    if history_engine.cursor == 0 {
        return Err(CombatError::Unrewindable);
    }
    let new_cursor = history_engine.cursor - 1;
    let target = history_engine
        .records
        .get(new_cursor as usize)
        .ok_or(CombatError::NotFound)?;

    // `order` is rebuilt only alongside a restore: the rebuilt order is derived from the
    // documents `history::restore` will re-`Create`, so without that restore there is nothing
    // for a re-added entry to name.
    let new_order = if snap.engine.rewind_restore {
        rebuild_order(
            &history::resulting_combatants(snap, target),
            &snap.engine.order,
        )
    } else {
        snap.engine.order.clone()
    };

    let mut post = snap.engine.clone();
    post.round = target.round;
    post.turn = Some(target.turn);
    post.order.clone_from(&new_order);
    if post.validate().is_err() {
        return Err(CombatError::RewindUnreachable);
    }

    let mut ops = if snap.engine.rewind_restore {
        history::restore(snap, target, now)?
    } else {
        Vec::new()
    };

    let mut combat_changes = vec![
        set_engine(&snap.combat, "/engine/round", json!(target.round))?,
        set_engine(&snap.combat, "/engine/turn", json!(target.turn))?,
    ];
    if new_order != snap.engine.order {
        combat_changes.push(set_engine(&snap.combat, "/engine/order", json!(new_order))?);
    }
    ops.push(Operation::Update {
        doc_id: snap.combat.id,
        changes: combat_changes,
    });

    let mut updated = history_engine.clone();
    updated.cursor = new_cursor;
    if !snap.engine.forward_restore {
        updated.records.truncate(new_cursor as usize + 1);
    }
    let history_change = whole_engine_replace(
        history_doc,
        serde_json::to_value(&updated).map_err(DataError::from)?,
    );
    ops.push(Operation::Update {
        doc_id: history_doc.id,
        changes: vec![history_change],
    });

    Ok(ops)
}

/// Ends a combat outright: expires every `on_combat_end`-policy effect
/// (when `effect_cleanup`), then deletes the combat document (whose cascade
/// removes its combatants/history — this never emits a child `Delete`
/// itself). Effect expiry ops always precede the `Delete`; an evaluation
/// failure skips its one effect and surfaces as a GM-only notice in the same
/// command (`world`/`author`/`now` exist for that notice).
pub fn end(
    snap: &CombatSnapshot,
    world: Uuid,
    author: Uuid,
    now: i64,
) -> Result<Vec<Operation>, CombatError> {
    let mut ops = Vec::new();
    if snap.engine.effect_cleanup {
        // `collect_all_effects` unions every combatant's refs; an unanchored
        // effect on a host shared by two combatants would otherwise appear
        // TWICE, producing two `Operation::Update`s against the same
        // (host, path) with the SAME stale pre-image (`end` never mutates a
        // working copy between combatants) — the second would fail its OCC
        // check against the real repository once the first has applied.
        let mut seen = HashSet::new();
        let refs: Vec<EffectRef> = collect_all_effects(snap)
            .into_iter()
            .map(|(_, r)| r)
            .filter(|r| seen.insert((r.host, r.path.clone())))
            .collect();
        let (expire_ops, failures) = expire_by_policy(
            &snap.hosts,
            &refs,
            |f| f.on_combat_end,
            &snap.engine.effect_lifecycle,
        )?;
        ops.extend(expire_ops);
        ops.extend(eval_notice(failures, world, author, now));
    }
    ops.push(Operation::Delete {
        doc: snap.combat.clone(),
    });
    Ok(ops)
}

/// Posts one already-rolled result per `(combatant_id, RollPost)` pair:
/// writes `initiative`, posts a roll message (whispered to the GM only when
/// the combatant is hidden), and rebuilds `order` from the updated
/// initiatives. Authorization (who may roll for whom) is the caller's
/// concern, not this function's.
pub fn roll(
    snap: &CombatSnapshot,
    results: &[(Uuid, RollPost)],
    world: Uuid,
    author: Uuid,
    channel: &str,
    now: i64,
) -> Result<Vec<Operation>, CombatError> {
    // INVARIANT: `results` must carry at most one entry per `combatant_id` —
    // a duplicate would build two `FieldChange`s against the same combatant
    // from the SAME unmutated `snap` pre-image, so the second write's OCC
    // `old` would already be stale by the time the first is applied. The
    // caller (the wire-dispatch layer) is responsible for this; nothing here
    // re-derives or dedupes it.
    let mut ops = Vec::new();
    for (id, post) in results {
        let c = snap
            .combatants
            .iter()
            .find(|c| c.doc.id == *id)
            .ok_or(CombatError::NotFound)?;
        let change = set_engine(
            &c.doc,
            "/engine/initiative",
            json!(post.outcome.total as f64),
        )?;
        ops.push(Operation::Update {
            doc_id: c.doc.id,
            changes: vec![change],
        });

        let hidden = is_hidden(c);
        let actor_owner = match &c.engine.kind {
            CombatantKind::Actor {
                token_id: Some(token_id),
                ..
            } => Some(ActorOwnerRef::TokenInstance {
                token_id: *token_id,
            }),
            CombatantKind::Actor {
                actor_id: Some(actor_id),
                ..
            } => Some(ActorOwnerRef::Actor {
                actor_id: *actor_id,
            }),
            _ => None,
        };
        let msg = build_message_doc(
            world,
            author,
            MessageDraft {
                channel: channel.to_string(),
                actor_owner,
                audience: if hidden {
                    Audience::GmOnly
                } else {
                    Audience::Public
                },
                kind: MessageKind::Roll,
                content: vec![Segment::RollEmbed {
                    formula: post.formula.clone(),
                    outcome: post.outcome.clone(),
                    roll_id: Uuid::new_v4(),
                    spec: Some(Box::new(post.spec.clone())),
                    raw: Some(Box::new(post.raw.clone())),
                    recalc_history: None,
                }],
                source: None,
            },
            now,
        );
        ops.push(Operation::Create { doc: msg });
    }

    let updated: Vec<Combatant> = snap
        .combatants
        .iter()
        .map(|c| {
            let mut engine = c.engine.clone();
            if let Some((_, post)) = results.iter().find(|(id, _)| *id == c.doc.id) {
                engine.initiative = Some(post.outcome.total as f64);
            }
            Combatant {
                doc: c.doc.clone(),
                engine,
            }
        })
        .collect();
    let new_order = rebuild_order(&updated, &snap.engine.order);
    let order_change = set_engine(&snap.combat, "/engine/order", json!(new_order))?;
    ops.push(Operation::Update {
        doc_id: snap.combat.id,
        changes: vec![order_change],
    });
    Ok(ops)
}

/// Adjusts one combatant's `Tracked` resource: `Delta` adds a signed amount,
/// `Set` overwrites outright; both clamp to `[0, max]` with `max` evaluated
/// over the combatant's formula host. The resource is looked up in the
/// registry (`NotFound` for an unknown key); an absent combatant entry reads
/// as full and is materialized by this write. A `Mirror`-bound key is
/// refused (`Forbidden`): its number lives on the actor and is changed by
/// writing the actor document through the ordinary path, never through the
/// clock. `Forbidden` when the requested amount/value is non-finite (never
/// silently truncated/ignored); an evaluation failure refuses with the same
/// uniform wording rather than guessing a ceiling.
pub fn resource(
    snap: &CombatSnapshot,
    combatant_id: Uuid,
    key: &str,
    op: ResourceOp,
) -> Result<Vec<Operation>, CombatError> {
    let c = snap
        .combatants
        .iter()
        .find(|c| c.doc.id == combatant_id)
        .ok_or(CombatError::NotFound)?;
    let def = snap
        .registry
        .as_ref()
        .and_then(|r| r.resources.get(key))
        .ok_or(CombatError::NotFound)?;
    if matches!(def.binding, ResourceBinding::Mirror { .. }) {
        return Err(CombatError::Forbidden);
    }
    let host = eval::formula_host(&snap.hosts, &c.engine.kind);
    let stored = c.engine.resources.get(key).map(|r| r.current);
    let nums = eval::resolved_resource(&def.binding, stored, host)
        .map_err(|e| CombatError::Data(DataError::OpFailed(e.detail)))?;
    let new = match op {
        ResourceOp::Delta { amount } => {
            if !amount.is_finite() {
                return Err(CombatError::Forbidden);
            }
            (nums.current + amount).clamp(0.0, nums.max)
        }
        ResourceOp::Set { value } => {
            if !value.is_finite() {
                return Err(CombatError::Forbidden);
            }
            value.clamp(0.0, nums.max)
        }
    };
    let change = set_engine(
        &c.doc,
        &format!("/engine/resources/{key}/current"),
        json!(new),
    )?;
    Ok(vec![Operation::Update {
        doc_id: c.doc.id,
        changes: vec![change],
    }])
}

/// Re-sorts `order` by `rebuild_order` and writes it back, or emits nothing
/// when the recomputed order is unchanged.
pub fn sort(snap: &CombatSnapshot) -> Result<Vec<Operation>, CombatError> {
    let new_order = rebuild_order(&snap.combatants, &snap.engine.order);
    if new_order == snap.engine.order {
        return Ok(Vec::new());
    }
    let change = set_engine(&snap.combat, "/engine/order", json!(new_order))?;
    Ok(vec![Operation::Update {
        doc_id: snap.combat.id,
        changes: vec![change],
    }])
}

/// Stable-sorts `existing`'s ids (any id absent from `combatants` dropped;
/// any combatant absent from `existing` appended) by initiative descending
/// (`None` last), then `tiebreak` descending, then — for a full tie —
/// `existing`'s own relative order (the sort's stability).
pub fn rebuild_order(combatants: &[Combatant], existing: &[Uuid]) -> Vec<Uuid> {
    let mut ids: Vec<Uuid> = existing
        .iter()
        .copied()
        .filter(|id| combatants.iter().any(|c| c.doc.id == *id))
        .collect();
    for c in combatants {
        if !ids.contains(&c.doc.id) {
            ids.push(c.doc.id);
        }
    }
    let lookup = |id: &Uuid| combatants.iter().find(|c| c.doc.id == *id);
    ids.sort_by(|x, y| {
        let (Some(cx), Some(cy)) = (lookup(x), lookup(y)) else {
            return std::cmp::Ordering::Equal;
        };
        let init_cmp = match (cx.engine.initiative, cy.engine.initiative) {
            (Some(vx), Some(vy)) => vy.partial_cmp(&vx).unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        };
        init_cmp.then_with(|| {
            cy.engine
                .tiebreak
                .partial_cmp(&cx.engine.tiebreak)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    ids
}
