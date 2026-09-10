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
///
/// # Examples
///
/// ```
/// use shadowcat::formula::evaluate::{evaluate, Resolve};
/// use shadowcat::formula::parser::parse;
/// use shadowcat::formula::types::FormulaValue;
///
/// struct Fixed(f64);
/// impl Resolve for Fixed {
///     fn resolve(&self, _path: &[String]) -> FormulaValue {
///         Ok(self.0)
///     }
/// }
///
/// let ast = parse("hp + 1").unwrap();
/// assert_eq!(evaluate(&ast, &Fixed(9.0)), Ok(10.0));
/// ```
pub trait Resolve {
    /// The value at `path`, or a `FormulaError` (typically `UnknownRef`/`Type`).
    ///
    /// # Examples
    ///
    /// ```
    /// use shadowcat::formula::evaluate::Resolve;
    /// use shadowcat::formula::types::FormulaValue;
    ///
    /// // The blanket `impl<F: Fn(&[String]) -> FormulaValue> Resolve for F`
    /// // lets a plain closure serve as a resolver.
    /// let resolver = |path: &[String]| -> FormulaValue { Ok(path.len() as f64) };
    /// assert_eq!(resolver.resolve(&["a".to_string(), "b".to_string()]), Ok(2.0));
    /// ```
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
///
/// # Examples
///
/// ```
/// use shadowcat::formula::evaluate::evaluate;
/// use shadowcat::formula::parser::parse;
/// use shadowcat::formula::types::FormulaValue;
///
/// let ast = parse("2 * (3 + 4)").unwrap();
/// let resolver = |_: &[String]| -> FormulaValue { Ok(0.0) }; // no references in this AST
/// assert_eq!(evaluate(&ast, &resolver), Ok(14.0));
/// ```
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
