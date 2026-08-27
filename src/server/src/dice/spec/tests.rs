use super::*;

#[test]
fn comparator_tests_each_op() {
    assert!(Comparator::Gte.test(7, 7));
    assert!(Comparator::Gte.test(8, 7));
    assert!(!Comparator::Gte.test(6, 7));
    assert!(Comparator::Lt.test(6, 7));
    assert!(!Comparator::Lt.test(7, 7));
    assert!(Comparator::Eq.test(5, 5));
    assert!(Comparator::Ne.test(5, 6));
}

#[test]
fn spec_serde_round_trips() {
    let spec = RollSpec {
        expr: Expr::Bin {
            op: BinOp::Add,
            lhs: Box::new(Expr::Dice(DiceGroup {
                label: None,
                count: 2,
                kind: DieKind::Numeric { min: 1, max: 6 },
                modifiers: vec![],
            })),
            rhs: Box::new(Expr::Const(ConstTerm {
                value: 3,
                label: None,
            })),
        },
        direction: Direction::HighWins,
        mode: Mode::Total(TotalConfig {
            difficulty: None,
            tiers: vec![],
        }),
    };
    let json = serde_json::to_string(&spec).unwrap();
    assert_eq!(spec, serde_json::from_str::<RollSpec>(&json).unwrap());
}

#[test]
fn const_term_deserializes_with_missing_label_key() {
    // `#[serde(default)]` on `ConstTerm.label` (mirrors `DiceGroup.label`
    // exactly): a client/older-persisted-roll payload omitting `label`
    // must still deserialize, defaulting to `None`.
    let mut value = serde_json::to_value(ConstTerm {
        value: 3,
        label: Some("dex".into()),
    })
    .unwrap();
    value.as_object_mut().unwrap().remove("label");
    let term: ConstTerm = serde_json::from_value(value).unwrap();
    assert_eq!(term.label, None);
}

#[test]
fn success_rule_defaults_to_numeric() {
    assert_eq!(
        SuccessRule::default(),
        SuccessRule::Numeric {
            comp: Comparator::Gte,
            target: 0
        }
    );
}

#[test]
fn comparator_defaults_to_gte() {
    assert_eq!(Comparator::default(), Comparator::Gte);
}

#[test]
fn faces_die_validate_rejects_empty_face_list() {
    let kind = DieKind::Faces { faces: vec![] };
    assert!(matches!(kind.validate(), Err(DieKindError::EmptyFaces)));
}

#[test]
fn faces_die_validate_accepts_nonempty_face_list() {
    let kind = DieKind::Faces {
        faces: vec![Face {
            value: Some(1),
            symbols: vec![],
        }],
    };
    assert!(kind.validate().is_ok());
}

#[test]
fn numeric_die_validate_is_always_ok() {
    assert!(DieKind::Numeric { min: 1, max: 6 }.validate().is_ok());
}

#[test]
fn numeric_is_always_ordered() {
    assert!(DieKind::Numeric { min: 1, max: 6 }.is_ordered());
}

#[test]
fn faces_all_valued_is_ordered() {
    let kind = DieKind::Faces {
        faces: vec![
            Face {
                value: Some(1),
                symbols: vec![],
            },
            Face {
                value: Some(2),
                symbols: vec!["x".into()],
            },
        ],
    };
    assert!(kind.is_ordered());
}

#[test]
fn faces_any_none_value_is_unordered() {
    let kind = DieKind::Faces {
        faces: vec![
            Face {
                value: Some(1),
                symbols: vec![],
            },
            Face {
                value: None,
                symbols: vec!["blank".into()],
            },
        ],
    };
    assert!(!kind.is_ordered());
}

#[test]
fn faces_all_none_value_is_unordered() {
    let kind = DieKind::Faces {
        faces: vec![Face {
            value: None,
            symbols: vec!["x".into()],
        }],
    };
    assert!(!kind.is_ordered());
}

#[test]
fn success_config_serde_round_trips() {
    let spec = RollSpec {
        expr: Expr::Dice(DiceGroup {
            label: None,
            count: 5,
            kind: DieKind::Numeric { min: 1, max: 10 },
            modifiers: vec![],
        }),
        direction: Direction::LowWins,
        mode: Mode::SuccessCount(SuccessConfig {
            success: SuccessRule::Numeric {
                comp: Comparator::Lte,
                target: 4,
            },
            required_successes: Some(2),
            tiers: vec![Tier {
                margin_offset: 2,
                label: Some("Great".into()),
                tier_value: Some(1),
            }],
            crit_success: Some(CritSuccess {
                trigger: CritTrigger::AtLeast(1),
                extra_successes: 1,
                positive_counter: 1,
            }),
            crit_fail: None,
            expertise: 0,
        }),
    };
    let json = serde_json::to_string(&spec).unwrap();
    assert_eq!(spec, serde_json::from_str::<RollSpec>(&json).unwrap());
}

#[test]
fn face_deserializes_with_missing_value_key() {
    let mut value = serde_json::to_value(Face {
        value: Some(3),
        symbols: vec!["x".into()],
    })
    .unwrap();
    value.as_object_mut().unwrap().remove("value");
    let face: Face = serde_json::from_value(value).unwrap();
    assert_eq!(face.value, None);
}

#[test]
fn tier_deserializes_with_missing_label_and_tier_value_keys() {
    let mut value = serde_json::to_value(Tier {
        margin_offset: 0,
        label: Some("x".into()),
        tier_value: Some(1),
    })
    .unwrap();
    let obj = value.as_object_mut().unwrap();
    obj.remove("label");
    obj.remove("tier_value");
    let tier: Tier = serde_json::from_value(value).unwrap();
    assert_eq!(tier.label, None);
    assert_eq!(tier.tier_value, None);
}

#[test]
fn total_config_deserializes_with_missing_difficulty_key() {
    let mut value = serde_json::to_value(TotalConfig {
        difficulty: Some(10),
        tiers: vec![],
    })
    .unwrap();
    value.as_object_mut().unwrap().remove("difficulty");
    let cfg: TotalConfig = serde_json::from_value(value).unwrap();
    assert_eq!(cfg.difficulty, None);
}

#[test]
fn success_config_deserializes_with_missing_optional_keys() {
    // The TODO'd partial-JSON gap: a client-authored SuccessConfig omitting
    // required_successes/crit_success/crit_fail must still deserialize,
    // defaulting each to None rather than failing the whole roll.
    let full = SuccessConfig {
        success: SuccessRule::Numeric {
            comp: Comparator::Gte,
            target: 5,
        },
        required_successes: Some(3),
        tiers: vec![],
        crit_success: Some(CritSuccess {
            trigger: CritTrigger::AtLeast(6),
            extra_successes: 1,
            positive_counter: 1,
        }),
        crit_fail: Some(CritFail {
            trigger: CritTrigger::AtLeast(1),
            lost: 1,
            negative_counter: 1,
            allow_negative: false,
        }),
        expertise: 2,
    };
    let mut value = serde_json::to_value(full).unwrap();
    let obj = value.as_object_mut().unwrap();
    obj.remove("required_successes");
    obj.remove("crit_success");
    obj.remove("crit_fail");
    let cfg: SuccessConfig = serde_json::from_value(value).unwrap();
    assert_eq!(cfg.required_successes, None);
    assert_eq!(cfg.crit_success, None);
    assert_eq!(cfg.crit_fail, None);
}

#[test]
fn fn_name_arity_matches_grammar() {
    assert_eq!(FnName::Floor.arity(), 1);
    assert_eq!(FnName::Ceil.arity(), 1);
    assert_eq!(FnName::Round.arity(), 1);
    assert_eq!(FnName::Abs.arity(), 1);
    assert_eq!(FnName::Min.arity(), 2);
    assert_eq!(FnName::Max.arity(), 2);
}

#[test]
fn call_expr_serde_round_trips() {
    let spec = RollSpec {
        expr: Expr::Call {
            name: FnName::Min,
            args: vec![
                Expr::Const(ConstTerm {
                    value: 3,
                    label: None,
                }),
                Expr::Const(ConstTerm {
                    value: 5,
                    label: None,
                }),
            ],
        },
        direction: Direction::HighWins,
        mode: Mode::Total(TotalConfig {
            difficulty: None,
            tiers: vec![],
        }),
    };
    let json = serde_json::to_string(&spec).unwrap();
    assert_eq!(spec, serde_json::from_str::<RollSpec>(&json).unwrap());
}
