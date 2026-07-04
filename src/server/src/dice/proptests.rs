use proptest::prelude::*;

use crate::dice::eval::{evaluate, roll};
use crate::dice::recalc::recalculate;
use crate::dice::rng::NoiseRng;
use crate::dice::spec::{
    Comparator, CritFail, CritSuccess, DiceGroup, DieKind, Direction, ExplodeKind, Expr,
    GroupModifier, Mode, RollSpec, SuccessConfig, SuccessRule,
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

    /// d10, HighWins target >=6 vs LowWins target <=5 are mirror images
    /// (face f maps to `mirror(f) = die_min+die_max-f`; f>=6  <=>  mirror(f)<=5).
    /// Same natural faces, so the success COUNT must match across the mirrored
    /// spec pair. `crit_success`/`crit_fail` are ALSO configured (mirrored
    /// thresholds) so this property exercises `eval::crit::reaches`'s
    /// `HighWins`/`LowWins` match arms, not just the base comparator — a config
    /// with both crit fields `None` short-circuits `score_die` to a no-op and
    /// would let a swapped `reaches` match arm slip past this test undetected.
    #[test]
    fn direction_flip_mirrors_success_count(seed in any::<u64>(), count in 1u32..8) {
        let die_min = 1i32;
        let die_max = 10i32;
        let mirror = |f: i32| die_min + die_max - f;

        let hi = RollSpec {
            expr: Expr::Dice(DiceGroup {
                count,
                kind: DieKind::Numeric { min: die_min, max: die_max },
                modifiers: vec![],
            }),
            direction: Direction::HighWins,
            mode: Mode::SuccessCount(SuccessConfig {
                success: SuccessRule { comp: Comparator::Gte, target: 6 },
                required_successes: None,
                tiers: vec![],
                // Fires at the max face (HighWins: value >= threshold).
                crit_success: Some(CritSuccess {
                    threshold: die_max,
                    extra_successes: 2,
                    positive_counter: 1,
                }),
                // Fires at the min face (HighWins: value <= threshold).
                crit_fail: Some(CritFail {
                    threshold: die_min,
                    lost: 1,
                    negative_counter: 1,
                    allow_negative: true,
                }),
            }),
        };
        let raws = roll(&hi, &mut NoiseRng::from_seed(seed));
        let hi_out = evaluate(&hi, &raws);
        // Mirror the faces into a fresh raws and evaluate the LowWins spec.
        let mut mirrored = raws.clone();
        for d in mirrored.dice.iter_mut() { d.natural = mirror(d.natural); }
        for r in mirrored.records.iter_mut() { r.value = mirror(r.value); r.natural = mirror(r.natural); }
        let lo = RollSpec {
            direction: Direction::LowWins,
            mode: Mode::SuccessCount(SuccessConfig {
                success: SuccessRule { comp: Comparator::Lte, target: mirror(6) },
                required_successes: None,
                tiers: vec![],
                // Mirror of hi's crit_success threshold (die_max -> mirror(die_max) == die_min);
                // LowWins fires at value <= threshold.
                crit_success: Some(CritSuccess {
                    threshold: mirror(die_max),
                    extra_successes: 2,
                    positive_counter: 1,
                }),
                // Mirror of hi's crit_fail threshold (die_min -> mirror(die_min) == die_max);
                // LowWins fires at value >= threshold.
                crit_fail: Some(CritFail {
                    threshold: mirror(die_min),
                    lost: 1,
                    negative_counter: 1,
                    allow_negative: true,
                }),
            }),
            ..hi.clone()
        };
        let lo_out = evaluate(&lo, &mirrored);
        prop_assert_eq!(hi_out.successes.unwrap(), lo_out.successes.unwrap());
        prop_assert_eq!(hi_out.crit_successes, lo_out.crit_successes);
        prop_assert_eq!(hi_out.crit_fails, lo_out.crit_fails);
        prop_assert_eq!(hi_out.positive_counter, lo_out.positive_counter);
        prop_assert_eq!(hi_out.negative_counter, lo_out.negative_counter);
    }

    /// For a fixed `1d<sides>t<target>` expression, the `t<N>` value must reach
    /// the mode-appropriate field regardless of the ambient `ModeKind` the
    /// caller parses under.
    #[test]
    fn t_target_reaches_mode_appropriate_field(sides in 2i32..20, target in 1i32..20) {
        use crate::dice::notation::{parse, ParseContext, ModeKind};
        let s = format!("1d{sides}t{target}");
        let total = parse(&s, ParseContext { mode: ModeKind::Total, direction: Direction::HighWins }).unwrap();
        let sc = parse(&s, ParseContext { mode: ModeKind::SuccessCount, direction: Direction::HighWins }).unwrap();
        match total.mode {
            Mode::Total(c) => prop_assert_eq!(c.difficulty, Some(target)),
            _ => prop_assert!(false, "expected Total mode"),
        }
        match sc.mode {
            Mode::SuccessCount(c) => prop_assert_eq!(c.success.target, target),
            _ => prop_assert!(false, "expected SuccessCount mode"),
        }
    }
}
