//! The engine's formula language, server side.
//!
//! It is an exact behavioural twin of the client's `@shadowcat/formula` package (lexer → parser →
//! evaluator → dependency graph).
//!
//! The two implementations are pinned together by one conformance corpus
//! (`src/client/formula/src/__fixtures__/conformance.json`) that both test suites read, so a
//! divergence fails on whichever side moved. INVARIANT: nothing here panics on any input and no
//! `Ok` value is ever non-finite — every failure is a `FormulaError` value. INVARIANT: the language
//! carries no game-system vocabulary; a reference path means whatever the `Resolve` implementation
//! says it means (`SystemLeafResolver` is the engine's one such decision).

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub(crate) mod chars;
pub mod evaluate;
pub mod graph;
pub mod lexer;
pub mod parser;
pub mod resolver;
pub mod template;
pub mod types;

#[cfg(test)]
mod proptests;
#[cfg(test)]
mod tests;

pub use evaluate::{evaluate, Resolve};
pub use graph::resolve_all;
pub use parser::{parse, BinOp, Expr, FnName};
pub use resolver::SystemLeafResolver;
pub use template::resolve_notation_template;
pub use types::{
    FormulaError, FormulaErrorKind, FormulaValue, MAX_AST_NODES, MAX_FORMULA_LENGTH,
    MAX_GRAPH_VISITS, MAX_PARSE_DEPTH,
};
