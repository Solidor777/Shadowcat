use crate::dice::outcome::{RawRoll, RollOutcome};
use crate::dice::spec::{BinOp, Expr, RollSpec};

/// Fold the AST to a total. Each `Dice` node contributes the sum of its group's kept
/// records (matched by `group_index`); a cursor consumes groups in AST order.
pub fn evaluate_sum(spec: &RollSpec, raws: &RawRoll) -> RollOutcome {
    let mut next_group = 0usize;
    let total = fold(&spec.expr, raws, &mut next_group);
    RollOutcome {
        total,
        records: raws.records.clone(),
        successes: None,
        pass: None,
        net_margin: None,
    }
}

fn fold(expr: &Expr, raws: &RawRoll, next_group: &mut usize) -> i64 {
    match expr {
        Expr::Const(c) => *c as i64,
        Expr::Neg(inner) => -fold(inner, raws, next_group),
        Expr::Dice(_) => {
            let gi = *next_group;
            *next_group += 1;
            raws.records
                .iter()
                .filter(|r| r.group_index == gi && r.kept)
                .map(|r| r.value as i64)
                .sum()
        }
        Expr::Bin { op, lhs, rhs } => {
            let l = fold(lhs, raws, next_group);
            let r = fold(rhs, raws, next_group);
            match op {
                BinOp::Add => l + r,
                BinOp::Sub => l - r,
                BinOp::Mul => l * r,
                // Division by zero yields 0 (documented; parser rejects literal `/0`).
                BinOp::Div => {
                    if r == 0 {
                        0
                    } else {
                        l / r
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::dice::eval::{evaluate, roll};
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
    fn sum_two_dice_plus_constant() {
        let spec = RollSpec {
            expr: Expr::Bin {
                op: BinOp::Add,
                lhs: Box::new(ng(2, 1, 6)),
                rhs: Box::new(Expr::Const(3)),
            },
            mode: Mode::Sum,
            success: None,
            required_successes: None,
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
                rhs: Box::new(Expr::Const(2)),
            },
            mode: Mode::Sum,
            success: None,
            required_successes: None,
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
        let spec = RollSpec {
            expr: Expr::Bin {
                op: BinOp::Add,
                lhs: Box::new(ng(1, 1, 6)),
                rhs: Box::new(ng(1, 1, 6)),
            },
            mode: Mode::Sum,
            success: None,
            required_successes: None,
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
}
