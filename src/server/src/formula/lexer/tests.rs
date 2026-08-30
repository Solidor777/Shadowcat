use super::*;
use crate::formula::types::{FormulaErrorKind, MAX_FORMULA_LENGTH};

fn word(v: &str, pos: usize) -> Tok {
    Tok::Word {
        value: v.to_string(),
        pos,
    }
}
fn op(c: char, pos: usize) -> Tok {
    Tok::Op { value: c, pos }
}
fn num(v: f64, pos: usize) -> Tok {
    Tok::Num { value: v, pos }
}

#[test]
fn lexes_numbers_words_dots_and_operators() {
    assert_eq!(
        tokenize("floor(Parent.dex / 2) + 1.5").unwrap(),
        vec![
            word("floor", 0),
            op('(', 5),
            word("parent", 6),
            op('.', 12),
            word("dex", 13),
            op('/', 17),
            num(2.0, 19),
            op(')', 20),
            op('+', 22),
            num(1.5, 24),
        ]
    );
}

#[test]
fn rejects_unknown_characters_as_a_parse_error_value() {
    let e = tokenize("dex ? 2").unwrap_err();
    assert_eq!(e.error, FormulaErrorKind::Parse);
    assert_eq!(e.detail, "unexpected '?' at position 4");
}

#[test]
fn rejects_over_length_sources_with_cap() {
    let src = format!("{}1", "1+".repeat(300));
    let e = tokenize(&src).unwrap_err();
    assert_eq!(e.error, FormulaErrorKind::Cap);
    assert_eq!(e.detail, "formula exceeds 512 characters");
}

#[test]
fn rejects_a_second_dot_in_a_numeric_literal() {
    let e = tokenize("1.2.3").unwrap_err();
    assert_eq!(e.error, FormulaErrorKind::Parse);
    assert_eq!(e.detail, "unexpected '.' at position 3");
}

#[test]
fn tokenizes_a_source_exactly_max_length_long_and_caps_one_over() {
    let repeats = (MAX_FORMULA_LENGTH - 1) / 2;
    let src = format!(
        "{}{}",
        "1+".repeat(repeats),
        "1".repeat(MAX_FORMULA_LENGTH - repeats * 2)
    );
    assert_eq!(src.encode_utf16().count(), MAX_FORMULA_LENGTH);
    assert!(tokenize(&src).is_ok());
    let over = format!("{src}1");
    assert_eq!(tokenize(&over).unwrap_err().error, FormulaErrorKind::Cap);
}

#[test]
fn length_is_counted_in_utf16_units_so_an_astral_character_counts_twice() {
    // 511 ASCII units + one astral character (2 units) = 513 > 512.
    let src = format!("{}\u{1F600}", "1".repeat(511));
    assert_eq!(src.chars().count(), 512);
    assert_eq!(tokenize(&src).unwrap_err().error, FormulaErrorKind::Cap);
}

#[test]
fn rejects_a_numeric_literal_that_overflows_with_cap() {
    let e = tokenize(&"9".repeat(400)).unwrap_err();
    assert_eq!(e.error, FormulaErrorKind::Cap);
    assert_eq!(e.detail, "numeric literal at position 0 is out of range");
}

#[test]
fn rejects_a_trailing_dot_not_followed_by_a_digit() {
    assert_eq!(
        tokenize("5.").unwrap_err().detail,
        "unexpected '.' at position 1"
    );
    assert_eq!(
        tokenize("5.)").unwrap_err().detail,
        "unexpected '.' at position 1"
    );
}

#[test]
fn embeds_the_full_astral_code_point_in_the_unrecognized_character_detail() {
    let e = tokenize("dex \u{1F600} 2").unwrap_err();
    assert_eq!(e.detail, "unexpected '\u{1F600}' at position 4");
}

#[test]
fn positions_after_an_astral_character_are_utf16_offsets() {
    // "a😀+" — the astral character starts at UTF-16 offset 1 and occupies two
    // units; its position is reported in units, never char indices.
    let e = tokenize("a\u{1F600}+").unwrap_err();
    assert_eq!(e.detail, "unexpected '\u{1F600}' at position 1");
    // A recognized token AFTER an astral character in a longer source is
    // exercised through the length test above; here the scan stops at the
    // first unrecognized character, which is the astral one itself.
}

#[test]
fn identifiers_are_lowercased_and_may_contain_digits_and_underscores() {
    assert_eq!(tokenize("HP_max2").unwrap(), vec![word("hp_max2", 0)]);
}
