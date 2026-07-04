use crate::dice::eval::groups::resolve_group;
use crate::dice::outcome::{RawDie, RawRoll};
use crate::dice::rng::{roll_uniform, RngSource};
use crate::dice::spec::{DieKind, Expr, RollSpec};

pub mod groups;

/// Roll every die in the spec's expression, left-to-right, running each group's
/// pipeline. The ONLY randomness step; `evaluate` reads `raws.records` deterministically.
pub fn roll(spec: &RollSpec, rng: &mut dyn RngSource) -> RawRoll {
    let mut raws = RawRoll::default();
    roll_expr(&spec.expr, rng, &mut raws);
    raws
}

fn roll_expr(expr: &Expr, rng: &mut dyn RngSource, raws: &mut RawRoll) {
    match expr {
        Expr::Dice(group) => {
            let DieKind::Numeric { min, max } = group.kind;
            let start = raws.dice.len();
            for _ in 0..group.count {
                let natural = roll_uniform(rng, min, max);
                raws.push(group.kind.clone(), natural);
            }
            let naturals: Vec<RawDie> = raws.dice[start..].to_vec();
            let recs = resolve_group(group, &naturals, rng, raws);
            raws.records.extend(recs);
        }
        Expr::Const(_) => {}
        Expr::Neg(inner) => roll_expr(inner, rng, raws),
        Expr::Bin { lhs, rhs, .. } => {
            roll_expr(lhs, rng, raws);
            roll_expr(rhs, rng, raws);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dice::rng::NoiseRng;
    use crate::dice::spec::{BinOp, DiceGroup, DieKind, Expr, Mode, RollSpec};

    fn ng(count: u32, min: i32, max: i32) -> Expr {
        Expr::Dice(DiceGroup {
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
            mode: Mode::Sum,
            success: None,
            required_successes: None,
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
            mode: Mode::Sum,
            success: None,
            required_successes: None,
        };
        let a = roll(&spec, &mut NoiseRng::from_seed(5));
        let b = roll(&spec, &mut NoiseRng::from_seed(5));
        assert_eq!(a, b);
    }
}
