use super::*;
use crate::dice::rng::NoiseRng;
use crate::dice::spec::{BinOp, DiceGroup, Direction, Expr, Mode, RollSpec, TotalConfig};

fn ng(count: u32, min: i32, max: i32) -> Expr {
    Expr::Dice(DiceGroup {
        label: None,
        count,
        kind: DieKind::Numeric { min, max },
        modifiers: vec![],
    })
}

#[test]
fn roll_produces_one_record_per_die_across_groups() {
    let spec = RollSpec {
        expr: Expr::Bin {
            op: BinOp::Add,
            lhs: Box::new(ng(3, 1, 6)),
            rhs: Box::new(ng(2, 1, 8)),
        },
        direction: Direction::HighWins,
        mode: Mode::Total(TotalConfig {
            difficulty: None,
            tiers: vec![],
        }),
    };
    let raws = roll(&spec, &mut NoiseRng::from_seed(99));
    assert_eq!(raws.records.len(), 5);
    assert_eq!(raws.dice.len(), 5); // no explode modifiers -> no extra dice
    for r in &raws.records[0..3] {
        assert!((1..=6).contains(&r.value));
    }
    for r in &raws.records[3..5] {
        assert!((1..=8).contains(&r.value));
    }
}

#[test]
fn roll_is_seed_stable() {
    let spec = RollSpec {
        expr: ng(4, 1, 20),
        direction: Direction::HighWins,
        mode: Mode::Total(TotalConfig {
            difficulty: None,
            tiers: vec![],
        }),
    };
    let a = roll(&spec, &mut NoiseRng::from_seed(5));
    let b = roll(&spec, &mut NoiseRng::from_seed(5));
    assert_eq!(a, b);
}

/// Parses `notation`, rolls it with a fixed seed and evaluates it: the same path chat takes for
/// untrusted notation.
fn eval_notation(notation: &str) -> crate::dice::outcome::RollOutcome {
    let spec = crate::dice::notation::parse(notation, Default::default()).unwrap();
    let raws = roll(&spec, &mut NoiseRng::from_seed(7));
    evaluate(&spec, &raws)
}

#[test]
fn single_die_base_plus_extra_successes_saturates() {
    let out = eval_notation("1d10cs>=1xs1:2147483647");
    assert_eq!(out.successes, Some(i32::MAX));
}

#[test]
fn pooled_extra_successes_across_dice_saturate() {
    let out = eval_notation("2d10cs>=11xs1:2147483647");
    assert_eq!(out.successes, Some(i32::MAX));
}

#[test]
fn pooled_positive_counters_saturate() {
    let out = eval_notation("2d10cs>=1xs1:0:2147483647");
    assert_eq!(out.positive_counter, i32::MAX);
}

#[test]
fn pooled_negative_counters_saturate() {
    let out = eval_notation("2d10cs>=1xf10:0:2147483647");
    assert_eq!(out.negative_counter, i32::MAX);
}

#[test]
fn pooled_lost_successes_saturate_when_negative_net_is_allowed() {
    let out = eval_notation("2d10cs>=11xf10:2147483647:0!");
    assert_eq!(out.successes, Some(i32::MIN));
}

#[test]
fn required_successes_margin_widens_before_subtracting() {
    let out = eval_notation("1d10cs>=11xf10:2147483647:0!rs2147483647");
    assert_eq!(out.margin, Some(-i64::from(i32::MAX) - i64::from(i32::MAX)));
}

#[test]
fn expertise_candidate_net_saturates() {
    let out = eval_notation("1d10cs>=1xs1:2147483647e1");
    assert_eq!(out.successes, Some(i32::MAX));
}

#[test]
fn expertise_pool_accumulation_saturates() {
    let out = eval_notation("2d10cs>=11xs1:2147483647:2147483647e1");
    assert_eq!(out.successes, Some(i32::MAX));
    assert_eq!(out.positive_counter, i32::MAX);
}

#[test]
fn expertise_fixed_term_from_excluded_faces_dice_saturates() {
    use crate::dice::spec::{
        BinOp, Comparator, CritSuccess, CritTrigger, DiceGroup, DieKind, Face, Mode, RollSpec,
        SuccessConfig, SuccessRule,
    };
    let faces = DieKind::Faces {
        faces: vec![Face {
            value: Some(1),
            symbols: vec![],
        }],
    };
    let group = |count, kind| {
        Expr::Dice(DiceGroup {
            label: None,
            count,
            kind,
            modifiers: vec![],
        })
    };
    let spec = RollSpec {
        expr: Expr::Bin {
            op: BinOp::Add,
            lhs: Box::new(group(2, faces)),
            rhs: Box::new(group(1, DieKind::Numeric { min: 1, max: 10 })),
        },
        direction: Direction::HighWins,
        mode: Mode::SuccessCount(SuccessConfig {
            success: SuccessRule::Numeric {
                comp: Comparator::Gte,
                target: 99,
            },
            required_successes: None,
            tiers: vec![],
            crit_success: Some(CritSuccess {
                trigger: CritTrigger::AtLeast(1),
                extra_successes: i32::MAX,
                positive_counter: 0,
            }),
            crit_fail: None,
            expertise: 1,
        }),
    };
    let raws = roll(&spec, &mut NoiseRng::from_seed(7));
    let out = evaluate(&spec, &raws);
    assert_eq!(out.successes, Some(i32::MAX));
}

#[test]
fn compound_explosion_chain_value_saturates() {
    let out = eval_notation("1d2147483647!!>=1");
    assert_eq!(out.records[0].value, i32::MAX);
}
