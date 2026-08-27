#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::dice::spec::{Direction, Tier};

/// Classification of an oriented margin (higher = better). Mutually exclusive
/// outputs: a roll reports EITHER a `pass` (default 2-rung ladder) OR a `tier`
/// (custom ladder), never both.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Classification {
    /// Default 2-rung ladder verdict (`None` when a custom ladder ran).
    pub pass: Option<bool>,
    /// Custom-ladder rung label.
    pub tier_label: Option<String>,
    /// Custom-ladder rung payload.
    pub tier_value: Option<i32>,
}

/// Orient a scalar-vs-reference difference so "better" is always more positive.
/// HighWins: higher scalar is better. LowWins (roll-under): lower scalar is better.
pub fn oriented_margin(direction: Direction, scalar: i64, reference: i64) -> i64 {
    match direction {
        Direction::HighWins => scalar - reference,
        Direction::LowWins => reference - scalar,
    }
}

/// Classify `margin` against `tiers`. Empty ladder => default 2-rung pass/fail
/// (`pass = margin >= 0`). Non-empty => the highest rung with `margin_offset <=
/// margin`; if none match (margin below the floor), fail closed to the lowest
/// rung. Order-independent (no sorted precondition). Well-formed ladders use
/// unique `margin_offset`s; a duplicate offset ties on `max_by_key`'s
/// last-equally-extreme-element / `min_by_key`'s first-equally-extreme-element
/// semantics, so which duplicate wins depends on
/// caller-supplied vec order.
pub fn classify(margin: i64, tiers: &[Tier]) -> Classification {
    if tiers.is_empty() {
        return Classification {
            pass: Some(margin >= 0),
            tier_label: None,
            tier_value: None,
        };
    }
    let chosen = tiers
        .iter()
        .filter(|t| (t.margin_offset as i64) <= margin)
        .max_by_key(|t| t.margin_offset)
        .or_else(|| tiers.iter().min_by_key(|t| t.margin_offset))
        .expect("tiers is non-empty");
    Classification {
        pass: None,
        tier_label: chosen.label.clone(),
        tier_value: chosen.tier_value,
    }
}

#[cfg(test)]
mod tests;
