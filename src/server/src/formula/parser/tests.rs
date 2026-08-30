use super::*;
use crate::formula::types::FormulaErrorKind;

fn num(v: f64) -> Expr {
    Expr::Num(v)
}
fn r(path: &[&str]) -> Expr {
    Expr::Ref(path.iter().map(|s| s.to_string()).collect())
}
fn bin(op: BinOp, l: Expr, rt: Expr) -> Expr {
    Expr::Bin {
        op,
        left: Box::new(l),
        right: Box::new(rt),
    }
}
fn kind(src: &str) -> FormulaErrorKind {
    parse(src).unwrap_err().error
}
fn detail(src: &str) -> String {
    parse(src).unwrap_err().detail
}
/// `prefix_negs` unary-neg nodes + 1 num + `plus_ones` × (num + bin).
fn node_count_formula(prefix_negs: usize, plus_ones: usize) -> String {
    format!("{}1{}", "-".repeat(prefix_negs), "+1".repeat(plus_ones))
}

#[test]
fn parses_precedence_unary_minus_over_mul_over_add() {
    assert_eq!(
        parse("1 + 2 * -dex").unwrap(),
        bin(
            BinOp::Add,
            num(1.0),
            bin(BinOp::Mul, num(2.0), Expr::Neg(Box::new(r(&["dex"]))))
        )
    );
}

#[test]
fn parses_dotted_refs_and_calls() {
    assert_eq!(
        parse("min(hp.max, floor(parent.str / 2))").unwrap(),
        Expr::Call {
            func: FnName::Min,
            args: vec![
                r(&["hp", "max"]),
                Expr::Call {
                    func: FnName::Floor,
                    args: vec![bin(BinOp::Div, r(&["parent", "str"]), num(2.0))],
                },
            ],
        }
    );
}

#[test]
fn a_word_before_paren_must_be_a_known_function() {
    assert_eq!(detail("dex(1)"), "unknown function 'dex' at position 0");
}

#[test]
fn enforces_arity() {
    assert_eq!(
        detail("floor(1, 2)"),
        "'floor' requires exactly 1 argument at position 0"
    );
    assert_eq!(
        detail("min()"),
        "'min' requires at least 1 argument at position 0"
    );
    assert_eq!(
        detail("ceil()"),
        "'ceil' requires exactly 1 argument at position 0"
    );
    assert_eq!(
        detail("round(1,2)"),
        "'round' requires exactly 1 argument at position 0"
    );
}

#[test]
fn caps_ast_size_and_nesting_depth() {
    assert_eq!(
        detail(&format!("1{}", "+1".repeat(200))),
        "formula exceeds 256 AST nodes"
    );
    assert_eq!(
        detail(&format!("{}1{}", "(".repeat(40), ")".repeat(40))),
        "formula exceeds max nesting depth of 32"
    );
}

#[test]
fn rejects_trailing_garbage() {
    assert_eq!(detail("1 + 2 3"), "unexpected trailing input at position 6");
}

#[test]
fn depth_cap_is_exactly_32_and_uniform_across_constructs() {
    let parens = |n: usize| format!("{}1{}", "(".repeat(n), ")".repeat(n));
    assert_eq!(parse(&parens(32)).unwrap(), num(1.0));
    assert_eq!(kind(&parens(33)), FormulaErrorKind::Cap);
    let calls = |n: usize| format!("{}1{}", "floor(".repeat(n), ")".repeat(n));
    assert!(matches!(
        parse(&calls(32)).unwrap(),
        Expr::Call {
            func: FnName::Floor,
            ..
        }
    ));
    assert_eq!(kind(&calls(33)), FormulaErrorKind::Cap);
    let negs = |n: usize| format!("{}1", "-".repeat(n));
    assert!(matches!(parse(&negs(32)).unwrap(), Expr::Neg(_)));
    assert_eq!(kind(&negs(33)), FormulaErrorKind::Cap);
}

#[test]
fn sad_path_battery() {
    assert_eq!(detail("(1+2"), "expected ')' at position 4");
    assert_eq!(detail("min(1"), "expected ')' at position 5");
    assert_eq!(detail("hp."), "expected identifier after '.' at position 3");
    assert_eq!(detail(""), "unexpected end of formula");
    assert_eq!(detail("   "), "unexpected end of formula");
    assert_eq!(detail("floor(1,)"), "unexpected token at position 8");
    assert_eq!(
        parse("floor + 1").unwrap(),
        bin(BinOp::Add, r(&["floor"]), num(1.0))
    );
    assert_eq!(
        detail("hp.max(1)"),
        "unexpected trailing input at position 6"
    );
}

#[test]
fn exact_max_ast_nodes_boundary_256_parses_257_caps() {
    let at256 = node_count_formula(1, 127);
    let at257 = node_count_formula(2, 127);
    assert!(at257.len() <= 512);
    assert!(parse(&at256).is_ok());
    assert_eq!(kind(&at257), FormulaErrorKind::Cap);
}

#[test]
fn exponent_notation_is_not_a_number() {
    assert_eq!(detail("1e999"), "unexpected trailing input at position 1");
}

#[test]
fn a_ref_is_one_node_regardless_of_segment_count() {
    // Far more segments than MAX_AST_NODES would allow as separate nodes; a
    // ref is one node however long its path.
    let src = (0..120).map(|_| "a").collect::<Vec<_>>().join(".");
    assert!(matches!(parse(&src).unwrap(), Expr::Ref(p) if p.len() == 120));
}
