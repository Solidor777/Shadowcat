use super::*;

#[test]
fn error_kinds_serialize_to_the_kebab_case_tags_the_client_uses() {
    let tags: Vec<String> = [
        FormulaErrorKind::Parse,
        FormulaErrorKind::UnknownRef,
        FormulaErrorKind::Type,
        FormulaErrorKind::DivZero,
        FormulaErrorKind::NonFinite,
        FormulaErrorKind::Cycle,
        FormulaErrorKind::Cap,
        FormulaErrorKind::RefError,
        FormulaErrorKind::ResolverError,
    ]
    .iter()
    .map(|k| serde_json::to_string(k).unwrap())
    .collect();
    assert_eq!(
        tags,
        [
            "\"parse\"",
            "\"unknown-ref\"",
            "\"type\"",
            "\"div-zero\"",
            "\"non-finite\"",
            "\"cycle\"",
            "\"cap\"",
            "\"ref-error\"",
            "\"resolver-error\"",
        ]
    );
}

#[test]
fn formula_error_round_trips_through_json_with_the_client_field_names() {
    let e = FormulaError::new(FormulaErrorKind::Cap, "formula exceeds 512 characters");
    let json = serde_json::to_value(&e).unwrap();
    assert_eq!(
        json,
        serde_json::json!({ "error": "cap", "detail": "formula exceeds 512 characters" })
    );
    let back: FormulaError = serde_json::from_value(json).unwrap();
    assert_eq!(back, e);
}

#[test]
fn caps_match_the_client_constants() {
    assert_eq!(MAX_FORMULA_LENGTH, 512);
    assert_eq!(MAX_AST_NODES, 256);
    assert_eq!(MAX_PARSE_DEPTH, 32);
    assert_eq!(MAX_GRAPH_VISITS, 2048);
}
