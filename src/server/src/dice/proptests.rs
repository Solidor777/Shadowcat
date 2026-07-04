use proptest::prelude::*;

use crate::dice::eval::{evaluate, roll};
use crate::dice::recalc::recalculate;
use crate::dice::rng::NoiseRng;
use crate::dice::spec::{
    Comparator, DiceGroup, DieKind, Direction, ExplodeKind, Expr, GroupModifier, Mode, RollSpec,
    SuccessConfig, SuccessRule,
};

fn simple_pool(count: u32, sides: i32, target: i32) -> RollSpec {
    RollSpec {
        expr: Expr::Dice(DiceGroup {
            count,
            kind: DieKind::Numeric { min: 1, max: sides },
            modifiers: vec![],
        }),
        direction: Direction::HighWins,
        mode: Mode::SuccessCount(SuccessConfig {
            success: SuccessRule {
                comp: Comparator::Gte,
                target,
            },
            required_successes: None,
            tiers: vec![],
            crit_success: None,
            crit_fail: None,
        }),
    }
}

fn pool_with_modifiers(
    count: u32,
    sides: i32,
    target: i32,
    modifiers: Vec<GroupModifier>,
) -> RollSpec {
    RollSpec {
        expr: Expr::Dice(DiceGroup {
            count,
            kind: DieKind::Numeric { min: 1, max: sides },
            modifiers,
        }),
        direction: Direction::HighWins,
        mode: Mode::SuccessCount(SuccessConfig {
            success: SuccessRule {
                comp: Comparator::Gte,
                target,
            },
            required_successes: None,
            tiers: vec![],
            crit_success: None,
            crit_fail: None,
        }),
    }
}

/// Strategy over `(count, sides, modifiers)` where `modifiers` sometimes forces
/// `kept != count` (KeepHighest/DropLowest with N <= count) and sometimes triggers
/// explosion (target == sides, guaranteeing every max-face die re-triggers). Mixing
/// in the empty-modifier case keeps the plain no-modifier path covered too.
fn count_sides_and_modifiers() -> impl Strategy<Value = (u32, i32, Vec<GroupModifier>)> {
    (1u32..20, 2i32..12).prop_flat_map(|(count, sides)| {
        let keep_drop_n = 1u32..=count;
        prop_oneof![
            Just(vec![]),
            keep_drop_n
                .clone()
                .prop_map(|n| vec![GroupModifier::KeepHighest(n)]),
            keep_drop_n.prop_map(|n| vec![GroupModifier::DropLowest(n)]),
            Just(vec![GroupModifier::Explode {
                kind: ExplodeKind::Standard,
                comp: Comparator::Gte,
                target: sides,
            }]),
        ]
        .prop_map(move |modifiers| (count, sides, modifiers))
    })
}

proptest! {
    #[test]
    fn evaluate_is_deterministic(seed in any::<u64>(), count in 1u32..12, sides in 2i32..20) {
        let spec = simple_pool(count, sides, sides / 2);
        let raws = roll(&spec, &mut NoiseRng::from_seed(seed));
        prop_assert_eq!(evaluate(&spec, &raws), evaluate(&spec, &raws));
    }

    #[test]
    fn successes_never_exceed_dice(
        seed in any::<u64>(),
        (count, sides, modifiers) in count_sides_and_modifiers(),
    ) {
        let spec = pool_with_modifiers(count, sides, 2, modifiers);
        let raws = roll(&spec, &mut NoiseRng::from_seed(seed));
        let out = evaluate(&spec, &raws);
        let kept = raws.records.iter().filter(|r| r.kept).count() as i32;
        prop_assert!(out.successes.unwrap() <= kept);
    }

    #[test]
    fn empty_recalc_is_identity(seed in any::<u64>(), count in 1u32..10, sides in 2i32..12) {
        let spec = simple_pool(count, sides, sides / 2);
        let mut rng = NoiseRng::from_seed(seed);
        let raws = roll(&spec, &mut rng);
        let base = evaluate(&spec, &raws);
        let (raws2, out2) = recalculate(&spec, &raws, &[], &mut rng);
        prop_assert_eq!(raws2, raws);
        prop_assert_eq!(out2, base);
    }
}
