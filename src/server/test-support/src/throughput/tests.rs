//! Baseline resolution and lazy construction of the throughput ladder.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use super::*;

fn addr() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 0))
}

/// A spec list whose constructors record their name in `started`; the baselines form a tree.
fn recording_specs(started: &Arc<Mutex<Vec<&'static str>>>) -> Vec<RungSpec> {
    let spec = |name: &'static str, baseline: Option<&'static str>| {
        let started = Arc::clone(started);
        RungSpec::new(name, baseline, move || async move {
            started.lock().unwrap().push(name);
            RungServer::bare(addr())
        })
    };
    vec![
        spec("root", None),
        spec("child", Some("root")),
        spec("grandchild", Some("child")),
        spec("sibling", Some("root")),
    ]
}

async fn built(names: Option<&[&str]>) -> Vec<Rung> {
    build_ladder(recording_specs(&Arc::default()), names).await
}

#[tokio::test]
async fn a_root_rung_has_no_baseline() {
    assert_eq!(baseline_index(&built(None).await, 0), None);
}

#[tokio::test]
async fn a_named_baseline_resolves_to_its_index() {
    let rungs = built(None).await;
    assert_eq!(baseline_index(&rungs, 1), Some(0));
    assert_eq!(baseline_index(&rungs, 2), Some(1));
    assert_eq!(baseline_index(&rungs, 3), Some(0));
}

#[tokio::test]
#[should_panic(expected = "no rung named \"roott\" to compare with")]
async fn an_unknown_baseline_name_panics() {
    let specs = vec![
        RungSpec::new("root", None, || async { RungServer::bare(addr()) }),
        RungSpec::new("child", Some("roott"), || async {
            RungServer::bare(addr())
        }),
    ];
    build_ladder(specs, None).await;
}

#[tokio::test]
#[should_panic(expected = "does not precede")]
async fn a_baseline_that_comes_later_in_the_ladder_panics() {
    let specs = vec![
        RungSpec::new("first", Some("second"), || async {
            RungServer::bare(addr())
        }),
        RungSpec::new("second", None, || async { RungServer::bare(addr()) }),
    ];
    build_ladder(specs, None).await;
}

#[tokio::test]
async fn a_mistyped_baseline_fails_before_any_constructor_runs_even_when_its_rung_is_unselected() {
    let started = Arc::new(Mutex::new(Vec::new()));
    let mut specs = recording_specs(&started);
    specs.push(RungSpec::new("stray", Some("roott"), || async {
        RungServer::bare(addr())
    }));
    let outcome = tokio::spawn(async move { build_ladder(specs, Some(&["root"])).await }).await;
    assert!(outcome
        .err()
        .expect("a mistyped baseline was accepted")
        .is_panic());
    assert!(started.lock().unwrap().is_empty());
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

#[tokio::test]
async fn a_selection_keeps_ladder_order_and_reroutes_a_missing_baseline_to_its_nearest_selected_ancestor(
) {
    let picked = built(Some(&["grandchild", "root"])).await;
    let names: Vec<_> = picked.iter().map(|r| r.name).collect();
    assert_eq!(names, ["root", "grandchild"]);
    assert_eq!(picked[1].baseline, Some("root"));
}

#[tokio::test]
async fn a_selected_rung_whose_baseline_is_selected_keeps_it() {
    let picked = built(Some(&["root", "child", "grandchild"])).await;
    assert_eq!(picked[2].baseline, Some("child"));
}

#[tokio::test]
async fn a_selected_rung_with_no_selected_ancestor_has_no_baseline() {
    let picked = built(Some(&["grandchild"])).await;
    assert_eq!(picked[0].baseline, None);
}

#[tokio::test]
#[should_panic(expected = "no rung named")]
async fn selecting_an_unknown_rung_panics() {
    built(Some(&["roott"])).await;
}

#[tokio::test]
async fn only_the_selected_rungs_are_started_and_an_unselected_baseline_is_never_built() {
    let started = Arc::new(Mutex::new(Vec::new()));
    let rungs = build_ladder(recording_specs(&started), Some(&["grandchild", "root"])).await;
    assert_eq!(*started.lock().unwrap(), ["root", "grandchild"]);
    let names: Vec<_> = rungs.iter().map(|r| r.name).collect();
    assert_eq!(names, ["root", "grandchild"]);
    assert_eq!(rungs[1].baseline, Some("root"));
}

#[tokio::test]
async fn no_selection_starts_every_rung_in_ladder_order() {
    let started = Arc::new(Mutex::new(Vec::new()));
    let rungs = build_ladder(recording_specs(&started), None).await;
    assert_eq!(
        *started.lock().unwrap(),
        ["root", "child", "grandchild", "sibling"]
    );
    assert_eq!(rungs[2].baseline, Some("child"));
}

#[tokio::test]
async fn an_unknown_rung_name_fails_before_any_constructor_runs() {
    let started = Arc::new(Mutex::new(Vec::new()));
    let specs = recording_specs(&started);
    let outcome =
        tokio::spawn(async move { build_ladder(specs, Some(&["root", "roott"])).await }).await;
    let join_error = outcome
        .err()
        .expect("an unknown rung name did not stop the build");
    assert!(join_error.is_panic(), "{join_error:?}");
    assert!(
        started.lock().unwrap().is_empty(),
        "a constructor ran before the selection was validated"
    );
}
