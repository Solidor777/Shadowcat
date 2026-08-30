use super::*;
use crate::formula::types::{FormulaError, FormulaErrorKind};
use std::cell::Cell;

fn unknown(k: &str) -> FormulaValue {
    Err(FormulaError::new(
        FormulaErrorKind::UnknownRef,
        k.to_string(),
    ))
}
fn keys(ks: &[&str]) -> Vec<String> {
    ks.iter().map(|s| s.to_string()).collect()
}
/// `n0` → 0; `n{i}` → get(`n{i-1}`): a root `n{K-1}` discovers exactly K keys.
fn countdown(k: &str, get: &mut dyn FnMut(&str) -> FormulaValue) -> FormulaValue {
    let n: usize = k[1..].parse().unwrap();
    if n == 0 {
        Ok(0.0)
    } else {
        get(&format!("n{}", n - 1))
    }
}
fn chain(k: &str, get: &mut dyn FnMut(&str) -> FormulaValue) -> FormulaValue {
    match k {
        "base" => Ok(2.0),
        "a" => get("base").map(|b| b + 1.0),
        "b" => get("a").map(|a| a * 2.0),
        other => unknown(other),
    }
}

#[test]
fn resolves_through_dependencies() {
    let r = resolve_all(&keys(&["b", "a", "base"]), chain);
    assert_eq!(r["b"], Ok(6.0));
    assert_eq!(r["a"], Ok(3.0));
}

#[test]
fn is_order_independent() {
    let r1 = resolve_all(&keys(&["base", "a", "b"]), chain);
    let r2 = resolve_all(&keys(&["b", "base", "a"]), chain);
    assert_eq!(r1, r2);
}

#[test]
fn marks_every_cycle_participant_errored_never_hangs() {
    let cyc = |k: &str, get: &mut dyn FnMut(&str) -> FormulaValue| match k {
        "x" => get("y"),
        "y" => get("x"),
        other => unknown(other),
    };
    let r = resolve_all(&keys(&["x", "y"]), cyc);
    assert_eq!(r["x"].as_ref().unwrap_err().error, FormulaErrorKind::Cycle);
    assert_eq!(r["y"].as_ref().unwrap_err().error, FormulaErrorKind::Cycle);
}

#[test]
fn reports_the_lexicographically_smallest_cycle_member_regardless_of_entry_order() {
    let cyc = |k: &str, get: &mut dyn FnMut(&str) -> FormulaValue| match k {
        "a" => get("b"),
        "b" => get("c"),
        "c" => get("a"),
        other => unknown(other),
    };
    let forward = resolve_all(&keys(&["a", "b", "c"]), cyc);
    let reverse = resolve_all(&keys(&["c", "b", "a"]), cyc);
    assert_eq!(forward, reverse);
    let expected = Err(FormulaError::new(
        FormulaErrorKind::Cycle,
        "reference cycle involving 'a'",
    ));
    assert_eq!(forward["a"], expected);
    assert_eq!(reverse["c"], expected);
}

#[test]
fn a_short_circuitable_cycle_resolves_identically_regardless_of_entry_order() {
    let sc = |k: &str, get: &mut dyn FnMut(&str) -> FormulaValue| match k {
        "s1" => {
            let _ = get("s2");
            unknown("u missing")
        }
        "s2" => get("s1"),
        other => unknown(other),
    };
    let a = resolve_all(&keys(&["s1", "s2"]), sc);
    let b = resolve_all(&keys(&["s2", "s1"]), sc);
    assert_eq!(a["s1"], b["s1"]);
    assert_eq!(a["s2"], b["s2"]);
}

#[test]
fn caps_total_visits() {
    let r = resolve_all(&keys(&["n5000"]), countdown);
    assert_eq!(
        r["n5000"],
        Err(FormulaError::new(
            FormulaErrorKind::Cap,
            "graph resolution exceeded visit cap"
        ))
    );
}

#[test]
fn exact_visit_cap_boundary_2048_keys_resolve_2049_cap() {
    assert_eq!(resolve_all(&keys(&["n2047"]), countdown)["n2047"], Ok(0.0));
    assert_eq!(
        resolve_all(&keys(&["n2048"]), countdown)["n2048"]
            .as_ref()
            .unwrap_err()
            .error,
        FormulaErrorKind::Cap
    );
}

#[test]
fn a_long_non_cyclic_chain_resolves_without_growing_the_call_stack() {
    // 2001 keys, under the cap: the driver iterates on an explicit heap stack,
    // so this runs on the default test thread stack.
    assert_eq!(resolve_all(&keys(&["n2000"]), countdown)["n2000"], Ok(0.0));
}

#[test]
fn a_shared_dependency_is_evaluated_exactly_once_across_two_roots() {
    let d_calls = Cell::new(0u32);
    let diamond = |k: &str, get: &mut dyn FnMut(&str) -> FormulaValue| match k {
        "d" => {
            d_calls.set(d_calls.get() + 1);
            Ok(4.0)
        }
        "a" => get("d").map(|d| d + 1.0),
        "c" => get("d").map(|d| d + 2.0),
        other => unknown(other),
    };
    let r = resolve_all(&keys(&["a", "c"]), diamond);
    assert_eq!(r["a"], Ok(5.0));
    assert_eq!(r["c"], Ok(6.0));
    assert_eq!(d_calls.get(), 1);
}

#[test]
fn the_restart_placeholder_never_reaches_a_caller_even_when_a_node_returns_it_verbatim() {
    // A node that hands back whatever `get` returned — including the internal
    // placeholder for a not-yet-resolved dependency — still resolves to the
    // dependency's real value, and no `RefError` appears anywhere in the result.
    let passthrough = |k: &str, get: &mut dyn FnMut(&str) -> FormulaValue| match k {
        "leaf" => Ok(7.0),
        "a" => get("leaf"),
        "b" => get("a"),
        other => unknown(other),
    };
    let r = resolve_all(&keys(&["b"]), passthrough);
    assert_eq!(r["b"], Ok(7.0));
    assert!(r
        .values()
        .all(|v| !matches!(v, Err(e) if e.error == FormulaErrorKind::RefError)));
}

#[test]
fn a_node_returning_a_non_finite_number_is_gated_to_non_finite() {
    let r = resolve_all(&keys(&["x"]), |_, _| Ok(f64::NAN));
    assert_eq!(
        r["x"].as_ref().unwrap_err().error,
        FormulaErrorKind::NonFinite
    );
}
