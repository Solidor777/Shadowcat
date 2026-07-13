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
            direction: Direction::default(),
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
    /// A second `e<N>` expertise token appeared in one roll. `expertise` is shared
    /// roll-level parser state (one `RollSpec`), so a silent overwrite would lose one.
    DuplicateExpertise,
    /// A `[...]` label was empty after trimming whitespace (e.g. `1d12[]` or `1d12[ ]`).
    EmptyLabel,
    /// A `[` was never closed by a matching `]` before the input ended.
    UnterminatedLabel,
    /// A `[...]` label contained a byte that is neither ASCII-printable nor a space
    /// (e.g. a C0 control byte or DEL) — design §3.1 restricts a label's charset to
    /// ASCII printable characters (plus space) except `]`.
    InvalidLabelChar,
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
            ParseError::EmptyLabel => write!(f, "a dice group label cannot be empty"),
            ParseError::UnterminatedLabel => {
                write!(f, "a dice group label is missing its closing ']'")
            }
            ParseError::InvalidLabelChar => {
                write!(f, "a dice group label contains an unsupported character")
            }
        }
    }
}

impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_debug_artifacts(s: &str) -> bool {
        !s.contains('{') && !s.contains("Some(") && !s.contains("None")
    }

    #[test]
    fn every_parse_error_variant_displays_without_debug_artifacts() {
        // Iterate every variant explicitly (including realistic `Unexpected`/
        // `Trailing` payloads, since those two wrap free text built at their
        // construction sites via `Token`'s Display, not this impl).
        let variants: Vec<ParseError> = vec![
            ParseError::Empty,
            ParseError::Unexpected("expected a number, found the number 5".to_string()),
            ParseError::Trailing("the number 5".to_string()),
            ParseError::InvalidDieSides(0),
            ParseError::DuplicateSuccessRule,
            ParseError::DuplicateExpertise,
            ParseError::EmptyLabel,
            ParseError::UnterminatedLabel,
            ParseError::InvalidLabelChar,
        ];
        assert_eq!(
            variants.len(),
            9,
            "update this test if a ParseError variant is added or removed"
        );
        for v in variants {
            let rendered = v.to_string();
            assert!(
                no_debug_artifacts(&rendered),
                "variant {v:?} rendered debug artifacts: {rendered:?}"
            );
        }
    }

    #[test]
    fn real_parse_failures_render_without_debug_artifacts() {
        let inputs = [
            "4d6 @ 2",                  // lexer: unexpected character
            "2d6 2d6",                  // trailing input
            "4d",                       // expect_int: missing sides
            "(1d4+1",                   // expect ')'
            "4d6xyz",                   // unknown modifier
            "6d6r",                     // cmp_target_required
            "café",                     // non-ASCII
            "999999999999999999999999", // invalid number literal
        ];
        for input in inputs {
            let err = parse(input, ParseContext::default())
                .expect_err("expected a parse error for malformed input");
            let rendered = err.to_string();
            assert!(
                no_debug_artifacts(&rendered),
                "input {input:?} produced debug artifacts: {rendered:?}"
            );
        }
    }
}
