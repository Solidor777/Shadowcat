//! Tokens → `Expr`. Twin of the client package's `parser.ts`: the same
//! recursive-descent grammar, node cap, depth cap and `detail` wording.
//! Grammar: `additive := multiplicative (('+'|'-') multiplicative)*`;
//! `multiplicative := unary (('*'|'/'|'%') unary)*`; `unary := '-' unary | primary`;
//! `primary := num | '(' additive ')' | word ('(' args ')')? ('.' word)*`.
//! A word immediately followed by `(` must be a known function name; a dotted
//! segment is never a call. INVARIANT: `depth` counts structural-nesting
//! boundaries only (paren-open, call-argument, unary-minus), so a flat operator
//! chain never trips the depth cap and the parser's own recursion is bounded
//! by that cap plus the constant depth of the production chain.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use super::lexer::{tokenize, Tok};
use super::types::{FormulaError, FormulaErrorKind, MAX_AST_NODES, MAX_PARSE_DEPTH};

/// A binary arithmetic operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/` — float division.
    Div,
    /// `%` — truncated remainder (the sign follows the dividend).
    Rem,
}

/// A builtin function. Arity: `Min`/`Max` at least 1; the rest exactly 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FnName {
    /// The least argument.
    Min,
    /// The greatest argument.
    Max,
    /// Round toward −∞.
    Floor,
    /// Round toward +∞.
    Ceil,
    /// Round to nearest, ties toward +∞ (JavaScript `Math.round`).
    Round,
}

impl FnName {
    /// The function a lowercased word names, if any.
    fn from_word(w: &str) -> Option<Self> {
        match w {
            "min" => Some(Self::Min),
            "max" => Some(Self::Max),
            "floor" => Some(Self::Floor),
            "ceil" => Some(Self::Ceil),
            "round" => Some(Self::Round),
            _ => None,
        }
    }

    /// The source spelling, for error details.
    fn name(self) -> &'static str {
        match self {
            Self::Min => "min",
            Self::Max => "max",
            Self::Floor => "floor",
            Self::Ceil => "ceil",
            Self::Round => "round",
        }
    }
}

/// The parsed expression AST — one node per grammar production.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A numeric literal.
    Num(f64),
    /// A dotted reference path in source order (`hp.max` → `["hp", "max"]`),
    /// meaningless to the library; a `Resolve` implementation interprets it.
    Ref(Vec<String>),
    /// Unary minus.
    Neg(Box<Expr>),
    /// A binary arithmetic node.
    Bin {
        /// The operator.
        op: BinOp,
        /// Evaluated first.
        left: Box<Expr>,
        /// Evaluated second.
        right: Box<Expr>,
    },
    /// A builtin call, arity-checked at parse time.
    Call {
        /// The builtin.
        func: FnName,
        /// Arguments in source order.
        args: Vec<Expr>,
    },
}

/// Recursive-descent parser state over one token stream.
struct Parser {
    /// The token stream, consumed by index.
    toks: Vec<Tok>,
    /// Index of the next unconsumed token.
    pos: usize,
    /// Nodes constructed so far, charged against `MAX_AST_NODES`.
    node_count: usize,
    /// Source length in UTF-16 units, the position an end-of-input error names.
    src_len: usize,
}

/// The UTF-16 offset a token starts at.
fn tok_pos(t: &Tok) -> usize {
    match t {
        Tok::Num { pos, .. } | Tok::Word { pos, .. } | Tok::Op { pos, .. } => *pos,
    }
}

/// A `Parse` error with `detail`.
fn parse_err(detail: String) -> FormulaError {
    FormulaError::new(FormulaErrorKind::Parse, detail)
}

impl Parser {
    /// The next unconsumed token, if any.
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    /// True when the next token is the operator `c`.
    fn at_op(&self, c: char) -> bool {
        matches!(self.peek(), Some(Tok::Op { value, .. }) if *value == c)
    }

    /// Position for an "expected X" error: the next token's, else end of input.
    fn error_pos(&self) -> usize {
        self.peek().map(tok_pos).unwrap_or(self.src_len)
    }

    /// Charges one node against the cap and returns it.
    fn node(&mut self, e: Expr) -> Result<Expr, FormulaError> {
        self.node_count += 1;
        if self.node_count > MAX_AST_NODES {
            return Err(FormulaError::new(
                FormulaErrorKind::Cap,
                format!("formula exceeds {MAX_AST_NODES} AST nodes"),
            ));
        }
        Ok(e)
    }

    /// Rejects a structural descent past `MAX_PARSE_DEPTH`.
    fn check_depth(depth: usize) -> Result<(), FormulaError> {
        if depth > MAX_PARSE_DEPTH {
            return Err(FormulaError::new(
                FormulaErrorKind::Cap,
                format!("formula exceeds max nesting depth of {MAX_PARSE_DEPTH}"),
            ));
        }
        Ok(())
    }

    /// Parses the whole stream, then requires end of input.
    fn parse_top(&mut self) -> Result<Expr, FormulaError> {
        let e = self.additive(0)?;
        if let Some(t) = self.peek() {
            return Err(parse_err(format!(
                "unexpected trailing input at position {}",
                tok_pos(t)
            )));
        }
        Ok(e)
    }

    /// `additive := multiplicative (('+'|'-') multiplicative)*`, left-associative.
    fn additive(&mut self, depth: usize) -> Result<Expr, FormulaError> {
        let mut left = self.multiplicative(depth)?;
        loop {
            let op = if self.at_op('+') {
                BinOp::Add
            } else if self.at_op('-') {
                BinOp::Sub
            } else {
                break;
            };
            self.pos += 1;
            let right = self.multiplicative(depth)?;
            left = self.node(Expr::Bin {
                op,
                left: Box::new(left),
                right: Box::new(right),
            })?;
        }
        Ok(left)
    }

    /// `multiplicative := unary (('*'|'/'|'%') unary)*`, left-associative.
    fn multiplicative(&mut self, depth: usize) -> Result<Expr, FormulaError> {
        let mut left = self.unary(depth)?;
        loop {
            let op = if self.at_op('*') {
                BinOp::Mul
            } else if self.at_op('/') {
                BinOp::Div
            } else if self.at_op('%') {
                BinOp::Rem
            } else {
                break;
            };
            self.pos += 1;
            let right = self.unary(depth)?;
            left = self.node(Expr::Bin {
                op,
                left: Box::new(left),
                right: Box::new(right),
            })?;
        }
        Ok(left)
    }

    /// `unary := '-' unary | primary`; a leading `-` is a structural boundary.
    fn unary(&mut self, depth: usize) -> Result<Expr, FormulaError> {
        if self.at_op('-') {
            let new_depth = depth + 1;
            Self::check_depth(new_depth)?;
            self.pos += 1;
            let operand = self.unary(new_depth)?;
            return self.node(Expr::Neg(Box::new(operand)));
        }
        self.primary(depth)
    }

    /// `primary := num | '(' additive ')' | word ('(' args ')')? ('.' word)*`.
    fn primary(&mut self, depth: usize) -> Result<Expr, FormulaError> {
        let Some(t) = self.peek().cloned() else {
            return Err(parse_err("unexpected end of formula".to_string()));
        };
        match t {
            Tok::Num { value, .. } => {
                self.pos += 1;
                self.node(Expr::Num(value))
            }
            Tok::Op { value: '(', .. } => {
                self.pos += 1;
                let new_depth = depth + 1;
                Self::check_depth(new_depth)?;
                let inner = self.additive(new_depth)?;
                if !self.at_op(')') {
                    return Err(parse_err(format!(
                        "expected ')' at position {}",
                        self.error_pos()
                    )));
                }
                self.pos += 1;
                Ok(inner)
            }
            Tok::Word { value, pos } => {
                self.pos += 1;
                if self.at_op('(') {
                    let Some(func) = FnName::from_word(&value) else {
                        return Err(parse_err(format!(
                            "unknown function '{value}' at position {pos}"
                        )));
                    };
                    self.pos += 1;
                    let new_depth = depth + 1;
                    Self::check_depth(new_depth)?;
                    let mut args = Vec::new();
                    if !self.at_op(')') {
                        loop {
                            args.push(self.additive(new_depth)?);
                            if self.at_op(',') {
                                self.pos += 1;
                                continue;
                            }
                            break;
                        }
                    }
                    if !self.at_op(')') {
                        return Err(parse_err(format!(
                            "expected ')' at position {}",
                            self.error_pos()
                        )));
                    }
                    self.pos += 1;
                    check_arity(func, args.len(), pos)?;
                    return self.node(Expr::Call { func, args });
                }
                let mut path = vec![value];
                while self.at_op('.') {
                    self.pos += 1;
                    match self.peek().cloned() {
                        Some(Tok::Word { value, .. }) => {
                            self.pos += 1;
                            path.push(value);
                        }
                        _ => {
                            return Err(parse_err(format!(
                                "expected identifier after '.' at position {}",
                                self.error_pos()
                            )));
                        }
                    }
                }
                // One ref = one node regardless of segment count; the path
                // length is bounded by MAX_FORMULA_LENGTH, not MAX_AST_NODES.
                self.node(Expr::Ref(path))
            }
            Tok::Op { pos, .. } => Err(parse_err(format!("unexpected token at position {pos}"))),
        }
    }
}

/// Validates a call's argument count at the FUNCTION's position.
fn check_arity(func: FnName, argc: usize, pos: usize) -> Result<(), FormulaError> {
    match func {
        FnName::Min | FnName::Max if argc < 1 => Err(parse_err(format!(
            "'{}' requires at least 1 argument at position {pos}",
            func.name()
        ))),
        FnName::Min | FnName::Max => Ok(()),
        _ if argc != 1 => Err(parse_err(format!(
            "'{}' requires exactly 1 argument at position {pos}",
            func.name()
        ))),
        _ => Ok(()),
    }
}

/// Lexes and parses `src`. Never panics; every failure is a value.
pub fn parse(src: &str) -> Result<Expr, FormulaError> {
    let toks = tokenize(src)?;
    let mut p = Parser {
        toks,
        pos: 0,
        node_count: 0,
        src_len: src.encode_utf16().count(),
    };
    p.parse_top()
}

#[cfg(test)]
mod tests;
