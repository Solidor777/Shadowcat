//! Source text → tokens. Twin of the client package's `lexer.ts` + `chars.ts`.
//! Positions are UTF-16 code-unit offsets (the client's string indexing) so
//! every error `detail` names the same position on both sides.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use super::chars::{is_digit, is_word_char, is_word_start};
use super::types::{FormulaError, FormulaErrorKind, MAX_FORMULA_LENGTH};

/// A lexed token. `pos` is the UTF-16 offset of the token's first character.
#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// A numeric literal, already finite-checked.
    Num {
        /// The literal's value.
        value: f64,
        /// UTF-16 offset of the first character.
        pos: usize,
    },
    /// An identifier or dotted-path segment, lowercased (identifiers are
    /// case-insensitive; casing is normalized here).
    Word {
        /// Lowercased text.
        value: String,
        /// UTF-16 offset of the first character.
        pos: usize,
    },
    /// One of `+ - * / % ( ) , .`.
    Op {
        /// The operator character.
        value: char,
        /// UTF-16 offset of the character.
        pos: usize,
    },
}

/// True for one of the nine operator/punctuator characters.
fn is_op(c: char) -> bool {
    matches!(c, '+' | '-' | '*' | '/' | '%' | '(' | ')' | ',' | '.')
}

/// Single left-to-right scan into tokens. Never panics; unrecognized input
/// is a `Parse` error value, an over-long source or an overflowing literal a
/// `Cap`.
pub fn tokenize(src: &str) -> Result<Vec<Tok>, FormulaError> {
    if src.encode_utf16().count() > MAX_FORMULA_LENGTH {
        return Err(FormulaError::new(
            FormulaErrorKind::Cap,
            format!("formula exceeds {MAX_FORMULA_LENGTH} characters"),
        ));
    }
    // (char, utf16 offset) pairs so indexing is by character while positions
    // stay UTF-16 — the client scans code units and an astral character is
    // two of them.
    let chars: Vec<(char, usize)> = {
        let mut out = Vec::with_capacity(src.len());
        let mut pos = 0usize;
        for c in src.chars() {
            out.push((c, pos));
            pos += c.len_utf16();
        }
        out
    };
    let n = chars.len();
    let mut toks = Vec::new();
    let mut i = 0usize;
    while i < n {
        let (ch, pos) = chars[i];
        if matches!(ch, ' ' | '\t' | '\n' | '\r') {
            i += 1;
            continue;
        }
        if is_digit(ch) {
            let start = i;
            let start_pos = pos;
            let mut saw_dot = false;
            i += 1;
            while i < n && (is_digit(chars[i].0) || chars[i].0 == '.') {
                if chars[i].0 == '.' {
                    let dot_pos = chars[i].1;
                    if saw_dot || !(i + 1 < n && is_digit(chars[i + 1].0)) {
                        return Err(FormulaError::new(
                            FormulaErrorKind::Parse,
                            format!("unexpected '.' at position {dot_pos}"),
                        ));
                    }
                    saw_dot = true;
                }
                i += 1;
            }
            let text: String = chars[start..i].iter().map(|(c, _)| *c).collect();
            // A digit run with at most one interior dot always parses; an
            // over-long run overflows to infinity, which is a cap violation.
            let value: f64 = text.parse().unwrap_or(f64::INFINITY);
            if !value.is_finite() {
                return Err(FormulaError::new(
                    FormulaErrorKind::Cap,
                    format!("numeric literal at position {start_pos} is out of range"),
                ));
            }
            toks.push(Tok::Num {
                value,
                pos: start_pos,
            });
            continue;
        }
        if is_word_start(ch) {
            let start = i;
            i += 1;
            while i < n && is_word_char(chars[i].0) {
                i += 1;
            }
            let value: String = chars[start..i]
                .iter()
                .map(|(c, _)| c.to_ascii_lowercase())
                .collect();
            toks.push(Tok::Word { value, pos });
            continue;
        }
        if is_op(ch) {
            toks.push(Tok::Op { value: ch, pos });
            i += 1;
            continue;
        }
        return Err(FormulaError::new(
            FormulaErrorKind::Parse,
            format!("unexpected '{ch}' at position {pos}"),
        ));
    }
    Ok(toks)
}

#[cfg(test)]
mod tests;
