use crate::dice::eval::crit;
use crate::dice::spec::{Direction, SuccessConfig};

/// Move `value` toward "better" (higher under `HighWins`, lower under `LowWins`)
/// by up to `k` steps, stopping at the die's better-end face (`max` / `min`).
///
/// INVARIANT: `adjust(_, v, _, _, 0) == v` for every `v`, INCLUDING values outside
/// `[min, max]`. A naive `clamp(v ± k, min, max)` would violate this — a Compound
/// die (`value > max`) would be pulled down to `max`, a Penetrate die (`value < min`)
/// pushed up to `min`, even at `k == 0`. The `max(0, headroom)` term makes the move
/// zero when the die is already at/past its better-end bound, so an out-of-range face
/// is never dragged back across a bound; expertise only ever moves a die toward better.
#[allow(dead_code)]
fn adjust(direction: Direction, value: i32, min: i32, max: i32, k: u32) -> i32 {
    let k = k as i32;
    match direction {
        Direction::HighWins => value + k.min((max - value).max(0)),
        Direction::LowWins => value - k.min((value - min).max(0)),
    }
}

/// Per-die value function `v_i(k)` for `k in 0..=e`: move the face `k` steps toward
/// better, then score that single face — base success (0/1) + crit deltas — as a
/// `(net_i, counter_i)` pair. `net_i = base + extra_successes − lost`;
/// `counter_i = positive_counter − negative_counter`. Direction is applied ONCE here
/// (via the face move + the parse-time success comparator + `crit::score_die`); the
/// DP over these pairs must not reapply it.
#[allow(dead_code)]
fn die_values(
    direction: Direction,
    cfg: &SuccessConfig,
    value: i32,
    min: i32,
    max: i32,
    e: u32,
) -> Vec<(i32, i32)> {
    (0..=e)
        .map(|k| {
            let f = adjust(direction, value, min, max, k);
            let base = i32::from(cfg.success.comp.test(f, cfg.success.target));
            let dc = crit::score_die(direction, f, cfg);
            (
                base + dc.extra_successes - dc.lost,
                dc.positive_counter - dc.negative_counter,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::spec::{Comparator, CritFail, CritSuccess, SuccessRule};

    #[test]
    fn adjust_preserves_value_at_zero_points_for_all_ranges() {
        // In-range, over-max (Compound), below-min (Penetrate) — k=0 is a no-op each way.
        for &v in &[4, 15, 0, 1, 6] {
            assert_eq!(adjust(Direction::HighWins, v, 1, 6, 0), v);
            assert_eq!(adjust(Direction::LowWins, v, 1, 6, 0), v);
        }
    }

    #[test]
    fn adjust_moves_up_toward_max_under_highwins() {
        assert_eq!(adjust(Direction::HighWins, 4, 1, 6, 1), 5);
        assert_eq!(adjust(Direction::HighWins, 4, 1, 6, 3), 6); // capped at max
        assert_eq!(adjust(Direction::HighWins, 4, 1, 6, 9), 6); // over-budget still capped
    }

    #[test]
    fn adjust_moves_down_toward_min_under_lowwins() {
        assert_eq!(adjust(Direction::LowWins, 4, 1, 6, 1), 3);
        assert_eq!(adjust(Direction::LowWins, 4, 1, 6, 5), 1); // floored at min
    }

    #[test]
    fn adjust_never_pulls_an_out_of_range_value_back_across_a_bound() {
        // Compound over-max under HighWins: already above the max ceiling, no up-move.
        assert_eq!(adjust(Direction::HighWins, 15, 1, 6, 3), 15);
        // Penetrate below-min under LowWins: already below the min floor, no down-move.
        assert_eq!(adjust(Direction::LowWins, 0, 1, 6, 3), 0);
    }

    fn cfg(cs: Option<CritSuccess>, cf: Option<CritFail>) -> SuccessConfig {
        SuccessConfig {
            success: SuccessRule {
                comp: Comparator::Gte,
                target: 5,
            },
            required_successes: None,
            tiers: vec![],
            crit_success: cs,
            crit_fail: cf,
            expertise: 0,
        }
    }

    #[test]
    fn die_values_zero_point_scores_current_face() {
        // value 4, target >=5: not a success at k=0; +1 gets to 5 (success).
        let c = cfg(None, None);
        let v = die_values(Direction::HighWins, &c, 4, 1, 6, 2);
        assert_eq!(v[0], (0, 0)); // face 4: no success, no crit
        assert_eq!(v[1], (1, 0)); // face 5: base success
        assert_eq!(v[2], (1, 0)); // face 6: still one base success (no crit configured)
    }

    #[test]
    fn die_values_folds_crit_extra_and_counters() {
        // crit_success at 6: +2 extra successes, +1 positive counter.
        let c = cfg(
            Some(CritSuccess {
                threshold: 6,
                extra_successes: 2,
                positive_counter: 1,
            }),
            None,
        );
        let v = die_values(Direction::HighWins, &c, 4, 1, 6, 2);
        assert_eq!(v[0], (0, 0)); // face 4
        assert_eq!(v[1], (1, 0)); // face 5: base success only
        assert_eq!(v[2], (3, 1)); // face 6: base 1 + crit extra 2 = 3 net; +1 counter
    }

    #[test]
    fn die_values_lowwins_moves_toward_low_target() {
        // LowWins, success at <=2 (comp set by caller/parse). face 4 -> down.
        let mut c = cfg(None, None);
        c.success = SuccessRule {
            comp: Comparator::Lte,
            target: 2,
        };
        let v = die_values(Direction::LowWins, &c, 4, 1, 6, 3);
        assert_eq!(v[0], (0, 0)); // face 4: > 2, no success
        assert_eq!(v[1], (0, 0)); // face 3: still > 2
        assert_eq!(v[2], (1, 0)); // face 2: success
        assert_eq!(v[3], (1, 0)); // face 1: success
    }
}
