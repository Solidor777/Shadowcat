use std::collections::BTreeMap;

use serde::Deserialize;

use crate::formula::types::{FormulaError, FormulaErrorKind, FormulaValue};
use crate::formula::{evaluate, parse, resolve_all, Expr};

/// Every dotted ref in `expr`, in source order, first occurrence only. The
/// client harness fetches a node's dependencies through `get` before calling
/// `evaluate` (its `evaluate` guards the resolver callback with a try/catch
/// that would swallow `resolveAll`'s restart signal); this side does the same
/// so both request an identical dependency set per node.
fn collect_refs(expr: &Expr, out: &mut Vec<String>) {
    match expr {
        Expr::Num(_) => {}
        Expr::Ref(path) => {
            let key = path.join(".");
            if !out.contains(&key) {
                out.push(key);
            }
        }
        Expr::Neg(inner) => collect_refs(inner, out),
        Expr::Bin { left, right, .. } => {
            collect_refs(left, out);
            collect_refs(right, out);
        }
        Expr::Call { args, .. } => {
            for a in args {
                collect_refs(a, out);
            }
        }
    }
}

/// A corpus value: a number or an error object.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Expected {
    Num(f64),
    Failure(FormulaError),
}

impl From<Expected> for FormulaValue {
    fn from(e: Expected) -> Self {
        match e {
            Expected::Num(n) => Ok(n),
            Expected::Failure(err) => Err(err),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ExpressionCase {
    name: String,
    source: String,
    #[serde(default)]
    refs: BTreeMap<String, Expected>,
    expect: Expected,
}

#[derive(Debug, Deserialize)]
struct GraphCase {
    name: String,
    nodes: BTreeMap<String, String>,
    roots: Vec<String>,
    expect: BTreeMap<String, Expected>,
}

#[derive(Debug, Deserialize)]
struct Corpus {
    expressions: Vec<ExpressionCase>,
    graphs: Vec<GraphCase>,
}

/// The shared corpus, read from the client package so both suites see one file.
fn corpus() -> Corpus {
    serde_json::from_str(include_str!(
        "../../../../client/formula/src/__fixtures__/conformance.json"
    ))
    .expect("conformance.json parses")
}

fn unknown_ref(key: &str) -> FormulaValue {
    Err(FormulaError::new(
        FormulaErrorKind::UnknownRef,
        format!("unknown reference '{key}'"),
    ))
}

#[test]
fn case_names_are_unique() {
    let c = corpus();
    let mut names: Vec<&str> = c
        .expressions
        .iter()
        .map(|e| e.name.as_str())
        .chain(c.graphs.iter().map(|g| g.name.as_str()))
        .collect();
    let total = names.len();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), total, "duplicate corpus case names");
}

#[test]
fn every_expression_case_matches() {
    let c = corpus();
    assert!(!c.expressions.is_empty());
    for case in c.expressions {
        let refs: BTreeMap<String, FormulaValue> =
            case.refs.into_iter().map(|(k, v)| (k, v.into())).collect();
        let expected: FormulaValue = case.expect.into();
        let actual = match parse(&case.source) {
            Err(e) => Err(e),
            Ok(ast) => evaluate(&ast, &|path: &[String]| {
                let key = path.join(".");
                refs.get(&key).cloned().unwrap_or_else(|| unknown_ref(&key))
            }),
        };
        assert_eq!(actual, expected, "expression case '{}'", case.name);
    }
}

#[test]
fn every_graph_case_matches() {
    let c = corpus();
    assert!(!c.graphs.is_empty());
    for case in c.graphs {
        let nodes = case.nodes;
        let result = resolve_all(&case.roots, |key, get| {
            let Some(source) = nodes.get(key) else {
                return unknown_ref(key);
            };
            match parse(source) {
                Err(e) => Err(e),
                Ok(ast) => {
                    let mut deps = Vec::new();
                    collect_refs(&ast, &mut deps);
                    let fetched: BTreeMap<String, FormulaValue> =
                        deps.into_iter().map(|d| (d.clone(), get(&d))).collect();
                    evaluate(&ast, &|path: &[String]| {
                        let key = path.join(".");
                        fetched
                            .get(&key)
                            .cloned()
                            .unwrap_or_else(|| unknown_ref(&key))
                    })
                }
            }
        });
        for (key, expected) in case.expect {
            let expected: FormulaValue = expected.into();
            assert_eq!(
                result.get(&key),
                Some(&expected),
                "graph case '{}' key '{key}'",
                case.name
            );
        }
    }
}
