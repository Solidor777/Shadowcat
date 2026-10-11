//! Baseline resolution of the throughput ladder.

use std::net::SocketAddr;

use super::*;

fn addr() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 0))
}

fn ladder() -> Vec<Rung> {
    vec![
        Rung::bare("root", addr()),
        Rung::bare("child", addr()).over("root"),
        Rung::bare("grandchild", addr()).over("child"),
        Rung::bare("sibling", addr()).over("root"),
    ]
}

#[test]
fn a_root_rung_has_no_baseline() {
    assert_eq!(baseline_index(&ladder(), 0), None);
}

#[test]
fn a_named_baseline_resolves_to_its_index() {
    let rungs = ladder();
    assert_eq!(baseline_index(&rungs, 1), Some(0));
    assert_eq!(baseline_index(&rungs, 2), Some(1));
    assert_eq!(baseline_index(&rungs, 3), Some(0));
}

#[test]
#[should_panic(expected = "no rung named")]
fn an_unknown_baseline_name_panics() {
    let rungs = vec![
        Rung::bare("root", addr()),
        Rung::bare("child", addr()).over("roott"),
    ];
    baseline_index(&rungs, 1);
}

#[test]
#[should_panic(expected = "does not precede")]
fn a_baseline_that_comes_later_in_the_ladder_panics() {
    let rungs = vec![
        Rung::bare("first", addr()).over("second"),
        Rung::bare("second", addr()),
    ];
    baseline_index(&rungs, 0);
}

#[test]
fn an_absent_worker_count_is_the_default() {
    assert_eq!(parse_worker_count(None), Ok(DEFAULT_WORKERS));
    assert_eq!(DEFAULT_WORKERS, 2);
}

#[test]
fn an_explicit_worker_count_is_used() {
    assert_eq!(parse_worker_count(Some("4")), Ok(4));
    assert_eq!(parse_worker_count(Some("1")), Ok(1));
}

#[test]
fn a_zero_or_non_numeric_worker_count_is_rejected() {
    for bad in ["0", "four", "", "-1", "2.5"] {
        assert!(parse_worker_count(Some(bad)).is_err(), "{bad:?} accepted");
    }
}

#[test]
fn a_selection_keeps_ladder_order_and_reroutes_a_missing_baseline_to_its_nearest_selected_ancestor()
{
    let picked = select_rungs(ladder(), &["grandchild", "root"]);
    let names: Vec<_> = picked.iter().map(|r| r.name).collect();
    assert_eq!(names, ["root", "grandchild"]);
    assert_eq!(picked[1].baseline, Some("root"));
}

#[test]
fn a_selected_rung_whose_baseline_is_selected_keeps_it() {
    let picked = select_rungs(ladder(), &["root", "child", "grandchild"]);
    assert_eq!(picked[2].baseline, Some("child"));
}

#[test]
fn a_selected_rung_with_no_selected_ancestor_has_no_baseline() {
    let picked = select_rungs(ladder(), &["grandchild"]);
    assert_eq!(picked[0].baseline, None);
}

#[test]
#[should_panic(expected = "no rung named")]
fn selecting_an_unknown_rung_panics() {
    select_rungs(ladder(), &["roott"]);
}
