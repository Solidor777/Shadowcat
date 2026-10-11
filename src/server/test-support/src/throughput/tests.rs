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
