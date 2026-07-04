use crate::dice::spec::{Direction, SuccessConfig};

/// Per-die crit scoring result. `is_success`/`is_fail` gate whether the die's
/// threshold fired; the remaining fields are the deltas the caller folds into
/// the pool's net successes/counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DieCrit {
    pub is_success: bool,
    pub is_fail: bool,
    pub extra_successes: i32,
    pub lost: i32,
    pub positive_counter: i32,
    pub negative_counter: i32,
}

/// HighWins: crit-success fires at value >= threshold, crit-fail at value <= threshold.
/// LowWins: both comparisons flip (crit-success at <= threshold, crit-fail at >= threshold).
fn reaches(direction: Direction, value: i32, threshold: i32, is_success_event: bool) -> bool {
    match (direction, is_success_event) {
        (Direction::HighWins, true) | (Direction::LowWins, false) => value >= threshold,
        (Direction::HighWins, false) | (Direction::LowWins, true) => value <= threshold,
    }
}

/// Scores a single kept die against the config's optional crit-success/crit-fail
/// rules. Independent checks — a die can (in principle) satisfy both configured
/// thresholds if the caller sets overlapping ranges; the caller folds both deltas.
pub fn score_die(direction: Direction, value: i32, cfg: &SuccessConfig) -> DieCrit {
    let mut out = DieCrit::default();
    if let Some(cs) = &cfg.crit_success {
        if reaches(direction, value, cs.threshold, true) {
            out.is_success = true;
            out.extra_successes = cs.extra_successes;
            out.positive_counter = cs.positive_counter;
        }
    }
    if let Some(cf) = &cfg.crit_fail {
        if reaches(direction, value, cf.threshold, false) {
            out.is_fail = true;
            out.lost = cf.lost;
            out.negative_counter = cf.negative_counter;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::spec::{Comparator, CritFail, CritSuccess, Direction, SuccessConfig, SuccessRule};

    fn cfg(cs: Option<CritSuccess>, cf: Option<CritFail>) -> SuccessConfig {
        SuccessConfig {
            success: SuccessRule {
                comp: Comparator::Gte,
                target: 7,
            },
            required_successes: None,
            tiers: vec![],
            crit_success: cs,
            crit_fail: cf,
        }
    }

    #[test]
    fn crit_success_at_or_above_threshold_highwins() {
        let c = cfg(
            Some(CritSuccess {
                threshold: 10,
                extra_successes: 2,
                positive_counter: 1,
            }),
            None,
        );
        let hit = score_die(Direction::HighWins, 10, &c);
        assert!(hit.is_success && hit.extra_successes == 2 && hit.positive_counter == 1);
        assert!(!score_die(Direction::HighWins, 9, &c).is_success);
    }

    #[test]
    fn crit_thresholds_flip_under_lowwins() {
        // LowWins crit-success fires at/below threshold.
        let c = cfg(
            Some(CritSuccess {
                threshold: 1,
                extra_successes: 1,
                positive_counter: 0,
            }),
            None,
        );
        assert!(score_die(Direction::LowWins, 1, &c).is_success);
        assert!(!score_die(Direction::LowWins, 2, &c).is_success);
    }

    #[test]
    fn crit_fail_reports_loss_and_negative_counter() {
        let c = cfg(
            None,
            Some(CritFail {
                threshold: 1,
                lost: 1,
                negative_counter: 1,
                allow_negative: false,
            }),
        );
        let f = score_die(Direction::HighWins, 1, &c);
        assert!(f.is_fail && f.lost == 1 && f.negative_counter == 1);
    }
}
