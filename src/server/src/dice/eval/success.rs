use crate::dice::eval::classify;
use crate::dice::eval::crit;
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
    let mut base = 0i32;
    let (mut extra, mut lost) = (0i32, 0i32);
    let (mut pos, mut neg) = (0i32, 0i32);
    let (mut crit_s, mut crit_f) = (0i32, 0i32);
    for r in records.iter_mut().filter(|r| r.kept) {
        if cfg.success.comp.test(r.value, cfg.success.target) {
            base += 1;
        }
        let dc = crit::score_die(spec.direction, r.value, cfg);
        r.crit_success = dc.is_success;
        r.crit_fail = dc.is_fail;
        if dc.is_success {
            crit_s += 1;
        }
        if dc.is_fail {
            crit_f += 1;
        }
        extra += dc.extra_successes;
        lost += dc.lost;
        pos += dc.positive_counter;
        neg += dc.negative_counter;
    }
    let raw_net = base + extra - lost;
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::eval::{evaluate, roll};
    use crate::dice::outcome::DieRecord;
    use crate::dice::rng::NoiseRng;
    use crate::dice::spec::{
        Comparator, DiceGroup, DieId, DieKind, Direction, Expr, Mode, RollSpec, SuccessRule,
    };

    fn pool(count: u32) -> RollSpec {
        RollSpec {
            expr: Expr::Dice(DiceGroup {
                count,
                kind: DieKind::Numeric { min: 1, max: 10 },
                modifiers: vec![],
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(SuccessConfig {
                success: SuccessRule {
                    comp: Comparator::Gte,
                    target: 7,
                },
                required_successes: None,
                tiers: vec![],
                crit_success: None,
                crit_fail: None,
            }),
        }
    }

    fn cfg_of(spec: &RollSpec) -> &SuccessConfig {
        match &spec.mode {
            Mode::SuccessCount(cfg) => cfg,
            _ => panic!("expected SuccessCount mode"),
        }
    }

    #[test]
    fn crit_success_adds_extra_and_counter() {
        use crate::dice::spec::CritSuccess;
        // 1d10 with target>=7, crit_success at 10 (+1 extra, +1 pos counter).
        let cfg = SuccessConfig {
            success: SuccessRule {
                comp: Comparator::Gte,
                target: 7,
            },
            required_successes: None,
            tiers: vec![],
            crit_success: Some(CritSuccess {
                threshold: 10,
                extra_successes: 1,
                positive_counter: 1,
            }),
            crit_fail: None,
        };
        let spec = RollSpec {
            expr: Expr::Dice(DiceGroup {
                count: 1,
                kind: DieKind::Numeric { min: 10, max: 10 },
                modifiers: vec![],
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(cfg),
        };
        let raws = roll(&spec, &mut NoiseRng::from_seed(1));
        let out = evaluate_success(&spec, cfg_of(&spec), &raws);
        assert_eq!(out.successes, Some(2)); // base 1 + crit extra 1
        assert_eq!(out.positive_counter, 1);
        assert_eq!(out.crit_successes, 1);
        assert!(out.records[0].crit_success);
    }

    #[test]
    fn crit_fail_clamps_net_at_zero_unless_allowed() {
        use crate::dice::spec::CritFail;
        // Single die at min=max=1: base success fails, crit_fail loses 1.
        let mk = |allow: bool| RollSpec {
            expr: Expr::Dice(DiceGroup {
                count: 1,
                kind: DieKind::Numeric { min: 1, max: 1 },
                modifiers: vec![],
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(SuccessConfig {
                success: SuccessRule {
                    comp: Comparator::Gte,
                    target: 7,
                },
                required_successes: None,
                tiers: vec![],
                crit_success: None,
                crit_fail: Some(CritFail {
                    threshold: 1,
                    lost: 1,
                    negative_counter: 1,
                    allow_negative: allow,
                }),
            }),
        };
        let clamped = mk(false);
        let raws1 = roll(&clamped, &mut NoiseRng::from_seed(1));
        let o1 = evaluate_success(&clamped, cfg_of(&clamped), &raws1);
        assert_eq!(o1.successes, Some(0)); // 0 base - 1 lost, clamped at 0
        assert_eq!(o1.negative_counter, 1);
        assert_eq!(o1.crit_fails, 1);

        let neg = mk(true);
        let raws2 = roll(&neg, &mut NoiseRng::from_seed(1));
        let o2 = evaluate_success(&neg, cfg_of(&neg), &raws2);
        assert_eq!(o2.successes, Some(-1)); // allow_negative
    }

    #[test]
    fn counts_dice_at_or_above_target() {
        let spec = pool(6);
        let raws = roll(&spec, &mut NoiseRng::from_seed(30));
        let out = evaluate_success(&spec, cfg_of(&spec), &raws);
        let expected = raws
            .records
            .iter()
            .filter(|r| r.kept && r.value >= 7)
            .count() as i32;
        assert_eq!(out.successes, Some(expected));
        assert_eq!(out.pass, None);
        assert_eq!(out.margin, None);
    }

    #[test]
    fn required_successes_sets_pass_and_margin() {
        let mut spec = pool(6);
        if let Mode::SuccessCount(cfg) = &mut spec.mode {
            cfg.required_successes = Some(2);
        }
        let raws = roll(&spec, &mut NoiseRng::from_seed(31));
        let out = evaluate_success(&spec, cfg_of(&spec), &raws);
        let s = out.successes.unwrap();
        assert_eq!(out.pass, Some(s >= 2));
        assert_eq!(out.margin, Some((s - 2) as i64));
    }

    /// Builds a `RawRoll` whose `records` are hand-set (not RNG-derived) so the
    /// pool-level test can pin exact per-die values.
    fn manual_raws(values: &[i32]) -> RawRoll {
        let mut raws = RawRoll::default();
        raws.records = values
            .iter()
            .map(|&v| DieRecord {
                id: raws.dice.len() as DieId,
                group_index: 0,
                natural: v,
                value: v,
                kept: true,
                exploded: false,
                rerolled_from: None,
                crit_success: false,
                crit_fail: false,
            })
            .collect();
        raws
    }

    #[test]
    fn overlapping_crit_thresholds_fold_correctly_at_pool_level() {
        use crate::dice::spec::{CritFail, CritSuccess};

        // cs fires at value >= 5 (+2 extra, +1 pos counter each).
        // cf fires at value <= 10 (-1 lost, +1 neg counter each), not allow_negative.
        // die0 = 7  -> both fire (overlap region [5, 10]).
        // die1 = 15 -> only cs fires (cf requires <= 10).
        // die2 = 2  -> only cf fires (cs requires >= 5).
        let cfg = SuccessConfig {
            success: SuccessRule {
                comp: Comparator::Gte,
                target: 6,
            },
            required_successes: None,
            tiers: vec![],
            crit_success: Some(CritSuccess {
                threshold: 5,
                extra_successes: 2,
                positive_counter: 1,
            }),
            crit_fail: Some(CritFail {
                threshold: 10,
                lost: 1,
                negative_counter: 1,
                allow_negative: false,
            }),
        };
        let spec = RollSpec {
            expr: Expr::Dice(DiceGroup {
                count: 3,
                kind: DieKind::Numeric { min: 1, max: 20 },
                modifiers: vec![],
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(cfg.clone()),
        };
        let raws = manual_raws(&[7, 15, 2]);
        let out = evaluate_success(&spec, &cfg, &raws);

        // base successes (>= 6): die0 (7) and die1 (15) => 2.
        // extra: die0 + die1 both hit cs => 2 + 2 = 4.
        // lost: die0 + die2 both hit cf => 1 + 1 = 2.
        // net = 2 + 4 - 2 = 4 (already >= 0, clamp is a no-op here).
        assert_eq!(out.successes, Some(4));
        assert_eq!(out.crit_successes, 2);
        assert_eq!(out.crit_fails, 2);
        assert_eq!(out.positive_counter, 2);
        assert_eq!(out.negative_counter, 2);

        assert!(out.records[0].crit_success && out.records[0].crit_fail);
        assert!(out.records[1].crit_success && !out.records[1].crit_fail);
        assert!(!out.records[2].crit_success && out.records[2].crit_fail);
    }

    #[test]
    fn successcount_tier_over_net_margin() {
        use crate::dice::spec::Tier;
        // required 1, tiers: offset 0 = "success", offset 2 = "success+2 extra".
        let tiers = vec![
            Tier {
                margin_offset: 0,
                label: Some("success".into()),
                tier_value: Some(1),
            },
            Tier {
                margin_offset: 2,
                label: Some("great".into()),
                tier_value: Some(2),
            },
        ];
        // 3 dice all min=max=10, target>=7 -> 3 successes, required 1 -> margin 2 -> "great".
        let spec = RollSpec {
            expr: Expr::Dice(DiceGroup {
                count: 3,
                kind: DieKind::Numeric { min: 10, max: 10 },
                modifiers: vec![],
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(SuccessConfig {
                success: SuccessRule {
                    comp: Comparator::Gte,
                    target: 7,
                },
                required_successes: Some(1),
                tiers,
                crit_success: None,
                crit_fail: None,
            }),
        };
        let out = evaluate(&spec, &roll(&spec, &mut NoiseRng::from_seed(1)));
        assert_eq!(out.margin, Some(2));
        assert_eq!(out.tier_value, Some(2));
        assert!(out.pass.is_none(), "custom ladder reports tier, not pass");
    }

    #[test]
    fn required_successes_pass_margin_use_net_not_base() {
        use crate::dice::spec::CritSuccess;
        // Active crit config: die0/die1 = 10 both hit cs (+2 extra each), no crit_fail.
        // base successes (>= 6) = 2; net = base(2) + extra(4) = 6.
        // If pass/margin were computed over `base` instead of `net`, pass would be
        // false (2 >= 3) and margin would be -1, diverging from the correct (true, 3).
        let cfg = SuccessConfig {
            success: SuccessRule {
                comp: Comparator::Gte,
                target: 6,
            },
            required_successes: Some(3),
            tiers: vec![],
            crit_success: Some(CritSuccess {
                threshold: 10,
                extra_successes: 2,
                positive_counter: 0,
            }),
            crit_fail: None,
        };
        let spec = RollSpec {
            expr: Expr::Dice(DiceGroup {
                count: 3,
                kind: DieKind::Numeric { min: 1, max: 20 },
                modifiers: vec![],
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(cfg.clone()),
        };
        let raws = manual_raws(&[10, 10, 1]);
        let out = evaluate_success(&spec, &cfg, &raws);

        assert_eq!(out.successes, Some(6));
        assert_eq!(out.pass, Some(true));
        assert_eq!(out.margin, Some(3));
    }
}
