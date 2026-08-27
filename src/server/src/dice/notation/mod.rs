#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

/// Notation tokenizer.
pub mod lexer;
/// Recursive-descent parser: tokens -> `RollSpec`.
pub mod parser;

pub use parser::parse;

use crate::dice::spec::Direction;

/// Which `Mode` a notation string should parse into when the string itself
/// carries no explicit `cs`/`cf`/`t<N>` disambiguator. An explicit `cs`/`cf`
/// modifier always forces `SuccessCount` regardless of this ambient setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeKind {
    /// Fold arithmetic to a total.
    Total,
    /// Count per-die successes.
    SuccessCount,
}

/// Caller-supplied ambient state the notation string does not itself encode:
/// `mode` resolves a bare `t<N>` target's
/// `Mode`, `direction` resolves its comparator (`HighWins` => `Gte`, `LowWins`
/// => `Lte`) and seeds `RollSpec::direction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseContext {
    /// Ambient mode a bare `t<N>` resolves against.
    pub mode: ModeKind,
    /// Orientation: seeds `RollSpec::direction` and picks `t<N>`'s comparator.
    pub direction: Direction,
}

impl Default for ParseContext {
    fn default() -> Self {
        ParseContext {
            mode: ModeKind::Total,
            direction: Direction::default(),
        }
    }
}

/// Why a notation string was refused. Messages are player-presentable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The input was empty/whitespace.
    Empty,
    /// An out-of-place token (carries the built message).
    Unexpected(String),
    /// Leftover tokens after a complete expression (carries the message).
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
    /// A second `e<N>` expertise token appeared in one roll. `expertise` is shared
    /// roll-level parser state (one `RollSpec`), so a silent overwrite would lose one.
    DuplicateExpertise,
    /// A second `rs<N>` required-successes token appeared in one roll. Shared roll-level
    /// state (one `SuccessConfig.required_successes`), so a silent overwrite would lose one.
    DuplicateRequiredSuccesses,
    /// A `[...]` label was empty after trimming whitespace (e.g. `1d12[]` or `1d12[ ]`).
    EmptyLabel,
    /// A `[` was never closed by a matching `]` before the input ended.
    UnterminatedLabel,
    /// A `[...]` label contained a byte that is neither ASCII-printable nor a space
    /// (e.g. a control byte below `0x20`, or DEL) — a label's charset is restricted to
    /// ASCII printable characters (plus space) except `]`.
    InvalidLabelChar,
    /// A second `xs<N>` crit-success trigger appeared in one roll. Shared roll-level state (one
    /// `SuccessConfig.crit_success`), so a silent overwrite would lose one.
    DuplicateCritSuccess,
    /// A second `xf<N>` crit-fail trigger appeared in one roll. Same reasoning as
    /// `DuplicateCritSuccess`.
    DuplicateCritFail,
}

/// Player-presentable rendering. `Unexpected`/`Trailing`'s inner `String` is
/// built at each construction site via `Token`'s own `Display` (see
/// `lexer::describe_token`), never `{:?}` — this impl is a thin wrapper, not a
/// place that itself formats a `Token`/`Option<Token>` with Debug.
impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Empty => write!(f, "the dice expression is empty"),
            ParseError::Unexpected(msg) => write!(f, "{msg}"),
            ParseError::Trailing(msg) => write!(f, "unexpected trailing input: {msg}"),
            ParseError::InvalidDieSides(n) => {
                write!(f, "a die must have at least 1 side (got {n})")
            }
            ParseError::DuplicateSuccessRule => {
                write!(f, "a roll can only set one success rule (cs, cf, or t<N>)")
            }
            ParseError::DuplicateExpertise => {
                write!(f, "a roll can only set one expertise budget (e<N>)")
            }
            ParseError::DuplicateRequiredSuccesses => {
                write!(
                    f,
                    "a roll can only set one required-successes target (rs<N>)"
                )
            }
            ParseError::EmptyLabel => write!(f, "a dice group label cannot be empty"),
            ParseError::UnterminatedLabel => {
                write!(f, "a dice group label is missing its closing ']'")
            }
            ParseError::InvalidLabelChar => {
                write!(f, "a dice group label contains an unsupported character")
            }
            ParseError::DuplicateCritSuccess => {
                write!(f, "a roll can only set one crit-success trigger (xs<N>)")
            }
            ParseError::DuplicateCritFail => {
                write!(f, "a roll can only set one crit-fail trigger (xf<N>)")
            }
        }
    }
}

impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests;
