//! Every combat formula evaluation, in one place: which document a
//! combatant's formulas read (`formula_host`), and the derivations consumers
//! act on (`resolved_resource`, `lifecycle_flags`, `duration_amount`).
//! Consumers never re-derive the host join or call the formula engine
//! directly — a second copy of either decision is how a preview and an
//! execution drift apart.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::HashMap;

use uuid::Uuid;

use crate::data::document::{embedded_actor_copy, Document};
use crate::data::engine::combat::{
    CombatantKind, EffectLifecycle, EffectLifecycleDefaults, Formula, ResourceBinding,
};
use crate::data::permission::TOKEN_DOC_TYPE;
use crate::formula::resolver::{NoHostResolver, SystemLeafResolver};
use crate::formula::{evaluate, parse, FormulaError, FormulaErrorKind};

/// The document a combatant's formulas read: the token-embedded actor copy
/// when the combatant names a `token_id` whose document embeds one (the
/// in-scene instance state), else the linked `actor_id` host. `None` for an
/// `Event` combatant or when every named host document is absent — the same
/// join `SceneEcs::combatant_for_token` and `effects::walk_any_host` perform,
/// so evaluation, effect discovery and the movement gate agree on the host by
/// construction. Takes the KIND rather than a whole combatant so callers that
/// hold only the parsed engine (the movement gate) share this one precedence
/// rule instead of re-deriving it.
pub(crate) fn formula_host<'a>(
    hosts: &'a HashMap<Uuid, Document>,
    kind: &CombatantKind,
) -> Option<&'a Document> {
    let CombatantKind::Actor { token_id, actor_id } = kind else {
        return None;
    };
    if let Some(token) = token_id.as_ref().and_then(|id| hosts.get(id)) {
        if let Some(actor) = embedded_actor_copy(token) {
            return Some(actor);
        }
    }
    actor_id.as_ref().and_then(|id| hosts.get(id))
}

/// The document an EFFECT's formulas evaluate over: the host itself for an
/// actor host, the token-embedded actor copy for a token host — the same two
/// shapes `effects::walk_any_host` walks. `None` for a token embedding no
/// copy (reference-free formulas still evaluate through the no-host path).
pub(crate) fn effect_host_doc(host: &Document) -> Option<&Document> {
    if host.doc_type == TOKEN_DOC_TYPE {
        embedded_actor_copy(host)
    } else {
        Some(host)
    }
}

/// Evaluates one `Formula` over `host`'s `system` band. A `Number` passes
/// through (ingress guarantees finiteness); `Text` is parsed and evaluated by
/// `crate::formula`, resolving references through `SystemLeafResolver` — or,
/// with no host, through `NoHostResolver`.
pub(crate) fn eval_formula(f: &Formula, host: Option<&Document>) -> Result<f64, FormulaError> {
    match f {
        Formula::Number(n) => Ok(*n),
        Formula::Text(t) => {
            let expr = parse(t)?;
            match host {
                Some(doc) => evaluate(&expr, &SystemLeafResolver::new(doc)),
                None => evaluate(&expr, &NoHostResolver),
            }
        }
    }
}

/// A resource's derived numbers for one combatant, at one point of use.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ResolvedNums {
    /// The value the consumer acts on (spend ceiling, gate budget, clamp base).
    pub(crate) current: f64,
    /// The ceiling `current` is clamped to.
    pub(crate) max: f64,
}

/// Derives a resource's `current`/`max` from its registry binding.
/// `Mirror` is pure derivation: both equal the evaluated `value`, and any
/// stored number is ignored. `Tracked` evaluates `max` (a negative result
/// clamps to `0` — evaluated text carries no ingress non-negativity
/// guarantee) and takes `stored` clamped to `[0, max]`; an ABSENT stored
/// entry means untouched, i.e. full (`current == max`) — the lazy-full rule
/// that replaces join-time seeding.
pub(crate) fn resolved_resource(
    def: &ResourceBinding,
    stored: Option<f64>,
    host: Option<&Document>,
) -> Result<ResolvedNums, FormulaError> {
    match def {
        ResourceBinding::Mirror { value } => {
            let v = eval_formula(value, host)?;
            Ok(ResolvedNums { current: v, max: v })
        }
        ResourceBinding::Tracked { max, .. } => {
            let max = eval_formula(max, host)?.max(0.0);
            let current = stored.unwrap_or(max).clamp(0.0, max);
            Ok(ResolvedNums { current, max })
        }
    }
}

/// An effect's lifecycle policy, evaluated at one boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LifecycleFlags {
    /// Expire at combat end (when the combat's `effect_cleanup` is set).
    pub(crate) on_combat_end: bool,
    /// Expire at the host combatant's turn end (same `effect_cleanup` gate).
    pub(crate) on_turn_end: bool,
    /// Matching boundaries decrement `Duration.remaining`.
    pub(crate) on_advance: bool,
}

/// One flag: the authored formula, else the chain default, else the engine
/// fallback; truthy = evaluated value `!= 0`.
fn flag(
    authored: Option<&Formula>,
    default: Option<&Formula>,
    fallback: bool,
    host: Option<&Document>,
) -> Result<bool, FormulaError> {
    match authored.or(default) {
        Some(f) => Ok(eval_formula(f, host)? != 0.0),
        None => Ok(fallback),
    }
}

/// Evaluates the three lifecycle flags for one effect: per flag, the effect's
/// authored formula wins, else the combat's snapshotted chain default, else
/// the engine fallback (expire at combat end, keep at turn end, decrement).
/// The first evaluation error wins, in field order.
pub(crate) fn lifecycle_flags(
    authored: Option<&EffectLifecycle>,
    defaults: &EffectLifecycleDefaults,
    host: Option<&Document>,
) -> Result<LifecycleFlags, FormulaError> {
    Ok(LifecycleFlags {
        on_combat_end: flag(
            authored.and_then(|a| a.on_combat_end.as_ref()),
            defaults.on_combat_end.as_ref(),
            true,
            host,
        )?,
        on_turn_end: flag(
            authored.and_then(|a| a.on_turn_end.as_ref()),
            defaults.on_turn_end.as_ref(),
            false,
            host,
        )?,
        on_advance: flag(
            authored.and_then(|a| a.on_advance.as_ref()),
            defaults.on_advance.as_ref(),
            true,
            host,
        )?,
    })
}

/// Evaluates a `Duration.amount` into a whole-unit count: `floor` of the
/// evaluated value. A result below `1` is refused (mirroring the authored
/// `Formula::Number >= 1` ingress rule); an evaluation error passes through.
/// The float→int cast saturates at `u32::MAX` for absurdly large results.
pub(crate) fn duration_amount(
    amount: &Formula,
    host: Option<&Document>,
) -> Result<u32, FormulaError> {
    let v = eval_formula(amount, host)?;
    if v < 1.0 {
        return Err(FormulaError::new(
            FormulaErrorKind::Type,
            "duration amount must be >= 1",
        ));
    }
    Ok(v.floor() as u32)
}

#[cfg(test)]
mod tests;
