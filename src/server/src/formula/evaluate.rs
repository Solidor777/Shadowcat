//! Structural recursion over `Expr`. Twin of the client package's
//! `evaluate.ts`: operands evaluate left-to-right and the FIRST error wins;
//! `/` is float division, `%` truncated remainder, `round` ties toward +∞;
//! every arithmetic result passes `finite`, so no infinity or NaN escapes.
//! Recursion depth is bounded by `MAX_AST_NODES` (one frame per node on a
//! left-deep chain), not `MAX_PARSE_DEPTH`.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use super::parser::{BinOp, Expr, FnName};
use super::types::{finite, FormulaError, FormulaErrorKind, FormulaValue};

/// Resolves a dotted reference path to a value. The library assigns the path
/// no meaning; an implementation does.
pub trait Resolve {
    /// The value at `path`, or a `FormulaError` (typically `UnknownRef`/`Type`).
    fn resolve(&self, path: &[String]) -> FormulaValue;
}

impl<F: Fn(&[String]) -> FormulaValue> Resolve for F {
    fn resolve(&self, path: &[String]) -> FormulaValue {
        self(path)
    }
}

/// JavaScript `Math.round`: nearest integer, ties toward +∞, and a negative
/// input that rounds to zero keeps its sign (JS yields `-0`).
pub(crate) fn js_round(x: f64) -> f64 {
    let f = x.floor();
    let r = if x - f >= 0.5 { f + 1.0 } else { f };
    if r == 0.0 && x < 0.0 {
        -0.0
    } else {
        r
    }
}

/// Evaluates `expr` against `resolve`. Never panics.
pub fn evaluate(expr: &Expr, resolve: &dyn Resolve) -> FormulaValue {
    match expr {
        Expr::Num(v) => Ok(*v),
        // A resolver's own error passes through unchanged; a number it returns
        // is gated exactly like an arithmetic result.
        Expr::Ref(path) => resolve.resolve(path).and_then(finite),
        Expr::Neg(operand) => finite(-evaluate(operand, resolve)?),
        Expr::Bin { op, left, right } => {
            let l = evaluate(left, resolve)?;
            let r = evaluate(right, resolve)?;
            eval_bin(*op, l, r)
        }
        Expr::Call { func, args } => {
            let mut vals = Vec::with_capacity(args.len());
            for a in args {
                vals.push(evaluate(a, resolve)?);
            }
            eval_call(*func, &vals)
        }
    }
}

/// Applies one binary operator to two finite operands.
fn eval_bin(op: BinOp, left: f64, right: f64) -> FormulaValue {
    if matches!(op, BinOp::Div | BinOp::Rem) && right == 0.0 {
        let sym = if op == BinOp::Div { "'/'" } else { "'%'" };
        return Err(FormulaError::new(
            FormulaErrorKind::DivZero,
            format!("division by zero ({sym})"),
        ));
    }
    finite(match op {
        BinOp::Add => left + right,
        BinOp::Sub => left - right,
        BinOp::Mul => left * right,
        BinOp::Div => left / right,
        BinOp::Rem => left % right,
    })
}

/// Applies a builtin to already-evaluated arguments. Arity is the parser's
/// obligation; a hand-built `Expr` with the wrong count reaches `finite`
/// as JavaScript would (`floor()` of a missing argument is NaN, `min()` of
/// nothing is +∞), never a panic.
fn eval_call(func: FnName, vals: &[f64]) -> FormulaValue {
    let first = vals.first().copied().unwrap_or(f64::NAN);
    finite(match func {
        FnName::Min => vals.iter().copied().fold(f64::INFINITY, f64::min),
        FnName::Max => vals.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        FnName::Floor => first.floor(),
        FnName::Ceil => first.ceil(),
        FnName::Round => js_round(first),
    })
}

#[cfg(test)]
mod tests;
