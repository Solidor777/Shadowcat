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

/// SuccessCount dimension 1: the per-die target a die must satisfy to score a success.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuccessRule {
    pub comp: Comparator,
    pub target: i32,
}

/// Which end of a margin/comparison is "better". `HighWins` (default): a higher
/// total/success-count beats a lower one. `LowWins`: the inverse (e.g. roll-under
/// systems). Global to the spec — orients every margin/tier/crit computation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Direction {
    #[default]
    HighWins,
    LowWins,
}

/// One rung of a classification ladder, evaluated on an oriented margin
/// (higher = better). `margin_offset` is the threshold the margin must reach.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tier {
    pub margin_offset: i32,
    pub label: Option<String>,
    pub tier_value: Option<i32>,
}

/// A crit-success event (SuccessCount mode). Fires when a kept die's value
/// reaches `threshold` (direction-aware). Adds `extra_successes` beyond the
/// die's base success and `positive_counter` to the positive tally.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CritSuccess {
    pub threshold: i32,
    pub extra_successes: i32,
    pub positive_counter: i32,
}

/// A crit-fail event (SuccessCount mode). Fires when a kept die's value
/// reaches `threshold` (direction-aware). Subtracts `lost` from net successes
/// (clamped at 0 unless `allow_negative`) and adds `negative_counter`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CritFail {
    pub threshold: i32,
    pub lost: i32,
    pub negative_counter: i32,
    pub allow_negative: bool,
}

/// Total-mode config: fold the expression to a total, optionally classify it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TotalConfig {
    /// Margin reference; `None` => report the bare total, no classification.
    pub difficulty: Option<i32>,
    /// Ladder over `margin = oriented(total, difficulty)`. Empty => default 2-rung pass/fail.
    pub tiers: Vec<Tier>,
}

/// SuccessCount-mode config: count net successes across the pooled kept dice.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuccessConfig {
    /// Per-die target (comparator + threshold) — REQUIRED in this mode.
    pub success: SuccessRule,
    /// Margin reference; `None` => report the bare success count.
    pub required_successes: Option<i32>,
    /// Ladder over `margin = net_successes - required_successes`. Empty => default 2-rung.
    pub tiers: Vec<Tier>,
    pub crit_success: Option<CritSuccess>,
    pub crit_fail: Option<CritFail>,
    /// Expertise budget (M11b-2): points distributed across the pooled kept dice
    /// to maximize net successes (tie-break net counters). 0 = off.
    #[serde(default)]
    pub expertise: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Total(TotalConfig),
    SuccessCount(SuccessConfig),
}

/// Canonical roll parameters. Notation parses INTO this; recalculation re-runs it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollSpec {
    pub expr: Expr,
    pub direction: Direction,
    pub mode: Mode,
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
    fn success_config_serde_round_trips() {
        let spec = RollSpec {
            expr: Expr::Dice(DiceGroup {
                count: 5,
                kind: DieKind::Numeric { min: 1, max: 10 },
                modifiers: vec![],
            }),
            direction: Direction::LowWins,
            mode: Mode::SuccessCount(SuccessConfig {
                success: SuccessRule {
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
                    threshold: 1,
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
}
