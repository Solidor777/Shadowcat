use super::*;
use crate::formula::types::FormulaValue;

/// A resolver answering every path with the same value.
fn constant(v: FormulaValue) -> impl Fn(&[String]) -> FormulaValue {
    move |_| v.clone()
}

/// A resolver that knows only the given paths and records what it was asked.
fn spying<'a>(
    bindings: &'a [(&'a str, f64)],
    asked: &'a std::cell::RefCell<Vec<String>>,
) -> impl Fn(&[String]) -> FormulaValue + 'a {
    move |path: &[String]| {
        let key = path.join(".");
        asked.borrow_mut().push(key.clone());
        bindings
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| Ok(*v))
            .unwrap_or_else(|| {
                Err(FormulaError::new(
                    FormulaErrorKind::UnknownRef,
                    format!("unknown reference '{key}'"),
                ))
            })
    }
}

#[test]
fn notation_keywords_are_the_declared_fifteen() {
    assert_eq!(
        NOTATION_KEYWORDS,
        ["d", "kh", "kl", "dh", "dl", "r", "ro", "cs", "cf", "t", "e", "tr", "rs", "xs", "xf"]
    );
}

#[test]
fn a_non_finite_resolver_value_becomes_a_non_finite_error() {
    for v in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        let r = resolve_notation_template("1d20 + str", &constant(Ok(v)));
        assert_eq!(
            r,
            Err(FormulaError::new(
                FormulaErrorKind::NonFinite,
                format!("arithmetic result is not finite ({})", js_number(v))
            )),
            "resolver value {v}"
        );
    }
}

#[test]
fn a_template_at_exactly_the_length_cap_scans() {
    let src = "1".repeat(MAX_FORMULA_LENGTH);
    let r = resolve_notation_template(&src, &constant(Ok(0.0)));
    assert_eq!(r, Ok(src));
}

#[test]
fn an_astral_character_counts_as_two_against_the_length_cap() {
    // 510 ASCII digits + one astral character = 512 UTF-16 units, which
    // passes; one more ASCII digit trips the cap.
    let mut src = "1".repeat(510);
    src.push('😀');
    assert!(resolve_notation_template(&src, &constant(Ok(0.0))).is_ok());
    src.push('1');
    assert_eq!(
        resolve_notation_template(&src, &constant(Ok(0.0))),
        Err(FormulaError::new(
            FormulaErrorKind::Cap,
            format!("template exceeds {MAX_FORMULA_LENGTH} characters")
        ))
    );
}

#[test]
fn the_resolver_is_offered_each_path_once_per_occurrence_in_raw_case() {
    let asked = std::cell::RefCell::new(Vec::new());
    let r = resolve_notation_template(
        "Hp.Max + hp.max",
        &spying(&[("Hp.Max", 10.0), ("hp.max", 4.0)], &asked),
    );
    assert_eq!(r, Ok("10[Hp.Max] + 4[hp.max]".to_string()));
    assert_eq!(
        asked.into_inner(),
        vec!["Hp.Max".to_string(), "hp.max".to_string()]
    );
}

#[test]
fn a_label_after_an_identifier_survives_the_substitution() {
    let r = resolve_notation_template("str[x]", &constant(Ok(3.0)));
    assert_eq!(r, Ok("3[str][x]".to_string()));
}

#[test]
fn the_dice_count_is_synthesized_after_any_non_integer_claim() {
    // After a literal claim (the open paren) the dice operator still gains a
    // count; after an integer claim it never does.
    assert_eq!(
        resolve_notation_template("(d6) + 2d8", &constant(Ok(0.0))),
        Ok("(1d6) + 2d8".to_string())
    );
}

#[test]
fn the_dice_operator_keeps_the_authors_case_when_a_count_is_synthesized() {
    assert_eq!(
        resolve_notation_template("D20", &constant(Ok(0.0))),
        Ok("1D20".to_string())
    );
}

#[test]
fn no_panic_on_empty_or_unclaimable_input() {
    assert_eq!(
        resolve_notation_template("", &constant(Ok(0.0))),
        Ok(String::new())
    );
    assert_eq!(
        resolve_notation_template("]", &constant(Ok(7.0))),
        Ok("]".to_string())
    );
}
