//! Engine bands for the combat clock: `combat` (a world document bound to a
//! scene), `combatant` (a child document of a combat), `resource-registry`
//! (the singleton turn-resource definitions), `effect` (clock-bound
//! expiry) and `combat-history` (the per-turn snapshot log of one combat).
//! A `Formula::Text` is engine-grammar source the server evaluates through
//! `crate::formula`; ingress requires it to parse. (The combat transitions
//! that consume the evaluator are wired by the combat-resolution milestone;
//! until then they still skip an unresolved value.)

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// How a movement budget converts into grid cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "snake_case")]
pub enum Interpretation {
    /// Budget is a distance: cells = budget / `GridDistance.per_cell`.
    #[default]
    PerCell,
    /// Budget is already a cell count.
    Spaces,
}

/// How the executor treats a move that exceeds the turn owner's budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "snake_case")]
pub enum Enforcement {
    /// Budget is tracked but never gates a move.
    #[default]
    None,
    /// Budget never gates a move; the route preview shows the overage.
    Warn,
    /// The walk truncates at the last affordable step; GMs are exempt.
    Hard,
}

/// Who may end the current turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "snake_case")]
pub enum TurnControl {
    /// The GM, or the owner of the current combatant (their own turn only).
    #[default]
    OwnerMayEnd,
    /// The GM only.
    GmOnly,
}

/// The movement rules a running combat snapshots at start (mirrors the
/// client's `MovementRules`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct MovementRules {
    /// The `resource-registry` key that is the movement budget; `None` = no
    /// budget tracking at all (the engine default).
    pub resource: Option<String>,
    /// Budget → cells conversion.
    pub interpretation: Interpretation,
    /// Gate policy.
    pub enforcement: Enforcement,
}

/// Field-by-field overrides of the combat rules, carried on world-settings
/// (world layer) and on a scene (scene layer). Every field is optional and
/// `null`/absent both mean "unset — fall through". `movement_resource` is
/// doubly optional: `Some(None)` explicitly CLEARS an inherited resource,
/// `None` inherits.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct CombatDefaults {
    /// Override of `MovementRules.resource`; outer `None` inherits, inner
    /// `None` clears.
    #[serde(
        default,
        deserialize_with = "deserialize_double_option",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(optional, type = "string | null")]
    pub movement_resource: Option<Option<String>>,
    /// Override of `MovementRules.interpretation`.
    #[ts(optional = nullable)]
    pub interpretation: Option<Interpretation>,
    /// Override of `MovementRules.enforcement`.
    #[ts(optional = nullable)]
    pub enforcement: Option<Enforcement>,
    /// Override of `CombatEngine.turn_control`.
    #[ts(optional = nullable)]
    pub turn_control: Option<TurnControl>,
    /// Override of `CombatEngine.effect_cleanup`.
    #[ts(optional = nullable)]
    pub effect_cleanup: Option<bool>,
    /// Override of `CombatEngine.effect_lifecycle`.
    #[ts(optional = nullable)]
    pub effect_lifecycle: Option<EffectLifecycleDefaults>,
    /// Override of `CombatEngine.rewind_restore`.
    #[ts(optional = nullable)]
    pub rewind_restore: Option<bool>,
    /// Override of `CombatEngine.forward_restore`.
    #[ts(optional = nullable)]
    pub forward_restore: Option<bool>,
}

impl CombatDefaults {
    /// Every lifecycle formula present parses. `at` names the field this
    /// override sits under in the enclosing document, for the error message.
    pub(crate) fn validate(&self, at: &str) -> Result<(), String> {
        match &self.effect_lifecycle {
            Some(l) => l.validate(&format!("{at}.effect_lifecycle")),
            None => Ok(()),
        }
    }
}

/// serde reads a missing key as `None` and an explicit `null` as `Some(None)`
/// for a doubly-optional field only with this helper; without it both
/// collapse to `None` and a scene could never CLEAR an inherited resource.
fn deserialize_double_option<'de, D>(d: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<String>::deserialize(d).map(Some)
}

/// The rules a `combat` document snapshots at start: the resolved
/// engine literal < system-defaults < world < scene chain.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedCombatRules {
    /// Resolved movement rules.
    pub movement: MovementRules,
    /// Resolved turn control.
    pub turn_control: TurnControl,
    /// Resolved `CombatEngine.effect_cleanup`.
    pub effect_cleanup: bool,
    /// Resolved `CombatEngine.rewind_restore`.
    pub rewind_restore: bool,
    /// Resolved `CombatEngine.forward_restore`.
    pub forward_restore: bool,
    /// Resolved `CombatEngine.effect_lifecycle`.
    pub effect_lifecycle: EffectLifecycleDefaults,
}

/// Resolve the combat rules for a scene: scene overrides beat world overrides
/// beat system-defaults overrides beat the engine fallback (no resource,
/// `PerCell`, `None`, `OwnerMayEnd`). Pure; the SOLE resolver of this chain —
/// the combat-start transition reads it and nothing re-derives the
/// precedence elsewhere.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::combat::{resolve_combat_rules, Enforcement};
///
/// let r = resolve_combat_rules(None, None, None);
/// assert_eq!(r.movement.enforcement, Enforcement::None);
/// assert!(r.movement.resource.is_none());
/// ```
pub fn resolve_combat_rules(
    system: Option<&CombatDefaults>,
    world: Option<&CombatDefaults>,
    scene: Option<&CombatDefaults>,
) -> ResolvedCombatRules {
    let pick = |f: fn(&CombatDefaults) -> Option<Option<String>>| {
        scene
            .and_then(f)
            .or_else(|| world.and_then(f))
            .or_else(|| system.and_then(f))
            .unwrap_or(None)
    };
    let resource = pick(|d| d.movement_resource.clone());
    let interpretation = scene
        .and_then(|d| d.interpretation)
        .or_else(|| world.and_then(|d| d.interpretation))
        .or_else(|| system.and_then(|d| d.interpretation))
        .unwrap_or_default();
    let enforcement = scene
        .and_then(|d| d.enforcement)
        .or_else(|| world.and_then(|d| d.enforcement))
        .or_else(|| system.and_then(|d| d.enforcement))
        .unwrap_or_default();
    let turn_control = scene
        .and_then(|d| d.turn_control)
        .or_else(|| world.and_then(|d| d.turn_control))
        .or_else(|| system.and_then(|d| d.turn_control))
        .unwrap_or_default();
    let effect_cleanup = scene
        .and_then(|d| d.effect_cleanup)
        .or_else(|| world.and_then(|d| d.effect_cleanup))
        .or_else(|| system.and_then(|d| d.effect_cleanup))
        .unwrap_or(true);
    let rewind_restore = scene
        .and_then(|d| d.rewind_restore)
        .or_else(|| world.and_then(|d| d.rewind_restore))
        .or_else(|| system.and_then(|d| d.rewind_restore))
        .unwrap_or(true);
    let forward_restore = scene
        .and_then(|d| d.forward_restore)
        .or_else(|| world.and_then(|d| d.forward_restore))
        .or_else(|| system.and_then(|d| d.forward_restore))
        .unwrap_or(false);
    let lifecycle_field = |f: fn(&EffectLifecycleDefaults) -> Option<Formula>| {
        scene
            .and_then(|d| d.effect_lifecycle.as_ref())
            .and_then(f)
            .or_else(|| world.and_then(|d| d.effect_lifecycle.as_ref()).and_then(f))
            .or_else(|| system.and_then(|d| d.effect_lifecycle.as_ref()).and_then(f))
    };
    let effect_lifecycle = EffectLifecycleDefaults {
        on_combat_end: lifecycle_field(|l| l.on_combat_end.clone()),
        on_turn_end: lifecycle_field(|l| l.on_turn_end.clone()),
        on_advance: lifecycle_field(|l| l.on_advance.clone()),
    };
    ResolvedCombatRules {
        movement: MovementRules {
            resource,
            interpretation,
            enforcement,
        },
        turn_control,
        effect_cleanup,
        rewind_restore,
        forward_restore,
        effect_lifecycle,
    }
}

/// The engine body of a `combat` document (mirrors the client's
/// `CombatEngine`). World-level, bound to one scene; at most one combat per
/// scene is `active` at a time, enforced at the `apply_intent` Create/Update
/// chokepoints via `SqliteRepository::active_combat_owner`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct CombatEngine {
    /// The scene this combat runs on.
    pub scene_id: Uuid,
    /// Whether this combat is the scene's running combat.
    pub active: bool,
    /// Current round; `0` = created, not started.
    pub round: u32,
    /// The current combatant's document id, or `None` when not running.
    pub turn: Option<Uuid>,
    /// Who may end a turn (snapshot of the resolved chain at start).
    pub turn_control: TurnControl,
    /// Turn order: combatant document ids. The single authority on sequence —
    /// nothing re-derives it from `CombatantEngine.initiative` at read time.
    pub order: Vec<Uuid>,
    /// Movement rules (snapshot of the resolved chain at start).
    pub movement: MovementRules,
    /// Whether an effect's evaluated lifecycle flags (`on_combat_end`/
    /// `on_turn_end`, resolved per boundary through the combat-defaults
    /// chain) actually expire it (snapshot of the resolved chain at start).
    pub effect_cleanup: bool,
    /// Whether ending combat/rewinding restores documents an anchored `TurnRecord` snapshot
    /// (snapshot of the resolved chain at start).
    pub rewind_restore: bool,
    /// Whether advancing PAST the current cursor (redo) restores documents from history
    /// instead of re-deriving them (snapshot of the resolved chain at start).
    pub forward_restore: bool,
    /// Default lifecycle formulas new effects inherit when authored ones are absent
    /// (snapshot of the resolved chain at start).
    pub effect_lifecycle: EffectLifecycleDefaults,
}

impl CombatEngine {
    /// `order` has no duplicate ids, `turn`, when set, is one of them, and
    /// every snapshotted lifecycle formula parses.
    pub(crate) fn validate(&self) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        for id in &self.order {
            if !seen.insert(*id) {
                return Err(format!("order contains {id} twice"));
            }
        }
        if let Some(t) = self.turn {
            if !seen.contains(&t) {
                return Err(format!("turn {t} is not in order"));
            }
        }
        self.effect_lifecycle.validate("effect_lifecycle")
    }
}

/// What a combatant is: a token/actor that acts, or a named event in the
/// order. Internally tagged on `type`, so serde cannot `deny_unknown_fields`
/// here; `normalize_engine`'s re-serialization drops any unknown key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CombatantKind {
    /// A token and/or actor; at least one id is required.
    Actor {
        /// The token on the combat's scene, if any.
        token_id: Option<Uuid>,
        /// The actor, if any (a token's `actor_id` resolves the same actor).
        actor_id: Option<Uuid>,
    },
    /// A named entry with no owner; its turn starts and ends in one command.
    Event {
        /// Remaining turns before the entry removes itself; `None` = infinite.
        lifespan: Option<u32>,
        /// Chat message posted when the event's turn starts, if any.
        message: Option<String>,
    },
}

/// A combatant's stored state for one `Tracked` registry resource (mirrors
/// the client's `CombatantResource`). Server-written: spends, recoveries and
/// resource intents move it, clamped to the binding's evaluated `max`. An
/// ABSENT entry means untouched — the server reads it as full (`current`
/// equal to the evaluated `max`) and materializes the entry on first change.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct CombatantResource {
    /// Current value.
    pub current: f64,
}

/// The engine body of a `combatant` document (mirrors the client's
/// `CombatantEngine`). Always a child of a `combat` (`parent_id`); hidden
/// combatants are simply unreadable documents (`permissions.default: none`),
/// so this band carries nothing a non-GM must not see.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct CombatantEngine {
    /// Actor-or-event discriminator with its payload.
    pub kind: CombatantKind,
    /// Rolled or hand-set initiative; `None` = unrolled.
    pub initiative: Option<f64>,
    /// Secondary ordering key (higher first); `0` when unused.
    pub tiebreak: f64,
    /// Per-resource numbers keyed by `resource-registry` id.
    pub resources: BTreeMap<String, CombatantResource>,
}

impl CombatantEngine {
    /// An actor kind names a token or an actor; every number is finite.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if let CombatantKind::Actor {
            token_id: None,
            actor_id: None,
        } = self.kind
        {
            return Err("actor combatant needs a token_id or an actor_id".into());
        }
        if let Some(i) = self.initiative {
            if !i.is_finite() {
                return Err("initiative must be finite".into());
            }
        }
        if !self.tiebreak.is_finite() {
            return Err("tiebreak must be finite".into());
        }
        for (key, r) in &self.resources {
            if !r.current.is_finite() {
                return Err(format!("resource {key} must be finite"));
            }
        }
        Ok(())
    }
}

/// A number or a formula source. Untagged on the wire (`30` or `"speed"`).
/// `Text` is `crate::formula` source: parsed at ingress by `validate`, and
/// evaluated server-side through the same module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(untagged)]
pub enum Formula {
    /// A literal.
    Number(f64),
    /// Formula source in the engine's expression grammar.
    Text(String),
}

impl Formula {
    /// Finite when a number; parses (within the formula caps) when text.
    fn validate(&self, at: &str) -> Result<(), String> {
        match self {
            Formula::Number(n) if !n.is_finite() => Err(format!("{at} must be finite")),
            Formula::Number(_) => Ok(()),
            Formula::Text(t) => crate::formula::parse(t)
                .map(|_| ())
                .map_err(|e| format!("{at}: {}", e.detail)),
        }
    }
}

/// Amounts a tracked resource recovers at each clock boundary; each defaults
/// to `0`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, default)]
pub struct Recovery {
    /// Added when the combatant's turn starts.
    pub turn_start: Formula,
    /// Added when the combatant's turn ends.
    pub turn_end: Formula,
    /// Added when a round starts.
    pub round_start: Formula,
    /// Added when a round ends.
    pub round_end: Formula,
}

impl Default for Recovery {
    fn default() -> Self {
        Recovery {
            turn_start: Formula::Number(0.0),
            turn_end: Formula::Number(0.0),
            round_start: Formula::Number(0.0),
            round_end: Formula::Number(0.0),
        }
    }
}

/// How a resource's value relates to the combatant's actor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResourceBinding {
    /// Continuously equals `value` evaluated over the combatant's formula
    /// host (e.g. HP). Derived at each use, never stored on the combatant.
    Mirror {
        /// Formula for both `current` and `max`.
        value: Formula,
    },
    /// Combat state moved by recoveries and spends; an absent combatant
    /// entry reads as full (`current` equal to the evaluated `max`).
    Tracked {
        /// Formula for `max`.
        max: Formula,
        /// Per-boundary recoveries.
        recover: Recovery,
    },
}

/// One turn-resource definition (mirrors the client's `Resource`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Resource {
    /// Display name.
    pub name: String,
    /// Display order (presentation only).
    pub order: u32,
    /// Mirror or tracked.
    pub binding: ResourceBinding,
}

/// The world's turn-resource registry: a singleton config document
/// (`RESOURCE_REGISTRY_DOC_TYPE` is in `SINGLETON_DOC_TYPES`), EMPTY by
/// default (the engine hooks up no resource, not even movement). Keyed by
/// resource id — a MAP for the single-key-Update reason every registry uses.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct ResourceRegistryEngine {
    /// Resources keyed by id (`CombatantEngine.resources` and
    /// `MovementRules.resource` reference keys).
    pub resources: BTreeMap<String, Resource>,
}

impl ResourceRegistryEngine {
    /// Every formula in every binding passes `Formula::validate`.
    pub(crate) fn validate(&self) -> Result<(), String> {
        for (key, r) in &self.resources {
            match &r.binding {
                ResourceBinding::Mirror { value } => value.validate(&format!("{key}.value"))?,
                ResourceBinding::Tracked { max, recover } => {
                    max.validate(&format!("{key}.max"))?;
                    recover
                        .turn_start
                        .validate(&format!("{key}.recover.turn_start"))?;
                    recover
                        .turn_end
                        .validate(&format!("{key}.recover.turn_end"))?;
                    recover
                        .round_start
                        .validate(&format!("{key}.recover.round_start"))?;
                    recover
                        .round_end
                        .validate(&format!("{key}.recover.round_end"))?;
                }
            }
        }
        Ok(())
    }
}

/// What a duration counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "snake_case")]
pub enum DurationUnit {
    /// Round wraps.
    Rounds,
    /// The anchor combatant's turn boundaries.
    Turns,
}

/// The clock boundary an effect expires on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(rename_all = "snake_case")]
pub enum ExpiryPoint {
    /// When the anchor's turn starts.
    TurnStart,
    /// When the anchor's turn ends.
    TurnEnd,
    /// When a round starts.
    RoundStart,
    /// When a round ends.
    RoundEnd,
}

/// A clock-bound lifetime for an effect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Duration {
    /// How many `unit`s the effect lasts; a `Formula::Number` must be `>= 1`.
    pub amount: Formula,
    /// Remaining `unit`s until expiry. Server-written: `None` = not yet
    /// ticked (full duration); the first matching boundary evaluates `amount`
    /// and materializes the countdown.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub remaining: Option<u32>,
    /// Rounds or turns.
    pub unit: DurationUnit,
    /// The combatant whose turns are counted; `None` = the combatant whose
    /// actor hosts the effect.
    pub anchor: Option<Uuid>,
    /// The boundary the effect expires on.
    pub expires: ExpiryPoint,
}

/// Authored lifecycle policy, evaluated server-side at each boundary. Every
/// formula is optional and falls through the combat-defaults chain to the
/// engine fallbacks (expire at combat end, keep at turn end, decrement).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, default)]
pub struct EffectLifecycle {
    /// Truthy ⇒ expire at `CombatEnd`.
    #[ts(optional = nullable)]
    pub on_combat_end: Option<Formula>,
    /// Truthy ⇒ expire at the host's turn end.
    #[ts(optional = nullable)]
    pub on_turn_end: Option<Formula>,
    /// Truthy ⇒ boundaries decrement `remaining`.
    #[ts(optional = nullable)]
    pub on_advance: Option<Formula>,
}

/// The three lifecycle formulas as chain-level defaults (no `resolved`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct EffectLifecycleDefaults {
    /// Default for `EffectLifecycle.on_combat_end`.
    #[ts(optional = nullable)]
    pub on_combat_end: Option<Formula>,
    /// Default for `EffectLifecycle.on_turn_end`.
    #[ts(optional = nullable)]
    pub on_turn_end: Option<Formula>,
    /// Default for `EffectLifecycle.on_advance`.
    #[ts(optional = nullable)]
    pub on_advance: Option<Formula>,
}

impl EffectLifecycleDefaults {
    /// Every present formula parses. `at` names the enclosing field for the
    /// error message.
    pub(crate) fn validate(&self, at: &str) -> Result<(), String> {
        let fields = [
            ("on_combat_end", &self.on_combat_end),
            ("on_turn_end", &self.on_turn_end),
            ("on_advance", &self.on_advance),
        ];
        for (name, formula) in fields {
            if let Some(f) = formula {
                f.validate(&format!("{at}.{name}"))?;
            }
        }
        Ok(())
    }
}

/// The engine body of an `effect` document (mirrors the client's
/// `EffectEngine`). Modifiers stay in the system band; the engine owns only
/// activation, transfer and the clock-bound lifetime. Absent `transfer` and
/// `duration` read as `false` / `None`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct EffectEngine {
    /// Whether the effect applies. Expiry sets this `false`; it never deletes.
    pub active: bool,
    /// An item-embedded effect reaches the item's owning actor when `true`.
    #[serde(default)]
    pub transfer: bool,
    /// Clock-bound lifetime; `None` = on while active.
    #[serde(default)]
    pub duration: Option<Duration>,
    /// Authored expiry/decrement policy; `None` = not on a clock (transient/permanent effect).
    #[serde(default)]
    #[ts(optional = nullable)]
    pub lifecycle: Option<EffectLifecycle>,
}

impl EffectEngine {
    /// A duration's `amount` formula is well-formed, and — when a literal number — `>= 1`;
    /// every present lifecycle formula is well-formed.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if let Some(d) = &self.duration {
            d.amount.validate("duration.amount")?;
            if let Formula::Number(n) = &d.amount {
                if *n < 1.0 {
                    return Err("duration amount must be >= 1".into());
                }
            }
        }
        if let Some(l) = &self.lifecycle {
            if let Some(f) = &l.on_combat_end {
                f.validate("lifecycle.on_combat_end")?;
            }
            if let Some(f) = &l.on_turn_end {
                f.validate("lifecycle.on_turn_end")?;
            }
            if let Some(f) = &l.on_advance {
                f.validate("lifecycle.on_advance")?;
            }
        }
        Ok(())
    }
}

/// Upper bound on retained turn records; the oldest drops first.
pub const MAX_TURN_HISTORY: usize = 200;

/// One anchored effect as it stood at a turn boundary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct EffectSnapshot {
    /// The document that embeds the effect (token or actor).
    pub host: Uuid,
    /// JSON pointer of the effect document inside `host` (e.g. `/embedded/effect/0`).
    pub path: String,
    /// The effect's engine band at the boundary.
    pub engine: EffectEngine,
}

/// One combatant as a turn boundary captured it: identity, placement,
/// access control, ownership and the two state bands — exactly what
/// `combat::history`'s `restore`/`live_equals` pass reads back, and nothing
/// else. INVARIANT: this is deliberately NOT a whole `Document`. A record
/// holds one entry per combatant and a combat retains `MAX_TURN_HISTORY`
/// records, so every captured byte is multiplied by both; capturing bands no
/// restore reads (`source`, `base`, `embedded`, timestamps) pushes the
/// history document's `/engine` band past `MAX_SYSTEM_BYTES` — a hard
/// ingress refusal that rolls back the whole combat transition — inside an
/// ordinary combat. HIDDEN COUPLING: a combatant `restore` re-`Create`s
/// (an exhausted `Event` deleted since the boundary) is rebuilt from these
/// fields alone, with its `scope`/`doc_type`/`parent_id` derived from the
/// combat it belongs to and an empty `embedded` map — the record restores the
/// CLOCK's state, never a general document backup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct CapturedCombatant {
    /// The combatant document's id.
    pub id: Uuid,
    /// Envelope display name (an `Event`'s row label).
    pub name: Option<String>,
    /// Its permission set at the boundary. Captured because a combatant's
    /// hidden state IS `permissions.default: none` — restoring the set
    /// restores the visibility a re-`Create`d combatant had.
    pub permissions: crate::data::document::PermissionSet,
    /// Its owner at the boundary.
    pub owner: Option<Uuid>,
    /// Its engine band at the boundary.
    pub engine: CombatantEngine,
    /// Its opaque system band at the boundary.
    #[ts(type = "unknown")]
    pub system: serde_json::Value,
}

/// Every combatant and anchored effect as they stood when `turn` began.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct TurnRecord {
    /// Round the turn belongs to.
    pub round: u32,
    /// The combatant whose turn it is.
    pub turn: Uuid,
    /// The narrowed per-combatant capture (restore re-creates a deleted one).
    pub combatants: Vec<CapturedCombatant>,
    /// Anchored effects.
    pub effects: Vec<EffectSnapshot>,
}

/// The engine body of a `combat-history` document: the per-turn snapshots of
/// one combat and the cursor of the current turn. GM-only by
/// `permissions.default: none`; bounded to `MAX_TURN_HISTORY` records AND,
/// independently, to a serialized-byte ceiling `combat::history::append_record`
/// enforces by evicting oldest-first (record COUNT alone does not bound
/// serialized SIZE, and only size is what `validate_system_size` refuses on).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct CombatHistoryEngine {
    /// Oldest first.
    pub records: Vec<TurnRecord>,
    /// Index of the record describing the current turn (`0` when empty).
    pub cursor: u32,
}

impl CombatHistoryEngine {
    /// At most `MAX_TURN_HISTORY` records; `cursor < len` unless empty; every
    /// captured band is itself valid, so a record can never hold a combatant
    /// or effect that `restore` would then fail to write back.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.records.len() > MAX_TURN_HISTORY {
            return Err(format!("history exceeds {MAX_TURN_HISTORY} records"));
        }
        if !self.records.is_empty() && (self.cursor as usize) >= self.records.len() {
            return Err("cursor out of range".into());
        }
        if self.records.is_empty() && self.cursor != 0 {
            return Err("cursor must be 0 for an empty history".into());
        }
        for (i, record) in self.records.iter().enumerate() {
            for (j, c) in record.combatants.iter().enumerate() {
                c.engine
                    .validate()
                    .map_err(|m| format!("records[{i}].combatants[{j}]: {m}"))?;
            }
            for (j, e) in record.effects.iter().enumerate() {
                e.engine
                    .validate()
                    .map_err(|m| format!("records[{i}].effects[{j}]: {m}"))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
