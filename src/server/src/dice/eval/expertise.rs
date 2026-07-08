use std::collections::HashMap;

use crate::dice::eval::crit;
use crate::dice::outcome::{DieRecord, RawRoll};
use crate::dice::spec::{DieId, DieKind, Direction, SuccessConfig};

/// Move `value` toward "better" (higher under `HighWins`, lower under `LowWins`)
/// by up to `k` steps, stopping at the die's better-end face (`max` / `min`).
///
/// INVARIANT: `adjust(_, v, _, _, 0) == v` for every `v`, INCLUDING values outside
/// `[min, max]`. A naive `clamp(v ± k, min, max)` would violate this — a Compound
/// die (`value > max`) would be pulled down to `max`, a Penetrate die (`value < min`)
/// pushed up to `min`, even at `k == 0`. The `max(0, headroom)` term makes the move
/// zero when the die is already at/past its better-end bound, so an out-of-range face
/// is never dragged back across a bound; expertise only ever moves a die toward better.
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

fn add(a: (i32, i32), b: (i32, i32)) -> (i32, i32) {
    (a.0 + b.0, a.1 + b.1)
}

/// Bounded knapsack DP over `dies` (each a `v[k]` table, `k in 0..=e`) with budget
/// `e`, choosing the allocation maximal under the total order `better`.
///
/// `dp[j]` = best `(net,counter)` using ≤ `j` points over the dice processed so far;
/// `dp'[j] = max_k dp[j-k] ⊕ v_i(k)`. O(N·E²). Deterministic. Tie-break: the inner
/// loop scans `k` ascending and replaces only on STRICTLY `better`, so the smallest
/// `k` wins a tie at each die; backtracking runs from the LAST die, where later dice
/// take the smallest optimal `k` (passing budget down), so points concentrate on the
/// earliest dice whenever spending is actually needed to reach the optimum — the R3
/// lowest-index-first canonical allocation. (When no `k` changes a die's value at all,
/// e.g. every allocation ties, this degrades to `k=0` everywhere — never spend a point
/// that provably does nothing, even to satisfy "concentrate on the earliest die".)
fn run_dp(
    dies: &[Vec<(i32, i32)>],
    e: u32,
    better: impl Fn((i32, i32), (i32, i32)) -> bool,
) -> (Vec<u32>, (i32, i32)) {
    let e = e as usize;
    let n = dies.len();
    let mut dp = vec![(0i32, 0i32); e + 1];
    let mut choice = vec![vec![0u32; e + 1]; n];
    for i in 0..n {
        let vi = &dies[i];
        let prev = dp.clone();
        for j in 0..=e {
            let mut best_pair = add(prev[j], vi[0]); // k = 0
            let mut best_k = 0u32;
            for k in 1..=j {
                let cand = add(prev[j - k], vi[k]);
                if better(cand, best_pair) {
                    best_pair = cand;
                    best_k = k as u32;
                }
            }
            dp[j] = best_pair;
            choice[i][j] = best_k;
        }
    }
    let mut alloc = vec![0u32; n];
    let mut budget = e;
    for i in (0..n).rev() {
        let k = choice[i][budget];
        alloc[i] = k;
        budget -= k as usize;
    }
    (alloc, dp[e])
}

/// Distribute `cfg.expertise` points across the pooled kept dice to maximize the
/// VISIBLE outcome: primary key = clamped net successes (`max(net,0)` unless
/// `allow_negative`), secondary = net counters (§R1). Mutates each chosen die's
/// `value` (adjusted face) and `expertise` (points spent). No-op if budget is 0 or
/// there are no kept dice.
///
/// Two-pass clamp handling: the lexicographic pass maximizes RAW `(net, counter)`.
/// If `allow_negative` OR the achieved net ≥ 1, that pass equals the clamped-lex
/// optimum (clamp is identity there). Otherwise every allocation clamps to net 0, so
/// successes tie and the objective degenerates to pure counter-maximization — a second
/// pass with the first key dropped. Both passes use the same R3 tie-break, so the
/// result matches the brute-force oracle exactly.
pub fn allocate(
    direction: Direction,
    cfg: &SuccessConfig,
    raws: &RawRoll,
    records: &mut [DieRecord],
) {
    let e = cfg.expertise;
    if e == 0 {
        return;
    }
    // Bounds per die: every record id maps to a RawDie carrying its Numeric kind.
    let bounds: HashMap<DieId, (i32, i32)> = raws
        .dice
        .iter()
        .map(|d| {
            let DieKind::Numeric { min, max } = d.kind;
            (d.id, (min, max))
        })
        .collect();
    // Contributing dice = the pooled kept dice, in record order.
    let kept: Vec<usize> = records
        .iter()
        .enumerate()
        .filter(|(_, r)| r.kept)
        .map(|(i, _)| i)
        .collect();
    if kept.is_empty() {
        return;
    }
    let dies: Vec<Vec<(i32, i32)>> = kept
        .iter()
        .map(|&idx| {
            let (min, max) = bounds[&records[idx].id];
            die_values(direction, cfg, records[idx].value, min, max, e)
        })
        .collect();

    let lex = |a: (i32, i32), b: (i32, i32)| a.0 > b.0 || (a.0 == b.0 && a.1 > b.1);
    let (lex_alloc, (net, _)) = run_dp(&dies, e, lex);
    let allow_neg = cfg
        .crit_fail
        .as_ref()
        .map(|c| c.allow_negative)
        .unwrap_or(false);
    let alloc = if allow_neg || net >= 1 {
        lex_alloc
    } else {
        // All-failed region: successes all clamp to 0; maximize counters only.
        let counter_only = |a: (i32, i32), b: (i32, i32)| a.1 > b.1;
        run_dp(&dies, e, counter_only).0
    };

    for (pos, &idx) in kept.iter().enumerate() {
        let k = alloc[pos];
        let (min, max) = bounds[&records[idx].id];
        records[idx].value = adjust(direction, records[idx].value, min, max, k);
        records[idx].expertise = k as i32;
    }
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

    #[test]
    fn run_dp_lexicographic_prefers_net_then_counters_then_low_index() {
        let lex = |a: (i32, i32), b: (i32, i32)| a.0 > b.0 || (a.0 == b.0 && a.1 > b.1);
        // Two identical dice, each: k0=(0,0), k1=(1,0). One point, either die gives +1 net.
        let dies = vec![vec![(0, 0), (1, 0)], vec![(0, 0), (1, 0)]];
        let (alloc, achieved) = run_dp(&dies, 1, lex);
        assert_eq!(achieved, (1, 0));
        // Lowest-index-first: the point lands on die 0, not die 1.
        assert_eq!(alloc, vec![1, 0]);
    }

    #[test]
    fn run_dp_allocates_the_more_valuable_die() {
        let lex = |a: (i32, i32), b: (i32, i32)| a.0 > b.0 || (a.0 == b.0 && a.1 > b.1);
        // die0: one point -> +1 net. die1: one point -> +2 net. Budget 1 -> spend on die1.
        let dies = vec![vec![(0, 0), (1, 0)], vec![(0, 0), (2, 0)]];
        let (alloc, achieved) = run_dp(&dies, 1, lex);
        assert_eq!(achieved, (2, 0));
        assert_eq!(alloc, vec![0, 1]);
    }

    use crate::dice::outcome::{DieRecord, RawDie, RawRoll};
    use crate::dice::spec::{DieId, DieKind};

    /// Build a RawRoll whose kept records have the given values, all d(min..=max).
    fn raws_of(values: &[i32], min: i32, max: i32) -> RawRoll {
        let mut raws = RawRoll::default();
        for (i, &v) in values.iter().enumerate() {
            let id = i as DieId;
            raws.dice.push(RawDie {
                id,
                kind: DieKind::Numeric { min, max },
                natural: v,
            });
            raws.records.push(DieRecord {
                label: None,
                id,
                group_index: 0,
                natural: v,
                value: v,
                kept: true,
                exploded: false,
                rerolled_from: None,
                crit_success: false,
                crit_fail: false,
                expertise: 0,
            });
        }
        raws.next_id = values.len() as DieId;
        raws
    }

    /// Re-score a pool exactly as `evaluate_success` will: clamped net + counters.
    fn score_pool(direction: Direction, cfg: &SuccessConfig, records: &[DieRecord]) -> (i32, i32) {
        let (mut base, mut extra, mut lost, mut pos, mut neg) = (0, 0, 0, 0, 0);
        for r in records.iter().filter(|r| r.kept) {
            base += i32::from(cfg.success.comp.test(r.value, cfg.success.target));
            let dc = crit::score_die(direction, r.value, cfg);
            extra += dc.extra_successes;
            lost += dc.lost;
            pos += dc.positive_counter;
            neg += dc.negative_counter;
        }
        let raw = base + extra - lost;
        let allow_neg = cfg
            .crit_fail
            .as_ref()
            .map(|c| c.allow_negative)
            .unwrap_or(false);
        (if allow_neg { raw } else { raw.max(0) }, pos - neg)
    }

    #[test]
    fn allocate_zero_budget_is_a_noop() {
        let c = cfg(None, None); // expertise: 0
        let mut raws = raws_of(&[4, 4, 4], 1, 6);
        let before = raws.records.clone();
        allocate(Direction::HighWins, &c, &raws.clone(), &mut raws.records);
        assert_eq!(raws.records, before);
    }

    #[test]
    fn allocate_spends_points_to_reach_the_success_target() {
        // Three dice at face 4, target >=5, 2 expertise points. Optimal: bump two
        // dice to 5 -> 2 successes (each bump costs 1). value/expertise recorded.
        let mut c = cfg(None, None);
        c.expertise = 2;
        let raws = raws_of(&[4, 4, 4], 1, 6);
        let mut records = raws.records.clone();
        allocate(Direction::HighWins, &c, &raws, &mut records);
        assert_eq!(score_pool(Direction::HighWins, &c, &records), (2, 0));
        // Lowest-index-first: dice 0 and 1 get the points, die 2 untouched.
        assert_eq!(records[0].expertise, 1);
        assert_eq!(records[1].expertise, 1);
        assert_eq!(records[2].expertise, 0);
        assert_eq!(records[0].value, 5);
        assert_eq!(records[2].value, 4);
    }

    #[test]
    fn allocate_all_failed_region_maximizes_counters() {
        // R1 fork: base success target is unreachable (target 100 on a d6), so net is
        // always <= 0 (0 with no crit fired, -1 if crit_fail fires) for EVERY allocation
        // -> the objective degenerates to pure counter-maximization.
        // Two dice at face 1 on a d6: crit_fail at <=1 (lost 1, negative_counter 1) fires
        // at face 1; crit_success at 6 (extra_successes 0, positive_counter 1) fires at
        // face 6. Budget 10 affords moving BOTH dice from face 1 to face 6 (5 points each).
        let c = SuccessConfig {
            success: SuccessRule {
                comp: Comparator::Gte,
                target: 100,
            },
            required_successes: None,
            tiers: vec![],
            crit_success: Some(CritSuccess {
                threshold: 6,
                extra_successes: 0,
                positive_counter: 1,
            }),
            crit_fail: Some(CritFail {
                threshold: 1,
                lost: 1,
                negative_counter: 1,
                allow_negative: false,
            }),
            expertise: 10,
        };
        let raws = raws_of(&[1, 1], 1, 6);
        let mut records = raws.records.clone();
        allocate(Direction::HighWins, &c, &raws, &mut records);
        // Clamped net stays 0 (base unreachable); counters maximized: both dice moved to
        // 6 -> both leave the crit_fail region (removing the -1 counter apiece) and land
        // on crit_success (+1 positive_counter apiece). score_pool: base 0, pos 2, neg 0
        // -> (0, 2).
        assert_eq!(score_pool(Direction::HighWins, &c, &records), (0, 2));
        assert_eq!(records[0].expertise, 5);
        assert_eq!(records[1].expertise, 5);
        assert_eq!(records[0].value, 6);
        assert_eq!(records[1].value, 6);
    }

    /// Every allocation of AT MOST `e` points across `n` dice (budget need not be
    /// fully spent — spending a point that doesn't help is never required). Feasible
    /// only because e, n are tiny — the whole point of the oracle.
    fn allocations_within_budget(n: usize, e: u32) -> Vec<Vec<u32>> {
        if n == 0 {
            return vec![vec![]];
        }
        let mut out = Vec::new();
        for first in 0..=e {
            for rest in allocations_within_budget(n - 1, e - first) {
                let mut one = vec![first];
                one.extend(rest);
                out.push(one);
            }
        }
        out
    }

    /// Brute-force optimum: for each allocation, apply `adjust` to a copy of each die's
    /// value, re-score (clamped net + counters), take the clamped-lex max. Among optima,
    /// mirror the DP's own tie-break EXACTLY: `run_dp` backtracks from the LAST die,
    /// giving it the smallest k that still hits the joint optimum, then recurses on the
    /// remaining budget for earlier dice — i.e. the canonical allocation minimizes spend
    /// starting from the highest index backward (NOT "spend the whole budget, biased
    /// toward the earliest die" — when no k does anything, k=0 everywhere is correct).
    /// Comparing allocations by their REVERSED vector (lexicographically smallest wins)
    /// reproduces exactly that backtrack order.
    fn oracle(
        direction: Direction,
        cfg: &SuccessConfig,
        records: &[DieRecord],
        bounds: &std::collections::HashMap<DieId, (i32, i32)>,
    ) -> ((i32, i32), Vec<u32>) {
        let kept: Vec<usize> = records
            .iter()
            .enumerate()
            .filter(|(_, r)| r.kept)
            .map(|(i, _)| i)
            .collect();
        let mut best: Option<((i32, i32), Vec<u32>)> = None;
        for alloc in allocations_within_budget(kept.len(), cfg.expertise) {
            let mut copy = records.to_vec();
            for (pos, &idx) in kept.iter().enumerate() {
                let (min, max) = bounds[&copy[idx].id];
                copy[idx].value = adjust(direction, copy[idx].value, min, max, alloc[pos]);
            }
            let score = score_pool(direction, cfg, &copy);
            let take = match &best {
                None => true,
                Some((bs, ba)) => {
                    score.0 > bs.0
                        || (score.0 == bs.0 && score.1 > bs.1)
                        // tie on objective -> reversed-lexicographically-smallest k-vector
                        // (minimize the LAST die's spend first, then the second-to-last, ...).
                        || (score == *bs
                            && alloc.iter().rev().collect::<Vec<_>>()
                                < ba.iter().rev().collect::<Vec<_>>())
                }
            };
            if take {
                best = Some((score, alloc));
            }
        }
        best.expect("at least the all-zero allocation exists")
    }

    #[test]
    fn dp_matches_brute_force_oracle_over_a_random_corpus() {
        // Deterministic pseudo-random corpus (no RNG dependency): a SplitMix walk over
        // a fixed seed, varying ranges, targets, crit configs, direction, allow_negative,
        // e in 0..=6, n in 1..=5. Objective-value equality is the load-bearing property;
        // allocation equality pins the canonical tie-break.
        let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = s;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        let mut pick = |lo: i32, hi: i32| lo + (next() % ((hi - lo + 1) as u64)) as i32;

        for _ in 0..4000 {
            let min = 1;
            let max = pick(2, 12);
            let direction = if pick(0, 1) == 0 {
                Direction::HighWins
            } else {
                Direction::LowWins
            };
            let comp = if direction == Direction::HighWins {
                Comparator::Gte
            } else {
                Comparator::Lte
            };
            let target = pick(min, max);
            let cs = if pick(0, 1) == 0 {
                Some(CritSuccess {
                    threshold: pick(min, max),
                    extra_successes: pick(-1, 3), // include a perverse negative
                    positive_counter: pick(-2, 2),
                })
            } else {
                None
            };
            let cf = if pick(0, 1) == 0 {
                Some(CritFail {
                    threshold: pick(min, max),
                    lost: pick(-1, 3),
                    negative_counter: pick(-2, 2),
                    allow_negative: pick(0, 1) == 0,
                })
            } else {
                None
            };
            let cfg = SuccessConfig {
                success: SuccessRule { comp, target },
                required_successes: None,
                tiers: vec![],
                crit_success: cs,
                crit_fail: cf,
                expertise: pick(0, 6) as u32, // 0..=6
            };
            let n = 1 + pick(0, 4) as usize; // 1..=5
            let values: Vec<i32> = (0..n).map(|_| pick(min, max)).collect();
            let raws = raws_of(&values, min, max);
            let bounds: std::collections::HashMap<DieId, (i32, i32)> = raws
                .dice
                .iter()
                .map(|d| {
                    let DieKind::Numeric { min, max } = d.kind;
                    (d.id, (min, max))
                })
                .collect();

            let mut records = raws.records.clone();
            allocate(direction, &cfg, &raws, &mut records);
            let dp_score = score_pool(direction, &cfg, &records);
            let (oracle_score, oracle_alloc) = oracle(direction, &cfg, &raws.records, &bounds);

            assert_eq!(
                dp_score, oracle_score,
                "objective mismatch: cfg={cfg:?} values={values:?}"
            );

            // Allocation equality: compare the DP's recorded per-die k against the oracle.
            let dp_alloc: Vec<u32> = records
                .iter()
                .filter(|r| r.kept)
                .map(|r| r.expertise as u32)
                .collect();
            assert_eq!(
                dp_alloc, oracle_alloc,
                "allocation mismatch: cfg={cfg:?} values={values:?}"
            );
        }
    }

    #[test]
    fn expertise_result_is_between_greedy_and_max_bounds() {
        // Sanity bounds: optimal net >= the no-expertise net, and <= the net with every
        // die individually maxed toward better (an unreachable upper bound when budget
        // is limited, but never exceeded).
        let mut c = cfg(
            Some(CritSuccess {
                threshold: 6,
                extra_successes: 1,
                positive_counter: 0,
            }),
            None,
        );
        c.expertise = 3;
        let raws = raws_of(&[3, 4, 5, 2], 1, 6);
        let base = score_pool(Direction::HighWins, &c, &raws.records).0;
        let mut records = raws.records.clone();
        allocate(Direction::HighWins, &c, &raws, &mut records);
        let got = score_pool(Direction::HighWins, &c, &records).0;
        let mut all_max = raws.records.clone();
        for r in all_max.iter_mut() {
            r.value = 6;
        }
        let ceiling = score_pool(Direction::HighWins, &c, &all_max).0;
        assert!(got >= base, "expertise never worsens the outcome");
        assert!(got <= ceiling, "expertise never beats every-die-maxed");
    }
}
