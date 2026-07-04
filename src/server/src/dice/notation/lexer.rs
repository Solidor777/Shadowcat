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
}

pub fn lex(input: &str) -> Result<Vec<Token>, ParseError> {
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
                let n: i32 = input[start..i]
                    .parse()
                    .map_err(|_| ParseError::Unexpected(input[start..i].to_string()))?;
                out.push(Token::Int(n));
            }
            'd' if !(i + 1 < bytes.len() && (bytes[i + 1] as char).is_ascii_alphabetic()) => {
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
            _ => return Err(ParseError::Unexpected(c.to_string())),
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
}
