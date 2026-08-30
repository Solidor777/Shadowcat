use super::*;
use crate::formula::parser::parse;
use crate::formula::types::{FormulaError, FormulaErrorKind};
use std::cell::Cell;
use std::collections::HashMap;

fn env(vals: &[(&str, FormulaValue)]) -> impl Fn(&[String]) -> FormulaValue {
    let map: HashMap<String, FormulaValue> = vals
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    move |path: &[String]| {
        let key = path.join(".");
        map.get(&key).cloned().unwrap_or_else(|| {
            Err(FormulaError::new(
                FormulaErrorKind::UnknownRef,
                format!("unknown reference '{key}'"),
            ))
        })
    }
}

fn eval(src: &str, vals: &[(&str, FormulaValue)]) -> FormulaValue {
    evaluate(&parse(src).unwrap(), &env(vals))
}

#[test]
fn computes_arithmetic_with_resolver_supplied_refs() {
    assert_eq!(
        eval(
            "floor(parent.str / 2) + dex",
            &[("parent.str", Ok(15.0)), ("dex", Ok(3.0))]
        ),
        Ok(10.0)
    );
}

#[test]
fn float_division_explicit_rounding_only() {
    assert_eq!(eval("7 / 2", &[]), Ok(3.5));
    assert_eq!(eval("round(7 / 2)", &[]), Ok(4.0));
}

#[test]
fn division_and_mod_by_zero_are_error_values() {
    assert_eq!(
        eval("1 / dex", &[("dex", Ok(0.0))]),
        Err(FormulaError::new(
            FormulaErrorKind::DivZero,
            "division by zero ('/')"
        ))
    );
    assert_eq!(
        eval("1 % 0", &[]),
        Err(FormulaError::new(
            FormulaErrorKind::DivZero,
            "division by zero ('%')"
        ))
    );
}

#[test]
fn propagates_resolver_errors_unchanged() {
    let cyc = Err(FormulaError::new(
        FormulaErrorKind::Cycle,
        "dex -> str -> dex",
    ));
    assert_eq!(eval("dex + 1", &[("dex", cyc.clone())]), cyc);
}

#[test]
fn unknown_refs_are_error_values() {
    assert_eq!(
        eval("ghost + 1", &[]),
        Err(FormulaError::new(
            FormulaErrorKind::UnknownRef,
            "unknown reference 'ghost'"
        ))
    );
}

#[test]
fn non_finite_results_are_error_values() {
    let big = format!("1{}", "0".repeat(160));
    assert_eq!(
        eval(&format!("{big} * {big}"), &[]),
        Err(FormulaError::new(
            FormulaErrorKind::NonFinite,
            "arithmetic result is not finite (Infinity)"
        ))
    );
}

#[test]
fn min_max_are_n_ary() {
    assert_eq!(eval("max(1, dex, 2)", &[("dex", Ok(9.0))]), Ok(9.0));
    assert_eq!(eval("min(3)", &[]), Ok(3.0));
}

#[test]
fn round_ties_toward_positive_infinity_like_js() {
    assert_eq!(eval("round(-2.5)", &[]), Ok(-2.0));
    assert_eq!(eval("round(2.5)", &[]), Ok(3.0));
    assert_eq!(eval("round(-3.5)", &[]), Ok(-3.0));
    assert_eq!(js_round(0.49999999999999994), 0.0);
    // JS yields -0 for a negative input that rounds to zero; the sign survives.
    assert!(js_round(-0.4).is_sign_negative());
}

#[test]
fn remainder_is_truncated_not_floored() {
    assert_eq!(eval("-7 % 2", &[]), Ok(-1.0));
    assert_eq!(eval("7 % -2", &[]), Ok(1.0));
}

#[test]
fn both_operands_erroring_short_circuits_on_the_left() {
    let right_calls = Cell::new(0u32);
    let resolve = |path: &[String]| -> FormulaValue {
        if path.join(".") == "dex" {
            return Err(FormulaError::new(
                FormulaErrorKind::UnknownRef,
                "unknown reference 'dex'",
            ));
        }
        right_calls.set(right_calls.get() + 1);
        Err(FormulaError::new(
            FormulaErrorKind::UnknownRef,
            "unknown reference 'str'",
        ))
    };
    let r = evaluate(&parse("dex + str").unwrap(), &resolve);
    assert_eq!(
        r,
        Err(FormulaError::new(
            FormulaErrorKind::UnknownRef,
            "unknown reference 'dex'"
        ))
    );
    assert_eq!(right_calls.get(), 0);
}

#[test]
fn a_resolver_returning_a_non_finite_number_is_gated_to_non_finite() {
    let resolve = |_: &[String]| -> FormulaValue { Ok(f64::INFINITY) };
    assert_eq!(
        evaluate(&parse("dex").unwrap(), &resolve),
        Err(FormulaError::new(
            FormulaErrorKind::NonFinite,
            "arithmetic result is not finite (Infinity)"
        ))
    );
}

#[test]
fn a_hand_built_call_with_missing_arguments_yields_non_finite_not_a_panic() {
    let floor_none = Expr::Call {
        func: FnName::Floor,
        args: vec![],
    };
    assert_eq!(
        evaluate(&floor_none, &env(&[])).unwrap_err().error,
        FormulaErrorKind::NonFinite
    );
    let min_none = Expr::Call {
        func: FnName::Min,
        args: vec![],
    };
    assert_eq!(
        evaluate(&min_none, &env(&[])).unwrap_err().error,
        FormulaErrorKind::NonFinite
    );
}
