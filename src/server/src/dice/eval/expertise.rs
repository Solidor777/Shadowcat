#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::HashMap;

use crate::dice::eval::crit;
use crate::dice::outcome::{DieRecord, RawRoll};
use crate::dice::spec::{DieId, DieKind, Direction, SuccessConfig, SuccessRule};

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
            let base_success = match &cfg.success {
                SuccessRule::Numeric { comp, target } => comp.test(f, *target),
                // die_values only ever runs over Numeric dice (expertise excludes
                // Faces dice entirely); moving a Numeric face never changes symbols,
                // so a HasSymbol rule can never fire here — exists only for exhaustiveness.
                SuccessRule::HasSymbol(_) => false,
            };
            let base = i32::from(base_success);
            // die_values only ever runs over Numeric dice (see the
            // SuccessRule::HasSymbol arm above); a Numeric die never carries
            // symbols, so an empty slice is exact here, not a placeholder.
            let dc = crit::score_die(direction, f, &[], cfg);
            (
                base + dc.extra_successes - dc.lost,
                dc.positive_counter - dc.negative_counter,
            )
        })
        .collect()
}

/// Component-wise pair addition (net successes, net counters).
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
/// earliest dice whenever spending is actually needed to reach the optimum — the
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
/// `allow_negative`), secondary = net counters. Mutates each chosen die's
/// `value` (adjusted face) and `expertise` (points spent). No-op if budget is 0 or
/// there are no kept dice.
///
/// Two-pass clamp handling: the lexicographic pass maximizes RAW `(net, counter)`.
/// If `allow_negative` OR the achieved net ≥ 1, that pass equals the clamped-lex
/// optimum (clamp is identity there). Otherwise every allocation clamps to net 0, so
/// successes tie and the objective degenerates to pure counter-maximization — a second
/// pass with the first key dropped. Both passes use the same lowest-index-first
/// tie-break, so the result matches the brute-force oracle exactly.
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
    // Bounds per die: only Numeric dice have a defined [min,max] adjust range.
    let bounds: HashMap<DieId, (i32, i32)> = raws
        .dice
        .iter()
        .filter_map(|d| match d.kind {
            DieKind::Numeric { min, max } => Some((d.id, (min, max))),
            DieKind::Faces { .. } => None,
        })
        .collect();
    // Contributing dice = the pooled kept NUMERIC dice, in record order.
    // A Faces die (ordered or not) is excluded: `adjust`'s "+1 toward better
    // within [min,max]" has no defined meaning over an arbitrary face-list —
    // there is no contiguous numeric range to move within, and mutating
    // `value` to a non-face integer would desync the die's `symbols`.
    let kept: Vec<usize> = records
        .iter()
        .enumerate()
        .filter(|(_, r)| r.kept && bounds.contains_key(&r.id))
        .map(|(i, _)| i)
        .collect();
    if kept.is_empty() {
        return;
    }
    // Fixed net contribution from kept dice EXCLUDED from adjustment (Faces
    // dice): their success/crit score is identical under every allocation, so
    // it's a constant term missing from the DP's own `net` (computed only over
    // `dies`, the adjustable Numeric subset). The branch below decides whether
    // the TRUE pool-wide clamped total can reach >= 1 — that decision needs
    // this constant folded in, or a nonzero fixed delta (e.g. a Faces die
    // hitting `crit_fail` via `HasSymbol`) silently answers a different
    // question than `evaluate_success` will actually score. `fixed` does NOT
    // need to feed into the DP's own per-allocation comparisons: it's the same
    // additive constant across every candidate allocation, so it never changes
    // which allocation is argmax within either pass — only the pass-choice
    // threshold cares about its absolute value.
    let fixed: i32 = records
        .iter()
        .filter(|r| r.kept && !bounds.contains_key(&r.id))
        .map(|r| crit::score_die_net(direction, cfg, r.value, &r.symbols).net())
        .sum();
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
    let alloc = if allow_neg || net + fixed >= 1 {
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
mod tests;
