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
