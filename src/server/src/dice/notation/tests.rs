use super::*;

fn no_debug_artifacts(s: &str) -> bool {
    !s.contains('{') && !s.contains("Some(") && !s.contains("None")
}

#[test]
fn every_parse_error_variant_displays_without_debug_artifacts() {
    // Iterate every variant explicitly (including realistic `Unexpected`/
    // `Trailing` payloads, since those two wrap free text built at their
    // construction sites via `Token`'s Display, not this impl).
    let variants: Vec<ParseError> = vec![
        ParseError::Empty,
        ParseError::Unexpected("expected a number, found the number 5".to_string()),
        ParseError::Trailing("the number 5".to_string()),
        ParseError::InvalidDieSides(0),
        ParseError::DuplicateSuccessRule,
        ParseError::DuplicateExpertise,
        ParseError::DuplicateRequiredSuccesses,
        ParseError::EmptyLabel,
        ParseError::UnterminatedLabel,
        ParseError::InvalidLabelChar,
        ParseError::DuplicateCritSuccess,
        ParseError::DuplicateCritFail,
    ];
    assert_eq!(
        variants.len(),
        12,
        "update this test if a ParseError variant is added or removed"
    );
    for v in variants {
        let rendered = v.to_string();
        assert!(
            no_debug_artifacts(&rendered),
            "variant {v:?} rendered debug artifacts: {rendered:?}"
        );
    }
}

#[test]
fn real_parse_failures_render_without_debug_artifacts() {
    let inputs = [
        "4d6 @ 2",                  // lexer: unexpected character
        "2d6 2d6",                  // trailing input
        "4d",                       // expect_int: missing sides
        "(1d4+1",                   // expect ')'
        "4d6xyz",                   // unknown modifier
        "6d6r",                     // cmp_target_required
        "café",                     // non-ASCII
        "999999999999999999999999", // invalid number literal
        "min(3)",                   // fn_call: wrong arity
        "foo(3)",                   // fn_call: unknown function name
        "4d6cs>=4rs2rs3",           // duplicate rs
        "4d6cs>=4xs5xs6",           // duplicate xs
    ];
    for input in inputs {
        let err = parse(input, ParseContext::default())
            .expect_err("expected a parse error for malformed input");
        let rendered = err.to_string();
        assert!(
            no_debug_artifacts(&rendered),
            "input {input:?} produced debug artifacts: {rendered:?}"
        );
    }
}
