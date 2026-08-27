#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use crate::dice::notation::ParseError;
use crate::dice::spec::Comparator;

/// One lexed notation token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    /// An integer literal.
    Int(i32),
    /// The die marker `d` / `D`.
    D,
    /// A bare word (modifier keywords like `kh`, `ro`, `t`, `cs`).
    Ident(String),
    /// `+`.
    Plus,
    /// `-`.
    Minus,
    /// `*`.
    Star,
    /// `/`.
    Slash,
    /// `(`.
    LParen,
    /// `)`.
    RParen,
    /// A comparator (`=`, `!=`, `>`, `<`, `>=`, `<=`).
    Cmp(Comparator),
    /// `!` — standard explode.
    Bang,
    /// `!!` — compound explode.
    BangBang,
    /// `!p` — penetrating explode.
    BangP,
    /// A `[bracketed]` label's text (printable ASCII + spaces only).
    Label(String),
    /// `,` — separates arguments in a `fn_call`.
    Comma,
    /// `:` — separates a modifier's threshold from its optional value fields
    /// (`tr<offset>:<value>`, `xs<N>:<extra>:<counter>`, `xf<N>:<lost>:<counter>`).
    Colon,
}

/// Player-presentable rendering of a single token, used to build `ParseError`
/// messages without leaking `{:?}` Debug output (e.g. `Cmp(Gte)`) into
/// player-facing text.
impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Token::Int(n) => write!(f, "the number {n}"),
            Token::D => write!(f, "'d'"),
            Token::Ident(s) => write!(f, "'{s}'"),
            Token::Plus => write!(f, "'+'"),
            Token::Minus => write!(f, "'-'"),
            Token::Star => write!(f, "'*'"),
            Token::Slash => write!(f, "'/'"),
            Token::LParen => write!(f, "'('"),
            Token::RParen => write!(f, "')'"),
            Token::Cmp(c) => write!(f, "the comparator '{}'", comparator_symbol(*c)),
            Token::Bang => write!(f, "'!'"),
            Token::BangBang => write!(f, "'!!'"),
            Token::BangP => write!(f, "'!p'"),
            Token::Label(s) => write!(f, "the label '{s}'"),
            Token::Comma => write!(f, "','"),
            Token::Colon => write!(f, "':'"),
        }
    }
}

/// The comparator's notation spelling (for player-facing messages).
fn comparator_symbol(c: Comparator) -> &'static str {
    match c {
        Comparator::Eq => "=",
        Comparator::Ne => "!=",
        Comparator::Gt => ">",
        Comparator::Lt => "<",
        Comparator::Gte => ">=",
        Comparator::Lte => "<=",
    }
}

/// Player-presentable rendering of an optional token (`None` = end of input),
/// used at every "found `X`, expected `Y`" `ParseError` construction site.
pub fn describe_token(tok: Option<&Token>) -> String {
    match tok {
        Some(t) => t.to_string(),
        None => "end of input".to_string(),
    }
}

/// Lexes dice notation into a token stream.
///
/// INVARIANT: `input` must be ASCII-only. Every match arm below operates on
/// `bytes[i] as char`, which widens a raw byte to a `char` (Latin-1
/// semantics, not UTF-8 decoding) and is only correct for single-byte code
/// points; slicing `input` at a byte index computed this way would panic on
/// a multi-byte UTF-8 sequence. This is enforced up front rather than relied
/// on implicitly, so a future non-ASCII operator arm can't reintroduce a
/// slice-at-non-char-boundary panic.
pub fn lex(input: &str) -> Result<Vec<Token>, ParseError> {
    if !input.is_ascii() {
        return Err(ParseError::Unexpected(
            "the roll expression must be ASCII-only".to_string(),
        ));
    }
    let mut out = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            ' ' | '\t' => i += 1,
            '0'..='9' => {
                let start = i;
                while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
                    i += 1;
                }
                let n: i32 = input[start..i].parse().map_err(|_| {
                    ParseError::Unexpected(format!("'{}' is not a valid number", &input[start..i]))
                })?;
                out.push(Token::Int(n));
            }
            'd' | 'D' if !(i + 1 < bytes.len() && (bytes[i + 1] as char).is_ascii_alphabetic()) => {
                out.push(Token::D);
                i += 1;
            }
            'a'..='z' | 'A'..='Z' => {
                let start = i;
                while i < bytes.len() && (bytes[i] as char).is_ascii_alphabetic() {
                    i += 1;
                }
                out.push(Token::Ident(input[start..i].to_lowercase()));
            }
            '+' => {
                out.push(Token::Plus);
                i += 1;
            }
            '-' => {
                out.push(Token::Minus);
                i += 1;
            }
            '*' => {
                out.push(Token::Star);
                i += 1;
            }
            '/' => {
                out.push(Token::Slash);
                i += 1;
            }
            '(' => {
                out.push(Token::LParen);
                i += 1;
            }
            ')' => {
                out.push(Token::RParen);
                i += 1;
            }
            ',' => {
                out.push(Token::Comma);
                i += 1;
            }
            ':' => {
                out.push(Token::Colon);
                i += 1;
            }
            '[' => {
                let start = i + 1;
                let mut j = start;
                while j < bytes.len() && bytes[j] as char != ']' {
                    let b = bytes[j];
                    if !(b.is_ascii_graphic() || b == b' ') {
                        return Err(ParseError::InvalidLabelChar);
                    }
                    j += 1;
                }
                if j >= bytes.len() {
                    return Err(ParseError::UnterminatedLabel);
                }
                let raw = input[start..j].trim();
                if raw.is_empty() {
                    return Err(ParseError::EmptyLabel);
                }
                out.push(Token::Label(raw.to_string()));
                i = j + 1; // skip past the ']'
            }
            '!' => {
                if input[i..].starts_with("!!") {
                    out.push(Token::BangBang);
                    i += 2;
                } else if input[i..].starts_with("!p") {
                    out.push(Token::BangP);
                    i += 2;
                } else {
                    out.push(Token::Bang);
                    i += 1;
                }
            }
            '>' | '<' | '=' => {
                let (cmp, adv) = if input[i..].starts_with(">=") {
                    (Comparator::Gte, 2)
                } else if input[i..].starts_with("<=") {
                    (Comparator::Lte, 2)
                } else if c == '>' {
                    (Comparator::Gt, 1)
                } else if c == '<' {
                    (Comparator::Lt, 1)
                } else {
                    (Comparator::Eq, 1)
                };
                out.push(Token::Cmp(cmp));
                i += adv;
            }
            _ => {
                return Err(ParseError::Unexpected(format!(
                    "unexpected character '{c}'"
                )))
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
