//! Pure combat-clock transitions: each public function takes a
//! `CombatSnapshot` and returns the `Operation`s for ONE command. Nothing
//! here touches a `Repository` or chooses a `WriteOrigin` — a later
//! caller commits the returned ops.
//!
//! `advance`/`start` share one core mechanism (`settle_turn`): entering a
//! combatant runs its `turn_start` boundary; an `Event` combatant, or a
//! hidden combatant under `TurnControl::OwnerMayEnd`, immediately resolves
//! and the walk continues to the next entry, bounded by `order.len()`
//! iterations so an all-auto-resolving order (e.g. every entry an infinite
//! `Event`) cannot loop forever — the guard's LAST iteration always stops
//! and settles the turn wherever it lands, rather than erroring.
//! INVARIANT: nothing this module does for a hidden combatant is
//! observable from outside its own document (no message, no distinguishing
//! op shape) — the SAME `recover`/`tick`/`expire_by_policy` calls run for a
//! hidden entry as for a visible one, writing only to that combatant's own
//! (permission-gated) document and its own hosts' effect documents.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::HashMap;

use serde_json::json;
use uuid::Uuid;

use crate::chat::{build_message_doc, ActorOwnerRef, Audience, MessageDraft, MessageKind, Segment};
use crate::data::command::{FieldChange, Operation};
use crate::data::document::{DocRole, Document};
use crate::data::engine::combat::{
    resolve_combat_rules, CombatEngine, CombatHistoryEngine, CombatantKind, DurationUnit,
    ExpiryPoint, Formula, Recovery, ResourceBinding, ResourceRegistryEngine, TurnControl,
    TurnRecord,
};
use crate::data::DataError;
use crate::dice::{RawRoll, RollOutcome, RollSpec};

use super::effects::{collect_all_effects, collect_effects, expire_by_policy, tick, EffectRef};
use super::history;
use super::ops::set_engine;
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

/// Whether `c`'s document is hidden from every non-owner/non-GM reader
/// (`permissions.default: none`) — the SAME test a real read-transition
/// gate applies; this module never re-derives visibility from any other
/// field.
fn is_hidden(c: &Combatant) -> bool {
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

/// Appends the resource-recovery `FieldChange`s `c` earns at `phase` to
/// `out`. Only a `Tracked`-bound resource recovers; a `Formula::Text` phase
/// formula is never evaluated server-side (the server has no formula
/// engine) and applies nothing — the resource's `current` is left exactly
/// as stored. A resolved `Formula::Number` of `0`, or one that would leave
/// `current` unchanged after clamping to `[0, max]`, emits no change either
/// (there is nothing to write).
fn recover(
    view: &CombatSnapshot,
    c: &Combatant,
    phase: Phase,
    out: &mut Vec<FieldChange>,
) -> Result<(), CombatError> {
    let Some(registry) = &view.registry else {
        return Ok(());
    };
    for (key, res) in &c.engine.resources {
        let Some(resource_def) = registry.resources.get(key) else {
            continue;
        };
        let ResourceBinding::Tracked { recover, .. } = &resource_def.binding else {
            continue;
        };
        let Formula::Number(n) = phase_formula(recover, phase) else {
            continue;
        };
        if *n == 0.0 {
            continue;
        }
        let new = (res.current + n).clamp(0.0, res.max);
        if new == res.current {
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
/// `commit`, which applies the change to the LOCAL copy before recording the
/// op — so a later `set_engine`/`set_effect_field` call in the same
/// transition reads a pre-image that reflects every prior write in this
/// same command, exactly as the real repository will see it when the ops
/// are applied in order.
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
}

/// Runs `recover(phase)` + `tick(boundary, unit)` for combatant `id`.
fn run_boundary(
    w: &mut Working,
    id: Uuid,
    phase: Phase,
    boundary: ExpiryPoint,
    unit: DurationUnit,
) -> Result<(), CombatError> {
    let mut changes = Vec::new();
    {
        let view = w.view();
        let c = w
            .combatants
            .iter()
            .find(|c| c.doc.id == id)
            .ok_or(CombatError::NotFound)?;
        recover(&view, c, phase, &mut changes)?;
    }
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
    let tick_ops = tick(&w.hosts, &refs, boundary, unit)?;
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
        let expire_ops = expire_by_policy(&w.hosts, &refs, |r| r.on_turn_end)?;
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
    )
}

/// A round boundary: `round += 1`, then `RoundEnd` then `RoundStart`
/// recovery/tick for every combatant in the CURRENT order.
fn round_wrap(w: &mut Working) -> Result<(), CombatError> {
    let new_round = w.engine.round + 1;
    let change = set_engine(&w.combat, "/engine/round", json!(new_round))?;
    w.commit_combat(vec![change])?;
    for id in w.engine.order.clone() {
        run_boundary(
            w,
            id,
            Phase::RoundEnd,
            ExpiryPoint::RoundEnd,
            DurationUnit::Rounds,
        )?;
        run_boundary(
            w,
            id,
            Phase::RoundStart,
            ExpiryPoint::RoundStart,
            DurationUnit::Rounds,
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
/// (`run_turn_start`) and, when it is an `Event` or a hidden combatant under
/// `TurnControl::OwnerMayEnd`, auto-resolving it and continuing to the next
/// entry. Bounded to `order.len()` iterations: the LAST iteration always
/// stops and settles `turn` on whatever entry it reached, so an order that
/// never contains a stopping entry (every entry an infinite `Event`, or
/// every entry hidden under `OwnerMayEnd`) cannot loop forever.
fn settle_turn(
    w: &mut Working,
    mut idx: usize,
    world: Uuid,
    author: Uuid,
    now: i64,
) -> Result<(), CombatError> {
    if w.engine.order.is_empty() {
        return Err(CombatError::Empty);
    }
    let guard = w.engine.order.len();
    for i in 0..guard {
        if w.engine.order.is_empty() {
            return Err(CombatError::Empty);
        }
        idx %= w.engine.order.len();
        let entry_id = w.engine.order[idx];
        run_turn_start(w, entry_id)?;
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
        if !auto_resolves || i + 1 == guard {
            w.set_turn(entry_id)?;
            return Ok(());
        }
        let removed = if is_event {
            resolve_event(w, entry_id, world, author, now)?
        } else {
            run_turn_end(w, entry_id)?;
            false
        };
        idx = advance_from(w, idx, removed)?;
    }
    unreachable!("the guard's final iteration always returns via the stop branch above")
}

/// Builds the first `combat-history` record document for `start`.
fn build_history_doc(w: &Working, first: Uuid, now: i64) -> Result<Document, CombatError> {
    let record = TurnRecord {
        round: w.engine.round,
        turn: first,
        combatants: w.combatants.iter().map(|c| c.doc.clone()).collect(),
        effects: Vec::new(),
    };
    let engine = CombatHistoryEngine {
        records: vec![record],
        cursor: 0,
    };
    Ok(Document {
        id: Uuid::new_v4(),
        scope: w.combat.scope.clone(),
        doc_type: crate::data::engine::COMBAT_HISTORY_DOC_TYPE.to_string(),
        schema_version: 1,
        name: None,
        source: None,
        base: None,
        owner: None,
        permissions: crate::data::document::PermissionSet {
            default: DocRole::None,
            ..Default::default()
        },
        embedded: std::collections::BTreeMap::new(),
        parent_id: Some(w.combat.id),
        engine: Some(serde_json::to_value(&engine).map_err(DataError::from)?),
        system: json!({}),
        created_at: now,
        updated_at: now,
    })
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

        for id in w.engine.order.clone() {
            run_boundary(
                &mut w,
                id,
                Phase::RoundStart,
                ExpiryPoint::RoundStart,
                DurationUnit::Rounds,
            )?;
        }
        settle_turn(&mut w, 0, world, author, now)?;

        let record = build_history_doc(&w, first, now)?;
        w.commit_create(record);
        history::append_record(snap, &mut w.ops);
    }
    let active_change = set_engine(&w.combat, "/engine/active", json!(true))?;
    w.commit_combat(vec![active_change])?;
    Ok(w.ops)
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
    if let Some(ops) = history::fast_forward(snap)? {
        return Ok(ops);
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
    settle_turn(&mut w, next_idx, world, author, now)?;
    history::append_record(snap, &mut w.ops);
    Ok(w.ops)
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

/// Ends a combat outright: expires every `on_combat_end`-policy effect
/// (when `effect_cleanup`), then deletes the combat document (whose cascade
/// removes its combatants/history — this never emits a child `Delete`
/// itself). Effect expiry ops always precede the `Delete`.
pub fn end(snap: &CombatSnapshot) -> Result<Vec<Operation>, CombatError> {
    let mut ops = Vec::new();
    if snap.engine.effect_cleanup {
        let refs: Vec<EffectRef> = collect_all_effects(snap)
            .into_iter()
            .map(|(_, r)| r)
            .collect();
        ops.extend(expire_by_policy(&snap.hosts, &refs, |r| r.on_combat_end)?);
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

/// Adjusts one combatant's tracked resource: `Delta` adds a signed amount,
/// `Set` overwrites outright; both clamp to `[0, max]`. `Forbidden` when the
/// requested amount/value is non-finite (never silently truncated/ignored).
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
    let res = c.engine.resources.get(key).ok_or(CombatError::NotFound)?;
    let new = match op {
        ResourceOp::Delta { amount } => {
            if !amount.is_finite() {
                return Err(CombatError::Forbidden);
            }
            (res.current + amount).clamp(0.0, res.max)
        }
        ResourceOp::Set { value } => {
            if !value.is_finite() {
                return Err(CombatError::Forbidden);
            }
            value.clamp(0.0, res.max)
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
