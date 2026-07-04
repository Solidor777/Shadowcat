pub mod lexer;
pub mod parser;

pub use parser::parse;

use crate::dice::spec::Direction;

/// Which `Mode` a notation string should parse into when the string itself
/// carries no explicit `cs`/`cf`/`t<N>` disambiguator. An explicit `cs`/`cf`
/// modifier always forces `SuccessCount` regardless of this ambient setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeKind {
    Total,
    SuccessCount,
}

/// Caller-supplied ambient state the notation string does not itself encode
/// (design's "notation pillar", §10): `mode` resolves a bare `t<N>` target's
/// `Mode`, `direction` resolves its comparator (`HighWins` => `Gte`, `LowWins`
/// => `Lte`) and seeds `RollSpec::direction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseContext {
    pub mode: ModeKind,
    pub direction: Direction,
}

impl Default for ParseContext {
    fn default() -> Self {
        ParseContext {
            mode: ModeKind::Total,
            direction: Direction::HighWins,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    Unexpected(String),
    Trailing(String),
    /// A dice factor's sides count is not a positive integer (`sides < 1`).
    /// Rejected here so `DieKind::Numeric { min: 1, max: sides }` can never be
    /// constructed with a degenerate (non-positive-span) range; `rng::roll_uniform`
    /// only `debug_assert!`s that invariant, which is a no-op in release builds.
    InvalidDieSides(i32),
    /// A second `cs`/`cf` modifier appeared anywhere in the expression, OR a
    /// `t<N>` target and a `cs`/`cf` rule both set the per-die success rule.
    /// `success`/`t_target` are shared parser state (one `RollSpec`, not
    /// per-`DiceGroup`), so a silent last-write-wins overwrite would discard
    /// one rule with no error.
    DuplicateSuccessRule,
}
