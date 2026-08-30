use super::*;
use crate::data::document::tests::sample_doc;
use crate::formula::evaluate::Resolve;
use crate::formula::types::{FormulaError, FormulaErrorKind};
use serde_json::json;

fn doc_with_system(system: serde_json::Value) -> crate::data::document::Document {
    let mut d = sample_doc();
    d.system = system;
    d
}
fn path(p: &[&str]) -> Vec<String> {
    p.iter().map(|s| s.to_string()).collect()
}

#[test]
fn reads_a_number_leaf_by_literal_dotted_path() {
    let d = doc_with_system(json!({ "speed": 30, "stats": { "hp": { "final": 12.5 } } }));
    let r = SystemLeafResolver::new(&d);
    assert_eq!(r.resolve(&path(&["speed"])), Ok(30.0));
    assert_eq!(r.resolve(&path(&["stats", "hp", "final"])), Ok(12.5));
}

#[test]
fn a_missing_leaf_at_any_depth_is_unknown_ref() {
    let d = doc_with_system(json!({ "stats": { "hp": { "final": 12 } } }));
    let r = SystemLeafResolver::new(&d);
    let err = |p: &str| {
        Err(FormulaError::new(
            FormulaErrorKind::UnknownRef,
            format!("unknown reference '{p}'"),
        ))
    };
    assert_eq!(r.resolve(&path(&["speed"])), err("speed"));
    assert_eq!(
        r.resolve(&path(&["stats", "str", "final"])),
        err("stats.str.final")
    );
    // Traversing THROUGH a non-object is a miss, not a type error.
    assert_eq!(
        r.resolve(&path(&["stats", "hp", "final", "x"])),
        err("stats.hp.final.x")
    );
}

#[test]
fn a_present_non_number_leaf_is_a_type_error() {
    let d = doc_with_system(json!({
        "name": "Ka", "flag": true, "obj": { "a": 1 }, "arr": [1], "nil": null
    }));
    let r = SystemLeafResolver::new(&d);
    for key in ["name", "flag", "obj", "arr", "nil"] {
        assert_eq!(
            r.resolve(&path(&[key])),
            Err(FormulaError::new(
                FormulaErrorKind::Type,
                format!("'{key}' is not a number")
            )),
            "{key}"
        );
    }
}

#[test]
fn an_integer_leaf_beyond_f64_precision_still_resolves_finite() {
    let d = doc_with_system(json!({ "big": 9007199254740993u64 }));
    let r = SystemLeafResolver::new(&d);
    assert!(r.resolve(&path(&["big"])).unwrap().is_finite());
}
