//! Server-authoritative dice engine. Pure library: a struct-canonical `RollSpec`
//! is rolled by `roll` (the only randomness step) and scored by `evaluate`
//! (deterministic). Randomness is a stateless noise function, so any roll is
//! reproducible from its seed. INVARIANT: (spec, raws) fully determines the outcome.

/// Deterministic evaluation pipeline (roll + score).
pub mod eval;
/// Notation string -> `RollSpec` (lexer + parser).
pub mod notation;
/// Wire/result types: raw naturals, per-die records, scored outcomes.
pub mod outcome;
#[cfg(test)]
mod proptests;
/// Targeted re-evaluation of an existing roll (reroll/replace/remove).
pub mod recalc;
/// Seeded noise-function RNG (the only randomness source).
pub mod rng;
/// The canonical roll-parameter types (AST, modes, crit/tier config).
pub mod spec;

pub use eval::{evaluate, roll};
pub use notation::{parse, ModeKind, ParseContext};
pub use outcome::{DieRecord, RawDie, RawRoll, RollOutcome, RollResult};
pub use recalc::{recalculate, RecalcOp};
pub use spec::{
    BinOp, Comparator, CritFail, CritSuccess, DiceGroup, DieId, DieKind, Direction, ExplodeKind,
    Expr, GroupModifier, Mode, RollSpec, SuccessConfig, SuccessRule, Tier, TotalConfig,
};
