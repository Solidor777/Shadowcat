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

#[test]
fn floor_call_wrapping_dice_group_recurses_into_nested_arithmetic() {
    // `roll_expr`'s `Call` arm must recurse into `args` to find and roll a dice
    // group nested inside a `Bin` argument; `fold`'s `Call` arm must recurse the
    // same way and apply `apply_fn` over the computed value -- the underlying
    // d20's individual result must still be recoverable in `raws.records`, not
    // just folded away.
    let spec = notation::parse("floor(1d20/2)", total_ctx()).unwrap();
    let raws = roll(&spec, &mut NoiseRng::from_seed(3));
    let out = evaluate(&spec, &raws);
    assert_eq!(
        raws.records.len(),
        1,
        "the 1d20 group still produces exactly one record"
    );
    let d20_value = raws.records[0].value as i64;
    assert_eq!(out.total, d20_value / 2);
}

#[test]
fn min_call_across_two_dice_groups_threads_group_index_through_both_args() {
    let spec = notation::parse("min(2d6, 1d20)", total_ctx()).unwrap();
    // Seed chosen so g1 < g0 (verified: g0=8, g1=6) -- required for this test to
    // be non-vacuous. A `fold` that fails to thread `next_group` across `Call`
    // args (re-reading group 0 for both arguments instead of advancing to group
    // 1) would compute `apply_fn(Min, [g0, g0]) == g0`; with g1 < g0 the correct
    // answer `g0.min(g1) == g1` differs from that buggy result, so the bug is
    // caught. Under `g0 <= g1` the two coincide and this assertion alone would
    // pass either way -- see the record-count checks below for a seed-independent
    // guard against the same class of bug.
    let raws = roll(&spec, &mut NoiseRng::from_seed(3));
    let out = evaluate(&spec, &raws);
    let group0: Vec<i64> = raws
        .records
        .iter()
        .filter(|r| r.group_index == 0 && r.kept)
        .map(|r| r.value as i64)
        .collect();
    let group1: Vec<i64> = raws
        .records
        .iter()
        .filter(|r| r.group_index == 1 && r.kept)
        .map(|r| r.value as i64)
        .collect();
    // Seed-independent: proves `roll_expr` stamped each Call argument's dice
    // group with a distinct, correctly-advancing group_index.
    assert_eq!(group0.len(), 2, "2d6 stamped as group 0");
    assert_eq!(group1.len(), 1, "1d20 stamped as group 1");
    let g0: i64 = group0.iter().sum();
    let g1: i64 = group1.iter().sum();
    assert!(
        g1 < g0,
        "chosen seed must make g1 < g0 so a stuck fold cursor (re-reading group 0 \
         for both args) is distinguishable from the correct min: g0={g0} g1={g1}"
    );
    assert_eq!(out.total, g0.min(g1));
}

#[test]
fn call_wrapping_labeled_const_arg_surfaces_in_labeled_consts() {
    let spec = notation::parse("floor(3[dex] + 2)", total_ctx()).unwrap();
    let raws = roll(&spec, &mut NoiseRng::from_seed(1));
    let out = evaluate(&spec, &raws);
    assert_eq!(out.labeled_consts.len(), 1);
    assert_eq!(out.labeled_consts[0].value, 3);
    assert_eq!(out.labeled_consts[0].label, Some("dex".to_string()));
}
