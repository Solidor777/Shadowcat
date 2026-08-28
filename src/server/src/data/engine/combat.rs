//! Engine bands for the combat clock: `combat` (a world document bound to a
//! scene), `combatant` (a child document of a combat), `resource-registry`
//! (the singleton turn-resource definitions) and `effect` (clock-bound
//! expiry). The server stores every `Formula::Text` verbatim and never
//! evaluates it: formulas are resolved to numbers on the client, and the
//! numbers land in `CombatantResource`.

// Ratchet: every item in this module must carry a doc comment, enforced by
// the two crate-level deny attributes this module declares.
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

/// Upper bound on a `Formula::Text` source, in characters. The server never
/// parses the text; this bounds storage only (the client's formula library
/// applies its own DoS caps when it evaluates).
pub const MAX_FORMULA_CHARS: usize = 512;

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
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
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
    pub interpretation: Option<Interpretation>,
    /// Override of `MovementRules.enforcement`.
    pub enforcement: Option<Enforcement>,
    /// Override of `CombatEngine.turn_control`.
    pub turn_control: Option<TurnControl>,
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
/// engine → world → scene chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCombatRules {
    /// Resolved movement rules.
    pub movement: MovementRules,
    /// Resolved turn control.
    pub turn_control: TurnControl,
}

/// Resolve the combat rules for a scene: scene overrides beat world overrides
/// beat the engine fallback (no resource, `PerCell`, `None`, `OwnerMayEnd`).
/// Pure; the SOLE resolver of this chain — the combat-start transition reads
/// it and nothing re-derives the precedence elsewhere.
///
/// # Examples
///
/// ```
/// use shadowcat::data::engine::combat::{resolve_combat_rules, Enforcement};
///
/// let r = resolve_combat_rules(None, None);
/// assert_eq!(r.movement.enforcement, Enforcement::None);
/// assert!(r.movement.resource.is_none());
/// ```
pub fn resolve_combat_rules(
    world: Option<&CombatDefaults>,
    scene: Option<&CombatDefaults>,
) -> ResolvedCombatRules {
    let pick = |f: fn(&CombatDefaults) -> Option<Option<String>>| {
        scene
            .and_then(f)
            .or_else(|| world.and_then(f))
            .unwrap_or(None)
    };
    let resource = pick(|d| d.movement_resource.clone());
    let interpretation = scene
        .and_then(|d| d.interpretation)
        .or_else(|| world.and_then(|d| d.interpretation))
        .unwrap_or_default();
    let enforcement = scene
        .and_then(|d| d.enforcement)
        .or_else(|| world.and_then(|d| d.enforcement))
        .unwrap_or_default();
    let turn_control = scene
        .and_then(|d| d.turn_control)
        .or_else(|| world.and_then(|d| d.turn_control))
        .unwrap_or_default();
    ResolvedCombatRules {
        movement: MovementRules {
            resource,
            interpretation,
            enforcement,
        },
        turn_control,
    }
}

/// The engine body of a `combat` document (mirrors the client's
/// `CombatEngine`). World-level, bound to one scene; at most one combat per
/// scene is intended to be `active` at a time.
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
}

impl CombatEngine {
    /// `order` has no duplicate ids and `turn`, when set, is one of them.
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
        Ok(())
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

/// A combatant's numeric view of one registry resource (mirrors the client's
/// `CombatantResource`). Numbers only: the client resolves formulas and
/// writes these; the server only ever adds, subtracts and clamps.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct CombatantResource {
    /// Current value.
    pub current: f64,
    /// Ceiling the server clamps `current` to; `>= 0`.
    pub max: f64,
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
    /// An actor kind names a token or an actor; every number is finite and
    /// every `max` is non-negative.
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
            if !r.current.is_finite() || !r.max.is_finite() {
                return Err(format!("resource {key} must be finite"));
            }
            if r.max < 0.0 {
                return Err(format!("resource {key} max must be >= 0"));
            }
        }
        Ok(())
    }
}

/// A number or a formula source. Untagged on the wire (`30` or `"speed"`).
/// The server never evaluates `Text`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(untagged)]
pub enum Formula {
    /// A literal.
    Number(f64),
    /// Formula source for the client's formula library.
    Text(String),
}

impl Formula {
    /// Finite when a number; non-empty and within `MAX_FORMULA_CHARS` when text.
    fn validate(&self, at: &str) -> Result<(), String> {
        match self {
            Formula::Number(n) if !n.is_finite() => Err(format!("{at} must be finite")),
            Formula::Number(_) => Ok(()),
            Formula::Text(t) if t.is_empty() => Err(format!("{at} formula is empty")),
            Formula::Text(t) if t.chars().count() > MAX_FORMULA_CHARS => Err(format!(
                "{at} formula exceeds {MAX_FORMULA_CHARS} characters"
            )),
            Formula::Text(_) => Ok(()),
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
    /// Continuously equals `value` over the actor (e.g. HP); the client keeps
    /// the combatant's numbers synced.
    Mirror {
        /// Formula for both `current` and `max`.
        value: Formula,
    },
    /// Combat state seeded to `max` on join and moved by recoveries and spends.
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

/// The world's turn-resource registry: intended to be one config document per
/// world, EMPTY by default (the engine hooks up no resource, not even
/// movement). Keyed by resource id — a MAP for the single-key-Update reason
/// every registry uses.
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

/// A position on the combat clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct ClockStamp {
    /// Round number.
    pub round: u32,
    /// Index into `CombatEngine.order` of the turn in progress.
    pub turn_index: u32,
}

/// A clock-bound lifetime for an effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../types/generated/engine/")]
#[serde(deny_unknown_fields)]
pub struct Duration {
    /// How many `unit`s the effect lasts; `>= 1`.
    pub amount: u32,
    /// Rounds or turns.
    pub unit: DurationUnit,
    /// The combatant whose turns are counted; `None` = the combatant whose
    /// actor hosts the effect.
    pub anchor: Option<Uuid>,
    /// The boundary the effect expires on.
    pub expires: ExpiryPoint,
    /// Where on the clock the effect began.
    pub started: ClockStamp,
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
}

impl EffectEngine {
    /// A duration's `amount` is at least 1.
    pub(crate) fn validate(&self) -> Result<(), String> {
        if let Some(d) = &self.duration {
            if d.amount == 0 {
                return Err("duration amount must be >= 1".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
