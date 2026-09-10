#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::dice::spec::{Direction, Tier};

/// Classification of an oriented margin (higher = better). Mutually exclusive
/// outputs: a roll reports EITHER a `pass` (default 2-rung ladder) OR a `tier`
/// (custom ladder), never both.
///
/// # Examples
///
/// ```
/// use shadowcat::dice::eval::classify::classify;
/// // Empty ladder: default 2-rung pass/fail at margin >= 0.
/// let c = classify(3, &[]);
/// assert_eq!(c.pass, Some(true));
/// assert_eq!(c.tier_label, None);
/// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::dice::eval::classify::oriented_margin;
/// use shadowcat::dice::spec::Direction;
/// // A 15 beating a difficulty of 10 is +5 margin either way it's framed...
/// assert_eq!(oriented_margin(Direction::HighWins, 15, 10), 5);
/// // ...but under a roll-under system, rolling 15 against a target of 10 is a loss.
/// assert_eq!(oriented_margin(Direction::LowWins, 15, 10), -5);
/// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::dice::eval::classify::classify;
/// use shadowcat::dice::spec::Tier;
/// let tiers = vec![
///     Tier { margin_offset: 0, label: Some("success".into()), tier_value: Some(1) },
///     Tier { margin_offset: 5, label: Some("critical".into()), tier_value: Some(2) },
/// ];
/// let c = classify(7, &tiers);
/// assert_eq!(c.tier_label.as_deref(), Some("critical"));
/// assert_eq!(c.pass, None); // a tier ladder reports tier_*, never pass
/// ```
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
