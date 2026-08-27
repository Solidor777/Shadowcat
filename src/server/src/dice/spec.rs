#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::{Deserialize, Serialize};

/// Stable identity of a rolled die within one roll; lets `recalculate` target a subset.
pub type DieId = u32;

/// A single face of a `DieKind::Faces` die. `value` is `Some` for a face that
/// participates numerically (ordering, totals); `None` for a face whose only
/// payload is `symbols`. A `Faces` die is "ordered" (see `eval::classify` /
/// `is_ordered`) iff EVERY face has `value: Some`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Face {
    /// Numeric worth, or `None` for a symbols-only face (makes the die
    /// unordered — see `DieKind::is_ordered`).
    #[serde(default)]
    pub value: Option<i32>,
    /// Opaque tags this face contributes (system-defined meaning).
    pub symbols: Vec<Symbol>,
}

/// An opaque tag on a `Face`; the system assigns meaning (e.g. Genesys "triumph").
pub type Symbol = String;

/// A die's face space. `Numeric`: an ordered inclusive range. `Faces`: an
/// explicit, possibly-unordered, possibly-symbolic list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DieKind {
    /// An ordered inclusive integer range.
    Numeric {
        /// Lowest face value.
        min: i32,
        /// Highest face value (inclusive).
        max: i32,
    },
    /// An explicit face list (possibly unordered/symbolic).
    Faces {
        /// The faces, uniformly sampled; must be non-empty (`validate`).
        faces: Vec<Face>,
    },
}

/// Construction-time validation error for a `DieKind`. `Numeric` has no
/// invalid state representable by this type (`sides >= 1` is enforced by the
/// notation parser's `ParseError::InvalidDieSides`, since only the notation
/// path constructs `Numeric` from untrusted input today).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DieKindError {
    /// `Faces { faces: [] }` — `roll_uniform(0, faces.len() - 1)` requires a
    /// non-degenerate range; `roll_uniform` only `debug_assert!`s this (a
    /// no-op in release). No notation path constructs `Faces` today (it is
    /// struct-only for face-lists); `chat::rolls::validate_pre_roll` calls
    /// `DieKind::validate()` on every parsed group before rolling, so this is
    /// the enforcement point at the untrusted-wire boundary.
    EmptyFaces,
}

impl DieKind {
    /// Construction-time check: rejects an empty `Faces` list (see
    /// `DieKindError::EmptyFaces`); `Numeric` always passes.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::dice::spec::DieKind;
    /// assert!(DieKind::Numeric { min: 1, max: 6 }.validate().is_ok());
    /// assert!(DieKind::Faces { faces: vec![] }.validate().is_err());
    /// ```
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
    /// against a valued sibling.
    pub fn is_ordered(&self) -> bool {
        match self {
            DieKind::Numeric { .. } => true,
            DieKind::Faces { faces } => faces.iter().all(|f| f.value.is_some()),
        }
    }
}

/// A binary comparison operator used by explode/reroll triggers and success
/// rules. Defaults to `Gte` (the common "meets or beats" reading).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Comparator {
    /// `value == target`.
    Eq,
    /// `value != target`.
    Ne,
    /// `value > target`.
    Gt,
    /// `value < target`.
    Lt,
    /// `value >= target` (the default).
    #[default]
    Gte,
    /// `value <= target`.
    Lte,
}

impl Comparator {
    /// Apply the comparison: `value <op> target`.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::dice::spec::Comparator;
    /// assert!(Comparator::Gte.test(7, 7));
    /// assert!(!Comparator::Lt.test(7, 7));
    /// ```
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

/// How an explode trigger spawns/merges extra rolls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExplodeKind {
    /// Roll an extra die per trigger; each extra can itself trigger.
    Standard,
    /// Add the extra roll into the same die's value (one combined result).
    Compound,
    /// Standard, but each successive extra die is reduced by 1.
    Penetrate,
}

/// One dice-group modifier. Applied in `DiceGroup.modifiers` vec order:
/// reroll/explode alter the die set, keep/drop select from it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupModifier {
    /// Keep only the N highest dice.
    KeepHighest(u32),
    /// Keep only the N lowest dice.
    KeepLowest(u32),
    /// Drop the N highest dice.
    DropHighest(u32),
    /// Drop the N lowest dice.
    DropLowest(u32),
    /// Roll extra dice while the trigger fires.
    Explode {
        /// Standard/compound/penetrating behavior.
        kind: ExplodeKind,
        /// Trigger comparison operator.
        comp: Comparator,
        /// Trigger threshold the die value is compared against.
        target: i32,
    },
    /// Re-roll dice matching the trigger.
    Reroll {
        /// Trigger comparison operator.
        comp: Comparator,
        /// Trigger threshold.
        target: i32,
        /// Re-roll at most once per die (`ro`) instead of repeatedly (`r`).
        once: bool,
    },
}

/// One rolled dice group (`NdX` plus modifiers).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiceGroup {
    /// Number of base dice rolled.
    pub count: u32,
    /// The face space every die in the group shares.
    pub kind: DieKind,
    /// Applied in vec order: reroll/explode alter the die set, keep/drop select from it.
    pub modifiers: Vec<GroupModifier>,
    /// Optional tag propagated onto every `DieRecord` this group produces
    /// (including exploded/penetrated children). Orthogonal to mode.
    /// `RollOutcome::by_label`/`compare_labels` read this. `None` = unlabeled.
    #[serde(default)]
    pub label: Option<String>,
}

/// Arithmetic operator of an `Expr::Bin` node (Sum mode only).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinOp {
    /// `lhs + rhs`.
    Add,
    /// `lhs - rhs`.
    Sub,
    /// `lhs * rhs`.
    Mul,
    /// `lhs / rhs` — integer division; a zero divisor evaluates to 0
    /// (`eval::sum`'s `Div` arm — the parser does NOT reject `/0`).
    Div,
}

/// A math function name recognized by the `factor := ... | fn_call` grammar production.
/// Fixed arity per variant (`FnName::arity`): `Floor`/`Ceil`/`Round`/`Abs` take exactly 1
/// argument, `Min`/`Max` take exactly 2 — checked at parse time
/// (`dice::notation::parser::P::fn_call`), never at evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FnName {
    /// Round toward negative infinity.
    Floor,
    /// Round toward positive infinity.
    Ceil,
    /// Round to nearest, ties AWAY FROM ZERO (Rust's `f64::round` semantics) — differs from
    /// `@shadowcat/formula`'s own `Round`, which is JS-native and ties toward positive infinity.
    /// This crate never calls into `@shadowcat/formula`; the two `Round` implementations are
    /// maintained independently and are not required to agree.
    Round,
    /// Absolute value.
    Abs,
    /// The lesser of two arguments.
    Min,
    /// The greater of two arguments.
    Max,
}

impl FnName {
    /// The fixed argument count this function requires.
    /// `dice::notation::parser` checks a call's actual argument count against this at parse
    /// time; `Expr::Call` itself carries no arity guarantee once constructed.
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::dice::spec::FnName;
    /// assert_eq!(FnName::Floor.arity(), 1);
    /// assert_eq!(FnName::Min.arity(), 2);
    /// ```
    pub fn arity(self) -> usize {
        match self {
            FnName::Floor | FnName::Ceil | FnName::Round | FnName::Abs => 1,
            FnName::Min | FnName::Max => 2,
        }
    }
}

/// A constant term. Mirrors `DiceGroup.label` exactly: an optional tag for
/// chat-embed display only. A bare constant has no dice pool, so `label` is
/// NEVER threaded into `RollOutcome::by_label`/`compare_labels` — those are
/// SuccessCount-mode dice-pool comparison features; a labeled `Const` is purely
/// a Sum-mode display/provenance decoration (see `RollOutcome::labeled_consts`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstTerm {
    /// The constant's numeric value.
    pub value: i32,
    /// Display-only tag (see the struct doc — never a pool-comparison label).
    #[serde(default)]
    pub label: Option<String>,
}

/// Roll expression AST. Sum mode folds this to a total; SuccessCount mode ignores
/// the arithmetic and pools the dice reachable from `Dice` nodes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Expr {
    /// A rolled dice group.
    Dice(DiceGroup),
    /// A constant term.
    Const(ConstTerm),
    /// A binary arithmetic node.
    Bin {
        /// The operator.
        op: BinOp,
        /// Left operand.
        lhs: Box<Expr>,
        /// Right operand.
        rhs: Box<Expr>,
    },
    /// Unary negation.
    Neg(Box<Expr>),
    /// A math function call (`floor`/`ceil`/`round`/`abs`/`min`/`max`), reached only via the
    /// `factor := ... | fn_call` grammar production. `eval::sum::fold` and
    /// `eval::roll_expr` both recurse through `args` in left-to-right order, threading the
    /// SAME group-index cursor sequentially through each argument — the same mechanism
    /// `Bin{lhs, rhs}` already uses for two children, generalized to N. A `Call` node itself
    /// introduces no new dice groups and consumes no group-index slots beyond what its
    /// arguments consume.
    Call {
        /// Which function.
        name: FnName,
        /// Evaluated argument expressions, left-to-right; length is checked against
        /// `name.arity()` at parse time.
        args: Vec<Expr>,
    },
}

/// SuccessCount dimension 1: the per-die predicate a die must satisfy to score
/// a success. Defaults to `Numeric` (comp: Gte, target: 0) so any `Default`- or
/// serde-defaulted `SuccessConfig` never silently becomes symbol-driven.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuccessRule {
    /// A die succeeds when its value passes the comparison.
    Numeric {
        /// Comparison operator.
        comp: Comparator,
        /// Threshold the die value is compared against.
        target: i32,
    },
    /// A die succeeds when any kept face carries this symbol.
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
/// systems). Global to `RollSpec` — orients every margin/tier/crit computation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Direction {
    /// Higher totals/success counts are better (the default).
    #[default]
    HighWins,
    /// Lower is better (roll-under systems).
    LowWins,
}

/// One rung of a classification ladder, evaluated on an oriented margin
/// (higher = better). `margin_offset` is the threshold the margin must reach.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tier {
    /// Oriented-margin threshold this rung requires. Unique per ladder
    /// (`validate_tiers` refuses duplicates — a tie would be nondeterministic).
    pub margin_offset: i32,
    /// Display label for the rung.
    #[serde(default)]
    pub label: Option<String>,
    /// System-defined numeric payload for the rung.
    #[serde(default)]
    pub tier_value: Option<i32>,
}

/// What makes a die's crit event fire. `AtLeast` is direction-aware: it flips
/// under `LowWins`. `HasSymbol` is direction-INSENSITIVE — a symbol is present
/// or absent, there is no "better end" to flip.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CritTrigger {
    /// Fires at-or-past this die value (orientation-flipped under `LowWins`).
    AtLeast(i32),
    /// Fires when the kept face carries this symbol (direction-insensitive).
    HasSymbol(Symbol),
}

/// A crit-success event (SuccessCount mode). Fires when a kept die's value or
/// symbols satisfy `trigger`. Adds `extra_successes` beyond the die's base
/// success and `positive_counter` to the positive tally.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CritSuccess {
    /// What fires the event.
    pub trigger: CritTrigger,
    /// Successes added beyond the die's base success.
    pub extra_successes: i32,
    /// Amount added to the positive counter tally.
    pub positive_counter: i32,
}

/// A crit-fail event (SuccessCount mode). Fires when a kept die's value or
/// symbols satisfy `trigger`. Subtracts `lost` from net successes (clamped at
/// 0 unless `allow_negative`) and adds `negative_counter`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CritFail {
    /// What fires the event.
    pub trigger: CritTrigger,
    /// Successes subtracted from the net count.
    pub lost: i32,
    /// Amount added to the negative counter tally.
    pub negative_counter: i32,
    /// Permit the net count to go below zero (else clamped at 0).
    pub allow_negative: bool,
}

/// Total-mode config: fold the expression to a total, optionally classify it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TotalConfig {
    /// Margin reference; `None` => report the bare total, no classification.
    #[serde(default)]
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
    #[serde(default)]
    pub required_successes: Option<i32>,
    /// Ladder over `margin = net_successes - required_successes`. Empty => default 2-rung.
    pub tiers: Vec<Tier>,
    /// Optional crit-success event (see `CritSuccess`).
    #[serde(default)]
    pub crit_success: Option<CritSuccess>,
    /// Optional crit-fail event (see `CritFail`).
    #[serde(default)]
    pub crit_fail: Option<CritFail>,
    /// Expertise budget: points distributed across the pooled kept dice
    /// to maximize net successes (tie-break net counters). 0 = off.
    #[serde(default)]
    pub expertise: u32,
}

/// The roll's scoring mode.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    /// Fold the expression arithmetic to a total.
    Total(TotalConfig),
    /// Count net successes across the pooled kept dice.
    SuccessCount(SuccessConfig),
}

/// Canonical roll parameters. Notation parses INTO this; recalculation re-runs it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollSpec {
    /// The roll expression AST.
    pub expr: Expr,
    /// Which end of every margin/comparison is "better".
    pub direction: Direction,
    /// Total vs SuccessCount scoring.
    pub mode: Mode,
}

#[cfg(test)]
mod tests;
