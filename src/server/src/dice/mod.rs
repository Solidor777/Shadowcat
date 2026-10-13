//! Server-authoritative dice engine.
//!
//! This module is a pure library: a struct-canonical `RollSpec` is rolled by `roll`, which draws
//! every die from an `RngSource`, and scored by `evaluate`, which draws nothing. `NoiseRng` derives
//! each draw from the stateless `noise` function, so a roll made with `NoiseRng::from_seed` is
//! reproducible from its seed. INVARIANT: `(spec, raws)` fully determines `evaluate`'s outcome.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub mod eval;
pub mod notation;
pub mod outcome;
#[cfg(test)]
mod proptests;
pub mod recalc;
pub mod rng;
pub mod spec;

pub use eval::{evaluate, roll};
pub use notation::{parse, ModeKind, ParseContext};
pub use outcome::{DieRecord, RawDie, RawRoll, RollOutcome, RollResult};
pub use recalc::{recalculate, RecalcOp};
pub use spec::{
    BinOp, Comparator, CritFail, CritSuccess, DiceGroup, DieId, DieKind, Direction, ExplodeKind,
    Expr, FnName, GroupModifier, Mode, RollSpec, SuccessConfig, SuccessRule, Tier, TotalConfig,
};
