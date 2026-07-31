#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::dice::eval::groups::resolve_group;
use crate::dice::outcome::{RawDie, RawRoll, RollOutcome};
use crate::dice::rng::{roll_uniform, RngSource};
use crate::dice::spec::{DieKind, Expr, Mode, RollSpec};

/// Margin -> pass/tier ladder classification.
pub mod classify;
/// Per-die crit-event scoring.
pub mod crit;
/// Expertise-budget allocation (bounded knapsack DP).
pub mod expertise;
/// Per-group modifier pipeline (reroll/explode/keep/drop).
pub mod groups;
/// SuccessCount-mode scoring.
pub mod success;
/// Total-mode arithmetic fold (saturating).
pub mod sum;

/// Roll every die in the spec's expression, left-to-right, running each group's
/// pipeline. The ONLY randomness step; `evaluate` reads `raws.records` deterministically.
pub fn roll(spec: &RollSpec, rng: &mut dyn RngSource) -> RawRoll {
    let mut raws = RawRoll::default();
    let mut group_index = 0usize;
    roll_expr(&spec.expr, rng, &mut raws, &mut group_index);
    raws
}

/// `group_index` increments once per `Dice` node in AST left-to-right order —
/// the same order `eval::sum::evaluate_total` walks, so a `DieRecord`'s stamped
/// `group_index` always matches the `Dice` node that produced it.
fn roll_expr(expr: &Expr, rng: &mut dyn RngSource, raws: &mut RawRoll, group_index: &mut usize) {
    match expr {
        Expr::Dice(group) => {
            let start = raws.dice.len();
            for _ in 0..group.count {
                let natural = match &group.kind {
                    DieKind::Numeric { min, max } => roll_uniform(rng, *min, *max),
                    DieKind::Faces { faces } => roll_uniform(rng, 0, faces.len() as i32 - 1),
                };
                raws.push(group.kind.clone(), natural);
            }
            raws.group_spans.push((start, group.count as usize));
            let naturals: Vec<RawDie> = raws.dice[start..].to_vec();
            let index = *group_index;
            *group_index += 1;
            let recs = resolve_group(group, index, &naturals, rng, raws);
            raws.records.extend(recs);
        }
        Expr::Const(_) => {}
        Expr::Neg(inner) => roll_expr(inner, rng, raws, group_index),
        Expr::Bin { lhs, rhs, .. } => {
            roll_expr(lhs, rng, raws, group_index);
            roll_expr(rhs, rng, raws, group_index);
        }
    }
}

/// Deterministic scoring: (spec, raws) -> outcome. Reads `raws.records`; NO randomness.
pub fn evaluate(spec: &RollSpec, raws: &RawRoll) -> RollOutcome {
    match &spec.mode {
        Mode::Total(cfg) => sum::evaluate_total(spec, cfg, raws),
        Mode::SuccessCount(cfg) => success::evaluate_success(spec, cfg, raws),
    }
}

#[cfg(test)]
mod tests {
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
}
