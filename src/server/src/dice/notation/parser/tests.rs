use super::parse;
use crate::dice::notation::{ModeKind, ParseContext, ParseError};
use crate::dice::spec::*;

fn dice(count: u32, min: i32, max: i32, mods: Vec<GroupModifier>) -> Expr {
    Expr::Dice(DiceGroup {
        label: None,
        count,
        kind: DieKind::Numeric { min, max },
        modifiers: mods,
    })
}

#[test]
fn parses_keep_highest_plus_const() {
    let spec = parse("4d6kh3+2", ParseContext::default()).unwrap();
    assert!(matches!(spec.mode, Mode::Total(_)));
    assert_eq!(
        spec.expr,
        Expr::Bin {
            op: BinOp::Add,
            lhs: Box::new(dice(4, 1, 6, vec![GroupModifier::KeepHighest(3)])),
            rhs: Box::new(Expr::Const(ConstTerm {
                value: 2,
                label: None,
            })),
        }
    );
}

#[test]
fn parses_success_pool() {
    let spec = parse("5d10cs>=7", ParseContext::default()).unwrap();
    match &spec.mode {
        Mode::SuccessCount(cfg) => assert_eq!(
            cfg.success,
            SuccessRule::Numeric {
                comp: Comparator::Gte,
                target: 7
            }
        ),
        other => panic!("expected SuccessCount mode, got {other:?}"),
    }
    assert_eq!(spec.expr, dice(5, 1, 10, vec![]));
}

#[test]
fn rejects_duplicate_success_rule_across_groups() {
    // `success` is shared parser state (one RollSpec, not per-DiceGroup); a
    // second cs/cf anywhere in the expression must error rather than silently
    // overwrite the first rule (last-write-wins data loss).
    match parse("4d6cs>=5+2d8cs>=3", ParseContext::default()) {
        Err(ParseError::DuplicateSuccessRule) => {}
        other => panic!("expected DuplicateSuccessRule, got {other:?}"),
    }
}

#[test]
fn parses_explode_default_target_is_die_max() {
    let spec = parse("6d6!", ParseContext::default()).unwrap();
    match spec.expr {
        Expr::Dice(g) => assert_eq!(
            g.modifiers[0],
            GroupModifier::Explode {
                kind: ExplodeKind::Standard,
                comp: Comparator::Gte,
                target: 6
            }
        ),
        _ => panic!("expected dice"),
    }
}

#[test]
fn parses_reroll() {
    let spec = parse("6d6r<2", ParseContext::default()).unwrap();
    match spec.expr {
        Expr::Dice(g) => assert!(matches!(
            g.modifiers[0],
            GroupModifier::Reroll {
                once: false,
                comp: Comparator::Lt,
                target: 2
            }
        )),
        _ => panic!("expected dice"),
    }
}

#[test]
fn parses_parentheses_and_mul() {
    assert!(matches!(
        parse("(1d4+1)*2", ParseContext::default()).unwrap().mode,
        Mode::Total(_)
    ));
}

#[test]
fn rejects_empty_and_trailing() {
    assert!(parse("", ParseContext::default()).is_err());
    assert!(parse("2d6 2d6", ParseContext::default()).is_err());
}

#[test]
fn rejects_zero_sides() {
    // sides < 1 must be a parse-time Err, never a constructed DieKind::Numeric
    // with a degenerate (non-positive-span) range.
    match parse("4d0", ParseContext::default()) {
        Err(ParseError::InvalidDieSides(0)) => {}
        other => panic!("expected InvalidDieSides(0), got {other:?}"),
    }
}

#[test]
fn rejects_negative_sides_token_sequence() {
    // The lexer never emits a signed Int for "d-3" (Minus and Int are separate
    // tokens), so this fails as an ordinary unexpected-token error rather than
    // InvalidDieSides -- still a hard Err, never a constructed invalid DieKind.
    assert!(parse("4d-3", ParseContext::default()).is_err());
}

#[test]
fn t_target_in_total_mode_sets_difficulty() {
    let spec = parse("1d20t10", ParseContext::default()).unwrap();
    match spec.mode {
        Mode::Total(c) => assert_eq!(c.difficulty, Some(10)),
        _ => panic!(),
    }
}

#[test]
fn t_target_in_successcount_uses_direction_comparator() {
    let hi = parse(
        "5d10t7",
        ParseContext {
            mode: ModeKind::SuccessCount,
            direction: Direction::HighWins,
        },
    )
    .unwrap();
    match hi.mode {
        Mode::SuccessCount(c) => assert_eq!(
            c.success,
            SuccessRule::Numeric {
                comp: Comparator::Gte,
                target: 7
            }
        ),
        _ => panic!(),
    }
    let lo = parse(
        "5d10t7",
        ParseContext {
            mode: ModeKind::SuccessCount,
            direction: Direction::LowWins,
        },
    )
    .unwrap();
    match lo.mode {
        Mode::SuccessCount(c) => assert_eq!(
            c.success,
            SuccessRule::Numeric {
                comp: Comparator::Lte,
                target: 7
            }
        ),
        _ => panic!(),
    }
}

#[test]
fn cs_forces_successcount_even_under_total_ambient() {
    let spec = parse(
        "5d10cs>=7",
        ParseContext {
            mode: ModeKind::Total,
            direction: Direction::HighWins,
        },
    )
    .unwrap();
    assert!(matches!(spec.mode, Mode::SuccessCount(_)));
}

#[test]
fn t_and_cs_collision_errors() {
    let e = parse(
        "5d10t6cs>=7",
        ParseContext {
            mode: ModeKind::SuccessCount,
            direction: Direction::HighWins,
        },
    );
    assert!(matches!(e, Err(ParseError::DuplicateSuccessRule)));
}

#[test]
fn successcount_without_target_or_rule_errors() {
    // Ambient SuccessCount with neither a cs/cf rule nor a t<N> target leaves
    // no per-die comparator to build a SuccessRule from -- must hard-error
    // rather than silently default (the (None, None) arm in `parse`).
    let e = parse(
        "5d10",
        ParseContext {
            mode: ModeKind::SuccessCount,
            direction: Direction::HighWins,
        },
    );
    assert!(matches!(e, Err(ParseError::Unexpected(_))));
}

#[test]
fn e_token_sets_expertise_under_successcount() {
    let spec = parse(
        "4d6t5e3",
        ParseContext {
            mode: ModeKind::SuccessCount,
            direction: Direction::HighWins,
        },
    )
    .unwrap();
    match spec.mode {
        Mode::SuccessCount(c) => assert_eq!(c.expertise, 3),
        other => panic!("expected SuccessCount, got {other:?}"),
    }
}

#[test]
fn e_token_is_discarded_under_total_ambient_without_error() {
    // A stray e<N> where the mode can't use it must NOT fail the roll.
    let spec = parse("1d20t10e3", ParseContext::default()).unwrap(); // ambient Total
    match spec.mode {
        Mode::Total(c) => assert_eq!(c.difficulty, Some(10)),
        other => panic!("expected Total, got {other:?}"),
    }
}

#[test]
fn duplicate_e_token_errors() {
    let e = parse(
        "4d6t5e3e4",
        ParseContext {
            mode: ModeKind::SuccessCount,
            direction: Direction::HighWins,
        },
    );
    assert!(matches!(e, Err(ParseError::DuplicateExpertise)));
}

#[test]
fn rs_token_sets_required_successes_under_successcount() {
    let spec = parse(
        "4d6t5rs2",
        ParseContext {
            mode: ModeKind::SuccessCount,
            direction: Direction::HighWins,
        },
    )
    .unwrap();
    match spec.mode {
        Mode::SuccessCount(c) => assert_eq!(c.required_successes, Some(2)),
        other => panic!("expected SuccessCount, got {other:?}"),
    }
}

#[test]
fn rs_token_is_discarded_under_total_ambient_without_error() {
    // A stray rs<N> where the mode can't use it must NOT fail the roll -- mirrors
    // e<N>'s exact silent-drop-under-Total precedent.
    let spec = parse("1d20t10rs2", ParseContext::default()).unwrap(); // ambient Total
    match spec.mode {
        Mode::Total(c) => assert_eq!(c.difficulty, Some(10)),
        other => panic!("expected Total, got {other:?}"),
    }
}

#[test]
fn duplicate_rs_token_errors() {
    let e = parse(
        "4d6t5rs2rs3",
        ParseContext {
            mode: ModeKind::SuccessCount,
            direction: Direction::HighWins,
        },
    );
    assert!(matches!(e, Err(ParseError::DuplicateRequiredSuccesses)));
}

#[test]
fn parses_label_onto_dice_group() {
    let spec = parse("1d12[Hope]", ParseContext::default()).unwrap();
    match spec.expr {
        Expr::Dice(g) => assert_eq!(g.label, Some("Hope".to_string())),
        _ => panic!("expected dice"),
    }
}

#[test]
fn parses_two_labeled_groups() {
    let spec = parse("1d12[Hope] + 1d12[Fear]", ParseContext::default()).unwrap();
    match spec.expr {
        Expr::Bin { lhs, rhs, .. } => {
            match *lhs {
                Expr::Dice(g) => assert_eq!(g.label, Some("Hope".to_string())),
                _ => panic!("expected dice lhs"),
            }
            match *rhs {
                Expr::Dice(g) => assert_eq!(g.label, Some("Fear".to_string())),
                _ => panic!("expected dice rhs"),
            }
        }
        _ => panic!("expected Bin"),
    }
}

#[test]
fn duplicate_labels_across_groups_are_not_an_error() {
    // Two groups intentionally sharing a label pool under by_label — not a parse error.
    assert!(parse("1d6[Pool] + 1d6[Pool]", ParseContext::default()).is_ok());
}

#[test]
fn parses_label_onto_bare_constant() {
    // The root-cause bug: a label immediately after a bare constant (not a dice
    // group) must be consumed by `factor()`, not left as unconsumed trailing input.
    let spec = parse("3[dex]", ParseContext::default()).unwrap();
    match spec.expr {
        Expr::Const(c) => {
            assert_eq!(c.value, 3);
            assert_eq!(c.label, Some("dex".to_string()));
        }
        other => panic!("expected Const, got {other:?}"),
    }
}

#[test]
fn parses_dice_group_plus_labeled_constant() {
    // A dice group followed by an additive labeled constant parses as a binary
    // expression, never as trailing input.
    let spec = parse("1d20 + 3[dex]", ParseContext::default()).unwrap();
    match spec.expr {
        Expr::Bin { lhs, rhs, .. } => {
            assert!(matches!(*lhs, Expr::Dice(_)));
            match *rhs {
                Expr::Const(c) => {
                    assert_eq!(c.value, 3);
                    assert_eq!(c.label, Some("dex".to_string()));
                }
                other => panic!("expected Const rhs, got {other:?}"),
            }
        }
        other => panic!("expected Bin, got {other:?}"),
    }
}

#[test]
fn unlabeled_bare_constant_has_no_label() {
    let spec = parse("3", ParseContext::default()).unwrap();
    match spec.expr {
        Expr::Const(c) => {
            assert_eq!(c.value, 3);
            assert_eq!(c.label, None);
        }
        other => panic!("expected Const, got {other:?}"),
    }
}

#[test]
fn parses_min_call_with_two_bare_consts() {
    let spec = parse("min(3,5)", ParseContext::default()).unwrap();
    assert_eq!(
        spec.expr,
        Expr::Call {
            name: FnName::Min,
            args: vec![
                Expr::Const(ConstTerm {
                    value: 3,
                    label: None
                }),
                Expr::Const(ConstTerm {
                    value: 5,
                    label: None
                }),
            ],
        }
    );
}

#[test]
fn parses_floor_call_wrapping_a_dice_group() {
    let spec = parse("floor(1d20/2)", ParseContext::default()).unwrap();
    match spec.expr {
        Expr::Call {
            name: FnName::Floor,
            args,
        } => {
            assert_eq!(args.len(), 1);
            assert!(matches!(args[0], Expr::Bin { op: BinOp::Div, .. }));
        }
        other => panic!("expected Call, got {other:?}"),
    }
}

#[test]
fn parses_nested_min_max_calls() {
    let spec = parse("max(min(1,2),3)", ParseContext::default()).unwrap();
    assert!(matches!(
        spec.expr,
        Expr::Call {
            name: FnName::Max,
            ..
        }
    ));
}

#[test]
fn rejects_min_with_wrong_arity() {
    match parse("min(3)", ParseContext::default()) {
        Err(ParseError::Unexpected(msg)) => {
            assert!(msg.contains("min"), "{msg}");
            assert!(msg.contains('2'), "{msg}");
        }
        other => panic!("expected an arity Unexpected error, got {other:?}"),
    }
}

#[test]
fn rejects_floor_with_wrong_arity() {
    match parse("floor(3,4)", ParseContext::default()) {
        Err(ParseError::Unexpected(msg)) => {
            assert!(msg.contains("floor"), "{msg}");
            assert!(msg.contains('1'), "{msg}");
        }
        other => panic!("expected an arity Unexpected error, got {other:?}"),
    }
}

#[test]
fn rejects_unknown_function_name() {
    match parse("foo(3)", ParseContext::default()) {
        Err(ParseError::Unexpected(msg)) => assert!(msg.contains("unknown function 'foo'")),
        other => panic!("expected an unknown-function Unexpected error, got {other:?}"),
    }
}

#[test]
fn bare_ident_not_followed_by_lparen_is_not_a_function_call() {
    assert!(parse("floor", ParseContext::default()).is_err());
}

#[test]
fn parses_single_tier_rung_with_value_and_label() {
    let spec = parse("4d6cs>4tr3:1[Good]", ParseContext::default()).unwrap();
    match spec.mode {
        Mode::SuccessCount(c) => assert_eq!(
            c.tiers,
            vec![Tier {
                margin_offset: 3,
                label: Some("Good".into()),
                tier_value: Some(1)
            }]
        ),
        other => panic!("expected SuccessCount, got {other:?}"),
    }
}

#[test]
fn parses_two_tier_rungs_appended_in_order() {
    let spec = parse("4d6cs>4tr3:1[Good]tr6:2[Great]", ParseContext::default()).unwrap();
    match spec.mode {
        Mode::SuccessCount(c) => assert_eq!(
            c.tiers,
            vec![
                Tier {
                    margin_offset: 3,
                    label: Some("Good".into()),
                    tier_value: Some(1)
                },
                Tier {
                    margin_offset: 6,
                    label: Some("Great".into()),
                    tier_value: Some(2)
                },
            ]
        ),
        other => panic!("expected SuccessCount, got {other:?}"),
    }
}

#[test]
fn tr_value_and_label_are_optional() {
    let spec = parse("1d20t10tr5", ParseContext::default()).unwrap();
    match spec.mode {
        Mode::Total(c) => assert_eq!(
            c.tiers,
            vec![Tier {
                margin_offset: 5,
                label: None,
                tier_value: None
            }]
        ),
        other => panic!("expected Total, got {other:?}"),
    }
}

#[test]
fn no_tr_leaves_tiers_empty() {
    let spec = parse("1d20t10", ParseContext::default()).unwrap();
    match spec.mode {
        Mode::Total(c) => assert!(c.tiers.is_empty()),
        other => panic!("expected Total, got {other:?}"),
    }
}

#[test]
fn parses_xs_with_defaults() {
    let spec = parse("4d6cs>=4xs20", ParseContext::default()).unwrap();
    match spec.mode {
        Mode::SuccessCount(c) => assert_eq!(
            c.crit_success,
            Some(CritSuccess {
                trigger: CritTrigger::AtLeast(20),
                extra_successes: 1,
                positive_counter: 1
            })
        ),
        other => panic!("expected SuccessCount, got {other:?}"),
    }
}

#[test]
fn parses_xs_with_explicit_extra_and_positive_counter() {
    let spec = parse("4d6cs>=4xs20:3:2", ParseContext::default()).unwrap();
    match spec.mode {
        Mode::SuccessCount(c) => assert_eq!(
            c.crit_success,
            Some(CritSuccess {
                trigger: CritTrigger::AtLeast(20),
                extra_successes: 3,
                positive_counter: 2
            })
        ),
        other => panic!("expected SuccessCount, got {other:?}"),
    }
}

#[test]
fn duplicate_xs_errors() {
    assert!(matches!(
        parse("4d6cs>=4xs20xs19", ParseContext::default()),
        Err(ParseError::DuplicateCritSuccess)
    ));
}

#[test]
fn parses_xf_with_defaults() {
    let spec = parse("4d6cs>=4xf1", ParseContext::default()).unwrap();
    match spec.mode {
        Mode::SuccessCount(c) => assert_eq!(
            c.crit_fail,
            Some(CritFail {
                trigger: CritTrigger::AtLeast(1),
                lost: 1,
                negative_counter: 1,
                allow_negative: false
            })
        ),
        other => panic!("expected SuccessCount, got {other:?}"),
    }
}

#[test]
fn parses_xf_with_bang_sets_allow_negative() {
    let spec = parse("4d6cs>=4xf1!", ParseContext::default()).unwrap();
    match spec.mode {
        Mode::SuccessCount(c) => assert!(c.crit_fail.unwrap().allow_negative),
        other => panic!("expected SuccessCount, got {other:?}"),
    }
}

#[test]
fn duplicate_xf_errors() {
    assert!(matches!(
        parse("4d6cs>=4xf1xf2", ParseContext::default()),
        Err(ParseError::DuplicateCritFail)
    ));
}

#[test]
fn xs_and_xf_under_total_ambient_are_silently_dropped() {
    // Mirrors e<N>'s exact silent-drop-under-Total precedent: xs/xf set roll-level
    // scratch fields that are only consumed when the resolved mode is SuccessCount.
    let spec = parse("1d20t10xs15xf1", ParseContext::default()).unwrap(); // ambient Total
    match spec.mode {
        Mode::Total(c) => assert_eq!(c.difficulty, Some(10)),
        other => panic!("expected Total, got {other:?}"),
    }
}
