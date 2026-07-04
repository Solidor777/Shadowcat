pub mod lexer;
pub mod parser;

pub use parser::parse;

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
    /// A second `cs`/`cf` modifier appeared anywhere in the expression. `success`
    /// is shared parser state (one `RollSpec`, not per-`DiceGroup`), so a silent
    /// last-write-wins overwrite would discard the first rule with no error.
    DuplicateSuccessRule,
}
