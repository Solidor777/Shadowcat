#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

use std::cell::RefCell;

use proptest::prelude::*;

use super::types::{FormulaError, FormulaErrorKind, FormulaValue};
use super::{evaluate, parse, resolve_all, resolve_notation_template};

/// Source strings drawn from the language's own alphabet so a useful fraction
/// parses, plus arbitrary junk so the error paths are exercised too.
fn source() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => "[0-9a-c_ .+*/%(),-]{0,80}",
        1 => "[a-z]+(\\.[a-z]+){0,3}",
        1 => "\\PC{0,40}",
    ]
}

/// A resolver outcome: a finite number, a non-finite number, or an error.
fn resolver_value() -> impl Strategy<Value = FormulaValue> {
    prop_oneof![
        any::<f64>().prop_map(Ok),
        Just(Ok(f64::INFINITY)),
        Just(Ok(f64::NAN)),
        Just(Err(FormulaError::new(
            FormulaErrorKind::UnknownRef,
            "unknown reference 'x'"
        ))),
    ]
}

/// Template sources: the notation alphabet including brackets and dots (so
/// label spans, keyword runs and dotted references all occur), plus arbitrary
/// junk for the error and literal-fallback paths.
fn template_source() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => "[0-9a-z_ .+*/%(),\\[\\]-]{0,60}",
        1 => "[a-z]+(\\.[a-z]+){0,3}",
        1 => "\\PC{0,40}",
    ]
}

proptest! {
    #[test]
    fn parse_and_evaluate_never_panic_and_never_return_non_finite(src in source(), v in resolver_value()) {
        if let Ok(ast) = parse(&src) {
            let r = evaluate(&ast, &|_: &[String]| v.clone());
            if let Ok(n) = r {
                prop_assert!(n.is_finite());
            }
        }
    }

    #[test]
    fn resolve_all_never_panics_over_random_node_graphs(
        nodes in prop::collection::btree_map("[a-e]", source(), 0..8),
        roots in prop::collection::vec("[a-f]", 0..6),
    ) {
        let r = resolve_all(&roots, |key, get| {
            let Some(src) = nodes.get(key) else {
                return Err(FormulaError::new(FormulaErrorKind::UnknownRef, format!("unknown reference '{key}'")));
            };
            match parse(src) {
                Err(e) => Err(e),
                Ok(ast) => {
                    // `get` is `FnMut`; the evaluator takes a `Fn` resolver.
                    let get = RefCell::new(get);
                    evaluate(&ast, &|path: &[String]| (*get.borrow_mut())(&path.join(".")))
                }
            }
        });
        for n in r.values().flatten() {
            prop_assert!(n.is_finite());
        }
    }

    #[test]
    fn resolve_notation_template_never_panics(
        src in template_source(),
        v in resolver_value(),
    ) {
        // The assertion is the call itself: the crate's never-panics rule
        // covers the template scan on every input and every resolver outcome.
        let _ = resolve_notation_template(&src, &|_: &[String]| v.clone());
    }
}
