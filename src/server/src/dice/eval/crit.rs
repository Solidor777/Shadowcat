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
mod tests {
    use super::*;
    use crate::dice::spec::{
        Comparator, CritFail, CritSuccess, CritTrigger, Direction, SuccessConfig, SuccessRule,
    };

    fn cfg(cs: Option<CritSuccess>, cf: Option<CritFail>) -> SuccessConfig {
        SuccessConfig {
            success: SuccessRule::Numeric {
                comp: Comparator::Gte,
                target: 7,
            },
            required_successes: None,
            tiers: vec![],
            crit_success: cs,
            crit_fail: cf,
            expertise: 0,
        }
    }

    #[test]
    fn crit_success_at_or_above_threshold_highwins() {
        let c = cfg(
            Some(CritSuccess {
                trigger: CritTrigger::AtLeast(10),
                extra_successes: 2,
                positive_counter: 1,
            }),
            None,
        );
        let hit = score_die(Direction::HighWins, 10, &[], &c);
        assert!(hit.is_success && hit.extra_successes == 2 && hit.positive_counter == 1);
        assert!(!score_die(Direction::HighWins, 9, &[], &c).is_success);
    }

    #[test]
    fn crit_thresholds_flip_under_lowwins() {
        // LowWins crit-success fires at/below threshold.
        let c = cfg(
            Some(CritSuccess {
                trigger: CritTrigger::AtLeast(1),
                extra_successes: 1,
                positive_counter: 0,
            }),
            None,
        );
        assert!(score_die(Direction::LowWins, 1, &[], &c).is_success);
        assert!(!score_die(Direction::LowWins, 2, &[], &c).is_success);
    }

    #[test]
    fn crit_fail_reports_loss_and_negative_counter() {
        let c = cfg(
            None,
            Some(CritFail {
                trigger: CritTrigger::AtLeast(1),
                lost: 1,
                negative_counter: 1,
                allow_negative: false,
            }),
        );
        let f = score_die(Direction::HighWins, 1, &[], &c);
        assert!(f.is_fail && f.lost == 1 && f.negative_counter == 1);
    }

    #[test]
    fn overlapping_thresholds_fire_both_crit_success_and_crit_fail() {
        // cs fires at value >= 5; cf fires at value <= 10 (HighWins). Overlap
        // region [5, 10] intentionally lets a single die satisfy both — the
        // caller (evaluate_success) folds both deltas. Pinning this contract.
        let c = cfg(
            Some(CritSuccess {
                trigger: CritTrigger::AtLeast(5),
                extra_successes: 2,
                positive_counter: 1,
            }),
            Some(CritFail {
                trigger: CritTrigger::AtLeast(10),
                lost: 1,
                negative_counter: 1,
                allow_negative: false,
            }),
        );
        let both = score_die(Direction::HighWins, 7, &[], &c);
        assert!(both.is_success && both.is_fail);
        assert_eq!(both.extra_successes, 2);
        assert_eq!(both.lost, 1);
        assert_eq!(both.positive_counter, 1);
        assert_eq!(both.negative_counter, 1);
    }

    #[test]
    fn overlapping_symbol_triggers_on_same_symbol_fire_both_crit_success_and_crit_fail() {
        // Mirrors overlapping_thresholds_fire_both_crit_success_and_crit_fail, but
        // both triggers key on the SAME symbol — an independent per-event check
        // (score_die evaluates cs and cf separately), so a single die showing that
        // symbol satisfies both, exactly as an overlapping numeric range does.
        let c = cfg(
            Some(CritSuccess {
                trigger: CritTrigger::HasSymbol("triumph".to_string()),
                extra_successes: 2,
                positive_counter: 1,
            }),
            Some(CritFail {
                trigger: CritTrigger::HasSymbol("triumph".to_string()),
                lost: 1,
                negative_counter: 1,
                allow_negative: false,
            }),
        );
        let symbols = vec!["triumph".to_string()];
        let both = score_die(Direction::HighWins, 0, &symbols, &c);
        assert!(both.is_success && both.is_fail);
        assert_eq!(both.extra_successes, 2);
        assert_eq!(both.lost, 1);
        assert_eq!(both.positive_counter, 1);
        assert_eq!(both.negative_counter, 1);
    }

    #[test]
    fn symbol_crit_success_fires_on_matching_symbol() {
        let c = SuccessConfig {
            success: SuccessRule::Numeric {
                comp: Comparator::Gte,
                target: 100,
            }, // unreachable numeric target
            required_successes: None,
            tiers: vec![],
            crit_success: Some(CritSuccess {
                trigger: CritTrigger::HasSymbol("triumph".to_string()),
                extra_successes: 1,
                positive_counter: 1,
            }),
            crit_fail: None,
            expertise: 0,
        };
        let hit = score_die(Direction::HighWins, 0, &["triumph".to_string()], &c);
        assert!(hit.is_success && hit.extra_successes == 1 && hit.positive_counter == 1);
        let miss = score_die(Direction::HighWins, 0, &["blank".to_string()], &c);
        assert!(!miss.is_success);
    }

    #[test]
    fn symbol_crit_trigger_is_direction_insensitive() {
        // A symbol is present or absent regardless of HighWins/LowWins — unlike
        // AtLeast, HasSymbol never flips.
        let c = SuccessConfig {
            success: SuccessRule::Numeric {
                comp: Comparator::Lte,
                target: 1,
            },
            required_successes: None,
            tiers: vec![],
            crit_success: Some(CritSuccess {
                trigger: CritTrigger::HasSymbol("despair".to_string()),
                extra_successes: 0,
                positive_counter: 1,
            }),
            crit_fail: None,
            expertise: 0,
        };
        let symbols = vec!["despair".to_string()];
        assert!(score_die(Direction::HighWins, 5, &symbols, &c).is_success);
        assert!(score_die(Direction::LowWins, 5, &symbols, &c).is_success);
    }
}
