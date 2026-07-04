//! Server-authoritative dice engine. Pure library: a struct-canonical `RollSpec`
//! is rolled by `roll` (the only randomness step) and scored by `evaluate`
//! (deterministic). Randomness is a stateless noise function, so any roll is
//! reproducible from its seed. INVARIANT: (spec, raws) fully determines the outcome.

pub mod eval;
pub mod notation;
pub mod outcome;
pub mod rng;
pub mod spec;

pub use eval::{evaluate, roll};
pub use notation::parse;
pub use outcome::{DieRecord, RawDie, RawRoll, RollOutcome, RollResult};
pub use spec::{
    BinOp, Comparator, DiceGroup, DieId, DieKind, ExplodeKind, Expr, GroupModifier, Mode, RollSpec,
    SuccessRule,
};
