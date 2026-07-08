use crate::dice::eval::classify;
use crate::dice::outcome::{RawRoll, RollOutcome};
use crate::dice::spec::{BinOp, Expr, RollSpec, TotalConfig};

/// Fold the AST to a total. Each `Dice` node contributes the sum of its group's kept
/// records (matched by `group_index`); a cursor consumes groups in AST order. If
/// `cfg.difficulty` is set, classifies `total` (oriented by `spec.direction`) against
/// `cfg.tiers` into `margin`/`pass`/`tier_label`/`tier_value`; otherwise reports a bare total.
pub fn evaluate_total(spec: &RollSpec, cfg: &TotalConfig, raws: &RawRoll) -> RollOutcome {
    let mut next_group = 0usize;
    let total = fold(&spec.expr, raws, &mut next_group);
    let (pass, margin, tier_label, tier_value) = match cfg.difficulty {
        None => (None, None, None, None),
        Some(diff) => {
            let m = classify::oriented_margin(spec.direction, total, diff as i64);
            let c = classify::classify(m, &cfg.tiers);
            (c.pass, Some(m), c.tier_label, c.tier_value)
        }
    };
    RollOutcome {
        total,
        records: raws.records.clone(),
        successes: None,
        pass,
        margin,
        tier_label,
        tier_value,
        crit_successes: 0,
        crit_fails: 0,
        positive_counter: 0,
        negative_counter: 0,
    }
}

fn fold(expr: &Expr, raws: &RawRoll, next_group: &mut usize) -> i64 {
    match expr {
        Expr::Const(c) => *c as i64,
        Expr::Neg(inner) => -fold(inner, raws, next_group),
        Expr::Dice(_) => {
            let gi = *next_group;
            *next_group += 1;
            raws.records
                .iter()
                .filter(|r| r.group_index == gi && r.kept)
                .map(|r| r.value as i64)
                .sum()
        }
        Expr::Bin { op, lhs, rhs } => {
            let l = fold(lhs, raws, next_group);
            let r = fold(rhs, raws, next_group);
            match op {
                BinOp::Add => l + r,
                BinOp::Sub => l - r,
                BinOp::Mul => l * r,
                // Division by zero yields 0 (documented; parser rejects literal `/0`).
                BinOp::Div => {
                    if r == 0 {
                        0
                    } else {
                        l / r
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::dice::eval::{evaluate, roll};
    use crate::dice::rng::NoiseRng;
    use crate::dice::spec::{
        BinOp, DiceGroup, DieKind, Direction, Expr, Mode, RollSpec, Tier, TotalConfig,
    };

    fn total_mode() -> Mode {
        Mode::Total(TotalConfig {
            difficulty: None,
            tiers: vec![],
        })
    }

    fn ng(count: u32, min: i32, max: i32) -> Expr {
        Expr::Dice(DiceGroup {
            label: None,
            count,
            kind: DieKind::Numeric { min, max },
            modifiers: vec![],
        })
    }

    #[test]
    fn sum_two_dice_plus_constant() {
        let spec = RollSpec {
            expr: Expr::Bin {
                op: BinOp::Add,
                lhs: Box::new(ng(2, 1, 6)),
                rhs: Box::new(Expr::Const(3)),
            },
            direction: Direction::HighWins,
            mode: total_mode(),
        };
        let raws = roll(&spec, &mut NoiseRng::from_seed(20));
        let out = evaluate(&spec, &raws);
        let dice_sum: i64 = raws
            .records
            .iter()
            .filter(|r| r.kept)
            .map(|r| r.value as i64)
            .sum();
        assert_eq!(out.total, dice_sum + 3);
        assert_eq!(out.successes, None);
    }

    #[test]
    fn sum_multiplication() {
        let spec = RollSpec {
            expr: Expr::Bin {
                op: BinOp::Mul,
                lhs: Box::new(ng(1, 1, 4)),
                rhs: Box::new(Expr::Const(2)),
            },
            direction: Direction::HighWins,
            mode: total_mode(),
        };
        let raws = roll(&spec, &mut NoiseRng::from_seed(21));
        let out = evaluate(&spec, &raws);
        let d: i64 = raws
            .records
            .iter()
            .filter(|r| r.kept)
            .map(|r| r.value as i64)
            .sum();
        assert_eq!(out.total, d * 2);
    }

    #[test]
    fn sum_two_groups_fold_independently() {
        // 1d6 + 1d6: each group folds its own kept sum; group_index disambiguates.
        // Weak by itself: Add is commutative, so a broken group cursor that never
        // advances (always reading group 0) would still coincidentally pass. See
        // `sum_two_groups_subtraction_is_order_sensitive` for the boundary-sensitive
        // variant.
        let spec = RollSpec {
            expr: Expr::Bin {
                op: BinOp::Add,
                lhs: Box::new(ng(1, 1, 6)),
                rhs: Box::new(ng(1, 1, 6)),
            },
            direction: Direction::HighWins,
            mode: total_mode(),
        };
        let raws = roll(&spec, &mut NoiseRng::from_seed(22));
        let out = evaluate(&spec, &raws);
        let all: i64 = raws
            .records
            .iter()
            .filter(|r| r.kept)
            .map(|r| r.value as i64)
            .sum();
        assert_eq!(out.total, all);
    }

    #[test]
    fn sum_two_groups_subtraction_is_order_sensitive() {
        // 1d6 - 1d8: unlike Add, Sub is non-commutative, so a broken group cursor
        // (e.g. one that never advances past group 0, or swaps lhs/rhs group
        // assignment) changes the numeric result instead of coincidentally
        // matching. `fold` visits lhs before rhs, so the lhs Dice node must land
        // on group_index 0 and rhs on group_index 1 — exactly mirroring the
        // stamping order `roll_expr` already applied when producing `raws`.
        let spec = RollSpec {
            expr: Expr::Bin {
                op: BinOp::Sub,
                lhs: Box::new(ng(1, 1, 6)),
                rhs: Box::new(ng(1, 1, 8)),
            },
            direction: Direction::HighWins,
            mode: total_mode(),
        };
        let raws = roll(&spec, &mut NoiseRng::from_seed(42));
        let out = evaluate(&spec, &raws);
        // Independent expected-value derivation: read `raws.records` directly by
        // `group_index`, bypassing `fold` entirely, then compare against `fold`'s
        // actual output. This does not merely "trust" `fold`'s own bookkeeping.
        let lhs_sum: i64 = raws
            .records
            .iter()
            .filter(|r| r.group_index == 0 && r.kept)
            .map(|r| r.value as i64)
            .sum();
        let rhs_sum: i64 = raws
            .records
            .iter()
            .filter(|r| r.group_index == 1 && r.kept)
            .map(|r| r.value as i64)
            .sum();
        // Seed(42) must produce distinct lhs/rhs sums, or a swapped-order bug
        // (rhs - lhs) would coincidentally match too.
        assert_ne!(
            lhs_sum, rhs_sum,
            "chosen seed must make lhs != rhs so a swapped-order bug is detectable"
        );
        assert_eq!(out.total, lhs_sum - rhs_sum);
    }

    #[test]
    fn sum_with_no_dice_nodes_does_not_panic() {
        // Expr::Bin{Add, Const(2), Const(3)} with zero Dice nodes: `fold`'s
        // `next_group` cursor must never be dereferenced against `raws.records`
        // when there is nothing to look up.
        let spec = RollSpec {
            expr: Expr::Bin {
                op: BinOp::Add,
                lhs: Box::new(Expr::Const(2)),
                rhs: Box::new(Expr::Const(3)),
            },
            direction: Direction::HighWins,
            mode: total_mode(),
        };
        let raws = roll(&spec, &mut NoiseRng::from_seed(1));
        assert!(raws.records.is_empty(), "no Dice nodes -> no records");
        let out = evaluate(&spec, &raws);
        assert_eq!(out.total, 5);
    }

    #[test]
    fn total_no_difficulty_reports_bare_total() {
        let spec = RollSpec {
            expr: Expr::Const(12),
            direction: Direction::HighWins,
            mode: Mode::Total(TotalConfig {
                difficulty: None,
                tiers: vec![],
            }),
        };
        let raws = roll(&spec, &mut NoiseRng::from_seed(7));
        let out = evaluate(&spec, &raws);
        assert!(out.pass.is_none());
        assert!(out.tier_label.is_none());
        assert!(out.margin.is_none());
    }

    #[test]
    fn total_with_difficulty_sets_pass_by_direction() {
        let spec_hi = RollSpec {
            expr: Expr::Const(12),
            direction: Direction::HighWins,
            mode: Mode::Total(TotalConfig {
                difficulty: Some(10),
                tiers: vec![],
            }),
        };
        let raws = roll(&spec_hi, &mut NoiseRng::from_seed(1));
        let out = evaluate(&spec_hi, &raws);
        assert_eq!(out.margin, Some(2));
        assert_eq!(out.pass, Some(true)); // 12 >= 10

        let spec_lo = RollSpec {
            direction: Direction::LowWins,
            ..spec_hi.clone()
        };
        let out_lo = evaluate(&spec_lo, &roll(&spec_lo, &mut NoiseRng::from_seed(1)));
        assert_eq!(out_lo.margin, Some(-2)); // roll-under: 12 vs 10 -> 10-12
        assert_eq!(out_lo.pass, Some(false));
    }

    #[test]
    fn total_with_ladder_reports_tier() {
        let tiers = vec![
            Tier {
                margin_offset: 0,
                label: Some("hit".into()),
                tier_value: Some(1),
            },
            Tier {
                margin_offset: 5,
                label: Some("crit".into()),
                tier_value: Some(2),
            },
        ];
        let spec = RollSpec {
            expr: Expr::Const(17),
            direction: Direction::HighWins,
            mode: Mode::Total(TotalConfig {
                difficulty: Some(10),
                tiers,
            }),
        };
        let out = evaluate(&spec, &roll(&spec, &mut NoiseRng::from_seed(1)));
        assert_eq!(out.tier_value, Some(2)); // margin 7 -> highest rung <= 7 is offset 5
        assert!(out.pass.is_none());
    }

    #[test]
    fn unordered_faces_die_contributes_zero_to_total() {
        use crate::dice::spec::{DiceGroup, DieKind, Face};
        let spec = RollSpec {
            expr: Expr::Dice(DiceGroup {
                count: 1,
                kind: DieKind::Faces {
                    faces: vec![Face {
                        value: None,
                        symbols: vec!["x".into()],
                    }],
                },
                modifiers: vec![],
                label: None,
            }),
            direction: Direction::HighWins,
            mode: total_mode(),
        };
        let raws = roll(&spec, &mut NoiseRng::from_seed(1));
        let out = evaluate(&spec, &raws);
        assert_eq!(out.total, 0);
    }
}
