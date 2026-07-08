use serde::{Deserialize, Serialize};

/// Stable identity of a rolled die within one roll; lets `recalculate` target a subset.
pub type DieId = u32;

/// A single face of a `DieKind::Faces` die. `value` is `Some` for a face that
/// participates numerically (ordering, totals); `None` for a face whose only
/// payload is `symbols`. A `Faces` die is "ordered" (see `eval::classify` /
/// `is_ordered`, M11b-3 §9) iff EVERY face has `value: Some`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Face {
    pub value: Option<i32>,
    pub symbols: Vec<Symbol>,
}

/// An opaque tag on a `Face`; the system assigns meaning (e.g. Genesys "triumph").
pub type Symbol = String;

/// A die's face space. `Numeric`: an ordered inclusive range. `Faces`: an
/// explicit, possibly-unordered, possibly-symbolic list (M11b-3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DieKind {
    Numeric { min: i32, max: i32 },
    Faces { faces: Vec<Face> },
}

/// Construction-time validation error for a `DieKind`. `Numeric` has no
/// invalid state representable by this type (`sides >= 1` is enforced by the
/// notation parser's `ParseError::InvalidDieSides`, since only the notation
/// path constructs `Numeric` from untrusted input today).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DieKindError {
    /// `Faces { faces: [] }` — `roll_uniform(0, faces.len() - 1)` requires a
    /// non-degenerate range; `roll_uniform` only `debug_assert!`s this (a
    /// no-op in release). No notation path constructs `Faces` today (M11b-3
    /// is struct-only for face-lists); this becomes the enforcement point at
    /// M11d's untrusted-wire boundary.
    EmptyFaces,
}

impl DieKind {
    pub fn validate(&self) -> Result<(), DieKindError> {
        match self {
            DieKind::Numeric { .. } => Ok(()),
            DieKind::Faces { faces } => {
                if faces.is_empty() {
                    Err(DieKindError::EmptyFaces)
                } else {
                    Ok(())
                }
            }
        }
    }

    /// A die participates in value-based operations (fold-into-total, keep/drop,
    /// comparator explode/reroll) iff its faces have a defined ordering.
    /// `Numeric` is always ordered. `Faces` is ordered iff EVERY face has
    /// `value: Some` — a single unordered face makes the whole die unrankable
    /// against a valued sibling (M11b-3 §9/design decision).
    pub fn is_ordered(&self) -> bool {
        match self {
            DieKind::Numeric { .. } => true,
            DieKind::Faces { faces } => faces.iter().all(|f| f.value.is_some()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Comparator {
    Eq,
    Ne,
    Gt,
    Lt,
    #[default]
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
    /// Optional tag propagated onto every `DieRecord` this group produces
    /// (including exploded/penetrated children). Orthogonal to mode.
    /// `RollOutcome::by_label`/`compare_labels` read this. `None` = unlabeled.
    #[serde(default)]
    pub label: Option<String>,
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

/// SuccessCount dimension 1: the per-die predicate a die must satisfy to score
/// a success. Defaults to `Numeric` (comp: Gte, target: 0) so any `Default`- or
/// serde-defaulted `SuccessConfig` never silently becomes symbol-driven.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuccessRule {
    Numeric { comp: Comparator, target: i32 },
    HasSymbol(Symbol),
}

impl Default for SuccessRule {
    // `#[derive(Default)]`'s `#[default]` attribute only supports a unit
    // (fieldless) enum variant; `Numeric` carries fields, so this is hand-written.
    fn default() -> Self {
        SuccessRule::Numeric {
            comp: Comparator::default(),
            target: 0,
        }
    }
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

/// What makes a die's crit event fire. `AtLeast` is direction-aware (flips
/// under `LowWins`, exactly as the old bare `threshold: i32` did). `HasSymbol`
/// is direction-INSENSITIVE — a symbol is present or absent, there is no
/// "better end" to flip.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CritTrigger {
    AtLeast(i32),
    HasSymbol(Symbol),
}

/// A crit-success event (SuccessCount mode). Fires when a kept die's value or
/// symbols satisfy `trigger`. Adds `extra_successes` beyond the die's base
/// success and `positive_counter` to the positive tally.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CritSuccess {
    pub trigger: CritTrigger,
    pub extra_successes: i32,
    pub positive_counter: i32,
}

/// A crit-fail event (SuccessCount mode). Fires when a kept die's value or
/// symbols satisfy `trigger`. Subtracts `lost` from net successes (clamped at
/// 0 unless `allow_negative`) and adds `negative_counter`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CritFail {
    pub trigger: CritTrigger,
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
                    label: None,
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
}
