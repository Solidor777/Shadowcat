//! Failure values, the value type and the DoS caps of the formula language.
//! Twin of the client package's `types.ts`; the tag spellings and cap values
//! are asserted equal by the conformance corpus and by `types/tests.rs`.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use serde::{Deserialize, Serialize};

/// Maximum accepted source length, in UTF-16 code units (the client's
/// `String.length`), so an astral character counts as 2 on both sides.
/// Rejected before lexing, so a hostile input forces no tokenization work.
pub const MAX_FORMULA_LENGTH: usize = 512;
/// Maximum node count of a parsed AST, charged once per constructed node.
pub const MAX_AST_NODES: usize = 256;
/// Maximum structural-nesting depth (parens, call arguments, unary minus);
/// a flat operator chain never counts against it.
pub const MAX_PARSE_DEPTH: usize = 32;
/// Maximum distinct keys visited during graph resolution, charged once per
/// key at first discovery.
pub const MAX_GRAPH_VISITS: usize = 2048;

/// Which failure category occurred. Serialized as the client's kebab-case
/// tags (`"unknown-ref"`, `"div-zero"`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FormulaErrorKind {
    /// Source text does not lex/parse.
    Parse,
    /// The resolver had no value for a reference.
    UnknownRef,
    /// A non-numeric operand (e.g. a text leaf).
    Type,
    /// `x / 0` or `x % 0`.
    DivZero,
    /// Arithmetic overflowed to infinity or produced NaN.
    NonFinite,
    /// A reference cycle in graph resolution.
    Cycle,
    /// A DoS bound tripped.
    Cap,
    /// A referenced value was itself an error (propagation wrapper; kept for
    /// tag parity with the client, which reserves it the same way).
    RefError,
    /// A resolver returned a malformed value.
    ResolverError,
}

/// A failure value. `detail` is player-presentable and never carries an
/// internal dump.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormulaError {
    /// The failure category.
    pub error: FormulaErrorKind,
    /// Player-presentable text, e.g. `unexpected '?' at position 4`.
    pub detail: String,
}

impl FormulaError {
    /// Builds an error of `kind` with `detail`.
    pub fn new(kind: FormulaErrorKind, detail: impl Into<String>) -> Self {
        Self {
            error: kind,
            detail: detail.into(),
        }
    }
}

/// A finite number, or a `FormulaError`. INVARIANT: an `Ok` is always finite.
pub type FormulaValue = Result<f64, FormulaError>;

/// Renders a number the way JavaScript's template interpolation does for the
/// spellings this library ever emits: `Infinity`, `-Infinity`, `NaN`; a
/// finite value uses Rust's shortest round-trip form, which agrees with JS for
/// every value the library interpolates (only non-finite results reach a
/// `detail`).
pub(crate) fn js_number(n: f64) -> String {
    if n.is_nan() {
        "NaN".to_string()
    } else if n == f64::INFINITY {
        "Infinity".to_string()
    } else if n == f64::NEG_INFINITY {
        "-Infinity".to_string()
    } else {
        format!("{n}")
    }
}

/// Gates an arithmetic result: a finite value passes through; anything else
/// becomes a `NonFinite` error, so no infinity or NaN leaves the library.
pub(crate) fn finite(n: f64) -> FormulaValue {
    if n.is_finite() {
        Ok(n)
    } else {
        Err(FormulaError::new(
            FormulaErrorKind::NonFinite,
            format!("arithmetic result is not finite ({})", js_number(n)),
        ))
    }
}

#[cfg(test)]
mod tests;
