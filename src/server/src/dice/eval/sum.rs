#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::dice::eval::classify;
use crate::dice::outcome::{RawRoll, RollOutcome};
use crate::dice::spec::{BinOp, ConstTerm, Expr, FnName, RollSpec, TotalConfig};

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
    collect_labeled_consts(&spec.expr, 1, &mut labeled_consts);
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

/// Collects every labeled `Const` term, in AST left-to-right order, for
/// chat-embed display (`RollOutcome::labeled_consts`), carrying the term's
/// EFFECTIVE additive sign: negation (`Neg`) and the right side of `Sub` flip
/// it, so `-3[dex]` and `1d20 - 3[dex]` both display -3. Multiplicative
/// context (`Mul`/`Div`) is NOT folded in — a `2 * 3[dex]` displays its
/// literal 3, mirroring how a `DieRecord`'s raw face is shown regardless of
/// the arithmetic around its group. Total mode only; SuccessCount ignores the
/// arithmetic entirely.
fn collect_labeled_consts(expr: &Expr, sign: i32, out: &mut Vec<ConstTerm>) {
    match expr {
        Expr::Const(c) => {
            if c.label.is_some() {
                out.push(ConstTerm {
                    // saturating_neg: i32::MIN has no i32 negation.
                    value: if sign < 0 {
                        c.value.saturating_neg()
                    } else {
                        c.value
                    },
                    label: c.label.clone(),
                });
            }
        }
        Expr::Dice(_) => {}
        Expr::Neg(inner) => collect_labeled_consts(inner, -sign, out),
        Expr::Bin {
            op: BinOp::Sub,
            lhs,
            rhs,
        } => {
            collect_labeled_consts(lhs, sign, out);
            collect_labeled_consts(rhs, -sign, out);
        }
        Expr::Bin { lhs, rhs, .. } => {
            collect_labeled_consts(lhs, sign, out);
            collect_labeled_consts(rhs, sign, out);
        }
        Expr::Call { args, .. } => {
            for arg in args {
                collect_labeled_consts(arg, sign, out);
            }
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
/// counts zero, so `chat::rolls::MAX_ROLL_DICE`/`chat::rolls::MAX_ROLL_RECORDS` caps
/// never see it -- and a chain of `Mul`-combined dice groups (`1d10000 *
/// 1d10000 * ...`) can overflow `i64` even within those caps. Overflow here
/// is genuinely reachable, so these saturate silently (no `debug_assert!`).
/// `l + r`, saturating at the i64 rails instead of overflowing.
fn add_saturating(l: i64, r: i64) -> i64 {
    l.checked_add(r)
        .unwrap_or(if r >= 0 { i64::MAX } else { i64::MIN })
}

/// `l - r`, saturating at the i64 rails.
fn sub_saturating(l: i64, r: i64) -> i64 {
    l.checked_sub(r)
        .unwrap_or(if r <= 0 { i64::MAX } else { i64::MIN })
}

/// `l * r`, saturating toward the sign-correct rail.
fn mul_saturating(l: i64, r: i64) -> i64 {
    l.checked_mul(r).unwrap_or(if (l >= 0) == (r >= 0) {
        i64::MAX
    } else {
        i64::MIN
    })
}

/// Recursive Total-mode fold: consts as-is, dice groups as their kept-record
/// sums (consumed left-to-right via `next_group`), operators saturating, `Call`
/// nodes folding each argument in left-to-right order (the same cursor-threading
/// `Bin` uses, generalized to N children) then applying `apply_fn`.
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
                // Division by zero yields 0 — the ONLY guard; the parser
                // accepts a literal `/0`, so this branch is reachable from
                // untrusted notation.
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
        Expr::Call { name, args } => {
            let mut vals = Vec::with_capacity(args.len());
            for arg in args {
                vals.push(fold(arg, raws, next_group));
            }
            apply_fn(*name, &vals)
        }
    }
}

/// Applies a math function to its already-`fold`ed argument values. Indexes
/// defensively (`unwrap_or(0)` via the local `a` closure, mirroring `fold`'s own
/// Div-by-zero-to-0 convention) rather than panicking on an argument count that
/// disagrees with `name.arity()` — unreachable from `dice::notation::parser`-constructed
/// input (arity is checked at parse time there), but this crate's own types stay
/// unvalidated by design for a hand-constructed `RollSpec`.
/// `Floor`/`Ceil`/`Round` are true integer no-ops (`fold` never produces a fractional
/// value for any other `Expr` variant — `BinOp::Div` truncates toward zero rather than
/// producing a fraction), NOT a round-trip through `f64`: an `i64 -> f64` cast itself
/// loses precision above `f64`'s 53-bit mantissa (~9e15, well under `i64::MAX`), which
/// would silently corrupt a large already-integer input before any rounding function
/// ever ran. These three functions exist for functional parity with
/// `@shadowcat/formula`'s own function set; if a future change gives this grammar a
/// true fractional value, their implementation must be revisited then.
fn apply_fn(name: FnName, args: &[i64]) -> i64 {
    let a = |i: usize| args.get(i).copied().unwrap_or(0);
    match name {
        FnName::Floor | FnName::Ceil | FnName::Round => a(0),
        FnName::Abs => a(0).saturating_abs(),
        FnName::Min => a(0).min(a(1)),
        FnName::Max => a(0).max(a(1)),
    }
}

#[cfg(test)]
mod tests {
    use crate::dice::eval::{evaluate, roll};
    use crate::dice::notation::{self, ModeKind, ParseContext};
    use crate::dice::rng::NoiseRng;
    use crate::dice::spec::{
        BinOp, ConstTerm, DiceGroup, DieKind, Direction, Expr, FnName, Mode, RollSpec, Tier,
        TotalConfig,
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
                rhs: Box::new(Expr::Const(ConstTerm {
                    value: 3,
                    label: None,
                })),
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
                rhs: Box::new(Expr::Const(ConstTerm {
                    value: 2,
                    label: None,
                })),
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
                lhs: Box::new(Expr::Const(ConstTerm {
                    value: 2,
                    label: None,
                })),
                rhs: Box::new(Expr::Const(ConstTerm {
                    value: 3,
                    label: None,
                })),
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
    fn labeled_const_display_carries_effective_sign() {
        // Negation: "-3[dex]" displays -3.
        let spec = notation::parse("-3[dex]", total_ctx()).unwrap();
        let out = evaluate(&spec, &roll(&spec, &mut NoiseRng::from_seed(1)));
        assert_eq!(out.labeled_consts[0].value, -3);
        // Subtraction (the common authoring shape): "1d20 - 3[dex]" → -3.
        let spec = notation::parse("1d20 - 3[dex]", total_ctx()).unwrap();
        let out = evaluate(&spec, &roll(&spec, &mut NoiseRng::from_seed(1)));
        assert_eq!(out.labeled_consts[0].value, -3);
        // Double negation cancels: "1d20 - -3[dex]" → 3.
        let spec = notation::parse("1d20 - -3[dex]", total_ctx()).unwrap();
        let out = evaluate(&spec, &roll(&spec, &mut NoiseRng::from_seed(1)));
        assert_eq!(out.labeled_consts[0].value, 3);
        // Multiplication keeps the literal (documented): "2 * 3[dex]" → 3.
        let spec = notation::parse("2 * 3[dex]", total_ctx()).unwrap();
        let out = evaluate(&spec, &roll(&spec, &mut NoiseRng::from_seed(1)));
        assert_eq!(out.labeled_consts[0].value, 3);
    }

    #[test]
    fn additive_negative_labeled_constant_surfaces_a_correctly_signed_chip() {
        // "1d20 + -3[dex]" is the shape a template-rewrite substitution emits for a
        // negative resolved value: a unary minus directly before a labeled integer, after
        // a binary `+`. `factor` checks for `Token::Minus` regardless of what precedes it,
        // so this parses as `Bin{Add, Dice, Neg(Const{value: 3, label: "dex"})}`, and
        // `collect_labeled_consts` flips the sign through `Expr::Neg`.
        let spec = notation::parse("1d20 + -3[dex]", total_ctx()).unwrap();
        let raws = roll(&spec, &mut NoiseRng::from_seed(1));
        let out = evaluate(&spec, &raws);
        assert_eq!(out.labeled_consts.len(), 1);
        assert_eq!(out.labeled_consts[0].value, -3);
        assert_eq!(out.labeled_consts[0].label, Some("dex".to_string()));
        assert_eq!(out.total, out.records[0].value as i64 - 3);
    }

    #[test]
    fn dice_group_plus_labeled_constant_parses_and_surfaces_both() {
        // A dice group followed by an additive labeled constant parses, and the
        // labeled constant shows up in labeled_consts alongside the dice group's
        // own records.
        let spec = notation::parse("1d20 + 3[dex]", total_ctx()).unwrap();
        let raws = roll(&spec, &mut NoiseRng::from_seed(1));
        let out = evaluate(&spec, &raws);
        assert_eq!(
            out.records.len(),
            1,
            "the 1d20 group still produces one record"
        );
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
            expr: Expr::Const(ConstTerm {
                value: 12,
                label: None,
            }),
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
            expr: Expr::Const(ConstTerm {
                value: 12,
                label: None,
            }),
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
            expr: Expr::Const(ConstTerm {
                value: 17,
                label: None,
            }),
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

    #[test]
    fn apply_fn_floor_ceil_round_are_noops_over_integer_input() {
        assert_eq!(super::apply_fn(FnName::Floor, &[7]), 7);
        assert_eq!(super::apply_fn(FnName::Ceil, &[-3]), -3);
        assert_eq!(super::apply_fn(FnName::Round, &[4]), 4);
    }

    #[test]
    fn apply_fn_floor_ceil_round_preserve_precision_above_f64_mantissa() {
        // Regression: a naive `as f64` round-trip loses precision above f64's 53-bit
        // mantissa (~9e15), silently corrupting an already-integer value before any
        // rounding function runs. `apply_fn` must return these arguments unchanged.
        let big: i64 = 1999999999 * 1999999999; // 3999999996000000001 -- not exactly f64-representable
        assert_eq!(super::apply_fn(FnName::Floor, &[big]), big);
        assert_eq!(super::apply_fn(FnName::Ceil, &[big]), big);
        assert_eq!(super::apply_fn(FnName::Round, &[big]), big);
    }

    #[test]
    fn apply_fn_abs_min_max() {
        assert_eq!(super::apply_fn(FnName::Abs, &[-7]), 7);
        assert_eq!(super::apply_fn(FnName::Min, &[3, 5]), 3);
        assert_eq!(super::apply_fn(FnName::Max, &[3, 5]), 5);
    }

    #[test]
    fn apply_fn_defends_against_missing_args_instead_of_panicking() {
        // Unreachable from parser-constructed input (arity is checked at parse
        // time), but a hand-constructed `Expr::Call` with too few args must not
        // panic -- mirrors `fold`'s own Div-by-zero-to-0 convention.
        assert_eq!(super::apply_fn(FnName::Min, &[3]), 0); // missing arg defaults to 0
    }
}
