#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::dice::spec::{CritTrigger, Direction, SuccessConfig, SuccessRule, Symbol};

/// Per-die crit scoring result. `is_success`/`is_fail` gate whether the die's
/// threshold fired; the remaining fields are the deltas the caller folds into
/// the pool's net successes/counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DieCrit {
    /// The crit-success trigger fired.
    pub is_success: bool,
    /// The crit-fail trigger fired (can coexist with `is_success`).
    pub is_fail: bool,
    /// Successes to add (from `CritSuccess::extra_successes`).
    pub extra_successes: i32,
    /// Successes to subtract (from `CritFail::lost`).
    pub lost: i32,
    /// Positive-counter delta.
    pub positive_counter: i32,
    /// Negative-counter delta.
    pub negative_counter: i32,
}

/// HighWins: `AtLeast` crit-success fires at value >= threshold, crit-fail at
/// value <= threshold. LowWins: both comparisons flip. `HasSymbol` is
/// direction-insensitive: presence/absence has no "better end" to flip.
fn reaches(
    direction: Direction,
    value: i32,
    symbols: &[Symbol],
    trigger: &CritTrigger,
    is_success_event: bool,
) -> bool {
    match trigger {
        CritTrigger::AtLeast(threshold) => match (direction, is_success_event) {
            (Direction::HighWins, true) | (Direction::LowWins, false) => value >= *threshold,
            (Direction::HighWins, false) | (Direction::LowWins, true) => value <= *threshold,
        },
        CritTrigger::HasSymbol(s) => symbols.contains(s),
    }
}

/// Scores a single kept die against the config's optional crit-success/crit-fail
/// rules. Independent checks — a die can (in principle) satisfy both configured
/// triggers if the caller sets overlapping ranges/symbols; the caller folds both
/// deltas.
pub fn score_die(
    direction: Direction,
    value: i32,
    symbols: &[Symbol],
    cfg: &SuccessConfig,
) -> DieCrit {
    let mut out = DieCrit::default();
    if let Some(cs) = &cfg.crit_success {
        if reaches(direction, value, symbols, &cs.trigger, true) {
            out.is_success = true;
            out.extra_successes = cs.extra_successes;
            out.positive_counter = cs.positive_counter;
        }
    }
    if let Some(cf) = &cfg.crit_fail {
        if reaches(direction, value, symbols, &cf.trigger, false) {
            out.is_fail = true;
            out.lost = cf.lost;
            out.negative_counter = cf.negative_counter;
        }
    }
    out
}

/// One kept die's base-success bit plus its `score_die` crit result — every
/// caller that needs BOTH (whether a die counts as a base success, and its
/// crit deltas/counters/flags) shares this single computation instead of
/// re-matching `cfg.success` and re-deriving the net formula independently.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DieScore {
    /// The die passed the base success rule.
    pub base_success: bool,
    /// Its crit deltas/flags.
    pub crit: DieCrit,
}

impl DieScore {
    /// This die's net success delta: base (0/1) + crit extra − crit lost.
    pub fn net(&self) -> i32 {
        i32::from(self.base_success) + self.crit.extra_successes - self.crit.lost
    }
}

/// Scores one kept die's base-success test (`cfg.success`) and crit result
/// (`score_die`) together — the shared computation behind every net-successes
/// and counter aggregation over a pool of kept dice.
pub fn score_die_net(
    direction: Direction,
    cfg: &SuccessConfig,
    value: i32,
    symbols: &[Symbol],
) -> DieScore {
    let base_success = match &cfg.success {
        SuccessRule::Numeric { comp, target } => comp.test(value, *target),
        SuccessRule::HasSymbol(s) => symbols.contains(s),
    };
    DieScore {
        base_success,
        crit: score_die(direction, value, symbols, cfg),
    }
}

#[cfg(test)]
mod tests;
