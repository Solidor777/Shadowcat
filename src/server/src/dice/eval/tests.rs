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
