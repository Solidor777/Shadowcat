use crate::dice::notation::ParseError;
use crate::dice::spec::Comparator;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    Int(i32),
    D,
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
    Cmp(Comparator),
    Bang,
    BangBang,
    BangP,
    Label(String),
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
        }
    }
}

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
mod tests {
    use super::*;
    use crate::dice::spec::Comparator;

    #[test]
    fn lex_basic_expression() {
        let toks = lex("4d6kh3+2").unwrap();
        assert_eq!(
            toks,
            vec![
                Token::Int(4),
                Token::D,
                Token::Int(6),
                Token::Ident("kh".into()),
                Token::Int(3),
                Token::Plus,
                Token::Int(2),
            ]
        );
    }

    #[test]
    fn lex_success_comparator() {
        let toks = lex("5d10cs>=7").unwrap();
        assert_eq!(
            toks,
            vec![
                Token::Int(5),
                Token::D,
                Token::Int(10),
                Token::Ident("cs".into()),
                Token::Cmp(Comparator::Gte),
                Token::Int(7),
            ]
        );
    }

    #[test]
    fn lex_rejects_garbage() {
        assert!(lex("4d6 @ 2").is_err());
    }

    #[test]
    fn lex_is_case_insensitive_for_dice_operator() {
        assert_eq!(lex("4D6KH3").unwrap(), lex("4d6kh3").unwrap());
    }

    #[test]
    fn lex_rejects_non_ascii_input() {
        assert!(lex("4d6café").is_err());
        assert!(lex("4d6+€").is_err());
    }

    #[test]
    fn lex_expertise_uses_the_identifier_arm() {
        // `4d6e3` needs no dedicated token: the alphabetic-run arm emits Ident("e")
        // and the digits become Int(3). The parser recognizes Ident("e") as expertise.
        let toks = lex("4d6e3").unwrap();
        assert_eq!(
            toks,
            vec![
                Token::Int(4),
                Token::D,
                Token::Int(6),
                Token::Ident("e".into()),
                Token::Int(3),
            ]
        );
    }

    #[test]
    fn lex_label_brackets_preserve_case() {
        let toks = lex("1d12[Hope]").unwrap();
        assert_eq!(
            toks,
            vec![
                Token::Int(1),
                Token::D,
                Token::Int(12),
                Token::Label("Hope".to_string()),
            ]
        );
    }

    #[test]
    fn lex_label_rejects_empty() {
        assert!(matches!(lex("1d12[]"), Err(ParseError::EmptyLabel)));
    }

    #[test]
    fn lex_label_rejects_unterminated() {
        assert!(matches!(
            lex("1d12[Hope"),
            Err(ParseError::UnterminatedLabel)
        ));
    }

    #[test]
    fn lex_label_trims_surrounding_whitespace() {
        let toks = lex("1d12[ Hope ]").unwrap();
        assert_eq!(
            toks,
            vec![
                Token::Int(1),
                Token::D,
                Token::Int(12),
                Token::Label("Hope".to_string())
            ]
        );
    }

    #[test]
    fn lex_label_rejects_control_byte() {
        assert!(matches!(
            lex("1d12[Hi\x01Bye]"),
            Err(ParseError::InvalidLabelChar)
        ));
    }

    #[test]
    fn lex_label_rejects_del_byte() {
        assert!(matches!(
            lex("1d12[Hi\x7FBye]"),
            Err(ParseError::InvalidLabelChar)
        ));
    }

    #[test]
    fn lex_label_allows_internal_space() {
        let toks = lex("1d12[Hope Fear]").unwrap();
        assert_eq!(
            toks,
            vec![
                Token::Int(1),
                Token::D,
                Token::Int(12),
                Token::Label("Hope Fear".to_string())
            ]
        );
    }
}
