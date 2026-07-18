use crate::dice::eval::classify;
use crate::dice::outcome::{RawRoll, RollOutcome};
use crate::dice::spec::{BinOp, ConstTerm, Expr, RollSpec, TotalConfig};

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
    let mut labeled_consts = Vec::new();
    collect_labeled_consts(&spec.expr, &mut labeled_consts);
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
        symbol_counts: Default::default(),
        labeled_consts,
    }
}

/// Collects every labeled `Const` term in the expression, in AST left-to-right
/// order, for chat-embed display (`RollOutcome::labeled_consts`). Mirrors how a
/// `DieRecord`'s value is shown independent of the arithmetic operator around its
/// group (e.g. a `1d6 - 1d8` shows both groups' raw positive rolled values, never
/// negated) — a labeled constant's displayed `value` is likewise its own literal,
/// unaffected by an enclosing `Neg`/`Bin` operator. Total mode only; SuccessCount
/// ignores the arithmetic entirely (see `success::evaluate_success`), so this is
/// never called for that mode.
fn collect_labeled_consts(expr: &Expr, out: &mut Vec<ConstTerm>) {
    match expr {
        Expr::Const(c) => {
            if c.label.is_some() {
                out.push(c.clone());
            }
        }
        Expr::Dice(_) => {}
        Expr::Neg(inner) => collect_labeled_consts(inner, out),
        Expr::Bin { lhs, rhs, .. } => {
            collect_labeled_consts(lhs, out);
            collect_labeled_consts(rhs, out);
        }
    }
}

/// Accumulates via `checked_add`, saturating at `i64::MAX`/`MIN` instead of
/// panicking on overflow. Unreachable while the chat boundary's roll-record
/// cap bounds a group's kept-die count well below what it takes to overflow
/// `i64`; guard is defense-in-depth against a pathological dice-group count.
fn checked_add_saturating(acc: i64, next: i64) -> i64 {
    match acc.checked_add(next) {
        Some(v) => v,
        None => {
            debug_assert!(false, "dice group total overflowed i64");
            if next >= 0 {
                i64::MAX
            } else {
                i64::MIN
            }
        }
    }
}

/// Saturating `Add`/`Sub`/`Mul` for `Expr::Bin` folds. Unlike
/// `checked_add_saturating` above (whose overflow is unreachable given the
/// chat boundary's per-group record cap), a pure-`Const` arithmetic chain
/// (`2000000000*2000000000*3`) has NO dice groups at all -- `walk_groups`
/// counts zero, so `chat/rolls.rs`'s `MAX_ROLL_DICE`/`MAX_ROLL_RECORDS` caps
/// never see it -- and a chain of `Mul`-combined dice groups (`1d10000 *
/// 1d10000 * ...`) can overflow `i64` even within those caps. Overflow here
/// is genuinely reachable, so these saturate silently (no `debug_assert!`).
fn add_saturating(l: i64, r: i64) -> i64 {
    l.checked_add(r)
        .unwrap_or(if r >= 0 { i64::MAX } else { i64::MIN })
}

fn sub_saturating(l: i64, r: i64) -> i64 {
    l.checked_sub(r)
        .unwrap_or(if r <= 0 { i64::MAX } else { i64::MIN })
}

fn mul_saturating(l: i64, r: i64) -> i64 {
    l.checked_mul(r).unwrap_or(if (l >= 0) == (r >= 0) {
        i64::MAX
    } else {
        i64::MIN
    })
}

fn fold(expr: &Expr, raws: &RawRoll, next_group: &mut usize) -> i64 {
    match expr {
        Expr::Const(c) => c.value as i64,
        // `i64::MIN.checked_neg()` is `None` (its magnitude has no positive
        // i64 representation); saturate to `i64::MAX` instead of the raw `-`
        // negation, which is checked-overflow (panics) even in release.
        Expr::Neg(inner) => fold(inner, raws, next_group)
            .checked_neg()
            .unwrap_or(i64::MAX),
        Expr::Dice(_) => {
            let gi = *next_group;
            *next_group += 1;
            raws.records
                .iter()
                .filter(|r| r.group_index == gi && r.kept)
                .map(|r| r.value as i64)
                .fold(0i64, checked_add_saturating)
        }
        Expr::Bin { op, lhs, rhs } => {
            let l = fold(lhs, raws, next_group);
            let r = fold(rhs, raws, next_group);
            match op {
                BinOp::Add => add_saturating(l, r),
                BinOp::Sub => sub_saturating(l, r),
                BinOp::Mul => mul_saturating(l, r),
                // Division by zero yields 0 (documented; parser rejects literal `/0`).
                BinOp::Div => {
                    if r == 0 {
                        0
                    } else {
                        // `i64::MIN / -1` is the one division that overflows i64
                        // (its magnitude has no positive i64 representation) --
                        // Rust's `/` is checked-overflow even in release, which
                        // would panic and abort the whole process under
                        // panic=abort. Reachable from untrusted chat input via a
                        // pure-const chain (`mul_saturating` can produce an exact
                        // `i64::MIN`, and `-1` comes from `Expr::Neg`), with zero
                        // dice groups so the roll-count caps never see it.
                        l.checked_div(r).unwrap_or(i64::MAX)
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::dice::eval::{evaluate, roll};
    use crate::dice::notation::{self, ModeKind, ParseContext};
    use crate::dice::rng::NoiseRng;
    use crate::dice::spec::{
        BinOp, ConstTerm, DiceGroup, DieKind, Direction, Expr, Mode, RollSpec, Tier, TotalConfig,
    };

    fn total_ctx() -> ParseContext {
        ParseContext {
            mode: ModeKind::Total,
            direction: Direction::HighWins,
        }
    }

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
                rhs: Box::new(Expr::Const(ConstTerm { value: 3, label: None })),
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
                rhs: Box::new(Expr::Const(ConstTerm { value: 2, label: None })),
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
                lhs: Box::new(Expr::Const(ConstTerm { value: 2, label: None })),
                rhs: Box::new(Expr::Const(ConstTerm { value: 3, label: None })),
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
    fn labeled_bare_constant_surfaces_in_labeled_consts() {
        // "3[dex]" alone: the root-cause bug (label rejected on a non-dice term)
        // fixed at the parser; here we verify the fixed shape is ALSO surfaced for
        // chat-embed rendering, not merely accepted-but-invisible.
        let spec = notation::parse("3[dex]", total_ctx()).unwrap();
        let raws = roll(&spec, &mut NoiseRng::from_seed(1));
        let out = evaluate(&spec, &raws);
        assert_eq!(out.total, 3);
        assert_eq!(out.labeled_consts.len(), 1);
        assert_eq!(out.labeled_consts[0].value, 3);
        assert_eq!(out.labeled_consts[0].label, Some("dex".to_string()));
    }

    #[test]
    fn dice_group_plus_labeled_constant_parses_and_surfaces_both() {
        // The exact failing case from the e2e report: "1d20 + 3[dex]" must parse
        // (no longer a trailing-input error) and the labeled constant must show up
        // in labeled_consts alongside the dice group's own records.
        let spec = notation::parse("1d20 + 3[dex]", total_ctx()).unwrap();
        let raws = roll(&spec, &mut NoiseRng::from_seed(1));
        let out = evaluate(&spec, &raws);
        assert_eq!(out.records.len(), 1, "the 1d20 group still produces one record");
        assert_eq!(out.labeled_consts.len(), 1);
        assert_eq!(out.labeled_consts[0].value, 3);
        assert_eq!(out.labeled_consts[0].label, Some("dex".to_string()));
        assert_eq!(out.total, out.records[0].value as i64 + 3);
    }

    #[test]
    fn unlabeled_constant_does_not_appear_in_labeled_consts() {
        let spec = RollSpec {
            expr: Expr::Const(ConstTerm {
                value: 7,
                label: None,
            }),
            direction: Direction::HighWins,
            mode: total_mode(),
        };
        let raws = roll(&spec, &mut NoiseRng::from_seed(1));
        let out = evaluate(&spec, &raws);
        assert!(out.labeled_consts.is_empty());
    }

    #[test]
    fn total_no_difficulty_reports_bare_total() {
        let spec = RollSpec {
            expr: Expr::Const(ConstTerm { value: 12, label: None }),
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
            expr: Expr::Const(ConstTerm { value: 12, label: None }),
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
            expr: Expr::Const(ConstTerm { value: 17, label: None }),
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
    #[should_panic(expected = "dice group total overflowed")]
    fn checked_add_saturating_debug_asserts_on_overflow() {
        // Debug/test builds have debug_assertions on, so the guard's
        // debug_assert! fires before the saturating fallback would run;
        // proves the guard actually triggers at the boundary.
        super::checked_add_saturating(i64::MAX, 1);
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

    #[test]
    fn div_i64_min_by_neg_one_saturates_without_panic() {
        // `2000000000*2000000000*-3` folds via `mul_saturating` to an exact
        // `i64::MIN` (sign-differing overflow: 4e18 * -3), then `/-1` is the
        // one division that overflows i64 -- Rust's `/` is checked-overflow
        // even in release, so this would panic (and, with panic=abort,
        // abort the whole process) without the `checked_div` guard. Zero
        // dice groups, so the chat-boundary roll-count caps never apply;
        // run under the debug profile (overflow-checks on) so reaching a
        // result at all proves no panic.
        let spec = notation::parse("2000000000*2000000000*-3/-1", total_ctx()).unwrap();
        let raws = roll(&spec, &mut NoiseRng::from_seed(1));
        let out = evaluate(&spec, &raws);
        assert_eq!(out.total, i64::MAX);
    }

    #[test]
    fn neg_over_i64_min_folding_subexpression_saturates_without_panic() {
        // `2000000000*2000000000*-2000000000` folds via `mul_saturating` to
        // an exact `i64::MIN`; negating that with raw `-` overflows (no
        // positive i64 can represent `i64::MIN`'s magnitude) and panics even
        // in release. `checked_neg` must saturate to `i64::MAX` instead.
        let spec = notation::parse("-(2000000000*2000000000*-2000000000)", total_ctx()).unwrap();
        let raws = roll(&spec, &mut NoiseRng::from_seed(1));
        let out = evaluate(&spec, &raws);
        assert_eq!(out.total, i64::MAX);
    }
}
