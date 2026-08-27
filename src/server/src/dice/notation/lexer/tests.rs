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

#[test]
fn lex_colon_separates_modifier_value_fields() {
    let toks = lex("tr3:1").unwrap();
    assert_eq!(
        toks,
        vec![
            Token::Ident("tr".into()),
            Token::Int(3),
            Token::Colon,
            Token::Int(1),
        ]
    );
}

#[test]
fn lex_comma_separates_call_arguments() {
    let toks = lex("min(3,5)").unwrap();
    assert_eq!(
        toks,
        vec![
            Token::Ident("min".into()),
            Token::LParen,
            Token::Int(3),
            Token::Comma,
            Token::Int(5),
            Token::RParen,
        ]
    );
}
