#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::collections::BTreeMap;

use crate::dice::eval::classify;
use crate::dice::eval::crit;
use crate::dice::eval::expertise;
use crate::dice::outcome::{RawRoll, RollOutcome};
use crate::dice::spec::{RollSpec, SuccessConfig};

/// Pool aggregation: count kept dice satisfying `cfg.success`, then fold each kept
/// die's crit event (`cfg.crit_success`/`cfg.crit_fail`) into net successes and the
/// positive/negative counters. Pools ALL kept records across every group —
/// SuccessCount mode ignores the AST arithmetic. Net successes clamp at 0 unless
/// `cfg.crit_fail.allow_negative` opts out of the clamp.
///
/// `required_successes`/`tiers` classify over `net - req` via the shared
/// `eval::classify`. Unlike Total mode, this margin is NOT run through
/// `oriented_margin`/direction: more successes is always better, and `direction`
/// was already applied per-die inside `crit::score_die`.
pub fn evaluate_success(spec: &RollSpec, cfg: &SuccessConfig, raws: &RawRoll) -> RollOutcome {
    let mut records = raws.records.clone();
    if cfg.expertise > 0 {
        expertise::allocate(spec.direction, cfg, raws, &mut records);
    }
    let mut raw_net = 0i32;
    let (mut pos, mut neg) = (0i32, 0i32);
    let (mut crit_s, mut crit_f) = (0i32, 0i32);
    let mut symbol_counts: BTreeMap<crate::dice::spec::Symbol, i32> = BTreeMap::new();
    for r in records.iter_mut().filter(|r| r.kept) {
        for s in &r.symbols {
            *symbol_counts.entry(s.clone()).or_insert(0) += 1;
        }
        let scored = crit::score_die_net(spec.direction, cfg, r.value, &r.symbols);
        raw_net += scored.net();
        let dc = scored.crit;
        r.crit_success = dc.is_success;
        r.crit_fail = dc.is_fail;
        if dc.is_success {
            crit_s += 1;
        }
        if dc.is_fail {
            crit_f += 1;
        }
        pos += dc.positive_counter;
        neg += dc.negative_counter;
    }
    let allow_neg = cfg
        .crit_fail
        .as_ref()
        .map(|c| c.allow_negative)
        .unwrap_or(false);
    let net = if allow_neg { raw_net } else { raw_net.max(0) };
    let total: i64 = records
        .iter()
        .filter(|r| r.kept)
        .map(|r| r.value as i64)
        .sum();
    let (pass, margin, tier_label, tier_value) = match cfg.required_successes {
        None => (None, None, None, None),
        Some(req) => {
            // Direction is NOT applied here: more successes is always better.
            let m = (net - req) as i64;
            let c = classify::classify(m, &cfg.tiers);
            (c.pass, Some(m), c.tier_label, c.tier_value)
        }
    };
    RollOutcome {
        total,
        records,
        successes: Some(net),
        pass,
        margin,
        tier_label,
        tier_value,
        crit_successes: crit_s,
        crit_fails: crit_f,
        positive_counter: pos,
        negative_counter: neg,
        symbol_counts,
        // SuccessCount ignores the AST arithmetic entirely (pools dice by
        // group membership, not by expression structure), so there is no
        // notion of a "labeled Const term" here — always empty, mirroring
        // how this mode never threads a Const's label into by_label either.
        labeled_consts: Vec::new(),
    }
}

#[cfg(test)]
mod tests;
