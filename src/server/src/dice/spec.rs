use serde::{Deserialize, Serialize};

/// Stable identity of a rolled die within one roll; lets `recalculate` target a subset.
pub type DieId = u32;

/// A die's face space. M11a: numeric only; M11b adds `Faces` for custom-symbol dice.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DieKind {
    Numeric { min: i32, max: i32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Comparator {
    Eq,
    Ne,
    Gt,
    Lt,
    Gte,
    Lte,
}

impl Comparator {
    pub fn test(self, value: i32, target: i32) -> bool {
        match self {
            Comparator::Eq => value == target,
            Comparator::Ne => value != target,
            Comparator::Gt => value > target,
            Comparator::Lt => value < target,
            Comparator::Gte => value >= target,
            Comparator::Lte => value <= target,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExplodeKind {
    /// Roll an extra die per trigger; each extra can itself trigger.
    Standard,
    /// Add the extra roll into the same die's value (one combined result).
    Compound,
    /// Standard, but each successive extra die is reduced by 1.
    Penetrate,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupModifier {
    KeepHighest(u32),
    KeepLowest(u32),
    DropHighest(u32),
    DropLowest(u32),
    Explode {
        kind: ExplodeKind,
        comp: Comparator,
        target: i32,
    },
    Reroll {
        comp: Comparator,
        target: i32,
        once: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiceGroup {
    pub count: u32,
    pub kind: DieKind,
    /// Applied in vec order: reroll/explode alter the die set, keep/drop select from it.
    pub modifiers: Vec<GroupModifier>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// Roll expression AST. Sum mode folds this to a total; SuccessCount mode ignores
/// the arithmetic and pools the dice reachable from `Dice` nodes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Expr {
    Dice(DiceGroup),
    Const(i32),
    Bin {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Neg(Box<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Sum,
    SuccessCount,
}

/// SuccessCount dimension 1: the per-die target a die must satisfy to score a success.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuccessRule {
    pub comp: Comparator,
    pub target: i32,
}

/// Canonical roll parameters. Notation parses INTO this; recalculation re-runs it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollSpec {
    pub expr: Expr,
    pub mode: Mode,
    /// Required when `mode == SuccessCount`.
    pub success: Option<SuccessRule>,
    /// SuccessCount dimension 2 (optional): successes needed for an overall pass.
    pub required_successes: Option<i32>,
}

#[cfg(test)]
mod tests {
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
                    count: 2,
                    kind: DieKind::Numeric { min: 1, max: 6 },
                    modifiers: vec![],
                })),
                rhs: Box::new(Expr::Const(3)),
            },
            mode: Mode::Sum,
            success: None,
            required_successes: None,
        };
        let json = serde_json::to_string(&spec).unwrap();
        let back: RollSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(spec, back);
    }
}
