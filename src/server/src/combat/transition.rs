//! This module holds the pure combat-clock transitions.
//!
//! Each public transition except `rebuild_order` takes a `CombatSnapshot` and returns the
//! `Operation`s for ONE command; `rebuild_order` takes the combatant slice and returns the turn
//! order's ids. Nothing here touches a `Repository` or chooses a `WriteOrigin`, because a later
//! caller commits the returned ops. The one other thing a transition hands back is a RECORD, not an
//! effect: `start_with_trigger_plan`/`advance_with_trigger_plan` also return the turn/round
//! boundaries the walk crossed (`TurnRoundBoundary`), the firing plan the room applies
//! turn/round-scoped region triggers from. Recording stays pure, since region rows, region
//! membership and trigger-effect application never reach this module.
//!
//! `advance`/`start` share one core mechanism (`settle_turn`). Entering a combatant runs its
//! `turn_start` boundary; an `Event` combatant, or a hidden combatant under
//! `TurnControl::OwnerMayEnd`, immediately resolves and the walk continues to the next entry. The
//! step budget starts at `order.len()` and grows by exactly one for every entry the walk deletes
//! (an exhausted `Event`), rather than resetting to a fresh full-length guard on each deletion. An
//! all-auto-resolving order (e.g. every entry an infinite `Event`) therefore still terminates, in
//! a number of steps linear in `order`'s initial length, and the walk's last allowed step always
//! settles the turn wherever it lands, rather than erroring.
//!
//! INVARIANT: nothing this module does for a hidden combatant is observable from outside its own
//! document (no message, no distinguishing op shape). The SAME `recover`/`tick`/`expire_by_policy`
//! calls run for a hidden entry as for a visible one, writing only to that combatant's own
//! (permission-gated) document and its own hosts' effect documents. Every `Operation::Update` that
//! `start` and `advance` accumulate is folded, by document id, through `Working::coalesce_updates`
//! once at the end. A document written from more than one boundary/phase within the same command
//! (an auto-resolved entry's start+end pair, or more than one `resolve_event` deletion in one
//! `settle_turn` walk) therefore always reaches the caller as ONE `Update` per document, with a
//! correct cumulative OCC pre-image.

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
///
/// # Examples
///
/// ```
/// use shadowcat::combat::transition::ResourceOp;
///
/// let delta = ResourceOp::Delta { amount: -3.0 };
/// let set = ResourceOp::Set { value: 5.0 };
/// assert!(matches!(delta, ResourceOp::Delta { amount } if amount == -3.0));
/// assert!(matches!(set, ResourceOp::Set { value } if value == 5.0));
/// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::combat::transition::RollPost;
/// use shadowcat::dice::rng::NoiseRng;
/// use shadowcat::dice::{evaluate, parse, roll, ParseContext};
///
/// let spec = parse("1d20", ParseContext::default()).unwrap();
/// let mut rng = NoiseRng::from_seed(1);
/// let raw = roll(&spec, &mut rng);
/// let outcome = evaluate(&spec, &raw);
/// let post = RollPost {
///     formula: "1d20".to_string(),
///     outcome,
///     spec,
///     raw,
/// };
/// assert_eq!(post.formula, "1d20");
/// ```
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
            None,
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
pub(crate) enum Phase {
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
pub(crate) fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::TurnStart => "turn_start",
        Phase::TurnEnd => "turn_end",
        Phase::RoundStart => "round_start",
        Phase::RoundEnd => "round_end",
    }
}

/// The `data::engine::TriggerEvent` a region trigger author picks to fire at
/// `phase` — the ONE mapping between the clock's phase vocabulary and the
/// region-trigger event vocabulary, so the two are identical by construction
/// rather than by two independently-typed spellings (the serde renames on
/// `TriggerEvent` spell exactly what `phase_name` returns; the
/// `turn_round_trigger_event_serde_spellings_match_the_phase_vocabulary`
/// test pins that identity).
pub(crate) fn phase_trigger_event(phase: Phase) -> crate::data::engine::TriggerEvent {
    match phase {
        Phase::TurnStart => crate::data::engine::TriggerEvent::TurnStart,
        Phase::TurnEnd => crate::data::engine::TriggerEvent::TurnEnd,
        Phase::RoundStart => crate::data::engine::TriggerEvent::RoundStart,
        Phase::RoundEnd => crate::data::engine::TriggerEvent::RoundEnd,
    }
}

/// One turn/round boundary a transition crossed, recorded for the
/// turn/round-scoped region-trigger family (`TriggerEvent::TurnStart` & kin).
/// `combat::transition` stays pure: it RECORDS the boundaries it runs
/// (alongside the ops it already returns) and nothing more — region rows,
/// membership, and effect application all live room-side
/// (`Room::fire_turn_round_triggers`), so this module takes on no
/// trigger-effect I/O it doesn't otherwise have. The boundaries recorded are
/// exactly the ones recovery/tick ran for: a `history::fast_forward`
/// short-circuit runs none and records none, and `roll`/`sort`/`resource`/
/// `pause`/`end`/`rewind` never reach here either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TurnRoundBoundary {
    /// The boundary's event, mapped through `phase_trigger_event`.
    pub event: crate::data::engine::TriggerEvent,
    /// The combatant whose TURN boundary this is; `None` for a round-scoped
    /// boundary (`RoundStart`/`RoundEnd`), which `covered` describes instead.
    pub combatant: Option<Uuid>,
    /// Every combatant in `/engine/order` at the moment a ROUND boundary
    /// crossed (empty for turn-scoped entries) — recorded at crossing time
    /// because a later phase of the same transition (an exhausted `Event`'s
    /// deletion) can shrink `order` before the room expands the boundary, and
    /// the firing must cover exactly the combatants the boundary's own
    /// recovery sweep covered.
    pub covered: Vec<Uuid>,
    /// The round the boundary belongs to: the turn identity for turn-scoped
    /// entries (a `(round, combatant)` pair is one turn), the NEW round for a
    /// `round_wrap` crossing. This is the boundary-instance component of the
    /// room's once-per-turn/round dedup key.
    pub round: u32,
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
    /// The turn/round boundaries this transition crossed, in crossing order —
    /// the region-trigger firing plan `start_with_trigger_plan`/
    /// `advance_with_trigger_plan` hand the room alongside the ops. Recording
    /// is all this module does with region triggers; firing is room-side.
    trigger_plan: Vec<TurnRoundBoundary>,
    /// Total `settle_turn` loop iterations taken. Test-only instrumentation
    /// for asserting the step budget stays linear in `order`'s initial
    /// length (see `settle_turn`'s own doc comment) — never read outside a
    /// test build.
    #[cfg(test)]
    settle_turn_steps: usize,
}

impl Working {
    /// Builds a working copy from an immutable snapshot, then heals
    /// `/engine/order` (`heal_order`) so every subsequent phase walks a
    /// membership-consistent order — see `heal_order`'s own doc comment.
    fn from_snapshot(snap: &CombatSnapshot) -> Result<Self, CombatError> {
        let mut w = Working {
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
            trigger_plan: Vec::new(),
            #[cfg(test)]
            settle_turn_steps: 0,
        };
        w.heal_order()?;
        Ok(w)
    }

    /// Drops any `/engine/order` entry that no longer names a member of
    /// `combatants` (freshly queried by `load_snapshot`, so it always
    /// reflects CURRENT `combat`-document parentage), through
    /// `drop_departed` — the same "still a member" test `reconcile_membership`
    /// (used by `rebuild_order`'s explicit resorts) shares rather than
    /// re-derives. `order` is a second, independently stored statement of
    /// combat membership: an ordinary document `Move` can re-parent a
    /// `combatant` OUT of this combat without ever touching `order`, which
    /// is exactly the never-fork defect class this closes rather than
    /// re-checks. Left unhealed, a departed combatant's id lingers in
    /// `order` until a turn walk reaches its slot and `settle_turn`'s
    /// combatant lookup refuses with `NotFound` — permanently, since the
    /// walk never advances past the stale entry to self-correct. Routing
    /// through `drop_departed` here means EVERY transition (`start`/
    /// `advance` are the only two callers of `from_snapshot`) heals on its
    /// own next commit, rather than only the transitions that already
    /// resort `order` for their own reasons (`sort`/`roll`/a restoring
    /// `rewind`).
    ///
    /// Deliberately DROP-ONLY: it never appends a `combatants` entry `order` doesn't name. A
    /// combatant re-parented INTO this combat (or `Create`d as one of its children from the
    /// start) without an `order` entry is a genuine, permanent design state — a staged/hidden
    /// combatant a GM adds to the clock only via an explicit `CombatRoll`/`CombatSort`, pinned
    /// by an end-to-end scenario asserting such a combatant "never enters `order`/`turn`"
    /// across an entire round. Appending (as `reconcile_membership` does) would silently roll
    /// the staged combatant into the clock on every `advance` — an ARRIVAL is not itself a
    /// defect the way a stale departed id is; it is indistinguishable, from `order` alone, from
    /// a combatant the GM deliberately never rolled in, and only an explicit resort may resolve
    /// that ambiguity.
    ///
    /// A no-op (nothing recorded) when the pruned order already matches;
    /// otherwise stages the correcting `Update` through `commit_combat`,
    /// which re-derives `self.engine` from the write so the rest of this
    /// transition observes the healed order immediately.
    fn heal_order(&mut self) -> Result<(), CombatError> {
        let healed = drop_departed(&self.combatants, &self.engine.order);
        if healed != self.engine.order {
            let change = set_engine(&self.combat, "/engine/order", json!(healed))?;
            self.commit_combat(vec![change])?;
        }
        Ok(())
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
            groups: Vec::new(),
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
    /// `SqliteRepository::apply_intent`'s validation pass snapshots ONE whole-document
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

    /// Records one turn-scoped boundary `id` just crossed at the CURRENT
    /// `engine.round` (the round is read at call time, so a boundary run
    /// after `round_wrap`'s commit belongs to the new round — the correct
    /// turn identity). Round-scoped boundaries call `record_round_boundary`
    /// instead. Region-trigger firing consumes this record room-side; nothing
    /// else here reads it.
    fn record_turn_boundary(&mut self, phase: Phase, id: Uuid) {
        self.trigger_plan.push(TurnRoundBoundary {
            event: phase_trigger_event(phase),
            combatant: Some(id),
            covered: Vec::new(),
            round: self.engine.round,
        });
    }

    /// Records one round-scoped boundary covering the CURRENT `engine.order`
    /// (the same list the boundary's own recovery sweep iterates), at the
    /// CURRENT `engine.round` — `round_wrap` commits the new round BEFORE
    /// recording, so the entry belongs to the round being entered.
    fn record_round_boundary(&mut self, phase: Phase) {
        self.trigger_plan.push(TurnRoundBoundary {
            event: phase_trigger_event(phase),
            combatant: None,
            covered: self.engine.order.clone(),
            round: self.engine.round,
        });
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
            system: None,
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
    w.record_turn_boundary(Phase::TurnEnd, id);
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
    )?;
    w.record_turn_boundary(Phase::TurnStart, id);
    Ok(())
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
/// category, never once per combatant that happens to share its host. The
/// region-trigger plan mirrors that structure exactly: ONE round-scoped
/// record per boundary category (never one per combatant — the room expands
/// it across `covered`), recorded after the round commit so it belongs to
/// the round being entered.
fn round_wrap(w: &mut Working) -> Result<(), CombatError> {
    let new_round = w.engine.round + 1;
    let change = set_engine(&w.combat, "/engine/round", json!(new_round))?;
    w.commit_combat(vec![change])?;
    w.record_round_boundary(Phase::RoundEnd);
    w.record_round_boundary(Phase::RoundStart);
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
                system: None,
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
/// is an `Event`, a defeated combatant (under every `TurnControl`) or a hidden
/// combatant under `TurnControl::OwnerMayEnd`,
/// auto-resolving it in full and continuing to the next entry — every
/// auto-resolving entry gets its own `run_turn_end` (a hidden or defeated actor
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
        let (is_event, hidden, defeated) = {
            let c = w
                .combatants
                .iter()
                .find(|c| c.doc.id == entry_id)
                .ok_or(CombatError::NotFound)?;
            (
                matches!(c.engine.kind, CombatantKind::Event { .. }),
                is_hidden(c),
                c.engine.defeated,
            )
        };
        // A defeated combatant has no turn to take, so it auto-resolves under
        // every `TurnControl`; a hidden one only under `OwnerMayEnd`, where the
        // policy is about who may end a turn.
        let auto_resolves =
            is_event || defeated || (hidden && w.engine.turn_control == TurnControl::OwnerMayEnd);

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
///
/// # Examples
///
/// ```
/// use shadowcat::combat::transition::start;
/// use shadowcat::combat::{CombatSnapshot, Combatant};
/// use shadowcat::data::command::Operation;
/// use shadowcat::data::document::{Document, PermissionSet, Scope};
/// use shadowcat::data::engine::combat::{
///     CombatEngine, CombatantEngine, CombatantKind, EffectLifecycleDefaults, Enforcement,
///     Interpretation, MovementRules, TurnControl,
/// };
/// use std::collections::HashMap;
/// use uuid::Uuid;
///
/// let world_id = Uuid::new_v4();
/// let combat_id = Uuid::new_v4();
/// let event_id = Uuid::new_v4();
///
/// let event_engine = CombatantEngine {
///     kind: CombatantKind::Event {
///         lifespan: Some(2),
///         message: None,
///     },
///     initiative: None,
///     tiebreak: 0.0,
///     defeated: false,
///     group: None,
///     resources: Default::default(),
/// };
/// let event_doc = Document {
///     id: event_id,
///     scope: Scope::World { world_id },
///     doc_type: "combatant".to_string(),
///     schema_version: 1,
///     name: Some("Trap".to_string()),
///     source: None,
///     base: None,
///     owner: None,
///     permissions: PermissionSet::default(),
///     embedded: Default::default(),
///     parent_id: Some(combat_id),
///     engine: Some(serde_json::to_value(&event_engine).unwrap()),
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
///
/// let combat_engine = CombatEngine {
///     scene_id: Uuid::new_v4(),
///     active: false,
///     round: 0,
///     turn: None,
///     turn_control: TurnControl::OwnerMayEnd,
///     order: vec![event_id],
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
/// let combat_doc = Document {
///     id: combat_id,
///     scope: Scope::World { world_id },
///     doc_type: "combat".to_string(),
///     schema_version: 1,
///     name: None,
///     source: None,
///     base: None,
///     owner: None,
///     permissions: PermissionSet::default(),
///     embedded: Default::default(),
///     parent_id: None,
///     engine: Some(serde_json::to_value(&combat_engine).unwrap()),
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
///
/// let snapshot = CombatSnapshot {
///     combat: combat_doc,
///     engine: combat_engine,
///     combatants: vec![Combatant {
///         doc: event_doc,
///         engine: event_engine,
///     }],
///     hosts: HashMap::new(),
///     history: None,
///     registry: None,
///     other_active: Vec::new(),
///     groups: Vec::new(),
///     chain: (None, None, None),
/// };
///
/// let ops = start(&snapshot, 0, world_id, Uuid::new_v4()).unwrap();
/// assert!(ops
///     .iter()
///     .any(|op| matches!(op, Operation::Create { doc } if doc.doc_type == "combat-history")));
/// assert!(ops
///     .iter()
///     .any(|op| matches!(op, Operation::Update { doc_id, .. } if *doc_id == combat_id)));
/// ```
pub fn start(
    snap: &CombatSnapshot,
    now: i64,
    world: Uuid,
    author: Uuid,
) -> Result<Vec<Operation>, CombatError> {
    start_with_trigger_plan(snap, now, world, author).map(|(ops, _)| ops)
}

/// `start` plus the turn/round boundary record the transition crossed — the
/// region-trigger firing input `combat::run_intent` hands
/// `Room::fire_turn_round_triggers` after the transition's own commit. The
/// public `start` keeps its one-command-ops shape for callers that only
/// apply the ops (the boundary record is derivable from nothing else — it
/// exists only because the walk recorded it).
pub(crate) fn start_with_trigger_plan(
    snap: &CombatSnapshot,
    now: i64,
    world: Uuid,
    author: Uuid,
) -> Result<(Vec<Operation>, Vec<TurnRoundBoundary>), CombatError> {
    let mut w = Working::from_snapshot(snap)?;
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
        w.record_round_boundary(Phase::RoundStart);
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
    Ok((w.ops, w.trigger_plan))
}

/// Shared implementation for `advance`: builds the ops for ending the
/// current turn, advancing to the next combatant (wrapping the round when
/// the order is exhausted), and settling the new turn. Returns the
/// transition's turn/round boundary record (the region-trigger firing plan)
/// and the `settle_turn` step count alongside the ops so
/// `advance_with_trigger_plan` and `advance_with_step_count` (test-only) can
/// expose each without re-deriving `advance`'s own wiring separately. A
/// `history::fast_forward` short-circuit crosses no boundary, so it returns
/// an EMPTY plan — recovery/tick do not run on a fast-forward and region
/// triggers mirror that exactly.
fn advance_impl(
    snap: &CombatSnapshot,
    world: Uuid,
    author: Uuid,
    now: i64,
) -> Result<(Vec<Operation>, Vec<TurnRoundBoundary>, usize), CombatError> {
    if let Some(ops) = history::fast_forward(snap, now)? {
        return Ok((ops, Vec::new(), 0));
    }
    if !snap.engine.active {
        return Err(CombatError::NotRunning);
    }
    if snap.engine.order.is_empty() {
        return Err(CombatError::Empty);
    }
    let current_id = snap.engine.turn.ok_or(CombatError::Empty)?;
    let mut w = Working::from_snapshot(snap)?;
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
    Ok((w.ops, w.trigger_plan, steps))
}

/// Ends the current turn, advances to the next combatant (wrapping the
/// round when the order is exhausted), and settles the new turn — see the
/// module doc for the auto-resolve/termination guarantee.
///
/// # Examples
///
/// ```
/// use shadowcat::combat::transition::advance;
/// use shadowcat::combat::{CombatSnapshot, Combatant};
/// use shadowcat::data::command::Operation;
/// use shadowcat::data::document::{Document, PermissionSet, Scope};
/// use shadowcat::data::engine::combat::{
///     CombatEngine, CombatantEngine, CombatantKind, EffectLifecycleDefaults, Enforcement,
///     Interpretation, MovementRules, TurnControl,
/// };
/// use std::collections::HashMap;
/// use uuid::Uuid;
///
/// let world_id = Uuid::new_v4();
/// let combat_id = Uuid::new_v4();
/// let event_id = Uuid::new_v4();
///
/// let event_engine = CombatantEngine {
///     kind: CombatantKind::Event {
///         lifespan: Some(3),
///         message: None,
///     },
///     initiative: None,
///     tiebreak: 0.0,
///     defeated: false,
///     group: None,
///     resources: Default::default(),
/// };
/// let event_doc = Document {
///     id: event_id,
///     scope: Scope::World { world_id },
///     doc_type: "combatant".to_string(),
///     schema_version: 1,
///     name: Some("Trap".to_string()),
///     source: None,
///     base: None,
///     owner: None,
///     permissions: PermissionSet::default(),
///     embedded: Default::default(),
///     parent_id: Some(combat_id),
///     engine: Some(serde_json::to_value(&event_engine).unwrap()),
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
///
/// let combat_engine = CombatEngine {
///     scene_id: Uuid::new_v4(),
///     active: true,
///     round: 1,
///     turn: Some(event_id),
///     turn_control: TurnControl::OwnerMayEnd,
///     order: vec![event_id],
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
/// let combat_doc = Document {
///     id: combat_id,
///     scope: Scope::World { world_id },
///     doc_type: "combat".to_string(),
///     schema_version: 1,
///     name: None,
///     source: None,
///     base: None,
///     owner: None,
///     permissions: PermissionSet::default(),
///     embedded: Default::default(),
///     parent_id: None,
///     engine: Some(serde_json::to_value(&combat_engine).unwrap()),
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
///
/// let snapshot = CombatSnapshot {
///     combat: combat_doc,
///     engine: combat_engine,
///     combatants: vec![Combatant {
///         doc: event_doc,
///         engine: event_engine,
///     }],
///     hosts: HashMap::new(),
///     history: None,
///     registry: None,
///     other_active: Vec::new(),
///     groups: Vec::new(),
///     chain: (None, None, None),
/// };
///
/// let ops = advance(&snapshot, world_id, Uuid::new_v4(), 0).unwrap();
/// // Wrapping the single-entry order bumps the round and re-settles the same turn.
/// assert!(ops
///     .iter()
///     .any(|op| matches!(op, Operation::Update { doc_id, .. } if *doc_id == combat_id)));
/// ```
pub fn advance(
    snap: &CombatSnapshot,
    world: Uuid,
    author: Uuid,
    now: i64,
) -> Result<Vec<Operation>, CombatError> {
    advance_impl(snap, world, author, now).map(|(ops, _, _)| ops)
}

/// `advance` plus the turn/round boundary record the transition crossed —
/// the region-trigger firing plan `combat::run_intent` hands
/// `Room::fire_turn_round_triggers` after the transition's own commit
/// (`start_with_trigger_plan`'s advance twin).
pub(crate) fn advance_with_trigger_plan(
    snap: &CombatSnapshot,
    world: Uuid,
    author: Uuid,
    now: i64,
) -> Result<(Vec<Operation>, Vec<TurnRoundBoundary>), CombatError> {
    advance_impl(snap, world, author, now).map(|(ops, plan, _)| (ops, plan))
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
    advance_impl(snap, world, author, now).map(|(ops, _, steps)| (ops, steps))
}

/// Pauses a running combat: clears `active` only. `NotRunning` if it is not
/// currently active.
///
/// # Examples
///
/// ```
/// use shadowcat::combat::transition::pause;
/// use shadowcat::combat::CombatSnapshot;
/// use shadowcat::data::command::Operation;
/// use shadowcat::data::document::{Document, PermissionSet, Scope};
/// use shadowcat::data::engine::combat::{
///     CombatEngine, EffectLifecycleDefaults, Enforcement, Interpretation, MovementRules,
///     TurnControl,
/// };
/// use std::collections::HashMap;
/// use uuid::Uuid;
///
/// let world_id = Uuid::new_v4();
/// let combat_id = Uuid::new_v4();
/// let combat_engine = CombatEngine {
///     scene_id: Uuid::new_v4(),
///     active: true,
///     round: 1,
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
/// let combat_doc = Document {
///     id: combat_id,
///     scope: Scope::World { world_id },
///     doc_type: "combat".to_string(),
///     schema_version: 1,
///     name: None,
///     source: None,
///     base: None,
///     owner: None,
///     permissions: PermissionSet::default(),
///     embedded: Default::default(),
///     parent_id: None,
///     engine: Some(serde_json::to_value(&combat_engine).unwrap()),
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
/// let snapshot = CombatSnapshot {
///     combat: combat_doc,
///     engine: combat_engine,
///     combatants: Vec::new(),
///     hosts: HashMap::new(),
///     history: None,
///     registry: None,
///     other_active: Vec::new(),
///     groups: Vec::new(),
///     chain: (None, None, None),
/// };
///
/// let ops = pause(&snapshot).unwrap();
/// assert_eq!(ops.len(), 1);
/// match &ops[0] {
///     Operation::Update { doc_id, changes } => {
///         assert_eq!(*doc_id, combat_id);
///         assert_eq!(changes[0].new, serde_json::json!(false));
///     }
///     _ => panic!("expected an Update"),
/// }
/// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::combat::transition::rewind;
/// use shadowcat::combat::{CombatError, CombatSnapshot};
/// use shadowcat::data::document::{Document, PermissionSet, Scope};
/// use shadowcat::data::engine::combat::{
///     CombatEngine, EffectLifecycleDefaults, Enforcement, Interpretation, MovementRules,
///     TurnControl,
/// };
/// use std::collections::HashMap;
/// use uuid::Uuid;
///
/// let world_id = Uuid::new_v4();
/// let combat_engine = CombatEngine {
///     scene_id: Uuid::new_v4(),
///     active: true,
///     round: 1,
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
/// let combat_doc = Document {
///     id: Uuid::new_v4(),
///     scope: Scope::World { world_id },
///     doc_type: "combat".to_string(),
///     schema_version: 1,
///     name: None,
///     source: None,
///     base: None,
///     owner: None,
///     permissions: PermissionSet::default(),
///     embedded: Default::default(),
///     parent_id: None,
///     engine: Some(serde_json::to_value(&combat_engine).unwrap()),
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
/// // No `combat-history` document yet: rewinding refuses rather than guessing a boundary.
/// let snapshot = CombatSnapshot {
///     combat: combat_doc,
///     engine: combat_engine,
///     combatants: Vec::new(),
///     hosts: HashMap::new(),
///     history: None,
///     registry: None,
///     other_active: Vec::new(),
///     groups: Vec::new(),
///     chain: (None, None, None),
/// };
///
/// let result = rewind(&snapshot, 0);
/// assert!(matches!(result, Err(CombatError::Unrewindable)));
/// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::combat::transition::end;
/// use shadowcat::combat::CombatSnapshot;
/// use shadowcat::data::command::Operation;
/// use shadowcat::data::document::{Document, PermissionSet, Scope};
/// use shadowcat::data::engine::combat::{
///     CombatEngine, EffectLifecycleDefaults, Enforcement, Interpretation, MovementRules,
///     TurnControl,
/// };
/// use std::collections::HashMap;
/// use uuid::Uuid;
///
/// let world_id = Uuid::new_v4();
/// let combat_id = Uuid::new_v4();
/// let combat_engine = CombatEngine {
///     scene_id: Uuid::new_v4(),
///     active: true,
///     round: 1,
///     turn: None,
///     turn_control: TurnControl::OwnerMayEnd,
///     order: Vec::new(),
///     movement: MovementRules {
///         resource: None,
///         interpretation: Interpretation::PerCell,
///         enforcement: Enforcement::None,
///     },
///     // No effect cleanup: `end` skips the effect-expiry pass entirely.
///     effect_cleanup: false,
///     rewind_restore: true,
///     forward_restore: false,
///     effect_lifecycle: EffectLifecycleDefaults::default(),
/// };
/// let combat_doc = Document {
///     id: combat_id,
///     scope: Scope::World { world_id },
///     doc_type: "combat".to_string(),
///     schema_version: 1,
///     name: None,
///     source: None,
///     base: None,
///     owner: None,
///     permissions: PermissionSet::default(),
///     embedded: Default::default(),
///     parent_id: None,
///     engine: Some(serde_json::to_value(&combat_engine).unwrap()),
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
/// let snapshot = CombatSnapshot {
///     combat: combat_doc,
///     engine: combat_engine,
///     combatants: Vec::new(),
///     hosts: HashMap::new(),
///     history: None,
///     registry: None,
///     other_active: Vec::new(),
///     groups: Vec::new(),
///     chain: (None, None, None),
/// };
///
/// let ops = end(&snapshot, world_id, Uuid::new_v4(), 0).unwrap();
/// assert_eq!(ops.len(), 1);
/// assert!(matches!(&ops[0], Operation::Delete { doc } if doc.id == combat_id));
/// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::chat::MESSAGE_DOC_TYPE;
/// use shadowcat::combat::transition::roll;
/// use shadowcat::combat::{CombatSnapshot, Combatant, RollPost};
/// use shadowcat::data::command::Operation;
/// use shadowcat::data::document::{Document, PermissionSet, Scope};
/// use shadowcat::data::engine::combat::{
///     CombatEngine, CombatantEngine, CombatantKind, EffectLifecycleDefaults, Enforcement,
///     Interpretation, MovementRules, TurnControl,
/// };
/// use shadowcat::dice::rng::NoiseRng;
/// use shadowcat::dice::{evaluate, parse, roll as roll_dice, ParseContext};
/// use std::collections::HashMap;
/// use uuid::Uuid;
///
/// let world_id = Uuid::new_v4();
/// let combat_id = Uuid::new_v4();
/// let combatant_id = Uuid::new_v4();
///
/// let combatant_engine = CombatantEngine {
///     kind: CombatantKind::Event {
///         lifespan: None,
///         message: None,
///     },
///     initiative: None,
///     tiebreak: 0.0,
///     defeated: false,
///     group: None,
///     resources: Default::default(),
/// };
/// let combatant_doc = Document {
///     id: combatant_id,
///     scope: Scope::World { world_id },
///     doc_type: "combatant".to_string(),
///     schema_version: 1,
///     name: None,
///     source: None,
///     base: None,
///     owner: None,
///     permissions: PermissionSet::default(),
///     embedded: Default::default(),
///     parent_id: Some(combat_id),
///     engine: Some(serde_json::to_value(&combatant_engine).unwrap()),
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
///
/// let combat_engine = CombatEngine {
///     scene_id: Uuid::new_v4(),
///     active: true,
///     round: 1,
///     turn: None,
///     turn_control: TurnControl::OwnerMayEnd,
///     order: vec![combatant_id],
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
/// let combat_doc = Document {
///     id: combat_id,
///     scope: Scope::World { world_id },
///     doc_type: "combat".to_string(),
///     schema_version: 1,
///     name: None,
///     source: None,
///     base: None,
///     owner: None,
///     permissions: PermissionSet::default(),
///     embedded: Default::default(),
///     parent_id: None,
///     engine: Some(serde_json::to_value(&combat_engine).unwrap()),
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
///
/// let snapshot = CombatSnapshot {
///     combat: combat_doc,
///     engine: combat_engine,
///     combatants: vec![Combatant {
///         doc: combatant_doc,
///         engine: combatant_engine,
///     }],
///     hosts: HashMap::new(),
///     history: None,
///     registry: None,
///     other_active: Vec::new(),
///     groups: Vec::new(),
///     chain: (None, None, None),
/// };
///
/// let spec = parse("1d20", ParseContext::default()).unwrap();
/// let mut rng = NoiseRng::from_seed(1);
/// let raw = roll_dice(&spec, &mut rng);
/// let outcome = evaluate(&spec, &raw);
/// let post = RollPost {
///     formula: "1d20".to_string(),
///     outcome,
///     spec,
///     raw,
/// };
///
/// let ops = roll(
///     &snapshot,
///     &[(combatant_id, post)],
///     world_id,
///     Uuid::new_v4(),
///     "combat",
///     0,
/// )
/// .unwrap();
/// assert!(ops
///     .iter()
///     .any(|op| matches!(op, Operation::Update { doc_id, .. } if *doc_id == combatant_id)));
/// assert!(ops
///     .iter()
///     .any(|op| matches!(op, Operation::Create { doc } if doc.doc_type == MESSAGE_DOC_TYPE)));
/// ```
pub fn roll(
    snap: &CombatSnapshot,
    results: &[(Uuid, RollPost)],
    world: Uuid,
    author: Uuid,
    channel: &str,
    now: i64,
) -> Result<Vec<Operation>, CombatError> {
    roll_with_groups(snap, results, &[], world, author, channel, now)
}

/// `roll` plus group rolls: each `group_results` entry names a
/// `combatant-group` document and ONE `RollPost`, which is stamped as the
/// initiative of EVERY member of that group (one dice result, never N rolls
/// forced equal) and posted as ONE chat message for the whole group, not one
/// per member. `results` entries behave exactly as in `roll`. A group the
/// snapshot does not hold, or one with no members, is `NotFound`.
///
/// INVARIANT: across `results` and every group's members, each combatant id
/// appears at most once; the wire-dispatch layer (`build_ops`) enforces it, as
/// it does for `roll`. The group's message is `Audience::GmOnly` when any
/// member is hidden (a public message would disclose the hidden member's
/// initiative), and carries no actor owner.
///
/// # Examples
///
/// ```
/// use shadowcat::combat::transition::{roll_with_groups, RollPost};
/// use shadowcat::combat::{CombatSnapshot, Combatant, CombatantGroup};
/// use shadowcat::data::command::Operation;
/// use shadowcat::data::document::{Document, PermissionSet, Scope};
/// use shadowcat::data::engine::combat::{
///     CombatEngine, CombatantEngine, CombatantGroupEngine, CombatantKind,
///     EffectLifecycleDefaults, Enforcement, Interpretation, MovementRules, TurnControl,
/// };
/// use shadowcat::dice::{evaluate, parse, roll_dice, NoiseRng, ParseContext};
/// use std::collections::HashMap;
/// use uuid::Uuid;
///
/// let world_id = Uuid::new_v4();
/// let combat_id = Uuid::new_v4();
/// let group_id = Uuid::new_v4();
/// let doc = |id: Uuid, doc_type: &str, engine: serde_json::Value| Document {
///     id,
///     scope: Scope::World { world_id },
///     doc_type: doc_type.to_string(),
///     schema_version: 1,
///     name: None,
///     source: None,
///     base: None,
///     owner: None,
///     permissions: PermissionSet::default(),
///     embedded: Default::default(),
///     parent_id: Some(combat_id),
///     engine: Some(engine),
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
/// let member = |id: Uuid| {
///     let engine = CombatantEngine {
///         kind: CombatantKind::Event { lifespan: None, message: None },
///         initiative: None,
///         tiebreak: 0.0,
///         defeated: false,
///         group: Some(group_id),
///         resources: Default::default(),
///     };
///     Combatant {
///         doc: doc(id, "combatant", serde_json::to_value(&engine).unwrap()),
///         engine,
///     }
/// };
/// let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
/// let combat_engine = CombatEngine {
///     scene_id: Uuid::new_v4(),
///     active: false,
///     round: 0,
///     turn: None,
///     turn_control: TurnControl::OwnerMayEnd,
///     order: vec![a, b],
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
/// let mut combat = doc(combat_id, "combat", serde_json::to_value(&combat_engine).unwrap());
/// combat.parent_id = None;
/// let group_engine = CombatantGroupEngine { name: "Rats".to_string(), color: None };
/// let snapshot = CombatSnapshot {
///     combat,
///     engine: combat_engine,
///     combatants: vec![member(a), member(b)],
///     groups: vec![CombatantGroup {
///         doc: doc(group_id, "combatant-group", serde_json::to_value(&group_engine).unwrap()),
///         engine: group_engine,
///     }],
///     hosts: HashMap::new(),
///     history: None,
///     registry: None,
///     other_active: Vec::new(),
///     chain: (None, None, None),
/// };
///
/// let spec = parse("1d20", ParseContext::default()).unwrap();
/// let mut rng = NoiseRng::from_seed(1);
/// let raw = roll_dice(&spec, &mut rng);
/// let outcome = evaluate(&spec, &raw);
/// let post = RollPost { formula: "1d20".to_string(), outcome, spec, raw };
///
/// let ops = roll_with_groups(&snapshot, &[], &[(group_id, post)], world_id, Uuid::new_v4(), "combat", 0)
///     .unwrap();
/// let creates = ops.iter().filter(|op| matches!(op, Operation::Create { .. })).count();
/// let member_updates = ops
///     .iter()
///     .filter(|op| matches!(op, Operation::Update { doc_id, .. } if *doc_id == a || *doc_id == b))
///     .count();
/// assert_eq!(creates, 1);
/// assert_eq!(member_updates, 2);
/// ```
pub fn roll_with_groups(
    snap: &CombatSnapshot,
    results: &[(Uuid, RollPost)],
    group_results: &[(Uuid, RollPost)],
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
    // Every (combatant, total) this call stamps, solo and group members alike,
    // for the re-sort below.
    let mut stamped: Vec<(Uuid, f64)> = results
        .iter()
        .map(|(id, post)| (*id, post.outcome.total as f64))
        .collect();
    for (group_id, post) in group_results {
        let group = snap
            .groups
            .iter()
            .find(|g| g.doc.id == *group_id)
            .ok_or(CombatError::NotFound)?;
        let members: Vec<&Combatant> = snap
            .combatants
            .iter()
            .filter(|c| c.engine.group == Some(*group_id))
            .collect();
        if members.is_empty() {
            return Err(CombatError::NotFound);
        }
        for c in &members {
            let change = set_engine(
                &c.doc,
                "/engine/initiative",
                json!(post.outcome.total as f64),
            )?;
            ops.push(Operation::Update {
                doc_id: c.doc.id,
                changes: vec![change],
            });
            stamped.push((c.doc.id, post.outcome.total as f64));
        }
        let msg = build_message_doc(
            world,
            author,
            MessageDraft {
                channel: channel.to_string(),
                actor_owner: None,
                audience: if members.iter().any(|c| is_hidden(c)) {
                    Audience::GmOnly
                } else {
                    Audience::Public
                },
                kind: MessageKind::Roll,
                content: vec![
                    Segment::Text {
                        text: group.engine.name.clone(),
                    },
                    Segment::RollEmbed {
                        formula: post.formula.clone(),
                        outcome: post.outcome.clone(),
                        roll_id: Uuid::new_v4(),
                        spec: Some(Box::new(post.spec.clone())),
                        raw: Some(Box::new(post.raw.clone())),
                        recalc_history: None,
                    },
                ],
                source: None,
                system: None,
            },
            now,
        );
        ops.push(Operation::Create { doc: msg });
    }
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
                system: None,
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
            if let Some((_, total)) = stamped.iter().find(|(id, _)| *id == c.doc.id) {
                engine.initiative = Some(*total);
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
///
/// # Examples
///
/// ```
/// use shadowcat::combat::transition::{resource, ResourceOp};
/// use shadowcat::combat::{CombatSnapshot, Combatant};
/// use shadowcat::data::command::Operation;
/// use shadowcat::data::document::{Document, PermissionSet, Scope};
/// use shadowcat::data::engine::combat::{
///     CombatEngine, CombatantEngine, CombatantKind, EffectLifecycleDefaults, Enforcement,
///     Formula, Interpretation, MovementRules, Recovery, Resource, ResourceBinding,
///     ResourceRegistryEngine, TurnControl,
/// };
/// use std::collections::{BTreeMap, HashMap};
/// use uuid::Uuid;
///
/// let world_id = Uuid::new_v4();
/// let combat_id = Uuid::new_v4();
/// let combatant_id = Uuid::new_v4();
///
/// let combatant_engine = CombatantEngine {
///     kind: CombatantKind::Event {
///         lifespan: None,
///         message: None,
///     },
///     initiative: None,
///     tiebreak: 0.0,
///     defeated: false,
///     group: None,
///     // No stored entry for "hp" yet: an absent entry reads as full.
///     resources: BTreeMap::new(),
/// };
/// let combatant_doc = Document {
///     id: combatant_id,
///     scope: Scope::World { world_id },
///     doc_type: "combatant".to_string(),
///     schema_version: 1,
///     name: None,
///     source: None,
///     base: None,
///     owner: None,
///     permissions: PermissionSet::default(),
///     embedded: Default::default(),
///     parent_id: Some(combat_id),
///     engine: Some(serde_json::to_value(&combatant_engine).unwrap()),
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
///
/// let mut resources = BTreeMap::new();
/// resources.insert(
///     "hp".to_string(),
///     Resource {
///         name: "HP".to_string(),
///         order: 0,
///         binding: ResourceBinding::Tracked {
///             max: Formula::Number(10.0),
///             recover: Recovery::default(),
///         },
///     },
/// );
/// let registry = ResourceRegistryEngine { resources };
///
/// let combat_engine = CombatEngine {
///     scene_id: Uuid::new_v4(),
///     active: true,
///     round: 1,
///     turn: None,
///     turn_control: TurnControl::OwnerMayEnd,
///     order: vec![combatant_id],
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
/// let combat_doc = Document {
///     id: combat_id,
///     scope: Scope::World { world_id },
///     doc_type: "combat".to_string(),
///     schema_version: 1,
///     name: None,
///     source: None,
///     base: None,
///     owner: None,
///     permissions: PermissionSet::default(),
///     embedded: Default::default(),
///     parent_id: None,
///     engine: Some(serde_json::to_value(&combat_engine).unwrap()),
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
///
/// let snapshot = CombatSnapshot {
///     combat: combat_doc,
///     engine: combat_engine,
///     combatants: vec![Combatant {
///         doc: combatant_doc,
///         engine: combatant_engine,
///     }],
///     hosts: HashMap::new(),
///     history: None,
///     registry: Some(registry),
///     other_active: Vec::new(),
///     groups: Vec::new(),
///     chain: (None, None, None),
/// };
///
/// let ops = resource(&snapshot, combatant_id, "hp", ResourceOp::Delta { amount: -3.0 }).unwrap();
/// assert_eq!(ops.len(), 1);
/// match &ops[0] {
///     Operation::Update { doc_id, changes } => {
///         assert_eq!(*doc_id, combatant_id);
///         // Absent entry reads as full (max 10.0), so a -3.0 delta lands at 7.0.
///         assert_eq!(changes[0].new, serde_json::json!(7.0));
///     }
///     _ => panic!("expected an Update"),
/// }
/// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::combat::transition::sort;
/// use shadowcat::combat::{CombatSnapshot, Combatant};
/// use shadowcat::data::command::Operation;
/// use shadowcat::data::document::{Document, PermissionSet, Scope};
/// use shadowcat::data::engine::combat::{
///     CombatEngine, CombatantEngine, CombatantKind, EffectLifecycleDefaults, Enforcement,
///     Interpretation, MovementRules, TurnControl,
/// };
/// use std::collections::HashMap;
/// use uuid::Uuid;
///
/// let world_id = Uuid::new_v4();
/// let combat_id = Uuid::new_v4();
/// let low_id = Uuid::new_v4();
/// let high_id = Uuid::new_v4();
///
/// let build_combatant = |id: Uuid, initiative: f64| {
///     let engine = CombatantEngine {
///         kind: CombatantKind::Event {
///             lifespan: None,
///             message: None,
///         },
///         initiative: Some(initiative),
///         tiebreak: 0.0,
///         defeated: false,
///         group: None,
///         resources: Default::default(),
///     };
///     let doc = Document {
///         id,
///         scope: Scope::World { world_id },
///         doc_type: "combatant".to_string(),
///         schema_version: 1,
///         name: None,
///         source: None,
///         base: None,
///         owner: None,
///         permissions: PermissionSet::default(),
///         embedded: Default::default(),
///         parent_id: Some(combat_id),
///         engine: Some(serde_json::to_value(&engine).unwrap()),
///         system: serde_json::json!({}),
///         created_at: 0,
///         updated_at: 0,
///     };
///     Combatant { doc, engine }
/// };
/// let low = build_combatant(low_id, 5.0);
/// let high = build_combatant(high_id, 15.0);
///
/// let combat_engine = CombatEngine {
///     scene_id: Uuid::new_v4(),
///     active: true,
///     round: 1,
///     turn: None,
///     turn_control: TurnControl::OwnerMayEnd,
///     // Stored in ASCENDING initiative order — `sort` re-orders descending.
///     order: vec![low_id, high_id],
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
/// let combat_doc = Document {
///     id: combat_id,
///     scope: Scope::World { world_id },
///     doc_type: "combat".to_string(),
///     schema_version: 1,
///     name: None,
///     source: None,
///     base: None,
///     owner: None,
///     permissions: PermissionSet::default(),
///     embedded: Default::default(),
///     parent_id: None,
///     engine: Some(serde_json::to_value(&combat_engine).unwrap()),
///     system: serde_json::json!({}),
///     created_at: 0,
///     updated_at: 0,
/// };
///
/// let snapshot = CombatSnapshot {
///     combat: combat_doc,
///     engine: combat_engine,
///     combatants: vec![low, high],
///     hosts: HashMap::new(),
///     history: None,
///     registry: None,
///     other_active: Vec::new(),
///     groups: Vec::new(),
///     chain: (None, None, None),
/// };
///
/// let ops = sort(&snapshot).unwrap();
/// assert_eq!(ops.len(), 1);
/// match &ops[0] {
///     Operation::Update { doc_id, changes } => {
///         assert_eq!(*doc_id, combat_id);
///         assert_eq!(changes[0].new, serde_json::json!([high_id, low_id]));
///     }
///     _ => panic!("expected an Update"),
/// }
/// ```
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

/// Ids from `existing` that still name a member of `combatants`, in
/// `existing`'s own relative order — the "still a member" test both
/// `reconcile_membership` (below) and `heal_order` share rather than each
/// re-deriving it, so the two can never disagree about who has departed.
fn drop_departed(combatants: &[Combatant], existing: &[Uuid]) -> Vec<Uuid> {
    existing
        .iter()
        .copied()
        .filter(|id| combatants.iter().any(|c| c.doc.id == *id))
        .collect()
}

/// The membership half of `rebuild_order`: `drop_departed`, then any
/// combatant in `combatants` absent from `existing` is appended, in
/// `combatants`' own order. Neither list's ORDER among survivors or
/// arrivals is otherwise touched — sorting by initiative is
/// `rebuild_order`'s own, separate, explicit-opt-in step. NOT used by
/// `heal_order`, which drops departures but deliberately never appends an
/// arrival — see that function's own doc comment for why appending here
/// would be wrong for the unconditional per-transition healing path.
fn reconcile_membership(combatants: &[Combatant], existing: &[Uuid]) -> Vec<Uuid> {
    let mut ids = drop_departed(combatants, existing);
    for c in combatants {
        if !ids.contains(&c.doc.id) {
            ids.push(c.doc.id);
        }
    }
    ids
}

/// Reconciles membership (`reconcile_membership`), then stable-sorts by
/// initiative descending (`None` last), then `tiebreak` descending, then —
/// for a full tie — the reconciled list's own relative order (the sort's
/// stability). Every caller of this function is an EXPLICIT resort
/// (`sort`, `roll` after assigning fresh initiatives, a restoring
/// `rewind`) — see `heal_order` for the membership-only reconciliation
/// every transition applies unconditionally.
///
/// # Examples
///
/// ```
/// use shadowcat::combat::transition::rebuild_order;
/// use shadowcat::combat::Combatant;
/// use shadowcat::data::document::{Document, PermissionSet, Scope};
/// use shadowcat::data::engine::combat::{CombatantEngine, CombatantKind};
/// use uuid::Uuid;
///
/// let world_id = Uuid::new_v4();
/// let combat_id = Uuid::new_v4();
/// let build = |id: Uuid, initiative: f64| {
///     let engine = CombatantEngine {
///         kind: CombatantKind::Event {
///             lifespan: None,
///             message: None,
///         },
///         initiative: Some(initiative),
///         tiebreak: 0.0,
///         defeated: false,
///         group: None,
///         resources: Default::default(),
///     };
///     let doc = Document {
///         id,
///         scope: Scope::World { world_id },
///         doc_type: "combatant".to_string(),
///         schema_version: 1,
///         name: None,
///         source: None,
///         base: None,
///         owner: None,
///         permissions: PermissionSet::default(),
///         embedded: Default::default(),
///         parent_id: Some(combat_id),
///         engine: Some(serde_json::to_value(&engine).unwrap()),
///         system: serde_json::json!({}),
///         created_at: 0,
///         updated_at: 0,
///     };
///     Combatant { doc, engine }
/// };
/// let low_id = Uuid::new_v4();
/// let high_id = Uuid::new_v4();
/// let combatants = vec![build(low_id, 5.0), build(high_id, 15.0)];
///
/// // `existing` is empty: both combatants are new arrivals, appended in the
/// // input's own order before the descending-initiative sort runs.
/// let order = rebuild_order(&combatants, &[]);
/// assert_eq!(order, vec![high_id, low_id]);
/// ```
pub fn rebuild_order(combatants: &[Combatant], existing: &[Uuid]) -> Vec<Uuid> {
    let mut ids = reconcile_membership(combatants, existing);
    let lookup = |id: &Uuid| combatants.iter().find(|c| c.doc.id == *id);

    // A group's members share ONE sort slot: the group's initiative is its
    // members' highest, its tiebreak likewise, and a full tie between two
    // slots resolves by the slot's first-appearing member's position in the
    // reconciled list. Every member of a group therefore carries an identical
    // sort key apart from the stable sort's own tie order, which keeps them
    // contiguous even when a member's own initiative has drifted from its
    // siblings'. An ungrouped combatant is its own slot, so its key is its own
    // initiative/tiebreak/position and ordering for combats without groups is
    // the plain stable initiative-then-tiebreak sort.
    let mut slots: HashMap<Uuid, GroupSlot> = HashMap::new();
    for (pos, id) in ids.iter().enumerate() {
        let Some(group) = lookup(id).and_then(|c| c.engine.group) else {
            continue;
        };
        let c = lookup(id).expect("id came from the combatants list");
        let slot = slots.entry(group).or_insert(GroupSlot {
            initiative: None,
            tiebreak: c.engine.tiebreak,
            first_pos: pos,
        });
        slot.initiative = match (slot.initiative, c.engine.initiative) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        slot.tiebreak = slot.tiebreak.max(c.engine.tiebreak);
    }
    let positions: HashMap<Uuid, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let key = |c: &Combatant| match c.engine.group.and_then(|g| slots.get(&g)) {
        Some(slot) => (slot.initiative, slot.tiebreak, slot.first_pos),
        None => (
            c.engine.initiative,
            c.engine.tiebreak,
            positions.get(&c.doc.id).copied().unwrap_or(usize::MAX),
        ),
    };
    ids.sort_by(|x, y| {
        let (Some(cx), Some(cy)) = (lookup(x), lookup(y)) else {
            return std::cmp::Ordering::Equal;
        };
        let (ix, tx, px) = key(cx);
        let (iy, ty, py) = key(cy);
        let init_cmp = match (ix, iy) {
            (Some(vx), Some(vy)) => vy.partial_cmp(&vx).unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        };
        init_cmp
            .then_with(|| ty.partial_cmp(&tx).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| px.cmp(&py))
    });
    ids
}

/// The sort key a combatant group contributes to `rebuild_order`, derived from
/// its members: the highest member initiative, the highest member tiebreak,
/// and the reconciled-list position of its first member.
struct GroupSlot {
    /// Highest member initiative, `None` while no member has rolled.
    initiative: Option<f64>,
    /// Highest member tiebreak.
    tiebreak: f64,
    /// Position of the group's first member in the reconciled id list.
    first_pos: usize,
}
